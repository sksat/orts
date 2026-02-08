use std::collections::BTreeMap;

use crate::component::Component;
use crate::components::{BodyRadius, GravitationalParameter, Position3D, Velocity3D};
use crate::entity_path::EntityPath;
use crate::recording::Recording;
use crate::timeline::{TimeIndex, TimelineName};

/// Save a Recording to a .rrd file using the Rerun SDK.
pub fn save_as_rrd(
    recording: &Recording,
    app_id: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let rec = rerun::RecordingStreamBuilder::new(app_id).save(path)?;

    for entity_path in recording.entity_paths() {
        let store = recording.entity(entity_path).unwrap();
        let rr_path = to_rerun_path(entity_path);

        // Log static data
        for (name, scalars) in &store.static_data {
            if *name == GravitationalParameter::component_name() {
                rec.log_static(
                    format!("{rr_path}/mu"),
                    &rerun::Scalars::new([scalars[0]]),
                )?;
            } else if *name == BodyRadius::component_name() {
                rec.log_static(
                    format!("{rr_path}/radius"),
                    &rerun::Scalars::new([scalars[0]]),
                )?;
            }
        }

        // Log temporal data (position + velocity)
        let pos_col = store.columns.get(&Position3D::component_name());
        let vel_col = store.columns.get(&Velocity3D::component_name());

        if let (Some(pos_col), Some(vel_col)) = (pos_col, vel_col) {
            let n = pos_col.num_rows();
            debug_assert_eq!(n, vel_col.num_rows());

            let sim_times = store.timelines.get(&TimelineName::SimTime);
            let steps = store.timelines.get(&TimelineName::Step);

            // Each log_orbital_state logs 2 components (position, velocity),
            // so the timeline has 2*n entries. The stride between logical rows is:
            let stride = sim_times
                .or(steps)
                .map(|tl| if n > 0 { tl.len() / n } else { 1 })
                .unwrap_or(1);

            for i in 0..n {
                let tl_idx = i * stride;

                if let Some(sim_times) = sim_times
                    && let Some(TimeIndex::Seconds(t)) = sim_times.get(tl_idx)
                {
                    rec.set_duration_secs("sim_time", *t);
                }
                if let Some(steps) = steps
                    && let Some(TimeIndex::Sequence(s)) = steps.get(tl_idx)
                {
                    rec.set_time_sequence("step", *s as i64);
                }

                // Log position as 3D point
                let pos = pos_col.get_row(i).unwrap();
                rec.log(
                    rr_path.clone(),
                    &rerun::Points3D::new([[pos[0], pos[1], pos[2]]]),
                )?;

                // Log velocity as individual scalars
                let vel = vel_col.get_row(i).unwrap();
                rec.log(format!("{rr_path}/vx"), &rerun::Scalars::new([vel[0]]))?;
                rec.log(format!("{rr_path}/vy"), &rerun::Scalars::new([vel[1]]))?;
                rec.log(format!("{rr_path}/vz"), &rerun::Scalars::new([vel[2]]))?;
            }
        }
    }

    rec.flush_blocking()?;
    Ok(())
}

/// A single row of orbital data extracted from an .rrd file.
#[derive(Debug, Clone)]
pub struct RrdRow {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
}

/// Load orbital data from an .rrd file and return rows sorted by time.
///
/// Note: Position data is stored as f32 in Rerun's Points3D component,
/// so precision is reduced compared to the original f64 values.
pub fn load_from_rrd(path: &str) -> Result<Vec<RrdRow>, Box<dyn std::error::Error>> {
    use rerun::external::re_log_encoding::DecoderApp;
    use rerun::external::re_log_types::LogMsg;
    use rerun::log::Chunk;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    // Collect position data: entity_path -> Vec<(time_ns, [f32; 3])>
    let mut positions: BTreeMap<String, Vec<(i64, [f32; 3])>> = BTreeMap::new();
    // Collect velocity scalars: entity_path -> Vec<(time_ns, f64)>
    let mut scalars: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();

    for msg in DecoderApp::decode_lazy(reader) {
        let msg = msg?;
        let LogMsg::ArrowMsg(_, arrow_msg) = msg else {
            continue;
        };
        let chunk = Chunk::from_arrow_msg(&arrow_msg)?;
        let entity_path = chunk.entity_path().to_string();
        let n = chunk.num_rows();
        // Find the sim_time timeline
        let sim_time_col = chunk.timelines().iter().find(|(name, _)| {
            name.as_str() == "sim_time"
        });
        let times: Vec<i64> = if let Some((_, col)) = sim_time_col {
            col.times_raw().to_vec()
        } else {
            vec![0; n]
        };

        // Check for Position3D component (from Points3D archetype)
        for comp_id in chunk.components_identifiers() {
            let comp_name = comp_id.as_str();
            if comp_name.contains("Position3D") || comp_name.contains("positions") {
                for (row_idx, &t) in times.iter().enumerate() {
                    let batch = chunk.component_batch::<rerun::components::Position3D>(
                        comp_id,
                        row_idx,
                    );
                    if let Some(Ok(positions_vec)) = batch {
                        for pos in positions_vec {
                            let v = pos.0;
                            positions
                                .entry(entity_path.clone())
                                .or_default()
                                .push((t, [v.0[0], v.0[1], v.0[2]]));
                        }
                    }
                }
            } else if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                for (row_idx, &t) in times.iter().enumerate() {
                    let batch = chunk.component_batch::<rerun::components::Scalar>(
                        comp_id,
                        row_idx,
                    );
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

    // Match position data with velocity scalars
    let mut rows: Vec<RrdRow> = Vec::new();
    for (entity_path, pos_data) in &positions {
        let vx_path = format!("{entity_path}/vx");
        let vy_path = format!("{entity_path}/vy");
        let vz_path = format!("{entity_path}/vz");

        let vx_data = scalars.get(&vx_path);
        let vy_data = scalars.get(&vy_path);
        let vz_data = scalars.get(&vz_path);

        for (i, (t_ns, pos)) in pos_data.iter().enumerate() {
            let t_sec = *t_ns as f64 / 1e9;
            rows.push(RrdRow {
                t: t_sec,
                x: pos[0] as f64,
                y: pos[1] as f64,
                z: pos[2] as f64,
                vx: vx_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                vy: vy_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
                vz: vz_data.and_then(|v| v.get(i)).map(|v| v.1).unwrap_or(0.0),
            });
        }
    }

    rows.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    Ok(rows)
}

fn to_rerun_path(path: &EntityPath) -> String {
    let s = path.to_string();
    s.strip_prefix('/').unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetypes::OrbitalState;
    use crate::timeline::TimePoint;
    use nalgebra::Vector3;

    #[test]
    fn save_recording_to_rrd() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        let sat = EntityPath::parse("/world/sat/default");

        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        let r0 = 6778.137;
        let v0 = (398600.4418_f64 / r0).sqrt();

        for i in 0..10u64 {
            let tp = TimePoint::new()
                .with_sim_time(i as f64 * 10.0)
                .with_step(i);
            let os = OrbitalState::new(
                Vector3::new(r0, 0.0, 0.0),
                Vector3::new(0.0, v0, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        assert!(path.exists(), ".rrd file should exist");
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0, ".rrd file should not be empty");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn to_rerun_path_strips_leading_slash() {
        let path = EntityPath::parse("/world/earth");
        assert_eq!(to_rerun_path(&path), "world/earth");
    }

    #[test]
    fn roundtrip_save_and_load_rrd() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        let sat = EntityPath::parse("/world/sat/default");

        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        let r0 = 6778.137;
        let v0 = (398600.4418_f64 / r0).sqrt();

        for i in 0..5u64 {
            let t = i as f64 * 10.0;
            let tp = TimePoint::new().with_sim_time(t).with_step(i);
            let os = OrbitalState::new(
                Vector3::new(r0, 0.0, 0.0),
                Vector3::new(0.0, v0, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_roundtrip.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let rows = load_from_rrd(path_str).expect("failed to load .rrd");

        assert_eq!(rows.len(), 5, "expected 5 rows, got {}", rows.len());

        // Check first row: t=0, position=(r0, 0, 0), velocity=(0, v0, 0)
        let row0 = &rows[0];
        assert!((row0.t - 0.0).abs() < 1e-6, "t[0] = {}", row0.t);
        // Position is f32 in Rerun, so tolerance ~1 km
        assert!((row0.x - r0).abs() < 1.0, "x[0] = {}", row0.x);
        assert!(row0.y.abs() < 1.0, "y[0] = {}", row0.y);
        assert!(row0.z.abs() < 1.0, "z[0] = {}", row0.z);
        // Velocity is f64 (stored as Scalar), so higher precision
        assert!(row0.vx.abs() < 1e-6, "vx[0] = {}", row0.vx);
        assert!((row0.vy - v0).abs() < 1e-6, "vy[0] = {}", row0.vy);
        assert!(row0.vz.abs() < 1e-6, "vz[0] = {}", row0.vz);

        // Check times are ordered
        for i in 1..rows.len() {
            assert!(
                rows[i].t >= rows[i - 1].t,
                "rows not time-ordered: t[{}]={} < t[{}]={}",
                i,
                rows[i].t,
                i - 1,
                rows[i - 1].t
            );
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_static_only_entity() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        let path = std::env::temp_dir().join("test_orts_static.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);

        let _ = std::fs::remove_file(&path);
    }
}
