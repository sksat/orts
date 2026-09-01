//! Minimal RRD (Rerun Recording Data) decoder for browser-side use.
//!
//! Decodes .rrd files into orbital state vectors + metadata.
//! Designed to be compiled to WASM for use in the viewer's Web Worker.
//! Does NOT compute Keplerian elements — that is done by arika WASM.

use std::collections::{BTreeMap, BTreeSet};
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

/// Join key identifying one logical row of a scalar column.
///
/// Every scalar field lives on its own entity path in an RRD (`<base>/x`,
/// `<base>/y`, …), so a state row has to be reassembled from several columns.
/// The key is the recording's own time index, not the position of a value
/// inside its column — the two coincide only as long as every column happens to
/// carry a value at every step. A chunk with no timeline at all has no time
/// index to key on, and only there does the column-local position stand in.
///
/// Every timeline the recording carries takes part in the key: `orts` writes
/// both `sim_time` and `step`, and two steps can share a `sim_time`, so keying
/// on time alone would join values from different steps.
///
/// The trailing counter distinguishes several values logged at the *same* time
/// index, so repeats are still separate rows instead of overwriting each other,
/// and the n-th repeat of one field joins the n-th repeat of the others.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum RowKey {
    /// A recording time index: `sim_time` \[ns\], the `step` sequence number,
    /// and the value on the recording's own named axis, each present only when
    /// the recording carries that timeline. Held in separate slots, so a
    /// `frame` of 1 is never the `step` 1 of a recording that has both. Ordered
    /// by time first, so rows without a `sim_time` all report `t = 0` but stay
    /// in step order.
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

/// Time index of every row in one chunk, per timeline the chunk carries. With
/// neither timeline, keys are assigned per column from its current length.
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

/// Decode an RRD stream into orbital data.
///
/// Accepts any `impl Read` — works with both `File` and `Cursor<&[u8]>`.
///
/// Columns are joined on the recording's time index, so a component that is
/// logged at only some of the time steps (or not at all) never shifts the
/// remaining components onto the wrong row. A row is emitted only when the
/// whole position triple — and, when the recording has velocity columns, the
/// whole velocity triple — is present at that exact time; incomplete rows are
/// dropped rather than padded with zeros.
///
/// That guarantee needs a time index to join on, which every recording orts
/// writes carries (`sim_time`, `step`, or both). A chunk with neither timeline
/// has no join key available, so its values are keyed by position within their
/// own column — the arrangement this join replaced, and one a sparse column
/// still shifts. Such a recording does not come from orts.
pub fn decode_rrd(reader: impl Read) -> Result<ParsedRrd, Box<dyn std::error::Error>> {
    let reader = std::io::BufReader::new(reader);

    // Collect f64 scalars: entity_path -> (time index -> value)
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
        // has velocity columns must supply all three at a time for the row to be
        // a state vector.
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

    const ENTITY: &str = "/world/sat/ragged";

    /// One logged sample: `(sim_time [s], [(field, value)])`. A field absent
    /// from a sample is simply not logged at that time, which is how a ragged
    /// recording arises in practice (a component logged conditionally).
    type Sample<'a> = (f64, Vec<(&'a str, f64)>);
    /// A sample tagged with its `step` sequence number.
    type SteppedSample<'a> = (f64, i64, Vec<(&'a str, f64)>);
    /// A sample whose fields each carry several values in one logged row.
    type BatchSample<'a> = (f64, Vec<(&'a str, Vec<f64>)>);

    /// Write an .rrd with `write`, then decode it back. The recording lives in
    /// a directory of its own that is removed when the call returns, so no two
    /// calls — including tests running in parallel — can meet on one path.
    fn decode_written(write: impl FnOnce(&re_sdk::RecordingStream)) -> ParsedRrd {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.rrd");
        let rec = re_sdk::RecordingStreamBuilder::new("rrd-wasm-test")
            .save(&path)
            .expect("recording stream");
        write(&rec);
        rec.flush_blocking().expect("flush");
        drop(rec);
        let bytes = std::fs::read(&path).expect("written rrd");
        decode_rrd(std::io::Cursor::new(&bytes)).expect("rrd should decode")
    }

    /// Log one scalar per field at the stream's current time.
    fn log_scalars(rec: &re_sdk::RecordingStream, fields: &[(&str, f64)]) {
        for (field, value) in fields {
            rec.log(
                format!("{ENTITY}/{field}"),
                &re_sdk_types::archetypes::Scalars::new([*value]),
            )
            .expect("log scalar");
        }
    }

    /// Decode an .rrd whose scalar columns carry exactly the given samples.
    fn decode_samples(samples: &[Sample]) -> ParsedRrd {
        decode_written(|rec| {
            for (t, fields) in samples {
                rec.set_duration_secs("sim_time", *t);
                log_scalars(rec, fields);
            }
        })
    }

    /// Decode an .rrd carrying both timelines, so one `sim_time` can span
    /// several `step`s the way it can in an `orts` recording.
    fn decode_stepped_samples(samples: &[SteppedSample]) -> ParsedRrd {
        decode_written(|rec| {
            for (t, step, fields) in samples {
                rec.set_duration_secs("sim_time", *t);
                rec.set_time_sequence("step", *step);
                log_scalars(rec, fields);
            }
        })
    }

    /// Decode an .rrd whose `Scalars` batches carry several values per logged
    /// row — one row of the recording, many values of the column.
    fn decode_batched_samples(samples: &[BatchSample]) -> ParsedRrd {
        decode_written(|rec| {
            for (t, fields) in samples {
                rec.set_duration_secs("sim_time", *t);
                for (field, values) in fields {
                    rec.log(
                        format!("{ENTITY}/{field}"),
                        &re_sdk_types::archetypes::Scalars::new(values.clone()),
                    )
                    .expect("log scalars");
                }
            }
        })
    }

    fn state(x: f64) -> Vec<(&'static str, f64)> {
        vec![
            ("x", x),
            ("y", x + 1.0),
            ("z", x + 2.0),
            ("vx", x + 3.0),
            ("vy", x + 4.0),
            ("vz", x + 5.0),
        ]
    }

    /// The states of `xs`, each component collected into one batch per field.
    fn state_batch(xs: &[f64]) -> Vec<(&'static str, Vec<f64>)> {
        let fields = ["x", "y", "z", "vx", "vy", "vz"];
        fields
            .iter()
            .enumerate()
            .map(|(k, field)| (*field, xs.iter().map(|x| x + k as f64).collect()))
            .collect()
    }

    /// Dense, aligned columns: every logged value must come back on the row of
    /// the time it was logged at. All six components differ from each other and
    /// from step to step, so any reshuffling of rows or columns shows up.
    #[test]
    fn dense_columns_keep_every_value_on_its_own_time() {
        let data = decode_samples(&[
            (0.0, state(100.0)),
            (10.0, state(200.0)),
            (20.0, state(300.0)),
        ]);

        assert_eq!(data.rows.len(), 3, "expected one row per time step");
        for (row, x) in data.rows.iter().zip([100.0, 200.0, 300.0]) {
            assert_eq!(
                (row.t, row.x, row.y, row.z, row.vx, row.vy, row.vz),
                (
                    (x - 100.0) / 10.0,
                    x,
                    x + 1.0,
                    x + 2.0,
                    x + 3.0,
                    x + 4.0,
                    x + 5.0
                ),
                "row {row:?} does not match the state logged at its time"
            );
        }
    }

    /// Two states logged at the same time index are two rows, not one: the
    /// repeat must not overwrite the earlier value, and the n-th repeat of each
    /// component must stay with the n-th repeat of the others.
    #[test]
    fn repeated_time_index_keeps_both_rows() {
        let data = decode_samples(&[
            (0.0, state(100.0)),
            (0.0, state(200.0)),
            (10.0, state(300.0)),
        ]);

        assert_eq!(data.rows.len(), 3, "got {:?}", data.rows);
        assert_eq!(
            data.rows
                .iter()
                .map(|r| (r.t, r.x, r.y))
                .collect::<Vec<_>>(),
            vec![
                (0.0, 100.0, 101.0),
                (0.0, 200.0, 201.0),
                (10.0, 300.0, 301.0)
            ]
        );
    }

    /// `sim_time` alone is not a row identity: an `orts` recording carries a
    /// `step` timeline as well, and several steps can share one `sim_time`.
    /// Keying on time alone pairs the n-th value of each column, so a component
    /// missing at one of those steps drags the next step's value onto the row.
    #[test]
    fn steps_sharing_a_sim_time_stay_separate_rows() {
        let mut middle = state(200.0);
        middle.retain(|(field, _)| *field != "y");
        let data = decode_stepped_samples(&[
            (0.0, 10, state(100.0)),
            (0.0, 11, middle),
            (0.0, 12, state(300.0)),
        ]);

        // step 11 has no y, so it is not a position row. Steps 10 and 12 keep
        // their own values instead of borrowing step 12's y for step 11.
        assert_eq!(data.rows.len(), 2, "got {:?}", data.rows);
        assert_eq!(
            data.rows
                .iter()
                .map(|r| (r.x, r.y, r.vz))
                .collect::<Vec<_>>(),
            vec![(100.0, 101.0, 105.0), (300.0, 301.0, 305.0)]
        );
    }

    /// A `Scalars` batch can hold several values at one time index. All of them
    /// are values of the recording, not just the first.
    #[test]
    fn every_value_of_a_scalar_batch_is_decoded() {
        let data = decode_batched_samples(&[(0.0, state_batch(&[100.0, 200.0]))]);

        assert_eq!(data.rows.len(), 2, "got {:?}", data.rows);
        assert_eq!(
            data.rows
                .iter()
                .map(|r| (r.x, r.y, r.vz))
                .collect::<Vec<_>>(),
            vec![(100.0, 101.0, 105.0), (200.0, 201.0, 205.0)]
        );
    }

    /// A component missing at one time step must not slide the later values of
    /// that column onto the earlier rows.
    #[test]
    fn sparse_position_column_is_joined_by_time() {
        let mut first = state(100.0);
        first.retain(|(field, _)| *field != "y");
        let data = decode_samples(&[(0.0, first), (10.0, state(200.0))]);

        // t=0 has no y at all, so it is not a position — the only complete row
        // is t=10, and it must carry *its own* y (201), not t=0's row index.
        assert_eq!(
            data.rows.len(),
            1,
            "incomplete position must be dropped, got {:?}",
            data.rows
        );
        assert_eq!((data.rows[0].t, data.rows[0].y), (10.0, 201.0));
    }

    /// Velocity logged for only part of a run must not be reported as zero
    /// velocity on the remaining rows.
    #[test]
    fn row_missing_a_velocity_component_is_dropped() {
        let mut second = state(200.0);
        second.retain(|(field, _)| *field != "vz");
        let data = decode_samples(&[(0.0, state(100.0)), (10.0, second)]);

        assert_eq!(
            data.rows.len(),
            1,
            "incomplete velocity must be dropped, got {:?}",
            data.rows
        );
        assert_eq!((data.rows[0].t, data.rows[0].vz), (0.0, 105.0));
    }

    /// Attitude logged at only some steps must attach to those steps.
    #[test]
    fn attitude_attaches_to_the_time_it_was_logged_at() {
        let mut attitude_step = state(200.0);
        attitude_step.extend([
            ("qw", 1.0),
            ("qx", 0.2),
            ("qy", 0.3),
            ("qz", 0.4),
            ("wx", 0.01),
            ("wy", 0.02),
            ("wz", 0.03),
        ]);
        let data = decode_samples(&[
            (0.0, state(100.0)),
            (10.0, attitude_step),
            (20.0, state(300.0)),
        ]);

        assert_eq!(data.rows.len(), 3);
        assert_eq!(data.rows[0].quaternion, None, "t=0 logged no attitude");
        assert_eq!(data.rows[1].quaternion, Some([1.0, 0.2, 0.3, 0.4]));
        assert_eq!(data.rows[1].angular_velocity, Some([0.01, 0.02, 0.03]));
        assert_eq!(data.rows[2].quaternion, None, "t=20 logged no attitude");
        assert_eq!(data.rows[2].angular_velocity, None);
    }

    /// A moment whose columns hold different numbers of values yields no row.
    ///
    /// Repeat ordinals are assigned per column, so they identify a row only
    /// while every required column has the same count at that moment. With two
    /// samples at t=0 where the first omits `y`, `x` holds repeats 0 and 1
    /// while `y` holds only repeat 0 — joining on the ordinal paired the first
    /// `x` with the second sample's `y` and dropped the second row, which is
    /// the same shift this join set out to remove, one level in.
    #[test]
    fn a_time_whose_columns_disagree_on_repeats_yields_no_row() {
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_scalars(rec, &[("x", 100.0), ("z", 0.0)]);
            log_scalars(rec, &[("x", 110.0), ("y", 201.0), ("z", 0.0)]);
        });
        assert!(
            parsed.rows.is_empty(),
            "ordinals cannot pair these values; expected no rows, got {:?}",
            parsed.rows
        );
    }

    /// Matching repeat counts still yield one row each, in order.
    #[test]
    fn matching_repeats_at_one_time_yield_a_row_each() {
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_scalars(rec, &[("x", 100.0), ("y", 1.0), ("z", 0.0)]);
            log_scalars(rec, &[("x", 110.0), ("y", 2.0), ("z", 0.0)]);
        });
        assert_eq!(parsed.rows.len(), 2, "{:?}", parsed.rows);
        assert_eq!(parsed.rows[0].x, 100.0);
        assert_eq!(parsed.rows[0].y, 1.0);
        assert_eq!(parsed.rows[1].x, 110.0);
        assert_eq!(parsed.rows[1].y, 2.0);
    }

    /// A recording with no timeline at all still decodes, keyed by position.
    ///
    /// Nothing `orts` writes lands here — every recording it produces carries
    /// `sim_time`, `step` or both — but a file from elsewhere can, and the
    /// fallback has to stay reachable. Rows keyed this way report `t = 0`.
    #[test]
    fn a_recording_with_no_timeline_falls_back_to_column_order() {
        let parsed = decode_written(|rec| {
            // No `set_duration_secs` and no `set_time_sequence`.
            log_scalars(rec, &[("x", 100.0), ("y", 200.0), ("z", 300.0)]);
            log_scalars(rec, &[("x", 110.0), ("y", 210.0), ("z", 310.0)]);
        });
        assert_eq!(parsed.rows.len(), 2, "{:?}", parsed.rows);
        for row in &parsed.rows {
            assert_eq!(row.t, 0.0, "an untimed row reports t = 0");
        }
        assert_eq!(parsed.rows[0].x, 100.0);
        assert_eq!(parsed.rows[1].x, 110.0);
    }

    /// Rows sharing a `t` keep the recording's own order.
    ///
    /// `sort_by` is stable and the rows are built in `RowKey` order, so several
    /// `step`s at one `sim_time` come out in step order rather than arbitrarily.
    #[test]
    fn rows_sharing_a_sim_time_keep_step_order() {
        let parsed = decode_stepped_samples(&[
            (5.0, 10, vec![("x", 1.0), ("y", 0.0), ("z", 0.0)]),
            (5.0, 11, vec![("x", 2.0), ("y", 0.0), ("z", 0.0)]),
            (5.0, 12, vec![("x", 3.0), ("y", 0.0), ("z", 0.0)]),
        ]);
        let xs: Vec<f64> = parsed.rows.iter().map(|r| r.x).collect();
        assert_eq!(xs, vec![1.0, 2.0, 3.0], "steps out of order: {xs:?}");
    }

    /// The optional columns count toward the repeat check too.
    ///
    /// Two complete states at one moment with attitude on the second only: the
    /// quaternion holds repeat 0, so joining on the ordinal attached it to the
    /// first state and left the second without one.
    #[test]
    fn an_optional_column_disagreeing_on_repeats_yields_no_row() {
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_scalars(rec, &[("x", 100.0), ("y", 0.0), ("z", 0.0)]);
            log_scalars(
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
            parsed.rows.is_empty(),
            "the quaternion cannot be assigned to either state; got {:?}",
            parsed.rows
        );
    }

    /// A moment with one value per column is never ambiguous, whichever
    /// optional columns are absent.
    #[test]
    fn a_single_sample_with_absent_optional_columns_still_yields_a_row() {
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            log_scalars(rec, &[("x", 100.0), ("y", 200.0), ("z", 300.0)]);
        });
        assert_eq!(parsed.rows.len(), 1, "{:?}", parsed.rows);
        assert_eq!(parsed.rows[0].x, 100.0);
        assert!(parsed.rows[0].quaternion.is_none());
    }

    /// A recording indexed by a timeline of its own naming still joins on it.
    ///
    /// `sim_time` and `step` are the names `orts` writes; another tool names its
    /// own, and treating that as no timeline at all fell back to column
    /// position. Measured with `y` logged at frame 2 alone: the one row came
    /// back as x = 101.0, the frame-1 `x` beside the frame-2 `y`.
    #[test]
    fn a_recording_on_its_own_named_timeline_joins_on_it() {
        let parsed = decode_written(|rec| {
            rec.set_time_sequence("frame", 1i64);
            log_scalars(rec, &[("x", 101.0), ("z", 0.0)]);
            rec.set_time_sequence("frame", 2i64);
            log_scalars(rec, &[("x", 102.0), ("y", 201.0), ("z", 0.0)]);
        });
        assert_eq!(parsed.rows.len(), 1, "{:?}", parsed.rows);
        assert_eq!(
            parsed.rows[0].x, 102.0,
            "the row is frame 2, the only frame with a whole position"
        );
        assert_eq!(parsed.rows[0].y, 201.0);
    }

    /// Two axes of the recording's own naming do not share a row.
    ///
    /// A `frame` of 1 and an `iteration` of 1 are separate dimensions, but the
    /// fallback kept only the raw value, so both became the same key. Measured
    /// with `x` and `z` on `frame` and `y` on `iteration`: one row came back as
    /// x = 100.0 beside y = 999.0, a position assembled from two axes. One axis
    /// serves the whole recording, so neither half is a whole position and the
    /// recording yields no row.
    #[test]
    fn a_second_named_axis_does_not_join_with_the_first() {
        let parsed = decode_written(|rec| {
            rec.set_time_sequence("frame", 1i64);
            log_scalars(rec, &[("x", 100.0), ("z", 300.0)]);
            rec.disable_timeline("frame");
            rec.set_time_sequence("iteration", 1i64);
            log_scalars(rec, &[("y", 999.0)]);
        });
        assert!(
            parsed.rows.is_empty(),
            "no axis carries a whole position: {:?}",
            parsed.rows
        );
    }

    /// A named axis is never a step of the same value.
    ///
    /// The fallback put the axis value in the same slot as a real `step`, so a
    /// `frame` of 1 and a `step` of 1 became one key and their halves were
    /// assembled into one position.
    #[test]
    fn a_named_axis_is_not_a_step_of_the_same_value() {
        let parsed = decode_written(|rec| {
            rec.set_time_sequence("step", 1i64);
            log_scalars(rec, &[("x", 100.0), ("z", 300.0)]);
            rec.disable_timeline("step");
            rec.set_time_sequence("frame", 1i64);
            log_scalars(rec, &[("y", 999.0)]);
        });
        assert!(
            parsed.rows.is_empty(),
            "neither the step nor the frame carries a whole position: {:?}",
            parsed.rows
        );
    }

    /// An axis beside `sim_time` is part of what identifies a row.
    ///
    /// A chunk carrying `sim_time` had its other axis dropped from the key, so
    /// rows sharing a `sim_time` but sitting at different `frame`s became
    /// repeats of one moment and paired by arrival order: the frame-1 `x` came
    /// back beside the frame-2 `y`. Only frame 2 is whole, so it is the one row.
    #[test]
    fn an_axis_beside_sim_time_identifies_the_row() {
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            for (frame, x) in [(1i64, 100.0f64), (2, 110.0)] {
                rec.set_time_sequence("frame", frame);
                log_scalars(rec, &[("x", x)]);
            }
            for (frame, v) in [(2i64, 201.0f64), (3, 202.0)] {
                rec.set_time_sequence("frame", frame);
                log_scalars(rec, &[("y", v), ("z", v)]);
            }
        });
        assert_eq!(parsed.rows.len(), 1, "{:?}", parsed.rows);
        let row = &parsed.rows[0];
        assert_eq!(
            (row.x, row.y, row.z),
            (110.0, 201.0, 201.0),
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
        let parsed = decode_written(|rec| {
            rec.set_duration_secs("sim_time", 0.0);
            rec.set_time_sequence("frame", 1i64);
            log_scalars(rec, &[("x", 100.0)]);
            rec.disable_timeline("frame");
            log_scalars(rec, &[("z", 300.0)]);
            rec.set_time_sequence("iteration", 7i64);
            log_scalars(rec, &[("y", 999.0)]);
        });
        assert!(
            parsed.rows.is_empty(),
            "the `iteration` chunk cannot be placed among the rest: {:?}",
            parsed.rows
        );
    }

    /// An empty velocity column leaves a position-only recording position-only.
    ///
    /// An empty `Scalars` batch leaves a column behind with no values in it,
    /// which made the recording look velocity-bearing: every row then wanted a
    /// whole velocity triple and none had one, so the positions that are there
    /// were all dropped.
    #[test]
    fn an_empty_velocity_column_does_not_drop_the_positions() {
        let parsed = decode_written(|rec| {
            for (t, x) in [(0.0f64, 100.0f64), (10.0, 110.0)] {
                rec.set_duration_secs("sim_time", t);
                log_scalars(rec, &[("x", x), ("y", 1.0), ("z", 2.0)]);
                rec.log(
                    format!("{ENTITY}/vx"),
                    &re_sdk_types::archetypes::Scalars::new(Vec::<f64>::new()),
                )
                .expect("log empty vx");
            }
        });
        assert_eq!(parsed.rows.len(), 2, "{:?}", parsed.rows);
        assert_eq!(parsed.rows[0].x, 100.0);
        assert_eq!(parsed.rows[1].x, 110.0);
    }
}
