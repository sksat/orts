//! Minimal RRD (Rerun Recording Data) decoder for browser-side use.
//!
//! Decodes .rrd files into orbital state vectors + metadata.
//! Designed to be compiled to WASM for use in the viewer's Web Worker.
//! Does NOT compute Keplerian elements — that is done by arika WASM.

use std::collections::BTreeMap;
use std::io::Read;

use re_chunk::Chunk;
use re_log_encoding::DecoderApp;
use re_log_types::LogMsg;

#[cfg(feature = "wasm")]
pub mod wasm;

/// Simulation metadata extracted from RRD meta/ entities.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RrdMetadata {
    pub epoch_jd: Option<f64>,
    pub mu: Option<f64>,
    pub body_radius: Option<f64>,
    pub body_name: Option<String>,
    pub altitude: Option<f64>,
    pub period: Option<f64>,
}

/// A single row of orbital state data.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RrdRow {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub entity_path: Option<String>,
    pub quaternion: Option<[f64; 4]>,
    pub angular_velocity: Option<[f64; 3]>,
}

/// Full decoded RRD data.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedRrd {
    pub metadata: RrdMetadata,
    pub rows: Vec<RrdRow>,
}

/// Decode an RRD stream into orbital data.
///
/// Accepts any `impl Read` — works with both `File` and `Cursor<&[u8]>`.
pub fn decode_rrd(reader: impl Read) -> Result<ParsedRrd, Box<dyn std::error::Error>> {
    let reader = std::io::BufReader::new(reader);

    // Collect f64 scalars: entity_path -> Vec<(time_ns, f64)>
    let mut scalars: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    // Collect metadata scalars
    let mut meta_scalars: BTreeMap<String, f64> = BTreeMap::new();
    // Collect text metadata
    let mut meta_texts: BTreeMap<String, String> = BTreeMap::new();

    for msg in DecoderApp::decode_lazy(reader) {
        let msg = msg?;
        let LogMsg::ArrowMsg(_, arrow_msg) = msg else {
            continue;
        };
        let chunk = Chunk::from_arrow_msg(&arrow_msg)?;
        let entity_path = chunk.entity_path().to_string();
        let n = chunk.num_rows();

        let normalized_path = entity_path.strip_prefix('/').unwrap_or(&entity_path);
        if normalized_path.starts_with("meta/sim/") {
            let entity_path = normalized_path.to_string();
            for comp_id in chunk.components_identifiers() {
                let comp_name = comp_id.as_str();
                if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                    for row_idx in 0..n {
                        let batch = chunk
                            .component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                        if let Some(Ok(scalar_vec)) = batch
                            && let Some(s) = scalar_vec.first()
                        {
                            meta_scalars.insert(entity_path.clone(), s.0.0);
                        }
                    }
                }
                if comp_name.contains("Text") || comp_name.contains("text") {
                    for row_idx in 0..n {
                        let batch = chunk
                            .component_batch::<re_sdk_types::components::Text>(comp_id, row_idx);
                        if let Some(Ok(text_vec)) = batch
                            && let Some(t) = text_vec.first()
                        {
                            meta_texts.insert(entity_path.clone(), t.to_string());
                        }
                    }
                }
            }
            continue;
        }

        let sim_time_col = chunk
            .timelines()
            .iter()
            .find(|(name, _)| name.as_str() == "sim_time");
        let times: Vec<i64> = if let Some((_, col)) = sim_time_col {
            col.times_raw().to_vec()
        } else {
            vec![0; n]
        };

        for comp_id in chunk.components_identifiers() {
            let comp_name = comp_id.as_str();
            if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                for (row_idx, &t) in times.iter().enumerate() {
                    let batch =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                    if let Some(Ok(scalar_vec)) = batch {
                        for s in scalar_vec {
                            scalars
                                .entry(entity_path.clone())
                                .or_default()
                                .push((t, s.0.0));
                        }
                    }
                }
            }
        }
    }

    let metadata = RrdMetadata {
        epoch_jd: meta_scalars.get("meta/sim/epoch_jd").copied(),
        mu: meta_scalars.get("meta/sim/mu").copied(),
        body_radius: meta_scalars.get("meta/sim/body_radius").copied(),
        altitude: meta_scalars.get("meta/sim/altitude").copied(),
        period: meta_scalars.get("meta/sim/period").copied(),
        body_name: meta_texts.get("meta/sim/body_name").cloned(),
    };

    // Find base entity paths with x/y/z/vx/vy/vz sub-entities
    let base_paths: std::collections::BTreeSet<String> = scalars
        .keys()
        .filter_map(|p| {
            let suffix = p.rsplit('/').next()?;
            if matches!(suffix, "x" | "y" | "z" | "vx" | "vy" | "vz") {
                Some(p.rsplit_once('/').unwrap().0.to_string())
            } else {
                None
            }
        })
        .collect();

    let mut rows: Vec<RrdRow> = Vec::new();
    for base in &base_paths {
        let x_data = scalars.get(&format!("{base}/x"));
        let y_data = scalars.get(&format!("{base}/y"));
        let z_data = scalars.get(&format!("{base}/z"));
        let vx_data = scalars.get(&format!("{base}/vx"));
        let vy_data = scalars.get(&format!("{base}/vy"));
        let vz_data = scalars.get(&format!("{base}/vz"));

        let Some(x_data) = x_data else { continue };

        let qw_data = scalars.get(&format!("{base}/qw"));
        let qx_data = scalars.get(&format!("{base}/qx"));
        let qy_data = scalars.get(&format!("{base}/qy"));
        let qz_data = scalars.get(&format!("{base}/qz"));
        let wx_data = scalars.get(&format!("{base}/wx"));
        let wy_data = scalars.get(&format!("{base}/wy"));
        let wz_data = scalars.get(&format!("{base}/wz"));

        for (i, (t_ns, x)) in x_data.iter().enumerate() {
            let t_sec = *t_ns as f64 / 1e9;

            let quaternion = qw_data.and_then(|qw| {
                let qw = qw.get(i)?.1;
                let qx = qx_data?.get(i)?.1;
                let qy = qy_data?.get(i)?.1;
                let qz = qz_data?.get(i)?.1;
                Some([qw, qx, qy, qz])
            });
            let angular_velocity = wx_data.and_then(|wx| {
                let wx = wx.get(i)?.1;
                let wy = wy_data?.get(i)?.1;
                let wz = wz_data?.get(i)?.1;
                Some([wx, wy, wz])
            });

            rows.push(RrdRow {
                t: t_sec,
                x: *x,
                y: y_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                z: z_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                vx: vx_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                vy: vy_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                vz: vz_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                entity_path: Some(base.clone()),
                quaternion,
                angular_velocity,
            });
        }
    }

    rows.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    Ok(ParsedRrd { rows, metadata })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_empty_bytes() {
        // Empty input produces empty result (no errors, just no data)
        let result = decode_rrd(std::io::Cursor::new(&[])).unwrap();
        assert!(result.rows.is_empty());
        assert!(result.metadata.epoch_jd.is_none());
    }

    #[test]
    fn test_decode_invalid_bytes() {
        let result = decode_rrd(std::io::Cursor::new(b"not an rrd file"));
        assert!(result.is_err());
    }

    /// Small committed fixture (40KB, single satellite, 10 min at dt=60s).
    const FIXTURE_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test_orbit.rrd");

    fn load_fixture() -> ParsedRrd {
        let bytes = std::fs::read(FIXTURE_PATH).expect("test fixture should exist");
        decode_rrd(std::io::Cursor::new(&bytes)).expect("fixture should decode")
    }

    #[test]
    fn test_roundtrip_with_fixture() {
        let data = load_fixture();

        // Should have metadata
        assert!(data.metadata.epoch_jd.is_some(), "Expected epoch_jd");
        assert!(data.metadata.mu.is_some(), "Expected mu");

        // Should have rows
        assert!(!data.rows.is_empty(), "Expected rows");

        // All rows should have entity_path
        for row in &data.rows {
            assert!(row.entity_path.is_some());
        }

        // Rows should be sorted by time
        for w in data.rows.windows(2) {
            assert!(w[0].t <= w[1].t, "Rows not sorted: {} > {}", w[0].t, w[1].t);
        }

        // Position should be non-zero for at least some rows
        assert!(
            data.rows.iter().any(|r| r.x.abs() > 1.0),
            "All positions are near zero"
        );

        eprintln!(
            "Decoded {} rows, epoch_jd={:?}",
            data.rows.len(),
            data.metadata.epoch_jd
        );
    }

    #[test]
    fn test_metadata_fields() {
        let data = load_fixture();
        let m = &data.metadata;

        assert!(m.mu.unwrap() > 0.0, "mu should be positive");
        assert!(
            m.body_radius.unwrap() > 0.0,
            "body_radius should be positive"
        );
        assert!(m.epoch_jd.is_some(), "epoch_jd should be set");
    }
}
