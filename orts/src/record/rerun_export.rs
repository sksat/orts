use std::collections::BTreeMap;

use crate::record::component::Component;
use crate::record::components::Position3D;
use crate::record::entity_path::EntityPath;
use crate::record::recording::{Recording, SimMetadata};
use crate::record::timeline::{TimeIndex, TimelineName};

/// Save a Recording to a .rrd file using the Rerun SDK.
///
/// All registered component types are exported generically via their
/// `field_names()`, so any `Component` logged through `log_temporal` or
/// `log_static` will appear in the output — no hard-coded component list.
///
/// As a convenience for Rerun 3D Viewer, entities that contain a
/// `Position3D` component also get a `Points3D` archetype logged.
pub fn save_as_rrd(
    recording: &Recording,
    app_id: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let rec = rerun::RecordingStreamBuilder::new(app_id).save(path)?;

    for entity_path in recording.entity_paths() {
        let store = recording.entity(entity_path).unwrap();
        let rr_path = to_rerun_path(entity_path);

        // Log static data (generic: uses component_registry for field names)
        for (comp_name, scalars) in &store.static_data {
            let fields = recording.lookup_component_fields(comp_name);
            for (k, field) in fields.iter().enumerate() {
                if let Some(&val) = scalars.get(k) {
                    rec.log_static(format!("{rr_path}/{field}"), &rerun::Scalars::new([val]))?;
                }
            }
        }

        // Log temporal data (generic: iterate all component columns)
        let sim_times = store.timelines.get(&TimelineName::SimTime);
        let steps = store.timelines.get(&TimelineName::Step);

        // Determine number of logical time rows from timelines (no stride hack needed)
        let n_rows = sim_times.or(steps).map(|tl| tl.len()).unwrap_or(0);

        if n_rows > 0 {
            for i in 0..n_rows {
                // Set timeline for this row (1:1 mapping, no stride)
                if let Some(sim_times) = sim_times
                    && let Some(TimeIndex::Seconds(t)) = sim_times.get(i)
                {
                    rec.set_duration_secs("sim_time", *t);
                }
                if let Some(steps) = steps
                    && let Some(TimeIndex::Sequence(s)) = steps.get(i)
                {
                    rec.set_time_sequence("step", *s as i64);
                }

                // Export all component columns as f64 Scalars
                for (comp_name, column) in &store.columns {
                    if let Some(row) = column.get_row(i) {
                        let fields = recording.lookup_component_fields(comp_name);
                        for (k, field) in fields.iter().enumerate() {
                            if let Some(&val) = row.get(k) {
                                rec.log(format!("{rr_path}/{field}"), &rerun::Scalars::new([val]))?;
                            }
                        }
                    }
                }

                // Orthogonal: if Position3D exists, also log Points3D for
                // Rerun 3D Viewer visualization. This intentionally duplicates the
                // position data already logged as f64 Scalars above — Points3D uses
                // f32 internally and is only consumed by the 3D spatial view.
                if let Some(pos_col) = store.columns.get(&Position3D::component_name())
                    && let Some(pos) = pos_col.get_row(i)
                {
                    rec.log(
                        rr_path.clone(),
                        &rerun::Points3D::new([[pos[0], pos[1], pos[2]]]),
                    )?;
                }
            }
        }
    }

    // Log component schema for each entity so load_as_recording() can
    // reconstruct ComponentColumns without field-name guessing.
    for entity_path in recording.entity_paths() {
        let store = recording.entity(entity_path).unwrap();
        let rr_path = to_rerun_path(entity_path);

        let mut schema_entries: Vec<serde_json::Value> = Vec::new();
        // Temporal components
        let mut comp_names: Vec<_> = store.columns.keys().collect();
        comp_names.sort();
        for comp_name in comp_names {
            let col = &store.columns[comp_name];
            let fields = recording.lookup_component_fields(comp_name);
            schema_entries.push(serde_json::json!({
                "name": &**comp_name,
                "fields": fields,
                "scalars_per_row": col.scalars_per_row,
            }));
        }
        // Static components
        let mut static_names: Vec<_> = store.static_data.keys().collect();
        static_names.sort();
        for comp_name in static_names {
            let fields = recording.lookup_component_fields(comp_name);
            schema_entries.push(serde_json::json!({
                "name": &**comp_name,
                "fields": fields,
                "static": true,
            }));
        }

        if !schema_entries.is_empty() {
            let schema_json = serde_json::to_string(&schema_entries).unwrap();
            rec.log_static(
                format!("meta/schema/{rr_path}"),
                &rerun::TextDocument::new(schema_json),
            )?;
        }
    }

    // Log simulation metadata as static data under meta/sim/
    let meta = &recording.metadata;
    if let Some(epoch_jd) = meta.epoch_jd {
        rec.log_static("meta/sim/epoch_jd", &rerun::Scalars::new([epoch_jd]))?;
    }
    if let Some(mu) = meta.mu {
        rec.log_static("meta/sim/mu", &rerun::Scalars::new([mu]))?;
    }
    if let Some(body_radius) = meta.body_radius {
        rec.log_static("meta/sim/body_radius", &rerun::Scalars::new([body_radius]))?;
    }
    if let Some(altitude) = meta.altitude {
        rec.log_static("meta/sim/altitude", &rerun::Scalars::new([altitude]))?;
    }
    if let Some(period) = meta.period {
        rec.log_static("meta/sim/period", &rerun::Scalars::new([period]))?;
    }
    if let Some(ref name) = meta.body_name {
        rec.log_static(
            "meta/sim/body_name",
            &rerun::TextDocument::new(name.as_str()),
        )?;
    }
    if let Some(ref iso) = meta.epoch_iso {
        rec.log_static(
            "meta/sim/epoch_iso",
            &rerun::TextDocument::new(iso.as_str()),
        )?;
    }
    if let Some(ref desc) = meta.orbit_description {
        rec.log_static(
            "meta/sim/orbit_description",
            &rerun::TextDocument::new(desc.as_str()),
        )?;
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
    /// Entity path this row belongs to (e.g., "world/sat/iss").
    pub entity_path: Option<String>,
    /// Body-to-inertial quaternion [w, x, y, z] (optional, for attitude-enabled runs).
    pub quaternion: Option<[f64; 4]>,
    /// Angular velocity in body frame [rad/s] (optional).
    pub angular_velocity: Option<[f64; 3]>,
}

/// Full data loaded from an .rrd file: trajectory rows + simulation metadata.
#[derive(Debug, Clone)]
pub struct RrdData {
    pub rows: Vec<RrdRow>,
    pub metadata: SimMetadata,
}

/// Load orbital data and metadata from an .rrd file.
pub fn load_rrd_data(path: &str) -> Result<RrdData, Box<dyn std::error::Error>> {
    use rerun::external::re_log_encoding::DecoderApp;
    use rerun::external::re_log_types::LogMsg;
    use rerun::log::Chunk;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    // Collect f64 scalars: entity_path -> Vec<(time_ns, f64)>
    let mut scalars: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    // Collect metadata scalars: entity_path -> f64 (static/timeless)
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

        // Check for metadata entities under meta/sim/
        // Rerun entity paths may or may not have a leading /
        let normalized_path = entity_path.strip_prefix('/').unwrap_or(&entity_path);
        if normalized_path.starts_with("meta/sim/") {
            let entity_path = normalized_path.to_string();
            // Try to extract scalar value
            for comp_id in chunk.components_identifiers() {
                let comp_name = comp_id.as_str();
                if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                    for row_idx in 0..n {
                        let batch =
                            chunk.component_batch::<rerun::components::Scalar>(comp_id, row_idx);
                        if let Some(Ok(scalar_vec)) = batch
                            && let Some(s) = scalar_vec.first()
                        {
                            meta_scalars.insert(entity_path.clone(), s.0.0);
                        }
                    }
                }
                if comp_name.contains("Text") || comp_name.contains("text") {
                    for row_idx in 0..n {
                        let batch =
                            chunk.component_batch::<rerun::components::Text>(comp_id, row_idx);
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
                        chunk.component_batch::<rerun::components::Scalar>(comp_id, row_idx);
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

    // Build metadata from extracted values
    let metadata = SimMetadata {
        epoch_jd: meta_scalars.get("meta/sim/epoch_jd").copied(),
        epoch_iso: meta_texts.get("meta/sim/epoch_iso").cloned(),
        mu: meta_scalars.get("meta/sim/mu").copied(),
        body_radius: meta_scalars.get("meta/sim/body_radius").copied(),
        altitude: meta_scalars.get("meta/sim/altitude").copied(),
        period: meta_scalars.get("meta/sim/period").copied(),
        body_name: meta_texts.get("meta/sim/body_name").cloned(),
        orbit_description: meta_texts.get("meta/sim/orbit_description").cloned(),
    };

    // Find base entity paths that have x/y/z/vx/vy/vz sub-entities.
    // e.g., /world/sat/default/x → base = /world/sat/default
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

        // Use x as the reference for row count and time
        let Some(x_data) = x_data else { continue };

        // Attitude components (optional)
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
    Ok(RrdData { rows, metadata })
}

/// Load orbital data from an .rrd file and return rows sorted by time.
///
/// Position and velocity are read from f64 Scalar components (x, y, z, vx, vy, vz).
pub fn load_from_rrd(path: &str) -> Result<Vec<RrdRow>, Box<dyn std::error::Error>> {
    Ok(load_rrd_data(path)?.rows)
}

/// Load an .rrd file and reconstruct a [`Recording`].
///
/// Uses component schema metadata (saved by [`save_as_rrd`]) to accurately
/// reconstruct `ComponentColumn`s. Falls back to field-name heuristics for
/// .rrd files saved before schema metadata was introduced.
///
/// This enables `orts convert` to produce the same CSV output as `orts run`.
pub fn load_as_recording(path: &str) -> Result<Recording, Box<dyn std::error::Error>> {
    use crate::record::recording::ComponentColumn;
    use rerun::external::re_log_encoding::DecoderApp;
    use rerun::external::re_log_types::LogMsg;
    use rerun::log::Chunk;
    use std::borrow::Cow;
    use std::collections::BTreeSet;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    // Collect all data from the rrd file
    // scalars: "entity_path/field_name" -> Vec<(time_ns, f64)>
    let mut scalars: BTreeMap<String, Vec<(i64, f64)>> = BTreeMap::new();
    let mut meta_scalars: BTreeMap<String, f64> = BTreeMap::new();
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
        if normalized_path.starts_with("meta/") {
            let entity_path = normalized_path.to_string();
            for comp_id in chunk.components_identifiers() {
                let comp_name = comp_id.as_str();
                if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                    for row_idx in 0..n {
                        let batch =
                            chunk.component_batch::<rerun::components::Scalar>(comp_id, row_idx);
                        if let Some(Ok(scalar_vec)) = batch
                            && let Some(s) = scalar_vec.first()
                        {
                            meta_scalars.insert(entity_path.clone(), s.0.0);
                        }
                    }
                }
                if comp_name.contains("Text") || comp_name.contains("text") {
                    for row_idx in 0..n {
                        let batch =
                            chunk.component_batch::<rerun::components::Text>(comp_id, row_idx);
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
                        chunk.component_batch::<rerun::components::Scalar>(comp_id, row_idx);
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

    // Build metadata
    let metadata = SimMetadata {
        epoch_jd: meta_scalars.get("meta/sim/epoch_jd").copied(),
        epoch_iso: meta_texts.get("meta/sim/epoch_iso").cloned(),
        mu: meta_scalars.get("meta/sim/mu").copied(),
        body_radius: meta_scalars.get("meta/sim/body_radius").copied(),
        altitude: meta_scalars.get("meta/sim/altitude").copied(),
        period: meta_scalars.get("meta/sim/period").copied(),
        body_name: meta_texts.get("meta/sim/body_name").cloned(),
        orbit_description: meta_texts.get("meta/sim/orbit_description").cloned(),
    };

    // Find all entity base paths (strip the leaf field name and leading slash)
    let base_paths: BTreeSet<String> = scalars
        .keys()
        .filter_map(|p| {
            let normalized = p.strip_prefix('/').unwrap_or(p);
            if normalized.starts_with("meta/") {
                return None;
            }
            let base = normalized.rsplit_once('/')?.0;
            Some(base.to_string())
        })
        .collect();

    let mut rec = Recording::new();
    rec.metadata = metadata;

    for base in &base_paths {
        let entity = EntityPath::parse(&format!("/{base}"));

        // Try to load schema from meta/schema/<base>
        let schema_key = format!("meta/schema/{base}");
        let schema: Option<Vec<serde_json::Value>> = meta_texts
            .get(&schema_key)
            .and_then(|json| serde_json::from_str(json).ok());

        // Collect all field names under this base path.
        // Scalar keys may have a leading slash; normalize for comparison.
        let field_names: Vec<String> = scalars
            .keys()
            .filter_map(|k| {
                let normalized = k.strip_prefix('/').unwrap_or(k);
                normalized
                    .strip_prefix(base)?
                    .strip_prefix('/')
                    .map(String::from)
            })
            .collect();

        if let Some(schema) = schema {
            // Schema-based reconstruction: group fields into components
            for entry in &schema {
                let comp_name_str = entry["name"].as_str().unwrap_or("");
                let is_static = entry
                    .get("static")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let fields: Vec<String> = entry["fields"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                if is_static {
                    // Reconstruct static data
                    let mut static_scalars = Vec::new();
                    for field in &fields {
                        if let Some(data) = get_scalar_data(&scalars, base, field)
                            && let Some((_, val)) = data.first()
                        {
                            static_scalars.push(*val);
                        } else if let Some(&val) = meta_scalars.get(&format!("{base}/{field}")) {
                            static_scalars.push(val);
                        }
                    }
                    if !static_scalars.is_empty() {
                        let comp_name: Cow<'static, str> = Cow::Owned(comp_name_str.to_string());
                        let store = rec.entity_mut(&entity);
                        store.static_data.insert(comp_name.clone(), static_scalars);
                        rec.register_component_fields(
                            comp_name,
                            fields.iter().map(|s| s.as_str()).collect(),
                        );
                    }
                } else {
                    // Reconstruct temporal ComponentColumn
                    let n_fields = fields.len();
                    if n_fields == 0 {
                        continue;
                    }

                    // Get data for first field to determine row count and times
                    let Some(first_data) = get_scalar_data(&scalars, base, &fields[0]) else {
                        continue;
                    };

                    let n_rows = first_data.len();
                    let mut column = ComponentColumn::new(n_fields);

                    for i in 0..n_rows {
                        let mut row = Vec::with_capacity(n_fields);
                        for field in &fields {
                            let val = get_scalar_data(&scalars, base, field)
                                .and_then(|data| data.get(i))
                                .map(|(_, v)| *v)
                                .unwrap_or(0.0);
                            row.push(val);
                        }
                        column.push(&row);
                    }

                    // Set timelines from first field's timestamps
                    let store = rec.entity_mut(&entity);
                    store
                        .timelines
                        .entry(TimelineName::SimTime)
                        .or_insert_with(|| {
                            first_data
                                .iter()
                                .map(|(t_ns, _)| TimeIndex::Seconds(*t_ns as f64 / 1e9))
                                .collect()
                        });
                    store.num_rows = n_rows;

                    let comp_name: Cow<'static, str> = Cow::Owned(comp_name_str.to_string());
                    store.columns.insert(comp_name.clone(), column);
                    rec.register_component_fields(
                        comp_name,
                        fields.iter().map(|s| s.as_str()).collect(),
                    );
                }
            }
        } else {
            // Fallback: no schema metadata (legacy rrd files).
            // Group fields by known Component field_names.
            // Build reverse lookup from field_names
            let known: &[(&str, &[&str])] = &[
                ("orts.Position3D", &["x", "y", "z"]),
                ("orts.Velocity3D", &["vx", "vy", "vz"]),
                ("orts.Quaternion4D", &["qw", "qx", "qy", "qz"]),
                ("orts.AngularVelocity3D", &["wx", "wy", "wz"]),
                ("orts.MtqCommand3D", &["mtq_mx", "mtq_my", "mtq_mz"]),
                ("orts.RwTorqueCommand3D", &["rw_tx", "rw_ty", "rw_tz"]),
                ("orts.RwMomentum3D", &["rw_hx", "rw_hy", "rw_hz"]),
            ];

            let field_set: BTreeSet<&str> = field_names.iter().map(|s| s.as_str()).collect();

            for &(comp_name_str, comp_fields) in known {
                if comp_fields.iter().all(|f| field_set.contains(f)) {
                    let Some(first_data) = get_scalar_data(&scalars, base, comp_fields[0]) else {
                        continue;
                    };
                    let n_rows = first_data.len();
                    let n_fields = comp_fields.len();
                    let mut column = ComponentColumn::new(n_fields);

                    for i in 0..n_rows {
                        let mut row = Vec::with_capacity(n_fields);
                        for field in comp_fields {
                            let val = get_scalar_data(&scalars, base, field)
                                .and_then(|data| data.get(i))
                                .map(|(_, v)| *v)
                                .unwrap_or(0.0);
                            row.push(val);
                        }
                        column.push(&row);
                    }

                    let store = rec.entity_mut(&entity);
                    store
                        .timelines
                        .entry(TimelineName::SimTime)
                        .or_insert_with(|| {
                            first_data
                                .iter()
                                .map(|(t_ns, _)| TimeIndex::Seconds(*t_ns as f64 / 1e9))
                                .collect()
                        });
                    store.num_rows = n_rows.max(store.num_rows);

                    let comp_name: Cow<'static, str> = Cow::Owned(comp_name_str.to_string());
                    store.columns.insert(comp_name.clone(), column);
                    rec.register_component_fields(comp_name, comp_fields.to_vec());
                }
            }
        }
    }

    Ok(rec)
}

/// Look up scalar data, trying both with and without leading slash.
fn get_scalar_data<'a>(
    scalars: &'a BTreeMap<String, Vec<(i64, f64)>>,
    base: &str,
    field: &str,
) -> Option<&'a Vec<(i64, f64)>> {
    scalars
        .get(&format!("{base}/{field}"))
        .or_else(|| scalars.get(&format!("/{base}/{field}")))
}

fn to_rerun_path(path: &EntityPath) -> String {
    let s = path.to_string();
    s.strip_prefix('/').unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::archetypes::OrbitalState;
    use crate::record::components::{BodyRadius, GravitationalParameter};
    use crate::record::timeline::TimePoint;
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
            let tp = TimePoint::new().with_sim_time(i as f64 * 10.0).with_step(i);
            let os = OrbitalState::new(Vector3::new(r0, 0.0, 0.0), Vector3::new(0.0, v0, 0.0));
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
            let os = OrbitalState::new(Vector3::new(r0, 0.0, 0.0), Vector3::new(0.0, v0, 0.0));
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_roundtrip.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let rows = load_from_rrd(path_str).expect("failed to load .rrd");

        assert_eq!(rows.len(), 5, "expected 5 rows, got {}", rows.len());

        // Check first row: t=0, position=(r0, 0, 0), velocity=(0, v0, 0)
        // All values are f64 (stored as Scalar), so full precision is preserved.
        let row0 = &rows[0];
        assert!((row0.t - 0.0).abs() < 1e-6, "t[0] = {}", row0.t);
        assert!((row0.x - r0).abs() < 1e-9, "x[0] = {}", row0.x);
        assert!(row0.y.abs() < 1e-9, "y[0] = {}", row0.y);
        assert!(row0.z.abs() < 1e-9, "z[0] = {}", row0.z);
        assert!(row0.vx.abs() < 1e-9, "vx[0] = {}", row0.vx);
        assert!((row0.vy - v0).abs() < 1e-9, "vy[0] = {}", row0.vy);
        assert!(row0.vz.abs() < 1e-9, "vz[0] = {}", row0.vz);

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
    fn roundtrip_metadata() {
        let mut rec = Recording::new();
        let body = EntityPath::parse("/world/earth");
        let sat = EntityPath::parse("/world/sat/default");

        rec.log_static(&body, &GravitationalParameter(398600.4418));
        rec.log_static(&body, &BodyRadius(6378.137));

        rec.metadata = SimMetadata {
            epoch_jd: Some(2460390.0),
            mu: Some(398600.4418),
            body_radius: Some(6378.137),
            body_name: Some("Earth".to_string()),
            altitude: Some(400.0),
            period: Some(5554.0),
            ..Default::default()
        };

        let r0 = 6778.137;
        let v0 = (398600.4418_f64 / r0).sqrt();
        for i in 0..3u64 {
            let tp = TimePoint::new().with_sim_time(i as f64 * 10.0).with_step(i);
            let os = OrbitalState::new(Vector3::new(r0, 0.0, 0.0), Vector3::new(0.0, v0, 0.0));
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_metadata.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let data = load_rrd_data(path_str).expect("failed to load .rrd");
        assert_eq!(data.rows.len(), 3);

        let meta = &data.metadata;
        assert!(
            (meta.epoch_jd.unwrap() - 2460390.0).abs() < 1e-6,
            "epoch_jd = {:?}",
            meta.epoch_jd
        );
        assert!(
            (meta.mu.unwrap() - 398600.4418).abs() < 1e-6,
            "mu = {:?}",
            meta.mu
        );
        assert!(
            (meta.body_radius.unwrap() - 6378.137).abs() < 1e-6,
            "body_radius = {:?}",
            meta.body_radius
        );
        assert!(
            (meta.altitude.unwrap() - 400.0).abs() < 1e-6,
            "altitude = {:?}",
            meta.altitude
        );
        assert!(
            (meta.period.unwrap() - 5554.0).abs() < 1e-6,
            "period = {:?}",
            meta.period
        );
        assert_eq!(
            meta.body_name.as_deref(),
            Some("Earth"),
            "body_name = {:?}",
            meta.body_name
        );

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

    #[test]
    fn save_position3d_only_entity() {
        // Position3D without Velocity3D must survive the generic export.
        // This was impossible before the stride hack fix.
        use crate::record::components::Position3D;

        let mut rec = Recording::new();
        let moon = EntityPath::parse("/world/moon");

        for i in 0..5u64 {
            let tp = TimePoint::new()
                .with_sim_time(i as f64 * 100.0)
                .with_step(i);
            let pos = Position3D(Vector3::new(-384400.0, i as f64 * 1000.0, 0.0));
            rec.log_temporal(&moon, &tp, &pos);
        }

        let path = std::env::temp_dir().join("test_orts_pos_only.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("Position3D-only entity should save");
        assert!(path.exists());

        // Load and verify we get rows (x/y/z present, vx/vy/vz default to 0)
        let data = load_rrd_data(path_str).expect("should load");
        assert_eq!(
            data.rows.len(),
            5,
            "expected 5 rows for Position3D-only entity"
        );

        let row0 = &data.rows[0];
        assert!((row0.x - (-384400.0)).abs() < 1e-6);
        assert!(row0.y.abs() < 1e-6);
        // vx/vy/vz should be 0 (no Velocity3D logged)
        assert!(row0.vx.abs() < 1e-9);
        assert!(row0.vy.abs() < 1e-9);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_quaternion_only_entity() {
        // Quaternion4D without Position3D must be written to the RRD file.
        // Note: `load_rrd_data` won't read this entity back because it only
        // looks for x/y/z/vx/vy/vz sub-entities. This test verifies the
        // *write* path doesn't panic or silently skip non-positional entities.
        use crate::record::components::Quaternion4D;

        let mut rec = Recording::new();
        let sensor = EntityPath::parse("/world/sensor");

        for i in 0..3u64 {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let q = Quaternion4D(nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0));
            rec.log_temporal(&sensor, &tp, &q);
        }

        let path = std::env::temp_dir().join("test_orts_quat_only.rrd");
        let path_str = path.to_str().unwrap();

        save_as_rrd(&rec, "test-orts", path_str).expect("Quaternion4D-only entity should save");
        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);

        let _ = std::fs::remove_file(&path);
    }
}
