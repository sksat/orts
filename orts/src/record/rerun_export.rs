use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use re_chunk::TimeColumn;

use crate::record::component::Component;
use crate::record::components::Position3D;
use crate::record::entity_path::EntityPath;
use crate::record::recording::{
    ComponentColumn, EntityStore, Recording, RowMap, SimMetadata, TimelineColumn,
};
use crate::record::timeline::{TimeIndex, TimelineName};

/// Rows per columnar chunk.
///
/// `send_columns` bypasses the batcher, so the chunk boundaries are ours to pick.
/// The batcher's own bound on sorted data is `flush_num_bytes`, 2 MiB by default;
/// a row here costs a scalar, a list offset, a row id and the two index values, so
/// 8192 rows stay well inside that while keeping a long `orts run` from emitting
/// one multi-million-row chunk that a reader has to materialize whole before it can
/// show anything.
const CHUNK_ROWS: usize = 8192;

/// The index columns for one entity, as plain values.
///
/// Only `sim_time` (a duration) and `step` (a sequence) are written, which is what
/// the export has always written; `WallClock` and `Custom` timelines have no
/// read-back contract yet.
struct EntityIndex {
    /// `sim_time` seconds per logical row; `None` where this axis covers no row.
    sim_time_secs: Vec<Option<f64>>,
    /// `step` per logical row.
    steps: Vec<Option<i64>>,
}

impl EntityIndex {
    fn from_store(store: &EntityStore) -> Self {
        let n_rows = store.num_rows;
        let mut sim_time_secs = vec![None; n_rows];
        let mut steps = vec![None; n_rows];

        // Address both axes by logical row. An axis need not cover every row (a
        // `TimePoint` need not name it), and a `TimeIndex` of the wrong variant
        // cannot go on that axis at all, so both show up as a `None` row.
        if let Some(axis) = store.timelines.get(&TimelineName::SimTime) {
            for (stored, index) in axis.data.iter().enumerate() {
                if let (TimeIndex::Seconds(t), Some(slot)) =
                    (index, sim_time_secs.get_mut(axis.logical_row_of(stored)))
                {
                    *slot = Some(*t);
                }
            }
        }
        if let Some(axis) = store.timelines.get(&TimelineName::Step) {
            for (stored, index) in axis.data.iter().enumerate() {
                if let (TimeIndex::Sequence(s), Some(slot)) =
                    (index, steps.get_mut(axis.logical_row_of(stored)))
                {
                    *slot = i64::try_from(*s).ok();
                }
            }
        }

        Self {
            sim_time_secs,
            steps,
        }
    }

    /// Which axes have a value on logical row `row`.
    ///
    /// Chunks are cut where this changes, so every row goes out on the axes that
    /// cover it instead of the whole chunk being dropped for want of one axis.
    fn axes_at(&self, row: usize) -> AxisMask {
        AxisMask {
            sim_time: self.sim_time_secs.get(row).copied().flatten().is_some(),
            step: self.steps.get(row).copied().flatten().is_some(),
        }
    }

    /// Index columns covering exactly `logical_rows`, in that order.
    ///
    /// An axis with no value on one of those rows is left out rather than
    /// written misaligned. `None` means no axis covers them all, and the caller
    /// must skip the rows: `send_columns` reads an empty index list as *static*
    /// data, which would shadow every temporal value at the same path.
    fn time_columns(&self, logical_rows: &[usize]) -> Option<Vec<TimeColumn>> {
        let mut columns = Vec::with_capacity(2);
        if let Some(secs) = gather(&self.sim_time_secs, logical_rows) {
            columns.push(TimeColumn::new_duration_secs("sim_time", secs));
        }
        if let Some(steps) = gather(&self.steps, logical_rows) {
            columns.push(TimeColumn::new_sequence("step", steps));
        }
        (!columns.is_empty()).then_some(columns)
    }
}

/// Which of the written axes cover a row.
///
/// `Copy` because the loop that cuts chunks holds one while walking the rows.
#[derive(Clone, Copy, PartialEq)]
struct AxisMask {
    sim_time: bool,
    step: bool,
}

/// The axis values for `rows`, or `None` if the axis misses any of them.
fn gather<T: Copy>(axis: &[Option<T>], rows: &[usize]) -> Option<Vec<T>> {
    rows.iter()
        .map(|row| axis.get(*row).copied().flatten())
        .collect()
}

/// The entity logical row of each stored row in `column`.
fn logical_rows_of(column: &ComponentColumn) -> Vec<usize> {
    (0..column.num_rows())
        .map(|stored| column.logical_row_of(stored))
        .collect()
}

/// Split the stored rows into ranges of at most [`CHUNK_ROWS`] that also share
/// one set of usable axes.
///
/// A run of rows an axis covers can end partway through a column, and one chunk
/// carries one set of index columns, so the cut has to follow the axes.
///
/// A recording whose `TimePoint` names different axes on every row therefore
/// gets a chunk per row, which costs what the columnar export saved. Every
/// caller in this repository names the same axes at every step.
fn chunk_ranges_by_axes(index: &EntityIndex, logical_rows: &[usize]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < logical_rows.len() {
        let axes = index.axes_at(logical_rows[start]);
        let limit = (start + CHUNK_ROWS).min(logical_rows.len());
        let mut end = start + 1;
        while end < limit && index.axes_at(logical_rows[end]) == axes {
            end += 1;
        }
        ranges.push(start..end);
        start = end;
    }
    ranges
}

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

        // Log temporal data (generic: iterate all component columns). Each field is
        // sent as whole columns, so the call count scales with the number of fields
        // rather than with the number of rows.
        let index = EntityIndex::from_store(store);

        for (comp_name, column) in &store.columns {
            let fields = recording.lookup_component_fields(comp_name);
            let scalars_per_row = column.scalars_per_row;
            // `lookup_component_fields` names one synthetic field for an
            // unregistered component, and a registered one may name fewer fields
            // than the column is wide. Export exactly the fields that are named and
            // that the column actually holds.
            let n_fields = fields.len().min(scalars_per_row);
            if n_fields == 0 {
                continue;
            }
            let paths: Vec<String> = fields[..n_fields]
                .iter()
                .map(|field| format!("{rr_path}/{field}"))
                .collect();

            // The times come from the logical rows this column actually covers, so
            // a component logged at only some steps keeps its own times.
            let logical_rows = logical_rows_of(column);

            for rows in chunk_ranges_by_axes(&index, &logical_rows) {
                // Built once per chunk and cloned per field: a `TimeColumn` clone is
                // a refcount bump on the Arrow buffer, whereas rebuilding one
                // re-scales every timestamp.
                let Some(indexes) = index.time_columns(&logical_rows[rows.clone()]) else {
                    continue;
                };
                for (k, path) in paths.iter().enumerate() {
                    let values: Vec<f64> = rows
                        .clone()
                        .map(|i| column.data[i * scalars_per_row + k])
                        .collect();
                    rec.send_columns(
                        path.as_str(),
                        indexes.iter().cloned(),
                        re_sdk_types::archetypes::Scalars::new(values).columns_of_unit_batches()?,
                    )?;
                }
            }
        }

        // Orthogonal: if Position3D exists, also log Points3D for Rerun 3D Viewer
        // visualization. This intentionally duplicates the position data already
        // logged as f64 Scalars above — Points3D uses f32 internally and is only
        // consumed by the 3D spatial view.
        if let Some(pos_col) = store.columns.get(&Position3D::component_name())
            && pos_col.scalars_per_row >= 3
        {
            let scalars_per_row = pos_col.scalars_per_row;
            // Same logical rows as the position scalars above; taking the range
            // instead would move the points off the times the scalars carry.
            let logical_rows = logical_rows_of(pos_col);
            for rows in chunk_ranges_by_axes(&index, &logical_rows) {
                let Some(indexes) = index.time_columns(&logical_rows[rows.clone()]) else {
                    continue;
                };
                let points: Vec<[f32; 3]> = rows
                    .map(|i| {
                        let start = i * scalars_per_row;
                        let row = &pos_col.data[start..start + 3];
                        [row[0] as f32, row[1] as f32, row[2] as f32]
                    })
                    .collect();
                rec.send_columns(
                    rr_path.as_str(),
                    indexes,
                    re_sdk_types::archetypes::Points3D::new(points).columns_of_unit_batches()?,
                )?;
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
    /// A recording time index: `sim_time` \[ns\], the `step` sequence number,
    /// and the value on the recording's own named axis, each present only when
    /// the recording carries that timeline. Held in separate slots, so a
    /// `frame` of 1 is never the `step` 1 of a recording that has both.
    Timed {
        time_ns: Option<i64>,
        step: Option<i64>,
        axis: Option<i64>,
        repeat: u32,
    },
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
            RowKey::Timed { time_ns: None, .. } | RowKey::Index(_) => 0.0,
        }
    }

    /// This key with its repeat number replaced. An `Index` key carries none
    /// and comes back unchanged.
    fn at_repeat(self, repeat: u32) -> RowKey {
        match self {
            RowKey::Timed {
                time_ns,
                step,
                axis,
                ..
            } => RowKey::Timed {
                time_ns,
                step,
                axis,
                repeat,
            },
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
///
/// The axis is settled by the first chunk of the file to carry one, so a
/// recording whose earlier chunks have none keys those without it and they no
/// longer join with the later ones. Deciding it up front would mean decoding
/// the file twice; the rows are lost rather than mixed, which is the failure
/// this decode prefers.
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

    let mut named: Vec<_> = chunk
        .timelines()
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "sim_time" | "step" | "log_time" | "log_tick"))
        .collect();
    // The timelines arrive as a set, so choose by name to stay reproducible
    // from run to run.
    named.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

    // More axes than the key can hold: nothing here says which of them makes a
    // row, and guessing is the mis-join this decode removes.
    if named.len() > 1 {
        return None;
    }

    let keys = |axis: Option<Vec<i64>>| {
        Some(ChunkKeys {
            sim_time: sim_time.clone(),
            step: step.clone(),
            axis,
        })
    };

    match (&axis, named.first()) {
        // The recording's axis, settled by the first chunk to carry one.
        (None, Some((name, col))) => {
            let times = col.times_raw().to_vec();
            *axis = Some(name.as_str().to_string());
            keys(Some(times))
        }
        (Some(chosen), Some((name, col))) if name.as_str() == chosen.as_str() => {
            keys(Some(col.times_raw().to_vec()))
        }
        // An axis that is not the recording's. Keying on `sim_time` and `step`
        // alone would drop it, and the row would then join fields that sit at
        // no value of it at all: `x` at `sim_time = 0` beside a `y` at
        // `sim_time = 0, iteration = 7`. The key holds one axis, so a chunk on
        // another is left out rather than projected onto fewer dimensions.
        (Some(_), Some(_)) => None,
        // No axis of its own: the two names above place the row, or its
        // column-local position does, which never joins with a timed key.
        (_, None) => keys(None),
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
    /// The recording's own named axis, which identifies a row beside the two
    /// above rather than in place of them.
    axis: Option<Vec<i64>>,
}

/// Where one chunk row sits on the recording's timelines.
#[derive(Clone, Copy)]
enum RowIndex {
    /// The chunk's timelines place the row at this time index.
    Timed {
        time_ns: Option<i64>,
        step: Option<i64>,
        axis: Option<i64>,
    },
    /// The chunk carries no timeline; keys come from the column-local position.
    Untimed,
    /// A timeline the chunk does carry has no value for this row — skip it.
    Missing,
}

impl ChunkKeys {
    fn row(&self, row_idx: usize) -> RowIndex {
        // A timeline the chunk carries must have a value for this row.
        let index = |times: &Option<Vec<i64>>| match times {
            Some(times) => times.get(row_idx).copied().map(Some).ok_or(()),
            None => Ok(None),
        };
        match (index(&self.sim_time), index(&self.step), index(&self.axis)) {
            (Ok(None), Ok(None), Ok(None)) => RowIndex::Untimed,
            (Ok(time_ns), Ok(step), Ok(axis)) => RowIndex::Timed {
                time_ns,
                step,
                axis,
            },
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
    // chunk to carry one and kept for the rest, whatever else that chunk has.
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
                // Resolved once for the whole component, as `column` is: the
                // entity's name is owned to key the map, and doing that per
                // value cost one allocation per field per row of a recording.
                let counters = repeat_counters.entry(entity_path.clone()).or_default();
                for row_idx in 0..n {
                    let batch =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                    let Some(Ok(scalar_vec)) = batch else {
                        continue;
                    };
                    let moment = match keys.row(row_idx) {
                        RowIndex::Timed {
                            time_ns,
                            step,
                            axis,
                        } => Some(RowKey::Timed {
                            time_ns,
                            step,
                            axis,
                            repeat: 0,
                        }),
                        RowIndex::Untimed => None,
                        RowIndex::Missing => continue,
                    };
                    // A batch usually holds one value per row, but `Scalars`
                    // takes a slice: several values at one time index become
                    // consecutive repeats rather than being dropped. The
                    // ordinal comes from a counter, so a long recording does
                    // not pay a scan of the column per value.
                    let counter = moment.map(|moment| counters.entry(moment).or_insert(0));
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
        // A column with no values in it carries the field no further than having
        // no column at all: an empty `Scalars` batch leaves one behind, and it
        // would otherwise make a position-only recording look velocity-bearing.
        let column = |field: &str| {
            scalars
                .get(&format!("{base}/{field}"))
                .filter(|col| !col.is_empty())
        };
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
    // chunk to carry one and kept for the rest, whatever else that chunk has.
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
                // Resolved once for the whole component, as `column` is: the
                // entity's name is owned to key the map, and doing that per
                // value cost one allocation per field per row of a recording.
                let counters = repeat_counters.entry(entity_path.clone()).or_default();
                for row_idx in 0..n {
                    let batch =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx);
                    let Some(Ok(scalar_vec)) = batch else {
                        continue;
                    };
                    let moment = match keys.row(row_idx) {
                        RowIndex::Timed {
                            time_ns,
                            step,
                            axis,
                        } => Some(RowKey::Timed {
                            time_ns,
                            step,
                            axis,
                            repeat: 0,
                        }),
                        RowIndex::Untimed => None,
                        RowIndex::Missing => continue,
                    };
                    let counter = moment.map(|moment| counters.entry(moment).or_insert(0));
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

        // A component the file has no values for cannot be reconstructed
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
        // A row therefore exists only where the state components are whole.
        // Anchoring on position and velocity keeps the trajectory intact when an
        // optional component such as attitude was recorded at only some of the
        // times; that component covers the rows it has, and `orts convert` leaves
        // the others empty rather than writing a zero, which downstream would
        // read as a measured value.
        //
        // The cost is that a time where the anchors are not whole is no row at
        // all, so a whole optional value recorded only at such a time is not
        // reported.
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
        // Dense over `row_keys`, which is the intersection of the anchor columns'
        // keys — so a time the file carries for this entity is a row only when
        // the anchors are whole there. A component that covers some of those rows
        // records which ones.
        let entity_times = TimelineColumn {
            data: row_keys
                .iter()
                .map(|k| TimeIndex::Seconds(k.t_secs()))
                .collect(),
            rows: RowMap::Dense,
        };

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
            // A component present on only some rows keeps those rows rather than
            // being dropped whole: the column records which logical rows it
            // covers. A row where some of its fields are missing is left out —
            // half a vector is not a value.
            let mut column = ComponentColumn::new(fields.len());
            // Resolve each field's column once: `get_scalar_data` formats two
            // candidate paths per call, and this walks every row. `temporal` was
            // retained on every field having a column, so each lookup is `Some`.
            let field_columns: Vec<&Column> = fields
                .iter()
                .filter_map(|field| get_scalar_data(&scalars, base, field))
                .collect();
            debug_assert_eq!(field_columns.len(), fields.len());
            for (logical_row, &key) in row_keys.iter().enumerate() {
                let row: Vec<f64> = field_columns
                    .iter()
                    .map_while(|col| col.get(&key).copied())
                    .collect();
                if row.len() == fields.len() {
                    column.push_at(&row, logical_row);
                }
            }
            // None of this entity's rows holds the component whole, so there is
            // nothing to report for it. That can also happen when the file does
            // carry a whole value, at a time that is not one of the rows — see
            // the anchor comment above. An entity with no rows at all still gets
            // its columns, empty, which is what keeps its schema entry.
            if !row_keys.is_empty() && column.num_rows() == 0 {
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
///
/// A column with no values in it comes back as `None`: an empty `Scalars` batch
/// leaves one behind, and it carries the field no further than having no column.
fn get_scalar_data<'a>(
    scalars: &'a BTreeMap<String, Column>,
    base: &str,
    field: &str,
) -> Option<&'a Column> {
    scalars
        .get(&format!("{base}/{field}"))
        .or_else(|| scalars.get(&format!("/{base}/{field}")))
        .filter(|col| !col.is_empty())
}

fn to_rerun_path(path: &EntityPath) -> String {
    let s = path.to_string();
    s.strip_prefix('/').unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::archetypes::OrbitalState;
    use crate::record::components::{BodyRadius, GravitationalParameter, Position3D};
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
        match &sim_times.times()[0] {
            TimeIndex::Seconds(t) => assert!((t - 0.0).abs() < 1e-9),
            other => panic!("expected Seconds, got {:?}", other),
        }
        match &sim_times.times()[2] {
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
            Some(&[TimeIndex::Seconds(10.0)].into_iter().collect())
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
        assert_eq!(times.times(), [TimeIndex::Seconds(10.0)]);
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
            Some(
                &[TimeIndex::Seconds(10.0), TimeIndex::Seconds(20.0)]
                    .into_iter()
                    .collect()
            ),
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

    /// A component whose fields never share a row is not reported at all.
    ///
    /// Restoring the rows a component covers must not turn into reporting a
    /// component the file holds no whole value for: `qw` at one time and `qx` at
    /// another is no quaternion at either.
    #[test]
    fn a_component_whose_fields_never_meet_is_not_reported() {
        let dir = std::env::temp_dir().join(format!(
            "orts_nomeet_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("nomeet.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-nomeet-test")
                .save(&path)
                .expect("recording stream");
            // Position everywhere, so the entity has rows.
            for t in [0.0f64, 10.0] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in [("x", t), ("y", 1.0), ("z", 2.0)] {
                    rec.log(
                        format!("/world/sat/nomeet/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
            }
            // All four attitude fields exist, but split across the two times, so
            // no row holds a whole quaternion.
            for (t, fields) in [(0.0f64, ["qw", "qx"]), (10.0, ["qy", "qz"])] {
                rec.set_duration_secs("sim_time", t);
                for field in fields {
                    rec.log(
                        format!("/world/sat/nomeet/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([1.0f64]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let store = loaded
            .entity(&EntityPath::parse("/world/sat/nomeet"))
            .expect("entity");
        assert_eq!(store.num_rows, 2, "the trajectory still has its rows");
        assert!(
            store.columns.keys().any(|name| name.contains("Position3D")),
            "the trajectory survives"
        );
        assert!(
            !store
                .columns
                .keys()
                .any(|name| name.contains("Quaternion4D")),
            "no row holds a whole quaternion, so none is reported"
        );
    }

    /// A component logged at only some steps survives a round trip through the
    /// file with the rows it covers.
    ///
    /// The writer places its values at the times they were logged at, and the
    /// loader now records which rows they came back on, so `orts convert` reports
    /// the component instead of dropping it.
    #[test]
    fn a_sparse_column_round_trips() {
        use crate::record::components::MtqCommand3D;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/roundtrip");

        const N: u64 = 6;
        const FIRST: u64 = 3;
        for i in 0..N {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let os = OrbitalState::new(
                Vector3::new(7000.0 + i as f64, 0.0, 0.0),
                Vector3::new(0.0, 7.5, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
            if i >= FIRST {
                rec.log_temporal(&sat, &tp, &MtqCommand3D(Vector3::new(i as f64, 0.0, 0.0)));
            }
        }

        let path = std::env::temp_dir().join("test_orts_sparse_roundtrip.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");
        let loaded = load_as_recording(path_str).expect("failed to load");
        let _ = std::fs::remove_file(&path);

        let store = loaded.entity(&sat).expect("entity");
        assert_eq!(store.num_rows, N as usize, "every step is a row");

        let mtq = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("MtqCommand3D"))
            .map(|(_, c)| c)
            .expect("the command column survives the round trip");
        assert_eq!(mtq.num_rows(), (N - FIRST) as usize);
        for row in 0..N as usize {
            let want = (row >= FIRST as usize).then(|| row as f64);
            assert_eq!(mtq.at_logical_row(row).map(|v| v[0]), want, "row {row}");
        }

        // The dense column is unchanged by the round trip.
        let pos = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("a position column");
        assert_eq!(pos.num_rows(), N as usize);
        assert_eq!(pos.rows, RowMap::Dense);
    }

    /// A component recorded at only some of the times comes back covering those
    /// times.
    ///
    /// This used to drop the component. Filling its absent rows with zeros would
    /// have put an attitude that was never logged into the CSV `orts convert`
    /// writes, where a zero quaternion reads as a measured one, and dropping the
    /// rows would have cost the trajectory — so the component was the part that
    /// went. A column now records which logical rows it covers and the CSV
    /// leaves the others empty, so neither the attitude nor the trajectory has
    /// to go (#375).
    #[test]
    fn a_partially_recorded_optional_component_covers_its_own_times() {
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

        let position = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("the trajectory survives");
        assert_eq!(position.num_rows(), 2);

        let attitude = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Quaternion4D"))
            .map(|(_, c)| c)
            .expect("the attitude survives, covering the time it was logged at");
        assert_eq!(attitude.num_rows(), 1);
        assert_eq!(
            attitude.at_logical_row(0),
            None,
            "t=0 logged no attitude, and no zero stands in for it"
        );
        assert_eq!(
            attitude.at_logical_row(1),
            Some([1.0, 0.0, 0.0, 0.0].as_slice()),
            "t=10's attitude sits on t=10's row"
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
            Some(
                &[TimeIndex::Seconds(0.0), TimeIndex::Seconds(10.0)]
                    .into_iter()
                    .collect()
            )
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
            Some(
                &[TimeIndex::Seconds(0.0), TimeIndex::Seconds(10.0)]
                    .into_iter()
                    .collect()
            )
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

    /// An axis beside `sim_time` is part of what identifies a row.
    ///
    /// A chunk carrying `sim_time` had its other axis dropped from the key, so
    /// rows sharing a `sim_time` but sitting at different `frame`s became
    /// repeats of one moment and paired by arrival order. Measured with `x` at
    /// frames 1 and 2 and `y`/`z` at frames 2 and 3, all at t=0:
    /// `Position3D` came back as `[100.0, 201.0, 201.0]`, the frame-1 `x`
    /// beside the frame-2 `y`. Only frame 2 is whole, so it is the one row.
    #[test]
    fn an_axis_beside_sim_time_identifies_the_row() {
        let dir = std::env::temp_dir().join(format!(
            "orts_xaxis_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("extra.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-extra-axis-test")
                .save(&path)
                .expect("recording stream");
            rec.set_duration_secs("sim_time", 0.0);
            for (frame, x) in [(1i64, 100.0f64), (2, 110.0)] {
                rec.set_time_sequence("frame", frame);
                rec.log(
                    "/world/sat/extra/x",
                    &re_sdk_types::archetypes::Scalars::new([x]),
                )
                .expect("log x");
            }
            for (frame, v) in [(2i64, 201.0f64), (3, 202.0)] {
                rec.set_time_sequence("frame", frame);
                for field in ["y", "z"] {
                    rec.log(
                        format!("/world/sat/extra/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([v]),
                    )
                    .expect("log");
                }
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/extra");
        let store = loaded.entity(&entity).expect("entity");
        let position = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Position3D"))
            .map(|(_, c)| c)
            .expect("a position column");
        assert_eq!(store.num_rows, 1, "frame 2 is the only whole position");
        assert_eq!(
            position.get_row(0).expect("the one row"),
            &[110.0, 201.0, 201.0],
            "every axis of the row must be frame 2's"
        );
    }

    /// No row is assembled across two axes of the recording's own naming.
    ///
    /// Keying a chunk on `sim_time` and `step` alone drops the axis it does
    /// carry, and the row can then join fields that sit at no value of it: `x`
    /// and `z` at `sim_time = 0` beside a `y` at `sim_time = 0, iteration = 7`.
    /// Which of two axes a file settles on depends on the order its chunks
    /// arrive, so this holds the outcome rather than reproducing one ordering:
    /// either way the position is never whole.
    #[test]
    fn no_row_is_assembled_across_two_named_axes() {
        let dir = std::env::temp_dir().join(format!(
            "orts_other_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("other.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-other-axis-test")
                .save(&path)
                .expect("recording stream");
            // The recording's axis, settled here: `frame`.
            rec.set_duration_secs("sim_time", 0.0);
            rec.set_time_sequence("frame", 1i64);
            rec.log(
                "/world/sat/other/x",
                &re_sdk_types::archetypes::Scalars::new([100.0f64]),
            )
            .expect("log x");
            rec.disable_timeline("frame");
            rec.log(
                "/world/sat/other/z",
                &re_sdk_types::archetypes::Scalars::new([300.0f64]),
            )
            .expect("log z");
            // `y` sits on an axis of its own.
            rec.set_time_sequence("iteration", 7i64);
            rec.log(
                "/world/sat/other/y",
                &re_sdk_types::archetypes::Scalars::new([999.0f64]),
            )
            .expect("log y");
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/other");
        let rows = loaded.entity(&entity).map_or(0, |store| store.num_rows);
        assert_eq!(
            rows, 0,
            "the `iteration` chunk cannot be placed among the rest"
        );
    }

    /// A field whose column holds no values carries it no further than none.
    ///
    /// An empty `Scalars` batch leaves a column behind with nothing in it. That
    /// column counted as the file carrying the field, so an empty `y` made
    /// position a state anchor with no moments at all, and the velocity rows
    /// that are there went with it.
    #[test]
    fn a_field_with_an_empty_column_does_not_narrow_another() {
        let dir = std::env::temp_dir().join(format!(
            "orts_empty_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("empty.rrd");
        let schema = r#"[
            {"name":"orts.Position3D","fields":["x","y","z"]},
            {"name":"orts.Velocity3D","fields":["vx","vy","vz"]}
        ]"#;
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-empty-column-test")
                .save(&path)
                .expect("recording stream");
            rec.log_static(
                "/meta/schema/world/sat/emptied",
                &re_sdk_types::archetypes::TextDocument::new(schema),
            )
            .expect("log schema");
            for (t, x, vx) in [(0.0f64, 100.0f64, 1.0f64), (10.0, 110.0, 4.0)] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in [("x", x), ("z", 0.0), ("vx", vx), ("vy", 2.0), ("vz", 3.0)] {
                    rec.log(
                        format!("/world/sat/emptied/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
                // `y` is logged as an empty batch: a column, no values.
                rec.log(
                    "/world/sat/emptied/y",
                    &re_sdk_types::archetypes::Scalars::new(Vec::<f64>::new()),
                )
                .expect("log empty y");
            }
            rec.flush_blocking().expect("flush");
        }

        let loaded = load_as_recording(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        let entity = EntityPath::parse("/world/sat/emptied");
        let store = loaded.entity(&entity).expect("entity");
        assert!(
            !store.columns.keys().any(|name| name.contains("Position3D")),
            "a position whose `y` holds no value is not reported"
        );
        assert_eq!(store.num_rows, 2, "the velocity keeps both of its times");
        let velocity = store
            .columns
            .iter()
            .find(|(name, _)| name.contains("Velocity3D"))
            .map(|(_, c)| c)
            .expect("a velocity column");
        assert_eq!(velocity.get_row(0).expect("row 0"), &[1.0, 2.0, 3.0]);
        assert_eq!(velocity.get_row(1).expect("row 1"), &[4.0, 2.0, 3.0]);
    }

    /// An empty velocity column leaves a position-only recording position-only.
    ///
    /// An empty `Scalars` batch leaves a column behind with no values in it,
    /// which made the recording look velocity-bearing: every row then wanted a
    /// whole velocity triple and none had one, so the positions that are there
    /// were all dropped.
    #[test]
    fn an_empty_velocity_column_does_not_drop_the_positions() {
        let dir = std::env::temp_dir().join(format!(
            "orts_emptyvel_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("emptyvel.rrd");
        {
            let rec = re_sdk::RecordingStreamBuilder::new("orts-empty-velocity-test")
                .save(&path)
                .expect("recording stream");
            for (t, x) in [(0.0f64, 100.0f64), (10.0, 110.0)] {
                rec.set_duration_secs("sim_time", t);
                for (field, value) in [("x", x), ("y", 1.0), ("z", 2.0)] {
                    rec.log(
                        format!("/world/sat/emptyvel/{field}"),
                        &re_sdk_types::archetypes::Scalars::new([value]),
                    )
                    .expect("log");
                }
                rec.log(
                    "/world/sat/emptyvel/vx",
                    &re_sdk_types::archetypes::Scalars::new(Vec::<f64>::new()),
                )
                .expect("log empty vx");
            }
            rec.flush_blocking().expect("flush");
        }

        let data = load_rrd_data(path.to_str().unwrap()).expect("load");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(data.rows.len(), 2, "{:?}", data.rows);
        for (row, x) in data.rows.iter().zip([100.0, 110.0]) {
            assert_eq!((row.x, row.y, row.z), (x, 1.0, 2.0));
            assert_eq!(
                (row.vx, row.vy, row.vz),
                (0.0, 0.0, 0.0),
                "a velocity with no values reports zero, as a position-only row does"
            );
        }
    }

    /// Decode every Arrow chunk in an .rrd file, in file order.
    fn decode_chunks(path: &str) -> Vec<re_chunk::Chunk> {
        use re_log_encoding::DecoderApp;
        use re_log_types::LogMsg;

        let file = std::fs::File::open(path).expect("open .rrd");
        let reader = std::io::BufReader::new(file);
        let mut chunks = Vec::new();
        for msg in DecoderApp::decode_lazy(reader) {
            if let LogMsg::ArrowMsg(_, arrow_msg) = msg.expect("decode log message") {
                chunks.push(re_chunk::Chunk::from_arrow_msg(&arrow_msg).expect("decode chunk"));
            }
        }
        chunks
    }

    /// Every field of every row carries a distinct value, so a row that shifts by
    /// one position shows up as a value mismatch. `roundtrip_save_and_load_rrd`
    /// writes the same state five times over and cannot see such a shift.
    #[test]
    fn each_value_lands_on_its_own_time() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/dense");

        const N: usize = 7;
        for i in 0..N {
            let t = i as f64 * 10.0;
            let b = i as f64;
            let os = OrbitalState::new(
                Vector3::new(7000.0 + b, 100.0 + b, 200.0 + b),
                Vector3::new(1.0 + b, 2.0 + b, 3.0 + b),
            );
            let tp = TimePoint::new().with_sim_time(t).with_step(i as u64);
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_dense_rows.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let rows = load_from_rrd(path_str).expect("failed to load .rrd");
        assert_eq!(rows.len(), N, "expected {N} rows");

        for (i, row) in rows.iter().enumerate() {
            let b = i as f64;
            // `t` gets a looser bar than the values: the timeline stores whole
            // nanoseconds, so 1e-9 s is its quantum and a rounding difference there
            // would read like a row misalignment. A shift of one row moves `t` by
            // the 10 s step, far above this.
            assert!(
                (row.t - i as f64 * 10.0).abs() < 1e-6,
                "t[{i}] = {}, expected {}",
                row.t,
                i as f64 * 10.0
            );
            for (name, got, want) in [
                ("x", row.x, 7000.0 + b),
                ("y", row.y, 100.0 + b),
                ("z", row.z, 200.0 + b),
                ("vx", row.vx, 1.0 + b),
                ("vy", row.vy, 2.0 + b),
                ("vz", row.vz, 3.0 + b),
            ] {
                assert!(
                    (got - want).abs() < 1e-9,
                    "{name}[{i}] = {got}, expected {want}"
                );
            }
        }

        let _ = std::fs::remove_file(&path);
    }

    /// The columnar writer carries its index columns explicitly, so the .rrd holds
    /// exactly the timelines the `Recording` had. `rec.log()` also injects
    /// `log_time` and `log_tick`, which nothing in this repository reads.
    #[test]
    fn export_writes_only_the_recording_timelines() {
        use std::collections::BTreeSet;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/timelines");

        for i in 0..4u64 {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let os = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_timelines.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let names: BTreeSet<String> = decode_chunks(path_str)
            .iter()
            .flat_map(|chunk| {
                chunk
                    .timelines()
                    .keys()
                    .map(|name| name.as_str().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();

        assert_eq!(
            names,
            BTreeSet::from(["sim_time".to_string(), "step".to_string()]),
            "unexpected timelines in the .rrd"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Scalar values per entity path, in file order — the same way every reader in
    /// the repository collects them. Paths keep the leading slash Rerun gives them.
    fn scalars_by_path(path: &str) -> BTreeMap<String, Vec<f64>> {
        let mut out: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for chunk in decode_chunks(path) {
            for comp_id in chunk.components_identifiers() {
                if !comp_id.as_str().contains("Scalar") && !comp_id.as_str().contains("scalars") {
                    continue;
                }
                for row_idx in 0..chunk.num_rows() {
                    if let Some(Ok(batch)) =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx)
                    {
                        out.entry(chunk.entity_path().to_string())
                            .or_default()
                            .extend(batch.iter().map(|s| s.0.0));
                    }
                }
            }
        }
        out
    }

    /// Scalar values with the `sim_time` they were written at, per entity path.
    fn timed_scalars_by_path(path: &str) -> BTreeMap<String, Vec<(f64, f64)>> {
        let mut out: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        for chunk in decode_chunks(path) {
            let Some((_, times)) = chunk
                .timelines()
                .iter()
                .find(|(name, _)| name.as_str() == "sim_time")
            else {
                continue;
            };
            let times = times.times_raw().to_vec();
            for comp_id in chunk.components_identifiers() {
                if !comp_id.as_str().contains("Scalar") && !comp_id.as_str().contains("scalars") {
                    continue;
                }
                for row_idx in 0..chunk.num_rows() {
                    if let Some(Ok(batch)) =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx)
                    {
                        let t = times[row_idx] as f64 / 1e9;
                        out.entry(chunk.entity_path().to_string())
                            .or_default()
                            .extend(batch.iter().map(|s| (t, s.0.0)));
                    }
                }
            }
        }
        out
    }

    /// The `(sim_time, x)` of every `Points3D` row, per entity path.
    fn timed_points_by_path(path: &str) -> BTreeMap<String, Vec<(f64, f32)>> {
        let mut out: BTreeMap<String, Vec<(f64, f32)>> = BTreeMap::new();
        for chunk in decode_chunks(path) {
            let Some((_, times)) = chunk
                .timelines()
                .iter()
                .find(|(name, _)| name.as_str() == "sim_time")
            else {
                continue;
            };
            let times = times.times_raw().to_vec();
            for comp_id in chunk.components_identifiers() {
                for row_idx in 0..chunk.num_rows() {
                    if let Some(Ok(batch)) = chunk
                        .component_batch::<re_sdk_types::components::Position3D>(comp_id, row_idx)
                    {
                        let t = times[row_idx] as f64 / 1e9;
                        out.entry(chunk.entity_path().to_string())
                            .or_default()
                            .extend(batch.iter().map(|p| (t, p.0.x())));
                    }
                }
            }
        }
        out
    }

    /// A recording of `n` rows on one satellite, `x` counting up from zero so a
    /// row can be identified by its value.
    fn counted_recording(entity: &str, n: usize) -> Recording {
        let mut rec = Recording::new();
        let sat = EntityPath::parse(entity);
        for i in 0..n {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i as u64);
            let os = OrbitalState::new(
                Vector3::new(i as f64, 0.0, 0.0),
                Vector3::new(0.0, 7.5, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }
        rec
    }

    /// `send_columns` writes whatever it is handed as a single chunk, so the row
    /// count per chunk is this module's responsibility. Every row must appear
    /// exactly once and no chunk may exceed the cap.
    #[test]
    fn chunk_rows_are_bounded() {
        let n = CHUNK_ROWS + 1;
        let rec = counted_recording("/world/sat/bounded", n);

        let path = std::env::temp_dir().join("test_orts_chunk_bounds.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let chunks = decode_chunks(path_str);
        let mut rows_per_path: BTreeMap<String, usize> = BTreeMap::new();
        for chunk in &chunks {
            assert!(
                chunk.num_rows() <= CHUNK_ROWS,
                "{} holds {} rows, cap is {CHUNK_ROWS}",
                chunk.entity_path(),
                chunk.num_rows()
            );
            *rows_per_path
                .entry(chunk.entity_path().to_string())
                .or_default() += chunk.num_rows();
        }

        for field in ["x", "y", "z", "vx", "vy", "vz"] {
            assert_eq!(
                rows_per_path.get(&format!("/world/sat/bounded/{field}")),
                Some(&n),
                "{field} row total"
            );
        }

        // The split must not reorder or drop rows either.
        let xs = &scalars_by_path(path_str)["/world/sat/bounded/x"];
        assert_eq!(xs.len(), n);
        for (i, x) in xs.iter().enumerate() {
            assert!((x - i as f64).abs() < 1e-9, "x[{i}] = {x}");
        }

        let _ = std::fs::remove_file(&path);
    }

    /// Every scalar row holds exactly one value. The readers count a row per
    /// value, so a wide batch would silently change how many rows they see.
    #[test]
    fn scalar_rows_stay_unit_batches() {
        let rec = counted_recording("/world/sat/unit", 5);

        let path = std::env::temp_dir().join("test_orts_unit_batches.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let mut checked = 0;
        for chunk in decode_chunks(path_str) {
            for comp_id in chunk.components_identifiers() {
                if !comp_id.as_str().contains("Scalar") && !comp_id.as_str().contains("scalars") {
                    continue;
                }
                for row_idx in 0..chunk.num_rows() {
                    if let Some(Ok(batch)) =
                        chunk.component_batch::<re_sdk_types::components::Scalar>(comp_id, row_idx)
                    {
                        assert_eq!(
                            batch.len(),
                            1,
                            "{} row {row_idx} holds {} scalars",
                            chunk.entity_path(),
                            batch.len()
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no scalar rows were inspected");

        let _ = std::fs::remove_file(&path);
    }

    /// No reader in the repository reconstructs the `step` timeline, so the only
    /// way to pin it is to read the decoded chunk directly.
    #[test]
    fn step_timeline_survives_the_columnar_export() {
        let n = 6;
        let rec = counted_recording("/world/sat/steps", n);

        let path = std::env::temp_dir().join("test_orts_step_timeline.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let expected: Vec<i64> = (0..n as i64).collect();
        let mut seen = 0;
        for chunk in decode_chunks(path_str) {
            if chunk.entity_path().to_string() != "/world/sat/steps/x" {
                continue;
            }
            let (_, steps) = chunk
                .timelines()
                .iter()
                .find(|(name, _)| name.as_str() == "step")
                .expect("step timeline missing");
            assert_eq!(steps.times_raw(), expected.as_slice());
            seen += 1;
        }
        assert_eq!(seen, 1, "expected one chunk for /world/sat/steps/x");

        let _ = std::fs::remove_file(&path);
    }

    /// The `Points3D` convenience archetype must cover the same rows as the
    /// position column it mirrors.
    #[test]
    fn points3d_rows_match_position_rows() {
        let n = 9;
        let rec = counted_recording("/world/sat/points", n);

        let path = std::env::temp_dir().join("test_orts_points3d_rows.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        // Only Points3D is logged at the entity itself; the scalars live one level
        // down, under the field names.
        let points_rows: usize = decode_chunks(path_str)
            .iter()
            .filter(|chunk| chunk.entity_path().to_string() == "/world/sat/points")
            .map(|chunk| chunk.num_rows())
            .sum();
        let position_rows = scalars_by_path(path_str)["/world/sat/points/x"].len();
        assert_eq!(position_rows, n, "the fixture should have written {n} rows");
        assert_eq!(
            points_rows, position_rows,
            "Points3D must cover the same rows as the position column it mirrors"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A negative `sim_time` reaches the file. The row-oriented export could not
    /// write one: `set_duration_secs` goes through `std::time::Duration`, which is
    /// unsigned, so it rejected the value and left the previous row's timestamp in
    /// place. `TimeColumn::new_duration_secs` takes it.
    #[test]
    fn negative_sim_time_reaches_the_file() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/countdown");

        let times = [-30.0, -20.0, -10.0, 0.0, 10.0];
        for (i, &t) in times.iter().enumerate() {
            let tp = TimePoint::new().with_sim_time(t).with_step(i as u64);
            let os = OrbitalState::new(
                Vector3::new(i as f64, 0.0, 0.0),
                Vector3::new(0.0, 7.5, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        let path = std::env::temp_dir().join("test_orts_negative_time.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let rows = load_from_rrd(path_str).expect("failed to load .rrd");
        assert_eq!(rows.len(), times.len());
        for (row, &want) in rows.iter().zip(times.iter()) {
            assert!(
                (row.t - want).abs() < 1e-6,
                "t = {}, expected {want}",
                row.t
            );
        }
        // Each row keeps its own x, so the negative times are not all collapsed
        // onto one index.
        for (i, row) in rows.iter().enumerate() {
            assert!((row.x - i as f64).abs() < 1e-9, "x[{i}] = {}", row.x);
        }

        let _ = std::fs::remove_file(&path);
    }

    /// A component whose logging starts late has fewer rows than the entity has
    /// logical rows. Every logged value must still reach the file — this pins the
    /// count and the order, not the timestamps, which are wrong for a different
    /// reason (see #375).
    #[test]
    fn sparse_column_writes_every_value() {
        use crate::record::components::MtqCommand3D;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/sparse");

        const N: u64 = 10;
        const FIRST_MTQ_STEP: u64 = 5;
        for i in 0..N {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let os = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));
            rec.log_orbital_state(&sat, &tp, &os);
            if i >= FIRST_MTQ_STEP {
                rec.log_temporal(&sat, &tp, &MtqCommand3D(Vector3::new(i as f64, 0.0, 0.0)));
            }
        }

        let path = std::env::temp_dir().join("test_orts_sparse_column.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let scalars = scalars_by_path(path_str);
        let expected: Vec<f64> = (FIRST_MTQ_STEP..N).map(|i| i as f64).collect();
        assert_eq!(
            scalars.get("/world/sat/sparse/mtq_mx"),
            Some(&expected),
            "every logged mtq_mx value must reach the file, in order"
        );
        assert_eq!(
            scalars["/world/sat/sparse/x"].len(),
            N as usize,
            "the dense column is unaffected"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A component logged only from a later step must be written at the times it
    /// was logged at. The store used to keep one timeline per entity and no
    /// per-column row mapping, so a short column lined up with the *leading*
    /// timeline entries: values from steps 5-9 landed on t=0-4 (#375).
    #[test]
    fn a_late_starting_component_keeps_its_own_times() {
        use crate::record::components::MtqCommand3D;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/late");

        const N: u64 = 10;
        const FIRST_MTQ_STEP: u64 = 5;
        for i in 0..N {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            let os = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));
            rec.log_orbital_state(&sat, &tp, &os);
            if i >= FIRST_MTQ_STEP {
                rec.log_temporal(&sat, &tp, &MtqCommand3D(Vector3::new(i as f64, 0.0, 0.0)));
            }
        }

        let path = std::env::temp_dir().join("test_orts_late_component.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let timed = timed_scalars_by_path(path_str);
        let got = timed
            .get("/world/sat/late/mtq_mx")
            .expect("mtq_mx missing from the file");
        let want: Vec<(f64, f64)> = (FIRST_MTQ_STEP..N).map(|i| (i as f64, i as f64)).collect();
        assert_eq!(
            got, &want,
            "each mtq_mx value should carry the sim_time of the step it was logged at"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A position column that starts late must put its `Points3D` on the times
    /// its own rows carry. The scalar paths and the `Points3D` path are built
    /// from the same logical rows but by separate code, so the 3D path can go
    /// back to leading-row alignment while the scalars stay right.
    #[test]
    fn a_late_starting_position_keeps_its_points_on_its_own_times() {
        use crate::record::components::{MtqCommand3D, Position3D};

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/late_points");

        const N: u64 = 8;
        const FIRST_POSITION_STEP: u64 = 3;
        for i in 0..N {
            let tp = TimePoint::new().with_sim_time(i as f64).with_step(i);
            // Logged at every step, so the entity's rows start at step 0 and the
            // position column is the sparse one.
            rec.log_temporal(&sat, &tp, &MtqCommand3D(Vector3::new(i as f64, 0.0, 0.0)));
            if i >= FIRST_POSITION_STEP {
                rec.log_temporal(&sat, &tp, &Position3D(Vector3::new(i as f64, 0.0, 0.0)));
            }
        }

        let dir = std::env::temp_dir().join(format!(
            "orts_rrd_late_points_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("late_points.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let want: Vec<(f64, f64)> = (FIRST_POSITION_STEP..N)
            .map(|i| (i as f64, i as f64))
            .collect();
        let scalars = timed_scalars_by_path(path_str);
        assert_eq!(
            scalars
                .get("/world/sat/late_points/x")
                .expect("x missing from the file"),
            &want,
            "each x should carry the sim_time of the step it was logged at"
        );

        let points = timed_points_by_path(path_str);
        let got = points
            .get("/world/sat/late_points")
            .expect("Points3D missing from the file");
        let want_points: Vec<(f64, f32)> = want.iter().map(|(t, x)| (*t, *x as f32)).collect();
        assert_eq!(
            got, &want_points,
            "and each point should sit at the same time as the scalars of the \
             row it came from"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An entity whose only timeline carries the wrong `TimeIndex` variant has no
    /// usable index left. `send_columns` reads an empty index list as *static* data,
    /// and static data unconditionally shadows every temporal value at the same path
    /// in the viewer, so such an entity must write nothing rather than write statics.
    #[test]
    fn an_entity_with_no_usable_index_writes_nothing() {
        use crate::record::recording::ComponentColumn;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/unindexed");
        {
            let store = rec.entity_mut(&sat);
            let mut column = ComponentColumn::new(3);
            column.push_at(&[1.0, 2.0, 3.0], 0);
            store.columns.insert(Position3D::component_name(), column);
            // A sequence on the sim-time axis, and no step axis at all.
            store.timelines.insert(
                TimelineName::SimTime,
                [TimeIndex::Sequence(0)].into_iter().collect(),
            );
            store.num_rows = 1;
        }
        rec.register_component_fields(Position3D::component_name(), vec!["x", "y", "z"]);

        let path = std::env::temp_dir().join("test_orts_unindexed.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let under_entity: Vec<String> = decode_chunks(path_str)
            .iter()
            .map(|chunk| chunk.entity_path().to_string())
            .filter(|entity| entity.starts_with("/world/sat/unindexed"))
            .collect();
        assert!(
            under_entity.is_empty(),
            "expected no chunks under the entity, got {under_entity:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// An unusable `sim_time` axis must not take a usable `step` axis down with it.
    /// Each axis covers the rows it can be written for, so these rows go out on
    /// `step` alone — which is what the row-oriented export did, since its
    /// `set_duration_secs` call was skipped on the variant mismatch while
    /// `set_time_sequence` still ran.
    #[test]
    fn a_usable_step_axis_carries_the_export_on_its_own() {
        use crate::record::recording::ComponentColumn;

        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/steponly");
        const N: usize = 3;
        {
            let store = rec.entity_mut(&sat);
            let mut column = ComponentColumn::new(3);
            for i in 0..N {
                column.push_at(&[i as f64, 0.0, 0.0], i);
            }
            store.columns.insert(Position3D::component_name(), column);
            // `sim_time` holds the wrong variant, so it covers no row at all.
            store.timelines.insert(
                TimelineName::SimTime,
                [TimeIndex::Sequence(0)].into_iter().collect(),
            );
            store.timelines.insert(
                TimelineName::Step,
                (0..N as u64).map(TimeIndex::Sequence).collect(),
            );
            store.num_rows = N;
        }
        rec.register_component_fields(Position3D::component_name(), vec!["x", "y", "z"]);

        let path = std::env::temp_dir().join("test_orts_step_only.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let xs = scalars_by_path(path_str);
        let xs = xs
            .get("/world/sat/steponly/x")
            .expect("the rows should have been exported on the step axis");
        assert_eq!(xs.len(), N, "every row should reach the file");
        for (i, x) in xs.iter().enumerate() {
            assert!((x - i as f64).abs() < 1e-9, "x[{i}] = {x}");
        }

        // Temporal, not static — and indexed by `step` only.
        let chunk = decode_chunks(path_str)
            .into_iter()
            .find(|chunk| chunk.entity_path().to_string() == "/world/sat/steponly/x")
            .expect("chunk missing");
        assert!(!chunk.is_static(), "the rows were written as static data");
        let names: Vec<String> = chunk
            .timelines()
            .keys()
            .map(|name| name.as_str().to_string())
            .collect();
        assert_eq!(names, vec!["step".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    /// Two axes can cover different rows: logging rows 0-4 with `step` alone and
    /// rows 5-9 with both leaves `step` over all ten rows and `sim_time` over the
    /// last five. The chunk is cut where the usable axes change, so rows 0-4 go out
    /// on `step` and rows 5-9 on both, and every row keeps its own times. Until
    /// #375 the row count came from `sim_time`, so only five rows were written and
    /// `step` was dropped for disagreeing on length.
    #[test]
    fn rows_are_split_where_the_usable_axes_change() {
        let mut rec = Recording::new();
        let sat = EntityPath::parse("/world/sat/divergent");

        for i in 0..10u64 {
            let tp = if i < 5 {
                TimePoint::new().with_step(i)
            } else {
                TimePoint::new()
                    .with_sim_time(i as f64 * 100.0)
                    .with_step(i)
            };
            let os = OrbitalState::new(
                Vector3::new(i as f64, 0.0, 0.0),
                Vector3::new(0.0, 7.5, 0.0),
            );
            rec.log_orbital_state(&sat, &tp, &os);
        }

        // The premise: the axes really do cover different rows in this shape.
        let store = rec.entity(&sat).expect("entity");
        assert_eq!(store.timelines[&TimelineName::SimTime].len(), 5);
        assert_eq!(store.timelines[&TimelineName::Step].len(), 10);
        assert_eq!(store.num_rows, 10);

        let path = std::env::temp_dir().join("test_orts_divergent_axes.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        // Per path: the rows with `step` alone, then the rows with both axes.
        let mut shapes: BTreeMap<String, Vec<(Vec<String>, usize)>> = BTreeMap::new();
        for chunk in decode_chunks(path_str) {
            let entity = chunk.entity_path().to_string();
            if !entity.starts_with("/world/sat/divergent") {
                continue;
            }
            let mut axes: Vec<String> = chunk
                .timelines()
                .keys()
                .map(|name| name.as_str().to_string())
                .collect();
            axes.sort();
            shapes
                .entry(entity)
                .or_default()
                .push((axes, chunk.num_rows()));
        }
        assert!(!shapes.is_empty(), "no chunk was written for the entity");
        for (entity, chunks) in &shapes {
            assert_eq!(
                chunks,
                &vec![
                    (vec!["step".to_string()], 5),
                    (vec!["sim_time".to_string(), "step".to_string()], 5),
                ],
                "{entity} chunk shapes"
            );
        }

        // No row is lost to the split, and the order holds.
        let xs = &scalars_by_path(path_str)["/world/sat/divergent/x"];
        let want: Vec<f64> = (0..10).map(|i| i as f64).collect();
        assert_eq!(xs, &want);

        let _ = std::fs::remove_file(&path);
    }

    /// `rrd-wasm` (and with it the viewer's file source) reads .rrd without any
    /// schema: it looks up leaf field names, the `sim_time` timeline, and the
    /// `meta/sim/*` statics, and it recognises components by a substring of their
    /// identifier. Pin that contract here rather than depending on `rrd-wasm`.
    #[test]
    fn rrd_wasm_read_contract_holds() {
        let mut rec = counted_recording("/world/sat/contract", 4);
        rec.metadata = SimMetadata {
            mu: Some(398600.4418),
            body_name: Some("Earth".to_string()),
            ..Default::default()
        };

        let path = std::env::temp_dir().join("test_orts_wasm_contract.rrd");
        let path_str = path.to_str().unwrap();
        save_as_rrd(&rec, "test-orts", path_str).expect("failed to save .rrd");

        let chunks = decode_chunks(path_str);

        // Leaf field names, with a `sim_time` index on each.
        for field in ["x", "y", "z", "vx", "vy", "vz"] {
            let wanted = format!("/world/sat/contract/{field}");
            let chunk = chunks
                .iter()
                .find(|chunk| chunk.entity_path().to_string() == wanted)
                .unwrap_or_else(|| panic!("{wanted} missing"));
            assert!(
                chunk
                    .timelines()
                    .iter()
                    .any(|(name, _)| name.as_str() == "sim_time"),
                "{wanted} has no sim_time index"
            );
            assert!(
                chunk
                    .components_identifiers()
                    .any(|id| id.as_str().contains("Scalar") || id.as_str().contains("scalars")),
                "{wanted} carries no component the readers recognise as a scalar"
            );
        }

        // `meta/sim/*` statics: a scalar and a text one, both timeless.
        for (wanted, needle) in [("/meta/sim/mu", "Scalar"), ("/meta/sim/body_name", "Text")] {
            let chunk = chunks
                .iter()
                .find(|chunk| chunk.entity_path().to_string() == wanted)
                .unwrap_or_else(|| panic!("{wanted} missing"));
            assert!(chunk.is_static(), "{wanted} is not static");
            assert!(
                chunk.components_identifiers().any(|id| {
                    let id = id.as_str();
                    id.contains(needle) || id.contains(&needle.to_lowercase())
                }),
                "{wanted} carries no component containing {needle:?}"
            );
        }

        let _ = std::fs::remove_file(&path);
    }
}
