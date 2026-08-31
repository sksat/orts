use std::collections::{BTreeMap, BTreeSet};

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
    let rec = re_sdk::RecordingStreamBuilder::new(app_id).save(path)?;

    for entity_path in recording.entity_paths() {
        let store = recording.entity(entity_path).unwrap();
        let rr_path = to_rerun_path(entity_path);

        // Log static data (generic: uses component_registry for field names)
        for (comp_name, scalars) in &store.static_data {
            let fields = recording.lookup_component_fields(comp_name);
            for (k, field) in fields.iter().enumerate() {
                if let Some(&val) = scalars.get(k) {
                    rec.log_static(
                        format!("{rr_path}/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([val]),
                    )?;
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
                                rec.log(
                                    format!("{rr_path}/{field}"),
                                    &re_sdk_types::archetypes::Scalars::new([val]),
                                )?;
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
                        &re_sdk_types::archetypes::Points3D::new([[pos[0], pos[1], pos[2]]]),
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
                &re_sdk_types::archetypes::TextDocument::new(schema_json),
            )?;
        }
    }

    // Log simulation metadata as static data under meta/sim/
    let meta = &recording.metadata;
    if let Some(epoch_jd) = meta.epoch_jd {
        rec.log_static(
            "meta/sim/epoch_jd",
            &re_sdk_types::archetypes::Scalars::new([epoch_jd]),
        )?;
    }
    if let Some(mu) = meta.mu {
        rec.log_static("meta/sim/mu", &re_sdk_types::archetypes::Scalars::new([mu]))?;
    }
    if let Some(body_radius) = meta.body_radius {
        rec.log_static(
            "meta/sim/body_radius",
            &re_sdk_types::archetypes::Scalars::new([body_radius]),
        )?;
    }
    if let Some(altitude) = meta.altitude {
        rec.log_static(
            "meta/sim/altitude",
            &re_sdk_types::archetypes::Scalars::new([altitude]),
        )?;
    }
    if let Some(period) = meta.period {
        rec.log_static(
            "meta/sim/period",
            &re_sdk_types::archetypes::Scalars::new([period]),
        )?;
    }
    if let Some(ref name) = meta.body_name {
        rec.log_static(
            "meta/sim/body_name",
            &re_sdk_types::archetypes::TextDocument::new(name.as_str()),
        )?;
    }
    if let Some(ref iso) = meta.epoch_iso {
        rec.log_static(
            "meta/sim/epoch_iso",
            &re_sdk_types::archetypes::TextDocument::new(iso.as_str()),
        )?;
    }
    if let Some(ref desc) = meta.orbit_description {
        rec.log_static(
            "meta/sim/orbit_description",
            &re_sdk_types::archetypes::TextDocument::new(desc.as_str()),
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

/// Where one scalar value sits on the recording's timelines.
///
/// Ordered by time first, so a recording whose chunks carry no `sim_time` still
/// yields rows in `step` order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RowKey {
    /// A recording time index: `sim_time` \[ns\] and the `step` sequence
    /// number, each present only when the recording has that timeline.
    Timed {
        time_ns: Option<i64>,
        step: Option<i64>,
        repeat: u32,
    },
    /// A recording indexed by a timeline of its own naming: the value the row
    /// sits at on that axis. Apart from `Timed` so that a `frame` of 1 is never
    /// the `step` 1 of a recording that carries both.
    Axis { value: i64, repeat: u32 },
    /// Neither timeline is present: fall back to the column-local index. Rows
    /// keyed this way carry no time information and all report `t = 0`.
    Index(usize),
}

impl RowKey {
    /// Simulation time of this row \[s\], or 0 when the recording carries none.
    fn t_secs(self) -> f64 {
        match self {
            RowKey::Timed {
                time_ns: Some(ns), ..
            } => ns as f64 / 1e9,
            RowKey::Timed { time_ns: None, .. } | RowKey::Axis { .. } | RowKey::Index(_) => 0.0,
        }
    }

    /// This key with its repeat number replaced. An `Index` key carries none
    /// and comes back unchanged.
    fn at_repeat(self, repeat: u32) -> RowKey {
        match self {
            RowKey::Timed { time_ns, step, .. } => RowKey::Timed {
                time_ns,
                step,
                repeat,
            },
            RowKey::Axis { value, .. } => RowKey::Axis { value, repeat },
            RowKey::Index(_) => self,
        }
    }

    /// The moment this key sits on, or `None` for a key that carries no time.
    fn moment(self) -> Option<Moment> {
        match self {
            RowKey::Index(_) => None,
            _ => Some(self.at_repeat(0)),
        }
    }
}

/// Where a chunk's rows sit, or `None` when nothing places them.
///
/// `sim_time` and `step` are the names `orts` writes. A recording from another
/// tool names its own timeline, and reading that as no timeline at all would
/// join its columns by position, the mix this decode replaces. One such name
/// serves the whole recording, held in `axis`: two axes are separate
/// dimensions, so a `frame` of 1 and an `iteration` of 1 are not one moment. A
/// chunk indexed only by some other axis has no place among the rest and is
/// left out.
///
/// `log_time` and `log_tick`, which rerun adds to every log call, say when a
/// value was logged rather than when it happened: two fields of one state carry
/// different ones and could never pair.
fn chunk_keys(chunk: &re_chunk::Chunk, axis: &mut Option<String>) -> Option<ChunkKeys> {
    let timeline = |wanted: &str| {
        chunk
            .timelines()
            .iter()
            .find(|(name, _)| name.as_str() == wanted)
            .map(|(_, col)| col.times_raw().to_vec())
    };
    let sim_time = timeline("sim_time");
    let step = timeline("step");
    if sim_time.is_some() || step.is_some() {
        return Some(ChunkKeys {
            sim_time,
            step,
            axis: None,
        });
    }

    let mut named: Vec<_> = chunk
        .timelines()
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "sim_time" | "step" | "log_time" | "log_tick"))
        .collect();
    // The timelines arrive as a set, so choose by name to stay reproducible
    // from run to run.
    named.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

    let on = |times: Vec<i64>| {
        Some(ChunkKeys {
            sim_time: None,
            step: None,
            axis: Some(times),
        })
    };
    // A chunk carrying no timeline of its own either: its keys come from the
    // column-local position, which never joins with a timed key.
    let untimed = || {
        Some(ChunkKeys {
            sim_time: None,
            step: None,
            axis: None,
        })
    };

    match axis {
        None => match named.first() {
            Some((name, col)) => {
                *axis = Some(name.as_str().to_string());
                on(col.times_raw().to_vec())
            }
            None => untimed(),
        },
        Some(chosen) => match named.iter().find(|(name, _)| name.as_str() == chosen) {
            Some((_, col)) => on(col.times_raw().to_vec()),
            None if named.is_empty() => untimed(),
            None => None,
        },
    }
}

/// One decoded scalar field: its value at each time index of the recording.
type Column = BTreeMap<RowKey, f64>;

/// One moment of a recording: a [`RowKey`] with its repeat number cleared, so
/// that every value logged at that moment ranges within it.
type Moment = RowKey;

/// Next repeat ordinal to assign, per column and moment.
///
/// A counter rather than a scan of the column: counting the existing repeats on
/// every value made decoding quadratic in the length of a recording, even one
/// with no repeats at all.
type RepeatCounters = BTreeMap<String, BTreeMap<Moment, u32>>;

/// How many values `column` holds at `moment`, over every repeat.
fn repeats_at(column: &Column, moment: Moment) -> usize {
    column.range(moment..=moment.at_repeat(u32::MAX)).count()
}

/// Time index of every row in one chunk, per timeline the chunk carries.
struct ChunkKeys {
    sim_time: Option<Vec<i64>>,
    step: Option<Vec<i64>>,
    /// The recording's own named axis, carried only by a chunk that has
    /// neither of the two above.
    axis: Option<Vec<i64>>,
}

/// Where one chunk row sits on the recording's timelines.
#[derive(Clone, Copy)]
enum RowIndex {
    /// The chunk's timelines place the row at this time index.
    Timed {
        time_ns: Option<i64>,
        step: Option<i64>,
    },
    /// The row sits at this value on the recording's own named axis.
    Axis(i64),
    /// The chunk carries no timeline; keys come from the column-local position.
    Untimed,
    /// A timeline the chunk does carry has no value for this row — skip it.
    Missing,
}

impl ChunkKeys {
    fn row(&self, row_idx: usize) -> RowIndex {
        if let Some(axis) = &self.axis {
            return match axis.get(row_idx) {
                Some(&value) => RowIndex::Axis(value),
                None => RowIndex::Missing,
            };
        }
        // A timeline the chunk carries must have a value for this row.
        let index = |times: &Option<Vec<i64>>| match times {
            Some(times) => times.get(row_idx).copied().map(Some).ok_or(()),
            None => Ok(None),
        };
        match (index(&self.sim_time), index(&self.step)) {
            (Ok(None), Ok(None)) => RowIndex::Untimed,
            (Ok(time_ns), Ok(step)) => RowIndex::Timed { time_ns, step },
            _ => RowIndex::Missing,
        }
    }
}

/// Load orbital data and metadata from an .rrd file.
///
/// Columns are joined on the recording's time index, so a component logged at
/// only some of the time steps never shifts the remaining components onto the
/// wrong row. A row is emitted only when the whole position triple — and, when
/// the recording has velocity columns, the whole velocity triple — is present at
/// that exact time; incomplete rows are dropped rather than padded with zeros.
///
/// That guarantee needs a time index to join on, which every recording orts
/// writes carries (`sim_time`, `step`, or both). A chunk with neither timeline
/// has no join key available, so its values are keyed by position within their
/// own column — the arrangement this join replaced, and one a sparse column
/// still shifts. Such a recording does not come from orts.
pub fn load_rrd_data(path: &str) -> Result<RrdData, Box<dyn std::error::Error>> {
    use re_chunk::Chunk;
    use re_log_encoding::DecoderApp;
    use re_log_types::LogMsg;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    // Collect f64 scalars: entity_path -> (time index -> value)
    let mut scalars: BTreeMap<String, Column> = BTreeMap::new();
    let mut repeat_counters: RepeatCounters = BTreeMap::new();
    // Collect metadata scalars: entity_path -> f64 (static/timeless)
    let mut meta_scalars: BTreeMap<String, f64> = BTreeMap::new();
    // Collect text metadata
    let mut meta_texts: BTreeMap<String, String> = BTreeMap::new();

    // The one timeline of the recording's own naming, settled by the first
    // chunk that carries neither `sim_time` nor `step` and kept for the rest.
    let mut recording_axis: Option<String> = None;

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

        let Some(keys) = chunk_keys(&chunk, &mut recording_axis) else {
            continue;
        };

        for comp_id in chunk.components_identifiers() {
            let comp_name = comp_id.as_str();
            if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                let column = scalars.entry(entity_path.clone()).or_default();
                for row_idx in 0..n {
                    let batch =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                    let Some(Ok(scalar_vec)) = batch else {
                        continue;
                    };
                    let moment = match keys.row(row_idx) {
                        RowIndex::Timed { time_ns, step } => Some(RowKey::Timed {
                            time_ns,
                            step,
                            repeat: 0,
                        }),
                        RowIndex::Axis(value) => Some(RowKey::Axis { value, repeat: 0 }),
                        RowIndex::Untimed => None,
                        RowIndex::Missing => continue,
                    };
                    // A batch usually holds one value per row, but `Scalars`
                    // takes a slice: several values at one time index become
                    // consecutive repeats rather than being dropped. The
                    // ordinal comes from a counter, so a long recording does
                    // not pay a scan of the column per value.
                    let counter = moment.map(|moment| {
                        repeat_counters
                            .entry(entity_path.clone())
                            .or_default()
                            .entry(moment)
                            .or_insert(0)
                    });
                    let mut next_repeat = counter.as_ref().map_or(0, |c| **c);
                    for value in scalar_vec.iter() {
                        let key = match moment {
                            Some(moment) => {
                                let key = moment.at_repeat(next_repeat);
                                next_repeat += 1;
                                key
                            }
                            None => RowKey::Index(column.len()),
                        };
                        column.insert(key, value.0.0);
                    }
                    if let Some(counter) = counter {
                        *counter = next_repeat;
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
        let column = |field: &str| scalars.get(&format!("{base}/{field}"));
        // Value of one field at one time index, or `None` when the recording
        // has no such column or no value there.
        let at = |col: Option<&Column>, key: RowKey| col?.get(&key).copied();

        // A row is a position, so the whole triple has to be there.
        let Some(x_col) = column("x") else { continue };
        let pos_cols = (Some(x_col), column("y"), column("z"));
        let vel_cols = (column("vx"), column("vy"), column("vz"));
        let quat_cols = (column("qw"), column("qx"), column("qy"), column("qz"));
        let omega_cols = (column("wx"), column("wy"), column("wz"));

        // A recording with no velocity column at all is position-only; one that
        // has velocity columns must supply all three at a time for the row to
        // be a state vector.
        let has_velocity = vel_cols.0.is_some() || vel_cols.1.is_some() || vel_cols.2.is_some();

        // Repeat ordinals are assigned per column, so they only identify a row
        // while every column present has the same number of values at that
        // moment. Where the counts disagree, which value pairs with which is
        // unknowable from the file: the moment is skipped rather than joined on
        // an ordinal that means different things in different columns.
        //
        // The optional columns count too. Attitude logged for only the second of
        // two states at one moment would otherwise attach to the first.
        let present: Vec<&Column> = [
            Some(x_col),
            pos_cols.1,
            pos_cols.2,
            vel_cols.0,
            vel_cols.1,
            vel_cols.2,
            quat_cols.0,
            quat_cols.1,
            quat_cols.2,
            quat_cols.3,
            omega_cols.0,
            omega_cols.1,
            omega_cols.2,
        ]
        .into_iter()
        .flatten()
        .collect();

        // Counted once per moment, not once per repeat: a `Scalars` batch puts
        // k values at one timestamp, and re-counting for each of them would
        // make reconstruction quadratic in k.
        let mut ambiguous: BTreeSet<Moment> = BTreeSet::new();
        let mut checked: BTreeSet<Moment> = BTreeSet::new();
        for &key in x_col.keys() {
            let Some(moment) = key.moment() else {
                continue;
            };
            if !checked.insert(moment) {
                continue;
            }
            // Every column that has values at this moment must have the same
            // number of them. A count of zero is a column absent there, which
            // is simply absent from the row; any other disagreement means the
            // ordinal points at different samples in different columns. Checked
            // whichever column repeats — `x` holding one value while `y` holds
            // two is just as unpairable as the other way round.
            let counts: Vec<usize> = present
                .iter()
                .map(|c| repeats_at(c, moment))
                .filter(|&n| n != 0)
                .collect();
            if counts.iter().any(|&n| n != counts[0]) {
                ambiguous.insert(moment);
            }
        }

        for &key in x_col.keys() {
            if key
                .moment()
                .is_some_and(|moment| ambiguous.contains(&moment))
            {
                continue;
            }

            let triple = |cols: (Option<&Column>, Option<&Column>, Option<&Column>)| {
                Some([at(cols.0, key)?, at(cols.1, key)?, at(cols.2, key)?])
            };

            let Some([x, y, z]) = triple(pos_cols) else {
                continue;
            };
            let velocity = triple(vel_cols);
            if has_velocity && velocity.is_none() {
                continue;
            }
            let [vx, vy, vz] = velocity.unwrap_or([0.0; 3]);

            let quaternion = (|| {
                Some([
                    at(quat_cols.0, key)?,
                    at(quat_cols.1, key)?,
                    at(quat_cols.2, key)?,
                    at(quat_cols.3, key)?,
                ])
            })();
            let angular_velocity = triple(omega_cols);

            rows.push(RrdRow {
                t: key.t_secs(),
                x,
                y,
                z,
                vx,
                vy,
                vz,
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
    use re_chunk::Chunk;
    use re_log_encoding::DecoderApp;
    use re_log_types::LogMsg;
    use std::borrow::Cow;
    use std::collections::BTreeSet;

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    // Collect all data from the rrd file
    // scalars: "entity_path/field_name" -> Vec<(time_ns, f64)>
    let mut scalars: BTreeMap<String, Column> = BTreeMap::new();
    let mut repeat_counters: RepeatCounters = BTreeMap::new();
    let mut meta_scalars: BTreeMap<String, f64> = BTreeMap::new();
    let mut meta_texts: BTreeMap<String, String> = BTreeMap::new();

    // The one timeline of the recording's own naming, settled by the first
    // chunk that carries neither `sim_time` nor `step` and kept for the rest.
    let mut recording_axis: Option<String> = None;

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

        let Some(keys) = chunk_keys(&chunk, &mut recording_axis) else {
            continue;
        };

        for comp_id in chunk.components_identifiers() {
            let comp_name = comp_id.as_str();
            if comp_name.contains("Scalar") || comp_name.contains("scalars") {
                let column = scalars.entry(entity_path.clone()).or_default();
                for row_idx in 0..n {
                    let batch =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                    let Some(Ok(scalar_vec)) = batch else {
                        continue;
                    };
                    let moment = match keys.row(row_idx) {
                        RowIndex::Timed { time_ns, step } => Some(RowKey::Timed {
                            time_ns,
                            step,
                            repeat: 0,
                        }),
                        RowIndex::Axis(value) => Some(RowKey::Axis { value, repeat: 0 }),
                        RowIndex::Untimed => None,
                        RowIndex::Missing => continue,
                    };
                    let counter = moment.map(|moment| {
                        repeat_counters
                            .entry(entity_path.clone())
                            .or_default()
                            .entry(moment)
                            .or_insert(0)
                    });
                    let mut next_repeat = counter.as_ref().map_or(0, |c| **c);
                    for value in scalar_vec.iter() {
                        let key = match moment {
                            Some(moment) => {
                                let key = moment.at_repeat(next_repeat);
                                next_repeat += 1;
                                key
                            }
                            None => RowKey::Index(column.len()),
                        };
                        column.insert(key, value.0.0);
                    }
                    if let Some(counter) = counter {
                        *counter = next_repeat;
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

        // Fields of this entity. A `<base>/child/x` key belongs to the child
        // entity, which gets a store of its own, so it is not a field here:
        // counting it would put the child's times among this entity's rows.
        // Scalar keys may have a leading slash; normalize for comparison.
        let field_names: Vec<String> = scalars
            .keys()
            .filter_map(|k| {
                let normalized = k.strip_prefix('/').unwrap_or(k);
                let field = normalized.strip_prefix(base)?.strip_prefix('/')?;
                (!field.contains('/')).then(|| field.to_string())
            })
            .collect();
        let field_set: BTreeSet<&str> = field_names.iter().map(|s| s.as_str()).collect();

        // The components this entity records over time, and its static ones
        // apart from them: a static value carries no timeline, so it is not a
        // row of the entity.
        let mut temporal: Vec<(String, Vec<String>)> = Vec::new();
        let mut statics: Vec<(String, Vec<String>)> = Vec::new();
        match &schema {
            Some(schema) => {
                for entry in schema {
                    let Some(name) = entry["name"].as_str() else {
                        continue;
                    };
                    let fields: Vec<String> = entry["fields"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    if fields.is_empty() {
                        continue;
                    }
                    if entry
                        .get("static")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        statics.push((name.to_string(), fields));
                    } else {
                        temporal.push((name.to_string(), fields));
                    }
                }
            }
            // Legacy .rrd files carry no schema: recognize a component by its
            // field names.
            None => {
                for &(name, fields) in KNOWN_COMPONENTS {
                    if fields.iter().all(|f| field_set.contains(f)) {
                        temporal.push((
                            name.to_string(),
                            fields.iter().map(|f| (*f).to_string()).collect(),
                        ));
                    }
                }
            }
        }

        // A component the file has no column for cannot be reconstructed
        // whatever the rows are, so it gets no say in what they are. Its
        // remaining fields would otherwise narrow them: `x` and `z` recorded at
        // one time, with `y` absent from the file, cost a whole velocity its
        // rows at every other time.
        temporal.retain(|(_, fields)| {
            fields
                .iter()
                .all(|field| get_scalar_data(&scalars, base, field).is_some())
        });

        // One row-key set for the whole entity. `EntityStore` keeps a single
        // timeline shared by every component column, so a component compacted
        // onto its own surviving times would leave the columns disagreeing on
        // what row 0 means: a position from one time beside a velocity from
        // another.
        //
        // `ComponentColumn` has no way to say "no value at this row", so a row
        // exists only where the state components are whole. Anchoring on
        // position and velocity keeps the trajectory intact when an optional
        // component such as attitude was recorded at only some of the times.
        // That component is then left out rather than filled with zeros, which
        // downstream would read as a measured value: `orts convert` writes them
        // to CSV and computes orbital elements from them.
        let anchors: Vec<&(String, Vec<String>)> = {
            let state: Vec<&(String, Vec<String>)> = temporal
                .iter()
                .filter(|(name, _)| name.ends_with("Position3D") || name.ends_with("Velocity3D"))
                .collect();
            if state.is_empty() {
                temporal.iter().collect()
            } else {
                state
            }
        };

        let row_keys: Vec<RowKey> = {
            // Every field of a surviving group has a column, so nothing is
            // dropped here.
            let columns_of = |groups: &[&(String, Vec<String>)]| -> Vec<&Column> {
                groups
                    .iter()
                    .flat_map(|(_, fields)| fields)
                    .filter_map(|field| get_scalar_data(&scalars, base, field))
                    .collect()
            };
            let anchor_columns = columns_of(&anchors);
            // The rows follow the anchors, but a disagreement anywhere in the
            // entity makes the moment unpairable: three attitudes beside two
            // states leave no way to say which attitude is which state's.
            let all_columns = columns_of(&temporal.iter().collect::<Vec<_>>());

            // A moment whose fields disagree on how many values they recorded
            // is left out entirely, as in the other two decoders: the repeat
            // numbers are per field, so with two samples at one time of which
            // the first omits `y`, repeat 0 would pair the first sample's `x`
            // with the second's `y`, the cross-sample mix this join removes.
            let mut ambiguous: BTreeSet<Moment> = BTreeSet::new();
            let mut checked: BTreeSet<Moment> = BTreeSet::new();
            for col in &all_columns {
                for &key in col.keys() {
                    let Some(moment) = key.moment() else {
                        continue;
                    };
                    // Counting once per moment: per repeat it would be
                    // quadratic in the size of a multi-value batch.
                    if !checked.insert(moment) {
                        continue;
                    }
                    let counts: Vec<usize> = all_columns
                        .iter()
                        .map(|c| repeats_at(c, moment))
                        .filter(|&n| n != 0)
                        .collect();
                    if counts.iter().any(|&n| n != counts[0]) {
                        ambiguous.insert(moment);
                    }
                }
            }

            let mut keys: Vec<RowKey> = match anchor_columns.split_first() {
                Some((first, rest)) => first
                    .keys()
                    .copied()
                    .filter(|key| rest.iter().all(|col| col.contains_key(key)))
                    .collect(),
                None => Vec::new(),
            };
            keys.retain(|key| {
                !key.moment()
                    .is_some_and(|moment| ambiguous.contains(&moment))
            });
            keys
        };
        // TODO: the keys also carry the step, or the named axis, a row sits on,
        // and only `SimTime` is filled from them. So a recording indexed by
        // `step` alone comes back with every row at 0 s and its step timeline
        // gone, and one on an axis of its own loses that axis. `Recording`'s
        // CSV path reads `SimTime` only, so filling the others changes what
        // `orts convert` reports. Left as it stands on main until the step
        // round-trip is settled with the writer.
        let entity_times: Vec<TimeIndex> = row_keys
            .iter()
            .map(|k| TimeIndex::Seconds(k.t_secs()))
            .collect();

        for (name, fields) in &statics {
            let mut static_scalars = Vec::new();
            for field in fields {
                if let Some(data) = get_scalar_data(&scalars, base, field)
                    && let Some((_, val)) = data.iter().next()
                {
                    static_scalars.push(*val);
                } else if let Some(&val) = meta_scalars.get(&format!("{base}/{field}")) {
                    static_scalars.push(val);
                }
            }
            if !static_scalars.is_empty() {
                let comp_name: Cow<'static, str> = Cow::Owned(name.clone());
                let store = rec.entity_mut(&entity);
                store.static_data.insert(comp_name.clone(), static_scalars);
                rec.register_component_fields(
                    comp_name,
                    fields.iter().map(|s| s.as_str()).collect(),
                );
            }
        }

        for (name, fields) in &temporal {
            let mut column = ComponentColumn::new(fields.len());
            let mut whole = true;
            for &key in &row_keys {
                let mut row = Vec::with_capacity(fields.len());
                for field in fields {
                    match get_scalar_data(&scalars, base, field).and_then(|col| col.get(&key)) {
                        Some(&v) => row.push(v),
                        None => {
                            whole = false;
                            break;
                        }
                    }
                }
                if !whole {
                    break;
                }
                column.push(&row);
            }
            if !whole {
                continue;
            }

            let comp_name: Cow<'static, str> = Cow::Owned(name.clone());
            let store = rec.entity_mut(&entity);
            store
                .timelines
                .entry(TimelineName::SimTime)
                .or_insert_with(|| entity_times.clone());
            store.num_rows = row_keys.len();
            store.columns.insert(comp_name.clone(), column);
            rec.register_component_fields(comp_name, fields.iter().map(|s| s.as_str()).collect());
        }
    }

    Ok(rec)
}

/// Components a legacy .rrd, one without schema metadata, is recognized by.
const KNOWN_COMPONENTS: &[(&str, &[&str])] = &[
    ("orts.Position3D", &["x", "y", "z"]),
    ("orts.Velocity3D", &["vx", "vy", "vz"]),
    ("orts.Quaternion4D", &["qw", "qx", "qy", "qz"]),
    ("orts.AngularVelocity3D", &["wx", "wy", "wz"]),
    ("orts.MtqCommand3D", &["mtq_mx", "mtq_my", "mtq_mz"]),
    ("orts.RwTorqueCommand3D", &["rw_tx", "rw_ty", "rw_tz"]),
    ("orts.RwMomentum3D", &["rw_hx", "rw_hy", "rw_hz"]),
];

/// Look up scalar data, trying both with and without leading slash.
fn get_scalar_data<'a>(
    scalars: &'a BTreeMap<String, Column>,
    base: &str,
    field: &str,
) -> Option<&'a Column> {
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
    use crate::record::components::{
        BodyRadius, GravitationalParameter, Position3D, Quaternion4D, Velocity3D,
    };
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

    /// Recording → rrd → load_as_recording roundtrip preserves all components.
    #[test]
    fn roundtrip_recording_all_components() {
        use crate::record::components::*;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/test");

        rec.metadata = SimMetadata {
            epoch_jd: Some(2461149.0),
            epoch_iso: Some("2026-04-18T12:00:00Z".to_string()),
            mu: Some(398600.4418),
            body_radius: Some(6378.137),
            body_name: Some("Earth".to_string()),
            altitude: Some(400.0),
            period: Some(5553.6),
            orbit_description: Some(
                "Initial orbit: circular at 400 km altitude (r = 6778.137 km)".to_string(),
            ),
        };

        // Log 3 rows with all component types
        for i in 0..3u64 {
            let t = i as f64 * 10.0;
            let tp = TimePoint::new().with_sim_time(t).with_step(i);

            let os = OrbitalState::new(
                Vector3::new(6778.0 + t, t * 0.1, 0.0),
                Vector3::new(0.0, 7.67 - t * 0.001, 0.0),
            );
            let q = Quaternion4D(nalgebra::Vector4::new(1.0 - t * 0.01, t * 0.005, 0.0, 0.0));
            let w = AngularVelocity3D(Vector3::new(0.05 - t * 0.001, -0.03, 0.04));
            rec.log_orbital_state_with_attitude(&sat, &tp, &os, Some(&q), Some(&w));

            rec.log_temporal(&sat, &tp, &MtqCommand3D(Vector3::new(t, -t * 0.5, t * 0.3)));
            rec.log_temporal(
                &sat,
                &tp,
                &RwTorqueCommand3D(Vector3::new(0.1 * t, 0.0, -0.1 * t)),
            );
            rec.log_temporal(&sat, &tp, &RwMomentum3D(Vector3::new(t * 0.01, 0.0, 0.0)));
        }

        // Save to rrd
        let path = std::env::temp_dir().join("test_orts_roundtrip_all.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-roundtrip", path_str).expect("save failed");

        // Load back as Recording
        let loaded = load_as_recording(path_str).expect("load failed");

        // Verify metadata roundtrip
        let m = &loaded.metadata;
        assert!((m.epoch_jd.unwrap() - 2461149.0).abs() < 1e-6);
        assert_eq!(m.epoch_iso.as_deref(), Some("2026-04-18T12:00:00Z"));
        assert!((m.mu.unwrap() - 398600.4418).abs() < 1e-6);
        assert!((m.body_radius.unwrap() - 6378.137).abs() < 1e-6);
        assert_eq!(m.body_name.as_deref(), Some("Earth"));
        assert_eq!(
            m.orbit_description.as_deref(),
            Some("Initial orbit: circular at 400 km altitude (r = 6778.137 km)")
        );

        // Find the satellite entity
        let sat_path = EntityPath::parse("/world/sat/test");
        let store = loaded.entity(&sat_path).expect("entity not found");

        // Verify all component columns exist with correct row count
        assert_eq!(store.num_rows, 3, "expected 3 rows");

        let pos = store
            .columns
            .get(&Position3D::component_name())
            .expect("Position3D missing");
        assert_eq!(pos.num_rows(), 3);

        let vel = store
            .columns
            .get(&Velocity3D::component_name())
            .expect("Velocity3D missing");
        assert_eq!(vel.num_rows(), 3);

        let quat = store
            .columns
            .get(&Quaternion4D::component_name())
            .expect("Quaternion4D missing");
        assert_eq!(quat.num_rows(), 3);

        let omega = store
            .columns
            .get(&AngularVelocity3D::component_name())
            .expect("AngularVelocity3D missing");
        assert_eq!(omega.num_rows(), 3);

        let mtq = store
            .columns
            .get(&MtqCommand3D::component_name())
            .expect("MtqCommand3D missing");
        assert_eq!(mtq.num_rows(), 3);

        let rw_torque = store
            .columns
            .get(&RwTorqueCommand3D::component_name())
            .expect("RwTorqueCommand3D missing");
        assert_eq!(rw_torque.num_rows(), 3);

        let rw_mom = store
            .columns
            .get(&RwMomentum3D::component_name())
            .expect("RwMomentum3D missing");
        assert_eq!(rw_mom.num_rows(), 3);

        // Verify data values for first row
        let pos0 = pos.get_row(0).unwrap();
        assert!(
            (pos0[0] - 6778.0).abs() < 1e-6,
            "pos x mismatch: {}",
            pos0[0]
        );

        let q0 = quat.get_row(0).unwrap();
        assert!((q0[0] - 1.0).abs() < 1e-6, "qw mismatch: {}", q0[0]);

        let w0 = omega.get_row(0).unwrap();
        assert!((w0[0] - 0.05).abs() < 1e-6, "wx mismatch: {}", w0[0]);

        // Verify data values for last row (i=2, t=20)
        let mtq2 = mtq.get_row(2).unwrap();
        assert!((mtq2[0] - 20.0).abs() < 1e-6, "mtq_mx at t=20: {}", mtq2[0]);

        // Verify timeline
        use crate::record::timeline::{TimeIndex, TimelineName};
        let sim_times = store
            .timelines
            .get(&TimelineName::SimTime)
            .expect("SimTime timeline missing");
        assert_eq!(sim_times.len(), 3);
        match &sim_times[0] {
            TimeIndex::Seconds(t) => assert!((t - 0.0).abs() < 1e-9),
            other => panic!("expected Seconds, got {:?}", other),
        }
        match &sim_times[2] {
            TimeIndex::Seconds(t) => assert!((t - 20.0).abs() < 1e-9),
            other => panic!("expected Seconds, got {:?}", other),
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

    /// Write an .rrd with `write`, then load it back.
    ///
    /// Writes through `re_sdk` directly rather than through [`save_as_rrd`],
    /// because that is the only way to put a sparse column in the file:
    /// `ComponentColumn` carries no timeline row, so `save_as_rrd` writes a
    /// component that is present at only some steps at the wrong times. What
    /// is under test here is the loader, so the file it reads has to be right.
    ///
    /// The recording lives in a directory of its own, removed when the call
    /// returns, so tests running in parallel cannot meet on one path.
    fn load_written(write: impl FnOnce(&re_sdk::RecordingStream)) -> Vec<RrdRow> {
        let dir = std::env::temp_dir().join(format!(
            "orts_rrd_load_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("test.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-loader-test")
                .save(&path)
                .expect("recording stream");
            write(&rec);
            rec.flush_blocking().expect("flush");
        }
        let rows = load_from_rrd(path.to_str().unwrap()).expect("failed to load .rrd");
        std::fs::remove_dir_all(&dir).ok();
        rows
    }

    /// Log one scalar per named field at the stream's current time.
    fn log_fields(rec: &re_sdk::RecordingStream, fields: &[(&str, f64)]) {
        for (field, value) in fields {
            rec.log(
                format!("/world/sat/loader/{field}"),
                &re_sdk_types::archetypes::Scalars::new([*value]),
            )
            .expect("log scalar");
        }
    }

    /// A column present at only some of the times does not shift its values
    /// onto earlier rows.
    ///
    /// The loader read every column at the position `x` happened to be at, so a
    /// `y` present only at t=10 landed on the t=0 row and t=10 read `y = 0.0`.
    /// Reached through `orts convert` and `orts replay` on a recording written
    /// elsewhere, and through any column this repository later logs
    /// conditionally.
    #[test]
    fn a_sparse_column_stays_on_its_own_time() {
        let rows = load_written(|rec| {
            for (t, fields) in [
                (0.0f64, vec![("x", 100.0f64), ("z", 0.0)]),
                (10.0, vec![("x", 110.0), ("y", 201.0), ("z", 0.0)]),
            ] {
                rec.set_duration_secs("sim_time", t);
                log_fields(rec, &fields);
            }
        });

        // The t=0 row has no `y`, so it is not a position and is dropped. The
        // t=10 row is the one that carries 201.0.
        assert_eq!(rows.len(), 1, "expected only the complete row: {rows:?}");
        assert!((rows[0].t - 10.0).abs() < 1e-9, "t = {}", rows[0].t);
        assert!((rows[0].y - 201.0).abs() < 1e-9, "y = {}", rows[0].y);
    }

    /// Distinct values per time arrive on their own rows.
    ///
    /// The round-trip test above writes the same state five times, so an
    /// index-based join passes it. Every value here differs, so a shifted
    /// column shows up as a wrong number.
    #[test]
    fn every_time_keeps_its_own_values() {
        let rows = load_written(|rec| {
            for i in 0..4u64 {
                let t = i as f64 * 10.0;
                rec.set_duration_secs("sim_time", t);
                log_fields(
                    rec,
                    &[
                        ("x", 1000.0 + t),
                        ("y", 2000.0 + t),
                        ("z", 3000.0 + t),
                        ("vx", 1.0 + t / 100.0),
                        ("vy", 2.0 + t / 100.0),
                        ("vz", 3.0 + t / 100.0),
                    ],
                );
            }
        });

        assert_eq!(rows.len(), 4, "expected 4 rows, got {}", rows.len());
        for (i, row) in rows.iter().enumerate() {
            let t = i as f64 * 10.0;
            assert!((row.t - t).abs() < 1e-9, "row {i} at t={}", row.t);
            assert!((row.x - (1000.0 + t)).abs() < 1e-9, "x at t={t}: {}", row.x);
            assert!((row.y - (2000.0 + t)).abs() < 1e-9, "y at t={t}: {}", row.y);
            assert!((row.z - (3000.0 + t)).abs() < 1e-9, "z at t={t}: {}", row.z);
            assert!(
                (row.vx - (1.0 + t / 100.0)).abs() < 1e-9,
                "vx at t={t}: {}",
                row.vx
            );
            assert!(
                (row.vy - (2.0 + t / 100.0)).abs() < 1e-9,
                "vy at t={t}: {}",
                row.vy
            );
            assert!(
                (row.vz - (3.0 + t / 100.0)).abs() < 1e-9,
                "vz at t={t}: {}",
                row.vz
            );
        }
    }

    /// Attitude present from the second time on does not attach to the first
    /// row.
    #[test]
    fn attitude_logged_late_does_not_attach_to_the_first_row() {
        let rows = load_written(|rec| {
            for i in 0..3u64 {
                let t = i as f64 * 10.0;
                rec.set_duration_secs("sim_time", t);
                let mut fields = vec![
                    ("x", 7000.0 + t),
                    ("y", 0.0),
                    ("z", 0.0),
                    ("vx", 0.0),
                    ("vy", 7.5),
                    ("vz", 0.0),
                ];
                if i > 0 {
                    // A quaternion distinguishable per time.
                    fields.extend([
                        ("qw", 1.0),
                        ("qx", 0.1 * i as f64),
                        ("qy", 0.0),
                        ("qz", 0.0),
                    ]);
                }
                log_fields(rec, &fields);
            }
        });

        let first = rows
            .iter()
            .find(|r| r.t < 5.0)
            .expect("the t=0 row is a complete state vector");
        assert!(
            first.quaternion.is_none(),
            "a later quaternion landed on the t=0 row: {:?}",
            first.quaternion
        );

        let second = rows
            .iter()
            .find(|r| (r.t - 10.0).abs() < 1e-9)
            .expect("the t=10 row is present");
        let q = second.quaternion.expect("t=10 logged a quaternion");
        assert!((q[1] - 0.1).abs() < 1e-9, "t=10 quaternion: {q:?}");
    }

    /// A moment whose columns hold different numbers of values yields no row.
    ///
    /// Repeat ordinals are assigned per column, so they identify a row only
    /// while every required column has the same count at that moment. With two
    /// samples at t=0 where the first omits `y`, joining on the ordinal paired
    /// the first `x` with the second sample's `y` — the same shift this join set
    /// out to remove, one level in.
    #[test]
    fn a_time_whose_columns_disagree_on_repeats_yields_no_row() {
        let rows = load_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_fields(rec, &[("x", 100.0), ("z", 0.0)]);
            log_fields(rec, &[("x", 110.0), ("y", 201.0), ("z", 0.0)]);
        });
        assert!(
            rows.is_empty(),
            "ordinals cannot pair these values; expected no rows, got {rows:?}"
        );
    }

    /// A recording with no timeline at all still decodes, keyed by position.
    ///
    /// Nothing `orts` writes lands here — every recording it produces carries
    /// `sim_time`, `step` or both — but a file read through `orts convert` can,
    /// and the fallback has to stay reachable.
    #[test]
    fn a_recording_with_no_timeline_falls_back_to_column_order() {
        let rows = load_written(|rec| {
            log_fields(rec, &[("x", 100.0), ("y", 200.0), ("z", 300.0)]);
            log_fields(rec, &[("x", 110.0), ("y", 210.0), ("z", 310.0)]);
        });
        assert_eq!(rows.len(), 2, "{rows:?}");
        for row in &rows {
            assert_eq!(row.t, 0.0, "an untimed row reports t = 0");
        }
        assert_eq!(rows[0].x, 100.0);
        assert_eq!(rows[1].x, 110.0);
    }

    /// The optional columns count toward the repeat check too.
    ///
    /// Two complete states at one moment with attitude on the second only: the
    /// quaternion holds repeat 0, so joining on the ordinal attached it to the
    /// first state and left the second without one.
    #[test]
    fn an_optional_column_disagreeing_on_repeats_yields_no_row() {
        let rows = load_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_fields(rec, &[("x", 100.0), ("y", 0.0), ("z", 0.0)]);
            log_fields(
                rec,
                &[
                    ("x", 110.0),
                    ("y", 0.0),
                    ("z", 0.0),
                    ("qw", 1.0),
                    ("qx", 0.5),
                    ("qy", 0.0),
                    ("qz", 0.0),
                ],
            );
        });
        assert!(
            rows.is_empty(),
            "the quaternion cannot be assigned to either state; got {rows:?}"
        );
    }

    /// A moment with one value per column is never ambiguous, whichever
    /// optional columns are absent.
    #[test]
    fn a_single_sample_with_absent_optional_columns_still_yields_a_row() {
        let rows = load_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_fields(rec, &[("x", 100.0), ("y", 200.0), ("z", 300.0)]);
        });
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].x, 100.0);
        assert!(rows[0].quaternion.is_none());
    }

    /// `load_as_recording` joins on the time index too.
    ///
    /// A third copy of the same index join lived here, reached by
    /// `orts convert`. It read each field at its own position, so a field
    /// present at only some of the times put its values on earlier rows:
    /// `Position3D` came back as `[100.0, 201.0, 0.0]`, pairing the t=0 `x` with
    /// the t=10 `y`.
    #[test]
    fn load_as_recording_joins_a_sparse_column_by_time() {
        let dir = std::env::temp_dir().join(format!(
            "orts_lar_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sparse.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-lar-test")
                .save(&path)
                .expect("recording stream");
            for (t, fields) in [
                (0.0f64, vec![("x", 100.0f64), ("z", 0.0)]),
                (10.0, vec![("x", 110.0), ("y", 201.0), ("z", 0.0)]),
            ] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in fields {
                    rec.log(
                        format!("/world/sat/loader/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = loaded
            .entity_paths()
            .next()
            .cloned()
            .expect("one entity was written");
        let store = loaded.entity(&entity).expect("entity");
        let column = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, col)| col)
            .expect("a position column");

        // `ComponentColumn` cannot say "no value here", so t=0 — whose position
        // lacks `y` — is no row at all rather than a row with a zeroed axis.
        assert_eq!(store.num_rows, 1, "only t=10 has a whole position");
        assert_eq!(
            store.timelines.get(&TimelineName::SimTime),
            Some(&vec![TimeIndex::Seconds(10.0)])
        );
        let row = column.get_row(0).expect("the one row");
        assert_eq!(
            row,
            &[110.0, 201.0, 0.0],
            "the row must be one time's values, not a mix"
        );
    }

    /// Every component column lines up with the entity's one timeline.
    ///
    /// `EntityStore` keeps a single timeline shared by all of an entity's
    /// columns, so a component that compacted onto its own surviving times left
    /// the columns disagreeing on what a row means. Measured with position
    /// whole only at t=10 and velocity at both: the timeline came back as
    /// `[10.0]` while `num_rows` was 2, and row 0 held the t=10 position beside
    /// the t=0 velocity.
    #[test]
    fn every_component_column_matches_the_entity_timeline() {
        let dir = std::env::temp_dir().join(format!(
            "orts_tl_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("mixed.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-timeline-test")
                .save(&path)
                .expect("recording stream");
            for (t, fields) in [
                // t=0: velocity whole, position missing y.
                (
                    0.0f64,
                    vec![
                        ("x", 100.0f64),
                        ("z", 0.0),
                        ("vx", 1.0),
                        ("vy", 2.0),
                        ("vz", 3.0),
                    ],
                ),
                // t=10: both whole.
                (
                    10.0,
                    vec![
                        ("x", 110.0),
                        ("y", 201.0),
                        ("z", 0.0),
                        ("vx", 4.0),
                        ("vy", 5.0),
                        ("vz", 6.0),
                    ],
                ),
            ] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in fields {
                    rec.log(
                        format!("/world/sat/loader/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = loaded.entity_paths().next().cloned().expect("one entity");
        let store = loaded.entity(&entity).expect("entity");

        let times = store
            .timelines
            .get(&TimelineName::SimTime)
            .expect("a sim_time timeline");
        assert_eq!(times.len(), store.num_rows, "timeline and rows must agree");
        assert_eq!(times, &vec![TimeIndex::Seconds(10.0)]);
        for (name, col) in &store.columns {
            assert_eq!(
                col.num_rows(),
                store.num_rows,
                "{name} has {} rows, entity has {}",
                col.num_rows(),
                store.num_rows
            );
        }

        // The one row is t=10 in both columns, which is what a shared timeline
        // promises.
        let position = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("a position column");
        let velocity = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Velocity3D"))
            .map(|(_, c)| c)
            .expect("a velocity column");
        assert_eq!(position.get_row(0).expect("row 0"), &[110.0, 201.0, 0.0]);
        assert_eq!(velocity.get_row(0).expect("row 0"), &[4.0, 5.0, 6.0]);
    }

    /// A moment whose fields disagree on repeat count yields no row.
    ///
    /// Repeat numbers are per field, so two samples at one time of which the
    /// first omits `y` made repeat 0 the first sample's `x` beside the second's
    /// `y`: measured as `Position3D = [100.0, 201.0, 0.0]`. That moment is left
    /// out entirely, as in the other two decoders.
    #[test]
    fn load_as_recording_drops_a_time_whose_fields_disagree_on_repeats() {
        let dir = std::env::temp_dir().join(format!(
            "orts_lar_rep_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("repeats.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-lar-repeat-test")
                .save(&path)
                .expect("recording stream");
            rec.set_duration_secs("sim_time", 0.0);
            for sample in [
                // The first sample at t=0 omits `y`.
                vec![("x", 100.0f64), ("z", 0.0)],
                vec![("x", 110.0), ("y", 201.0), ("z", 0.0)],
            ] {
                for (field, value) in sample {
                    rec.log(
                        format!("/world/sat/loader/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = loaded.entity_paths().next().cloned().expect("one entity");
        let store = loaded.entity(&entity).expect("entity");
        assert_eq!(
            store.num_rows, 0,
            "the one moment is unpairable, so it yields no row"
        );
        for (name, col) in &store.columns {
            assert_eq!(col.num_rows(), 0, "{name} must be empty too");
        }
    }

    /// Only this entity's own timed fields decide its rows.
    ///
    /// The row keys were the union over every scalar key under the entity's
    /// path, which reaches further than the entity: a static field has no
    /// timeline and a child entity has times of its own. Measured with a static
    /// `mass`, a position at t=10 and t=20, and a child at t=11 and t=12: five
    /// rows came back where two are recorded, three of them a position of
    /// `[0.0, 0.0, 0.0]` that was never logged.
    #[test]
    fn a_static_field_and_a_child_entity_add_no_rows() {
        let dir = std::env::temp_dir().join(format!(
            "orts_scope_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("scope.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-scope-test")
                .save(&path)
                .expect("recording stream");
            rec.log_static(
                "/world/sat/scoped/mass",
                &re_sdk_types::archetypes::Scalars::new([12.5f64]),
            )
            .expect("log static");
            for (t, x) in [(10.0f64, 100.0f64), (20.0, 110.0)] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in [("x", x), ("y", 1.0), ("z", 2.0)] {
                    rec.log(
                        format!("/world/sat/scoped/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            // A child entity, logged at times the parent never records.
            for t in [11.0f64, 12.0] {
                rec.set_duration_secs("sim_time", t);
                rec.log(
                    "/world/sat/scoped/child/x",
                    &re_sdk_types::archetypes::Scalars::new([7.0f64]),
                )
                .expect("log child");
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/scoped");
        let store = loaded.entity(&entity).expect("the parent entity");
        assert_eq!(
            store.timelines.get(&TimelineName::SimTime),
            Some(&vec![TimeIndex::Seconds(10.0), TimeIndex::Seconds(20.0)]),
            "only the parent's own times are rows"
        );
        assert_eq!(store.num_rows, 2);
        let position = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("a position column");
        assert_eq!(position.get_row(0).expect("row 0"), &[100.0, 1.0, 2.0]);
        assert_eq!(position.get_row(1).expect("row 1"), &[110.0, 1.0, 2.0]);
    }

    /// A component recorded at only some of the times is left out.
    ///
    /// Filling its absent rows with zeros would put an attitude that was never
    /// logged into the CSV `orts convert` writes, where a zero quaternion is
    /// indistinguishable from a measured one. Dropping those rows instead would
    /// cost the trajectory, so the rows follow position and velocity and the
    /// optional component is the part that goes.
    #[test]
    fn a_partially_recorded_optional_component_is_left_out() {
        let dir = std::env::temp_dir().join(format!(
            "orts_opt_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("optional.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-optional-test")
                .save(&path)
                .expect("recording stream");
            for (t, with_attitude) in [(0.0f64, false), (10.0, true)] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in [("x", t), ("y", 1.0), ("z", 2.0)] {
                    rec.log(
                        format!("/world/sat/opt/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
                if with_attitude {
                    for (field, value) in [("qw", 1.0f64), ("qx", 0.0), ("qy", 0.0), ("qz", 0.0)] {
                        rec.log(
                            format!("/world/sat/opt/{field}"),
                            &re_sdk_types::archetypes::Scalars::new([value]),
                        )
                        .expect("log");
                    }
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/opt");
        let store = loaded.entity(&entity).expect("entity");
        assert_eq!(store.num_rows, 2, "both positions are kept");
        assert!(
            store.columns.keys().any(|name| name.contains("Position3D")),
            "the trajectory survives"
        );
        assert!(
            !store
                .columns
                .keys()
                .any(|name| name.contains("Quaternion4D")),
            "an attitude present at only one of the two times is not reported"
        );
    }

    /// A recording indexed by a timeline of its own naming still joins on time.
    ///
    /// `sim_time` and `step` are the names `orts` writes; another tool names its
    /// own, and treating that as no timeline at all fell back to column
    /// position. Measured with `y` recorded at frame 2 alone: `Position3D` came
    /// back as `[100.0, 201.0, 0.0]`, the frame-1 `x` beside the frame-2 `y`.
    #[test]
    fn a_recording_on_its_own_named_timeline_joins_on_it() {
        let dir = std::env::temp_dir().join(format!(
            "orts_ctl_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("named.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-named-timeline-test")
                .save(&path)
                .expect("recording stream");
            for (frame, fields) in [
                (1i64, vec![("x", 100.0f64), ("z", 0.0)]),
                (2, vec![("x", 110.0), ("y", 201.0), ("z", 0.0)]),
            ] {
                rec.set_time_sequence("frame", frame);
                for (field, value) in fields {
                    rec.log(
                        format!("/world/sat/named/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/named");
        let store = loaded.entity(&entity).expect("entity");
        let position = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("a position column");
        assert_eq!(store.num_rows, 1, "only frame 2 has a whole position");
        assert_eq!(
            position.get_row(0).expect("the one row"),
            &[110.0, 201.0, 0.0],
            "the row must be one frame's values, not a mix"
        );
    }

    /// Two axes of the recording's own naming do not share a row.
    ///
    /// A `frame` of 1 and an `iteration` of 1 are separate dimensions, but the
    /// fallback kept only the raw value, so both became the same key. Measured
    /// with `x` and `z` on `frame` and `y` on `iteration`: `Position3D` came
    /// back as `[100.0, 999.0, 300.0]`, assembled from two axes. One axis serves
    /// the whole recording, so neither half is a whole position and the entity
    /// gets no row.
    #[test]
    fn a_second_named_axis_does_not_join_with_the_first() {
        let dir = std::env::temp_dir().join(format!(
            "orts_axis_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("axes.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-two-axes-test")
                .save(&path)
                .expect("recording stream");
            rec.set_time_sequence("frame", 1i64);
            for (field, value) in [("x", 100.0f64), ("z", 300.0)] {
                rec.log(
                    format!("/world/sat/axes/{field}"),
                    &re_sdk_types::archetypes::Scalars::new([value]),
                )
                .expect("log");
            }
            rec.disable_timeline("frame");
            rec.set_time_sequence("iteration", 1i64);
            rec.log(
                "/world/sat/axes/y",
                &re_sdk_types::archetypes::Scalars::new([999.0f64]),
            )
            .expect("log");
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        // With no whole position on either axis the entity has no temporal
        // component at all, so it may be absent rather than empty.
        let entity = EntityPath::parse("/world/sat/axes");
        let rows = loaded.entity(&entity).map_or(0, |store| store.num_rows);
        assert_eq!(
            rows, 0,
            "no axis carries a whole position, so there is no row to report"
        );
    }

    /// A component the file never carries costs only that component.
    ///
    /// The row keys come from the fields the anchors actually have a column
    /// for. With a schema declaring `Position3D(x, y, z)` over a file that
    /// never logged `y`, the rows follow velocity alone: position is left out,
    /// since it is never whole, and the velocity that is there keeps its own
    /// times rather than being discarded with it.
    ///
    /// This is also the schema path itself, which `meta/schema/<entity>` selects
    /// over the field-name table.
    #[test]
    fn a_component_the_file_never_carries_costs_only_that_component() {
        let dir = std::env::temp_dir().join(format!(
            "orts_anchor_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("anchor.rrd");
        let schema = r#"[
            {"name":"orts.Position3D","fields":["x","y","z"]},
            {"name":"orts.Velocity3D","fields":["vx","vy","vz"]}
        ]"#;
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-anchor-test")
                .save(&path)
                .expect("recording stream");
            rec.log_static(
                "/meta/schema/world/sat/anchored",
                &re_sdk_types::archetypes::TextDocument::new(schema),
            )
            .expect("log schema");
            for (t, x, vx) in [(0.0f64, 100.0f64, 1.0f64), (10.0, 110.0, 4.0)] {
                rec.set_duration_secs("sim_time", t);
                // `y` is never logged, so `Position3D` has no whole row anywhere.
                for (field, value) in [("x", x), ("z", 0.0), ("vx", vx), ("vy", 2.0), ("vz", 3.0)] {
                    rec.log(
                        format!("/world/sat/anchored/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/anchored");
        let store = loaded.entity(&entity).expect("entity");
        assert!(
            !store.columns.keys().any(|name| name.contains("Position3D")),
            "a position that is never whole is not reported"
        );
        let velocity = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Velocity3D"))
            .map(|(_, c)| c)
            .expect("the velocity the file does carry");
        assert_eq!(store.num_rows, 2);
        assert_eq!(
            store.timelines.get(&TimelineName::SimTime),
            Some(&vec![TimeIndex::Seconds(0.0), TimeIndex::Seconds(10.0)])
        );
        assert_eq!(velocity.get_row(0).expect("row 0"), &[1.0, 2.0, 3.0]);
        assert_eq!(velocity.get_row(1).expect("row 1"), &[4.0, 2.0, 3.0]);
    }

    /// An optional component disagreeing on repeats leaves the moment out.
    ///
    /// The repeat count was checked across the anchor columns alone, so three
    /// attitudes beside two states passed: the quaternion has keys for repeats
    /// 0 and 1, and the first two attitudes were attached as if they were those
    /// states'. Which attitude belongs to which state is unknowable, so the
    /// moment yields no row.
    #[test]
    fn an_optional_component_disagreeing_on_repeats_leaves_the_moment_out() {
        let dir = std::env::temp_dir().join(format!(
            "orts_optrep_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("optrep.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-optional-repeat-test")
                .save(&path)
                .expect("recording stream");
            rec.set_duration_secs("sim_time", 0.0);
            // Two whole states at one time.
            for x in [100.0f64, 110.0] {
                for (field, value) in [("x", x), ("y", 1.0), ("z", 2.0)] {
                    rec.log(
                        format!("/world/sat/optrep/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            // Three attitudes at the same time.
            for qw in [1.0f64, 0.9, 0.8] {
                for (field, value) in [("qw", qw), ("qx", 0.0), ("qy", 0.0), ("qz", 0.0)] {
                    rec.log(
                        format!("/world/sat/optrep/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/optrep");
        let rows = loaded.entity(&entity).map_or(0, |store| store.num_rows);
        assert_eq!(
            rows, 0,
            "two states beside three attitudes cannot be paired, so neither is reported"
        );
    }

    /// A named axis is never a step of the same value.
    ///
    /// The fallback put the axis value in the same slot as a real `step`, so a
    /// `frame` of 1 and a `step` of 1 became one key. Measured with `x` and `z`
    /// at `step = 1` and `y` at `frame = 1`: `Position3D` came back as
    /// `[100.0, 999.0, 300.0]`, a position assembled across the two.
    #[test]
    fn a_named_axis_is_not_a_step_of_the_same_value() {
        let dir = std::env::temp_dir().join(format!(
            "orts_coll_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("collide.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-axis-step-test")
                .save(&path)
                .expect("recording stream");
            rec.set_time_sequence("step", 1i64);
            for (field, value) in [("x", 100.0f64), ("z", 300.0)] {
                rec.log(
                    format!("/world/sat/coll/{field}"),
                    &re_sdk_types::archetypes::Scalars::new([value]),
                )
                .expect("log");
            }
            rec.disable_timeline("step");
            rec.set_time_sequence("frame", 1i64);
            rec.log(
                "/world/sat/coll/y",
                &re_sdk_types::archetypes::Scalars::new([999.0f64]),
            )
            .expect("log");
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/coll");
        let rows = loaded.entity(&entity).map_or(0, |store| store.num_rows);
        assert_eq!(
            rows, 0,
            "neither the step nor the frame carries a whole position"
        );
    }

    /// A component the file cannot carry does not narrow another's rows.
    ///
    /// Rows follow the state components, and a component missing one of its
    /// fields was still among them: its remaining fields narrowed the rows to
    /// the times they happen to hold. Measured with `y` absent from the file,
    /// `x` and `z` at t=0, and a whole velocity at t=0 and t=10: one row came
    /// back, the t=10 velocity dropped for a position that is never reported
    /// either way.
    #[test]
    fn a_component_the_file_cannot_carry_does_not_narrow_another() {
        let dir = std::env::temp_dir().join(format!(
            "orts_narrow_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("narrow.rrd");
        let schema = r#"[
            {"name":"orts.Position3D","fields":["x","y","z"]},
            {"name":"orts.Velocity3D","fields":["vx","vy","vz"]}
        ]"#;
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-narrow-test")
                .save(&path)
                .expect("recording stream");
            rec.log_static(
                "/meta/schema/world/sat/narrowed",
                &re_sdk_types::archetypes::TextDocument::new(schema),
            )
            .expect("log schema");
            // `y` is never logged, and `x` and `z` appear at t=0 alone.
            rec.set_duration_secs("sim_time", 0.0);
            for (field, value) in [
                ("x", 100.0f64),
                ("z", 300.0),
                ("vx", 1.0),
                ("vy", 2.0),
                ("vz", 3.0),
            ] {
                rec.log(
                    format!("/world/sat/narrowed/{field}"),
                    &re_sdk_types::archetypes::Scalars::new([value]),
                )
                .expect("log");
            }
            rec.set_duration_secs("sim_time", 10.0);
            for (field, value) in [("vx", 4.0f64), ("vy", 5.0), ("vz", 6.0)] {
                rec.log(
                    format!("/world/sat/narrowed/{field}"),
                    &re_sdk_types::archetypes::Scalars::new([value]),
                )
                .expect("log");
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/narrowed");
        let store = loaded.entity(&entity).expect("entity");
        assert!(
            !store.columns.keys().any(|name| name.contains("Position3D")),
            "a position without a `y` column is not reported"
        );
        assert_eq!(store.num_rows, 2, "the velocity keeps both of its times");
        assert_eq!(
            store.timelines.get(&TimelineName::SimTime),
            Some(&vec![TimeIndex::Seconds(0.0), TimeIndex::Seconds(10.0)])
        );
        let velocity = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Velocity3D"))
            .map(|(_, c)| c)
            .expect("a velocity column");
        assert_eq!(velocity.get_row(0).expect("row 0"), &[1.0, 2.0, 3.0]);
        assert_eq!(velocity.get_row(1).expect("row 1"), &[4.0, 5.0, 6.0]);
    }
}
