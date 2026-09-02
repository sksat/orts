use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use orts::record::entity_path::EntityPath;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::sim::core::{AttitudePayload, AttitudeSource, HistoryState, make_history_state};

/// Maximum number of overview points retained per satellite (entity path).
///
/// Each distinct `EntityPath` in the history gets its own adaptively-sampled
/// overview buffer, so the total overview returned on connect scales as
/// `num_satellites * OVERVIEW_MAX_POINTS_PER_ENTITY`. This is still O(1)
/// with respect to sim duration, which is the property that matters for
/// reconnect latency.
///
/// Sized so the JSON payload stays well under a MiB for typical constellations
/// (1–10 sats) and deserializes in a handful of milliseconds on the client,
/// regardless of how long the simulation has been running.
pub const OVERVIEW_MAX_POINTS_PER_ENTITY: usize = 1000;

/// Per-entity adaptively-sampled overview buffer.
///
/// Each entity gets its own sample-rate + counter so that satellites pushed
/// at different cadences (or counts) are all given fair time coverage. A
/// single shared buffer with stride-based halving would systematically bias
/// against some satellites depending on their push order parity — this
/// per-entity split eliminates that failure mode.
/// Invariant: `buffer.back()` is always the most recent push for this
/// entity, even when that push did not fall on a sampling boundary. This
/// is maintained by the "tail overwrite" trick in
/// [`HistoryBuffer::push`]: non-sampling pushes replace the trailing
/// slot in place instead of being discarded, so reconnecting clients see
/// "where this sat is right now" regardless of where the sample rate
/// happens to land. `halve()` preserves the same invariant by explicitly
/// keeping the last element after the stride pass.
struct EntityOverview {
    buffer: VecDeque<HistoryState>,
    /// Only every Nth `push()` for this entity opens a *new* slot in the
    /// buffer. Non-sampling pushes in between overwrite the trailing slot
    /// to maintain the "back = most recent push" invariant.
    sample_rate: usize,
    /// Counter for sample-rate divisibility.
    push_counter: usize,
}

impl EntityOverview {
    fn new() -> Self {
        Self {
            buffer: VecDeque::with_capacity(OVERVIEW_MAX_POINTS_PER_ENTITY + 1),
            sample_rate: 1,
            push_counter: 0,
        }
    }

    /// Halve the buffer in-place: keep every other point, always retain the
    /// most recent one so the client sees "where this sat is right now" on
    /// reconnect. Doubles the sample rate so subsequent pushes are ingested
    /// at the new coarser cadence.
    fn halve(&mut self) {
        let n = self.buffer.len();
        if n == 0 {
            return;
        }
        let last_idx = n - 1;
        let mut new_buffer = VecDeque::with_capacity(OVERVIEW_MAX_POINTS_PER_ENTITY + 1);
        for i in (0..n).step_by(2) {
            if i == last_idx {
                continue;
            }
            new_buffer.push_back(self.buffer[i].clone());
        }
        new_buffer.push_back(self.buffer[last_idx].clone());
        self.buffer = new_buffer;
        self.sample_rate *= 2;
    }
}

/// Bounded buffer that accumulates history states and periodically spills the
/// oldest half to a segment file on disk.
pub struct HistoryBuffer {
    /// Recent states kept in memory.
    pub states: VecDeque<HistoryState>,
    /// Maximum number of states to keep in memory before flushing.
    pub capacity: usize,
    /// Directory for spilled segment files.
    pub data_dir: PathBuf,
    /// Number of segment files written so far.
    pub segment_count: u32,
    /// In-memory length above which the next `push` attempts a spill.
    ///
    /// Normally `capacity`. After a failed spill it is raised so the retry
    /// happens once the buffer has grown by another `capacity` states
    /// rather than on every push.
    flush_at: usize,
    /// Consecutive failed spill attempts (reset by the next success).
    failed_spills: u32,
    /// Gravitational parameter (for computing Keplerian elements from loaded data).
    pub mu: f64,
    /// Central body radius [km] (for computing derived values from loaded data).
    pub body_radius: f64,

    // Incremental per-entity overview
    //
    // Maintained in O(1) amortized per `push()` call, read in
    // O(num_entities * OVERVIEW_MAX_POINTS_PER_ENTITY) with no disk I/O.
    // This lets re-connects to long-running simulations return the history
    // overview instantly, without re-reading every spilled segment from disk
    // on the manager task. Per-entity bookkeeping ensures every satellite
    // gets fair coverage regardless of push order or count.
    overview_per_entity: HashMap<EntityPath, EntityOverview>,
}

impl HistoryBuffer {
    pub fn new(capacity: usize, data_dir: PathBuf, mu: f64, body_radius: f64) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
        HistoryBuffer {
            states: VecDeque::new(),
            capacity,
            data_dir,
            segment_count: 0,
            flush_at: capacity,
            failed_spills: 0,
            mu,
            body_radius,
            overview_per_entity: HashMap::new(),
        }
    }

    /// Push a state into the buffer. Spills to disk if capacity is exceeded,
    /// and incrementally updates the per-entity overview buffers.
    ///
    /// Clone cost: non-sampling pushes perform one `state.clone()` into
    /// the trailing overview slot (the tail-overwrite that preserves the
    /// "back = most recent push" invariant). This is a tiny regression
    /// compared to a pure sample-and-skip approach but keeps the overview
    /// useful on reconnect without a separate "latest per sat" slot.
    pub fn push(&mut self, state: HistoryState) {
        // Update the per-entity overview first. We always ensure the entity's
        // buffer ends with the most recent push for that entity, so
        // reconnecting clients see "where the sat is right now" even if the
        // most recent push did not fall on a sampling boundary. Non-sampling
        // pushes overwrite the tail slot in place; sampling boundaries
        // append a new slot and may trigger a halve.
        let entry = self
            .overview_per_entity
            .entry(state.entity_path.clone())
            .or_insert_with(EntityOverview::new);
        entry.push_counter += 1;
        let on_sampling_boundary = entry.push_counter.is_multiple_of(entry.sample_rate);
        if on_sampling_boundary || entry.buffer.is_empty() {
            entry.buffer.push_back(state.clone());
            if entry.buffer.len() > OVERVIEW_MAX_POINTS_PER_ENTITY {
                entry.halve();
            }
        } else if let Some(slot) = entry.buffer.back_mut() {
            // Between sampling boundaries, replace the trailing slot so
            // `buffer.back()` invariantly holds the entity's latest push.
            *slot = state.clone();
        }

        self.states.push_back(state);
        if self.states.len() >= self.flush_at {
            self.flush();
        }
        // The cap is a promise about memory, so it holds on every push. The
        // retry cadence above only decides when the next write is attempted:
        // measured before this call was here, a failing spill at capacity 4
        // reached 35 states against a cap of 32, because the backoff moved the
        // next attempt three pushes past it.
        self.enforce_memory_cap();
    }

    /// Return a snapshot of the overview: the union of every entity's
    /// bounded adaptive-sample buffer, sorted chronologically.
    ///
    /// Reads from memory only: does not touch disk, does not call
    /// `load_all()`. Cost is
    /// O(num_entities * OVERVIEW_MAX_POINTS_PER_ENTITY) regardless of how
    /// many points have been pushed or how many segments have been flushed.
    pub fn overview(&self) -> Vec<HistoryState> {
        let mut all: Vec<HistoryState> = self
            .overview_per_entity
            .values()
            .flat_map(|e| e.buffer.iter().cloned())
            .collect();
        all.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        all
    }

    /// Spill the oldest half of the buffer to a segment file.
    ///
    /// The states are written from a borrow and drained only after the write
    /// succeeded, so the in-memory buffer stays the single source of truth
    /// for everything not yet on disk: a failed spill degrades to "memory
    /// holds more than `capacity`", never to lost history.
    pub fn flush(&mut self) {
        let flush_count = self.states.len() / 2;
        if flush_count == 0 {
            return;
        }

        let seg_path = self.segment_path(self.segment_count);
        match Self::write_segment(&seg_path, self.states.iter().take(flush_count)) {
            Ok(()) => {
                self.states.drain(..flush_count);
                self.segment_count += 1;
                self.flush_at = self.capacity;
                self.failed_spills = 0;
            }
            Err(e) => {
                // Drop a half-written file: the next attempt writes this
                // same index, and leaving a truncated segment behind only
                // invites reading it.
                let _ = std::fs::remove_file(&seg_path);
                self.failed_spills += 1;
                eprintln!(
                    "Warning: failed to spill history to {}: {e} \
                     ({} states kept in memory)",
                    seg_path.display(),
                    self.states.len()
                );
                self.enforce_memory_cap();
                // Retry once the buffer has grown by another `capacity`
                // states, so a permanently unwritable spill directory costs
                // one failed write per `capacity` pushes instead of one per
                // push.
                //
                // Kept at or below the memory cap, because the cap is what
                // bounds the length: a threshold above it is never reached
                // again, and a directory that becomes writable later would
                // never be tried. Measured with an unclamped threshold: after
                // 100 failing pushes at capacity 4 the threshold sat at 36
                // against a cap of 32, and 100 further pushes to a writable
                // directory wrote no segment.
                //
                // `saturating_add`: the capacity comes from the caller, and a
                // wrapped sum would put the retry threshold below the current
                // length, spilling on every push — the opposite of the backoff.
                self.flush_at = self
                    .states
                    .len()
                    .saturating_add(self.capacity)
                    .min(self.memory_cap());
            }
        }
    }

    /// Drop the oldest states once a failing spill has grown the buffer past
    /// `capacity * MAX_RETAINED_BUFFERS`.
    ///
    /// Retaining unflushed history is the right trade against a transient
    /// write failure, but `serve` is a long-running process and a
    /// permanently unwritable spill directory would otherwise grow the
    /// buffer without bound. Past the cap bounded memory wins — loudly, and
    /// the per-entity overview still covers the discarded span.
    ///
    /// Called from `push`, so the cap holds between write attempts too. A
    /// failed write moves the next attempt `capacity` pushes out, and those
    /// pushes would otherwise sit above the cap.
    /// The most states the buffer holds while its writes keep failing.
    fn memory_cap(&self) -> usize {
        self.capacity.saturating_mul(MAX_RETAINED_BUFFERS)
    }

    fn enforce_memory_cap(&mut self) {
        let cap = self.memory_cap();
        // At the cap, not only past it. A failed write leaves the length there
        // and the retry threshold there, so waiting for one more push would
        // spend a second failed write on the same cycle — measured, attempts
        // landed on pushes 31 and 32, then 36 and 37, against a documented one
        // per `capacity` pushes.
        if self.states.len() < cap {
            return;
        }
        let keep = cap.saturating_sub(self.capacity);
        let drop_count = self.states.len() - keep;
        let until_t = self.states.get(drop_count).map(|s| s.t);
        eprintln!(
            "Warning: history spill has failed {} times; discarding the {drop_count} oldest \
             states to keep memory bounded (full-fidelity history before t = {} is gone)",
            self.failed_spills,
            until_t.map_or("end of run".to_string(), |t| format!("{t}"))
        );
        self.states.drain(..drop_count);
    }

    fn segment_path(&self, index: u32) -> PathBuf {
        self.data_dir.join(format!("seg_{index:04}.jsonl"))
    }

    /// Write `states` as one JSON record per line.
    fn write_segment<'a>(
        path: &Path,
        states: impl Iterator<Item = &'a HistoryState>,
    ) -> std::io::Result<()> {
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        for hs in states {
            serde_json::to_writer(&mut w, &SegmentRecord::from_state(hs))
                .map_err(std::io::Error::other)?;
            w.write_all(b"\n")?;
        }
        w.flush()
    }

    /// Read back a segment written by [`Self::write_segment`].
    ///
    /// A malformed line is reported and skipped rather than failing the whole
    /// segment: one bad record must not cost the rest of the window.
    fn read_segment(&self, path: &Path) -> std::io::Result<Vec<HistoryState>> {
        let reader = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut states = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SegmentRecord>(&line) {
                Ok(rec) => states.push(rec.into_state(self.mu, self.body_radius)),
                Err(e) => eprintln!(
                    "Warning: skipping malformed history record {}:{}: {e}",
                    path.display(),
                    i + 1
                ),
            }
        }
        Ok(states)
    }

    /// Load all data: spilled segments + in-memory buffer, sorted by time.
    pub fn load_all(&self) -> Vec<HistoryState> {
        let mut all = Vec::new();

        for i in 0..self.segment_count {
            let seg_path = self.segment_path(i);
            match self.read_segment(&seg_path) {
                Ok(states) => all.extend(states),
                Err(e) => eprintln!("Warning: failed to read segment {i}: {e}"),
            }
        }

        // Append in-memory buffer
        all.extend(self.states.iter().cloned());

        // Sort by time for multi-satellite interleaving
        all.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        all
    }

    /// Query states within a time range, optionally downsampled.
    ///
    /// Two-tier read path:
    /// - **Fast path (no disk I/O)**: if `t_min` is newer than the oldest
    ///   point currently in the in-memory tail (`self.states`), every state
    ///   in the requested window must already be in memory. Filter the
    ///   tail and skip reading any spilled segments.
    /// - **Slow path (full load)**: otherwise, the window reaches into
    ///   flushed segments on disk; fall back to `load_all()` + filter.
    ///
    /// The fast path is what makes the viewer's proactive initial
    /// `query_range` on (re)connect cheap: the client typically asks for
    /// "the last `timeRange` seconds", which for any sane `timeRange`
    /// fits entirely inside the in-memory tail (bounded by `capacity`).
    /// Without the fast path, every reconnect would stall the sim loop on
    /// a full segment read, undoing the O(1) handshake cost won by the
    /// overview cache.
    ///
    /// When `entity_path` is `Some`, only states belonging to that
    /// entity are returned. The filter is applied **before**
    /// `max_points` downsampling so the budget goes entirely to the
    /// target entity instead of being diluted across every interleaved
    /// satellite in the window.
    pub fn query_range(
        &self,
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        entity_path: Option<&EntityPath>,
    ) -> Vec<HistoryState> {
        let in_memory_sufficient = self.states.front().is_some_and(|oldest| oldest.t <= t_min);

        let matches = |s: &HistoryState| {
            s.t >= t_min && s.t <= t_max && entity_path.is_none_or(|ep| s.entity_path == *ep)
        };

        let filtered: Vec<HistoryState> = if in_memory_sufficient {
            self.states.iter().filter(|s| matches(s)).cloned().collect()
        } else {
            self.load_all().into_iter().filter(matches).collect()
        };

        match max_points {
            Some(mp) => Self::downsample(&filtered, mp),
            None => filtered,
        }
    }

    /// Downsample a list of states to at most `max_points`, always preserving first and last.
    pub fn downsample(states: &[HistoryState], max_points: usize) -> Vec<HistoryState> {
        crate::sim::core::downsample_states(states, max_points)
    }
}

/// How many `capacity`-sized buffers may accumulate in memory while the
/// spill keeps failing, before history is discarded to bound memory.
const MAX_RETAINED_BUFFERS: usize = 8;

/// One line of a spilled history segment.
///
/// The spill is a private implementation detail of `serve` — it lives in a
/// per-pid temp directory and nothing outside this module reads it — so it
/// stores the payload verbatim instead of going through `.rrd`: `RrdRow` has
/// no room for the per-force acceleration breakdown (a map with
/// model-defined keys) or for reaction-wheel momentum (one entry per wheel),
/// and dropping them made the viewer draw flushed windows with a gravity
/// acceleration of 0.
///
/// Only the independent state is stored; the derived columns (Keplerian
/// elements, altitude, energy, …) are rebuilt on load by
/// [`make_history_state`] — the same function that produced them for the
/// in-memory copy — so a state read back from disk is field-for-field
/// identical to the one that was pushed.
#[derive(Serialize, Deserialize)]
struct SegmentRecord {
    entity_path: EntityPath,
    t: F64,
    position: [F64; 3],
    velocity: [F64; 3],
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    accelerations: HashMap<String, F64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attitude: Option<SegmentAttitude>,
}

#[derive(Serialize, Deserialize)]
struct SegmentAttitude {
    quaternion_wxyz: [F64; 4],
    angular_velocity_body: [F64; 3],
    source: AttitudeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rw_momentum: Option<Vec<F64>>,
}

impl SegmentRecord {
    fn from_state(hs: &HistoryState) -> Self {
        SegmentRecord {
            entity_path: hs.entity_path.clone(),
            t: F64(hs.t),
            position: hs.position.map(F64),
            velocity: hs.velocity.map(F64),
            accelerations: hs
                .accelerations
                .iter()
                .map(|(k, v)| (k.clone(), F64(*v)))
                .collect(),
            attitude: hs.attitude.as_ref().map(|att| SegmentAttitude {
                quaternion_wxyz: att.quaternion_wxyz.map(F64),
                angular_velocity_body: att.angular_velocity_body.map(F64),
                source: att.source.clone(),
                rw_momentum: att
                    .rw_momentum
                    .as_ref()
                    .map(|h| h.iter().copied().map(F64).collect()),
            }),
        }
    }

    fn into_state(self, mu: f64, body_radius: f64) -> HistoryState {
        let position = self.position.map(|f| f.0);
        let velocity = self.velocity.map(|f| f.0);
        make_history_state(
            self.entity_path,
            self.t.0,
            &nalgebra::Vector3::from_row_slice(&position),
            &nalgebra::Vector3::from_row_slice(&velocity),
            mu,
            body_radius,
            self.accelerations
                .into_iter()
                .map(|(k, v)| (k, v.0))
                .collect(),
            self.attitude.map(|att| AttitudePayload {
                quaternion_wxyz: att.quaternion_wxyz.map(|f| f.0),
                angular_velocity_body: att.angular_velocity_body.map(|f| f.0),
                source: att.source,
                rw_momentum: att
                    .rw_momentum
                    .map(|h| h.into_iter().map(|f| f.0).collect()),
            }),
        )
    }
}

/// A float that survives a JSON round-trip exactly.
///
/// Two hazards make plain JSON numbers unsafe for a lossless spill.
/// `serde_json` writes NaN and ±∞ as `null`, which then fails to read back —
/// a diverging run would lose exactly the rows an operator wants to look at.
/// And its default number parser does not guarantee that the `f64` read back
/// is the one that was written (that needs the `float_roundtrip` feature):
/// `0.03 + 2e-5` comes back one ulp away. Both disappear when the value
/// travels as text: Rust's `f64` `Display`/`FromStr` pair round-trips every
/// finite value bit for bit, and carries `NaN` and `±inf` across as
/// themselves. (A `NaN`'s sign and payload are not preserved — `Display`
/// folds every `NaN` to `"NaN"` — which is what a spilled history needs:
/// "this row diverged", not which bit pattern the FPU produced.)
#[derive(Clone, Copy, Debug, PartialEq)]
struct F64(f64);

impl Serialize for F64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for F64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        text.parse().map(F64).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MU: f64 = 398600.4418;
    const TEST_BODY_RADIUS: f64 = 6378.137;

    fn make_state(t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        make_history_state(
            EntityPath::parse("/world/sat/default"),
            t,
            &pos,
            &vel,
            TEST_MU,
            TEST_BODY_RADIUS,
            HashMap::new(),
            None,
        )
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orts-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    fn cleanup_dir(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // Incremental overview buffer
    //
    // The `overview()` method must return a bounded, time-spanning summary
    // of the full simulation history in constant time, independent of how
    // many points have been pushed or how many segments have been flushed
    // to disk. This is the regression gate for the "viewer blank after
    // reload" problem on long-running sims.

    #[test]
    fn overview_empty_buffer() {
        let dir = temp_data_dir("overview-empty");
        let buf = HistoryBuffer::new(5000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        assert_eq!(buf.overview().len(), 0);
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_returns_all_points_below_cap() {
        let dir = temp_data_dir("overview-below-cap");
        let mut buf = HistoryBuffer::new(5000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..500 {
            buf.push(make_state(i as f64));
        }
        let ov = buf.overview();
        assert_eq!(ov.len(), 500);
        assert!((ov[0].t - 0.0).abs() < 1e-9);
        assert!((ov[499].t - 499.0).abs() < 1e-9);
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_is_bounded_above_cap() {
        let dir = temp_data_dir("overview-bounded");
        let mut buf = HistoryBuffer::new(5000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..5_000 {
            buf.push(make_state(i as f64));
        }
        let ov = buf.overview();
        assert!(
            ov.len() <= OVERVIEW_MAX_POINTS_PER_ENTITY,
            "single-entity overview should be bounded at {OVERVIEW_MAX_POINTS_PER_ENTITY}, got {}",
            ov.len()
        );
        // Most recent push must always be retained so the client can render
        // "where the sim is right now" immediately after (re)connect.
        let last = ov.last().expect("non-empty");
        assert!(
            (last.t - 4999.0).abs() < 1e-9,
            "last overview point must be the most recent push, got t={}",
            last.t
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_survives_many_flushes() {
        // Small in-memory capacity so flush() fires many times. Overview
        // must still give full time coverage and remain bounded.
        let dir = temp_data_dir("overview-flushes");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..20_000 {
            buf.push(make_state(i as f64));
        }
        assert!(
            buf.segment_count > 0,
            "precondition: many flushes should have occurred"
        );
        let ov = buf.overview();
        assert!(ov.len() <= OVERVIEW_MAX_POINTS_PER_ENTITY);
        let last = ov.last().expect("non-empty");
        assert!((last.t - 19_999.0).abs() < 1e-9);
        // Earliest retained point should span the full time range — it must
        // come from early in the sim, not from the most-recent in-memory
        // window. Adaptive sampling drops in-between points, but the
        // leading edge should still be near the start.
        assert!(
            ov[0].t < 1_000.0,
            "overview must cover the full sim time range; earliest t={} is too late",
            ov[0].t
        );
        cleanup_dir(&dir);
    }

    /// Push a state for a specific satellite id. The overview buffer must
    /// give fair coverage to each distinct `entity_path`, even when
    /// satellites push interleaved into the same buffer. Without per-entity
    /// bookkeeping, a stride-based halving systematically drops one of the
    /// satellites on each halve (especially with an even number of sats).
    fn make_state_for(sat_id: &str, t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        make_history_state(
            EntityPath::parse(&format!("/world/sat/{sat_id}")),
            t,
            &pos,
            &vel,
            TEST_MU,
            TEST_BODY_RADIUS,
            HashMap::new(),
            None,
        )
    }

    #[test]
    fn overview_preserves_coverage_for_multiple_satellites() {
        // Two interleaved satellites for many pushes. A naive stride-based
        // halving drops one of them entirely (indices 0, 2, 4, ... all
        // belong to sat-a when the push order is a,b,a,b,...). Per-entity
        // overview bookkeeping keeps both represented.
        let dir = temp_data_dir("overview-multisat");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let sats = ["sat-a", "sat-b"];
        for i in 0..20_000 {
            let sat = sats[i % sats.len()];
            buf.push(make_state_for(sat, i as f64));
        }
        assert!(
            buf.segment_count > 0,
            "precondition: flushes should have occurred"
        );

        let ov = buf.overview();
        assert!(!ov.is_empty(), "overview must not be empty");

        // Count coverage per satellite. Each sat should have a substantial
        // number of points — not just 1 (the boundary retention) and
        // certainly not 0.
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for s in &ov {
            *counts.entry(s.entity_path.to_string()).or_insert(0) += 1;
        }
        for sat in &sats {
            let key = format!("/world/sat/{sat}");
            let count = counts.get(&key).copied().unwrap_or(0);
            assert!(
                count >= 100,
                "satellite {sat} should have substantial overview coverage, \
                 got {count} points; full counts = {counts:?}",
            );
        }
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_preserves_most_recent_per_satellite() {
        // Each satellite's most recent push must survive halving so the
        // client can render "where each sat is right now" on reconnect.
        let dir = temp_data_dir("overview-recent-per-sat");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let sats = ["sat-a", "sat-b", "sat-c"];
        for i in 0..10_000 {
            let sat = sats[i % sats.len()];
            buf.push(make_state_for(sat, i as f64));
        }
        let ov = buf.overview();
        // Compute expected most-recent t per sat from the push schedule.
        // Last push index per sat in 0..10_000 is the largest i where
        // i % sats.len() == sat_idx.
        for (sat_idx, sat) in sats.iter().enumerate() {
            let last_i = (0..10_000)
                .rev()
                .find(|i| i % sats.len() == sat_idx)
                .unwrap();
            let expected_t = last_i as f64;
            let key = format!("/world/sat/{sat}");
            let actual_max_t = ov
                .iter()
                .filter(|s| s.entity_path.to_string() == key)
                .map(|s| s.t)
                .fold(f64::NEG_INFINITY, f64::max);
            assert!(
                (actual_max_t - expected_t).abs() < 1e-9,
                "satellite {sat}: expected max t={expected_t}, got {actual_max_t}"
            );
        }
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_cost_is_constant_regardless_of_disk_segments() {
        // Regression gate. With the old `load_all()` based implementation the
        // cost scaled with the number of flushed segments (disk I/O + decode +
        // sort). The incremental overview buffer must answer from memory in
        // ~O(OVERVIEW_MAX_POINTS_PER_ENTITY) time however many segments exist,
        // so pushing 4x as much must not cost more — which is what makes this a
        // raw-time check rather than a millisecond budget.
        let pushes = [5_000usize, 10_000, 20_000];

        assert_scaling_stable("overview vs segments", 3, || {
            let samples = typical_per_size(&pushes, pushes.len(), |n| {
                let dir = temp_data_dir(&format!("overview-perf-{n}"));
                let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
                for i in 0..n {
                    buf.push(make_state(i as f64));
                }
                assert!(
                    buf.segment_count >= 5,
                    "precondition: enough flushes to make load_all expensive, got {}",
                    buf.segment_count
                );

                let start = std::time::Instant::now();
                let ov = buf.overview();
                let elapsed = start.elapsed();

                assert!(
                    ov.len() <= OVERVIEW_MAX_POINTS_PER_ENTITY,
                    "overview must stay bounded, got {}",
                    ov.len()
                );
                cleanup_dir(&dir);
                elapsed.as_micros()
            });
            // Same bar and reasoning as the downsample check: the samples are
            // small, so timer granularity and cache effects weigh more than they
            // do on the millisecond-scale ones.
            check_raw_time_flat(&samples, 3.0)
        });
    }

    #[test]
    fn overview_multi_entity_cost_is_bounded() {
        // The per-entity overview design flattens every entity buffer into a Vec
        // and sorts by `t` on each read, so its cost should grow about linearly
        // with the number of entities. This guards against accidental O(N^2) or
        // disk-touching regressions if `OVERVIEW_MAX_POINTS_PER_ENTITY` is
        // bumped, or if `overview()` grows auxiliary computation.
        //
        // Behaviour is checked at every entity count below; the cost check is a
        // ratio across counts rather than a millisecond ceiling, because a
        // ceiling here failed once in five runs under deliberate CPU load with
        // nothing wrong.
        let dir = temp_data_dir("overview-multi-perf");
        // Small capacity keeps `flush()` I/O bounded during setup; the
        // per-entity overview fills up regardless of flush cadence.
        let mut buf = HistoryBuffer::new(500, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let sats = [
            "sat-0", "sat-1", "sat-2", "sat-3", "sat-4", "sat-5", "sat-6", "sat-7", "sat-8",
            "sat-9",
        ];
        // 10 sats × 2500 interleaved pushes = 25_000 total. Each sat
        // exceeds OVERVIEW_MAX_POINTS_PER_ENTITY (1000), triggering one
        // halving per entity and reaching the steady-state shape we want
        // to measure.
        for i in 0..25_000 {
            let sat = sats[i % sats.len()];
            buf.push(make_state_for(sat, i as f64));
        }

        let ov = buf.overview();

        // Size bound: at most num_entities × cap points.
        assert!(
            ov.len() <= sats.len() * OVERVIEW_MAX_POINTS_PER_ENTITY,
            "overview size must be bounded, got {}",
            ov.len()
        );
        // Every satellite must appear.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for s in &ov {
            seen.insert(s.entity_path.to_string());
        }
        for sat in &sats {
            let key = format!("/world/sat/{sat}");
            assert!(seen.contains(&key), "missing satellite {sat}");
        }
        // The final Vec must be chronologically sorted (multi-entity
        // flatten + sort contract).
        let mut prev = f64::NEG_INFINITY;
        for s in &ov {
            assert!(s.t >= prev, "overview must be sorted by t");
            prev = s.t;
        }
        cleanup_dir(&dir);

        // Cost per entity must not climb as entities are added. Measured with
        // the same push count per satellite at each width, so only the entity
        // count varies.
        let widths = [5usize, 10, 20];
        assert_scaling_stable("overview vs entities", 3, || {
            let samples = typical_per_size(&widths, widths.len(), |w| {
                let dir = temp_data_dir(&format!("overview-width-{w}"));
                let pushes = w * 2_000;
                // Capacity above the push count so nothing flushes: this check is
                // about `overview()` scaling with entities, and writing hundreds
                // of .rrd segments during setup made it a 60-second test. The
                // sibling test above covers the disk-touching claim with real
                // segments.
                let mut buf =
                    HistoryBuffer::new(pushes + 1, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
                let names: Vec<String> = (0..w).map(|i| format!("sat-{i}")).collect();
                for i in 0..pushes {
                    buf.push(make_state_for(&names[i % w], i as f64));
                }
                assert_eq!(buf.segment_count, 0, "setup must not flush");

                let start = std::time::Instant::now();
                let ov = buf.overview();
                let elapsed = start.elapsed();

                assert!(
                    ov.len() <= w * OVERVIEW_MAX_POINTS_PER_ENTITY,
                    "overview size must be bounded, got {} for {w} entities",
                    ov.len()
                );
                cleanup_dir(&dir);
                elapsed.as_micros()
            });
            // Looser than SCALING_BAR, because this cost legitimately grows a
            // little with entity count. `overview()` concatenates every entity's
            // buffer into one Vec and sorts that whole vector by `t`, so adding
            // entities adds both length and interleaving.
            //
            // Measured rather than derived: on an idle machine across this 4x
            // range the cost is 173.0 / 181.1 / 251.8 us per entity, a 1.46x
            // rise, and under 16-way CPU load the same ratio reached 2.0-2.15x.
            // SCALING_BAR at 2.0 sits inside that, so this check takes 3.0 —
            // 2.1x clear of the idle measurement, 1.4x clear of the loaded one,
            // and still below the 4x or more that quadratic work would show
            // over this range.
            check_cost_per_unit_flat(&samples, 3.0)
        });
    }

    // query_range in-memory fast path
    //
    // The proactive initial `query_range` the viewer fires on every connect
    // asks for "the last N seconds" of history. For any reasonable N that
    // fits inside the in-memory tail (bounded by `capacity`), this must not
    // touch disk — otherwise every reconnect stalls the sim loop on full
    // segment reads, undoing the overview cache's O(1) handshake cost.

    #[test]
    fn query_range_recent_window_skips_disk() {
        // Push enough to trigger many flushes, then query a window small
        // enough to be fully covered by the in-memory tail. The query must
        // complete in ~memory-speed time regardless of how many segments
        // sit on disk.
        let dir = temp_data_dir("query-range-recent");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..20_000 {
            buf.push(make_state(i as f64));
        }
        assert!(
            buf.segment_count >= 10,
            "precondition: enough flushes to make load_all expensive"
        );
        let oldest_in_memory = buf.states.front().expect("non-empty tail").t;
        let latest = 19_999.0;

        // Ask for a window fully inside the in-memory tail.
        let t_min = oldest_in_memory + 10.0;

        let start = std::time::Instant::now();
        let result = buf.query_range(t_min, latest, Some(500), None);
        let elapsed = start.elapsed();

        assert!(!result.is_empty(), "result should contain in-window points");
        assert!(
            result.iter().all(|s| s.t >= t_min && s.t <= latest),
            "all returned states must lie in the requested window"
        );
        assert!(
            elapsed.as_millis() < 10,
            "query_range on a recent window fully covered by the in-memory \
             tail should not touch disk; took {}ms with {} segments on disk",
            elapsed.as_millis(),
            buf.segment_count
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_historical_window_falls_back_to_disk() {
        // A query reaching back before the in-memory tail must still
        // return the correct data, even if that means reading segments.
        // This guards against the fast path being too aggressive.
        let dir = temp_data_dir("query-range-historical");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..20_000 {
            buf.push(make_state(i as f64));
        }
        // Pick a window that is definitely inside an early flushed segment
        // (t=100..200 is long before the in-memory tail starts).
        let oldest_in_memory = buf.states.front().expect("non-empty tail").t;
        assert!(
            oldest_in_memory > 300.0,
            "precondition: tail starts past t=300"
        );

        let result = buf.query_range(100.0, 200.0, None, None);
        assert!(
            !result.is_empty(),
            "historical window should return data from disk"
        );
        assert!(
            result.iter().all(|s| s.t >= 100.0 && s.t <= 200.0),
            "all returned states must be in range"
        );
        let min_t = result.iter().map(|s| s.t).fold(f64::INFINITY, f64::min);
        let max_t = result.iter().map(|s| s.t).fold(f64::NEG_INFINITY, f64::max);
        assert!(min_t < 150.0, "should include early part of window");
        assert!(max_t > 150.0, "should include late part of window");
        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_entity_filter_applied_before_downsample() {
        // Regression: when `SimCommand::QueryRange` downsampled to
        // `max_points` *before* filtering by `entity_path`, multi-sat
        // windows shared the downsample budget across every satellite.
        // With 3 sats and `max_points = 300`, each sat ended up with
        // only ~100 of its own points instead of the full 300 budget.
        // The fix pushes the entity filter down into `query_range` so
        // the budget applies to the already-filtered set.
        let dir = temp_data_dir("query-range-entity-filter");
        let mut buf = HistoryBuffer::new(5_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let sats = ["sat-a", "sat-b", "sat-c"];
        // 300 points per sat (900 total), all in the in-memory tail.
        for i in 0..900 {
            let sat = sats[i % sats.len()];
            buf.push(make_state_for(sat, i as f64));
        }

        let sat_a_path = EntityPath::parse("/world/sat/sat-a");
        let result = buf.query_range(0.0, 900.0, Some(300), Some(&sat_a_path));

        // Every returned point must belong to sat-a.
        for s in &result {
            assert_eq!(
                s.entity_path.to_string(),
                "/world/sat/sat-a",
                "entity filter must apply before downsample"
            );
        }
        // The downsample budget (300) applies to the filtered set: sat-a
        // has exactly 300 points in the window, max_points=300, and
        // `downsample_states` returns the input unchanged when
        // `n <= max_points`, so the result is deterministically 300
        // points. Pre-fix ("downsample 900 interleaved → 300, then keep
        // ~1/3 as sat-a") yielded ~100.
        assert_eq!(
            result.len(),
            300,
            "sat-a should get the full 300-point budget after entity filter",
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_entity_filter_applied_on_slow_path() {
        // The slow path (`load_all()` + filter) must also respect the
        // entity_path argument. The fast-path test above only exercises
        // the in-memory branch; this one forces the slow path by
        // requesting a window older than the in-memory tail, on a
        // multi-sat buffer that has flushed segments.
        let dir = temp_data_dir("query-range-entity-slow");
        let mut buf = HistoryBuffer::new(500, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let sats = ["sat-a", "sat-b", "sat-c"];
        // 1500 interleaved pushes → ~3 flushes, in-memory tail covers
        // only the last ~500 points; the early window goes to disk.
        for i in 0..1500 {
            let sat = sats[i % sats.len()];
            buf.push(make_state_for(sat, i as f64));
        }
        assert!(
            buf.segment_count > 0,
            "precondition: flushes should have occurred"
        );
        let oldest_in_memory = buf.states.front().expect("non-empty tail").t;
        assert!(
            oldest_in_memory > 100.0,
            "precondition: in-memory tail should start past t=100"
        );

        // Window [0, 100] is entirely inside a flushed segment.
        let sat_b_path = EntityPath::parse("/world/sat/sat-b");
        let result = buf.query_range(0.0, 100.0, None, Some(&sat_b_path));

        assert!(
            !result.is_empty(),
            "slow path should return sat-b points from disk"
        );
        for s in &result {
            assert_eq!(
                s.entity_path.to_string(),
                "/world/sat/sat-b",
                "slow-path entity filter must drop other sats"
            );
            assert!(s.t >= 0.0 && s.t <= 100.0);
        }
        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_entity_filter_none_returns_all_entities() {
        // Sanity: passing `None` for `entity_path` preserves the old
        // behaviour of returning every entity's points in the window.
        let dir = temp_data_dir("query-range-entity-none");
        let mut buf = HistoryBuffer::new(5_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..300 {
            let sat = ["sat-a", "sat-b"][i % 2];
            buf.push(make_state_for(sat, i as f64));
        }

        let result = buf.query_range(0.0, 300.0, None, None);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for s in &result {
            seen.insert(s.entity_path.to_string());
        }
        assert_eq!(seen.len(), 2, "both sats should be present");
        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_window_spanning_both_tiers_returns_full_coverage() {
        // Window partially in flushed segments and partially in the
        // in-memory tail must return the union (no gap, no duplicates).
        let dir = temp_data_dir("query-range-spanning");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..10_000 {
            buf.push(make_state(i as f64));
        }
        let oldest_in_memory = buf.states.front().expect("non-empty tail").t;
        assert!(oldest_in_memory > 0.0 && oldest_in_memory < 9_999.0);

        // Window that straddles the disk/memory boundary.
        let t_min = oldest_in_memory - 500.0;
        let t_max = oldest_in_memory + 500.0;
        let result = buf.query_range(t_min, t_max, None, None);

        assert!(
            !result.is_empty(),
            "straddling window should return coverage from both tiers"
        );
        for s in &result {
            assert!(s.t >= t_min && s.t <= t_max);
        }
        // Points from both sides of the boundary should be present.
        let has_pre_boundary = result.iter().any(|s| s.t < oldest_in_memory);
        let has_post_boundary = result.iter().any(|s| s.t >= oldest_in_memory);
        assert!(
            has_pre_boundary && has_post_boundary,
            "result must span both flushed segment and in-memory tail"
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_push_and_read() {
        let dir = temp_data_dir("push-read");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        buf.push(make_state(0.0));
        buf.push(make_state(10.0));
        buf.push(make_state(20.0));

        let all = buf.load_all();
        assert_eq!(all.len(), 3);
        assert!((all[0].t - 0.0).abs() < 1e-9);
        assert!((all[1].t - 10.0).abs() < 1e-9);
        assert!((all[2].t - 20.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_flush_creates_segment() {
        let dir = temp_data_dir("flush-seg");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..5 {
            buf.push(make_state(i as f64 * 10.0));
        }

        assert_eq!(buf.segment_count, 1);
        assert!(dir.join("seg_0000.jsonl").exists());
        assert_eq!(buf.states.len(), 3);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_load_all_includes_flushed_and_buffered() {
        let dir = temp_data_dir("load-all");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..8 {
            buf.push(make_state(i as f64 * 10.0));
        }

        assert!(buf.segment_count > 0);

        let all = buf.load_all();
        assert_eq!(all.len(), 8);

        let mut times: Vec<f64> = all.iter().map(|s| s.t).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for i in 0..8 {
            assert!(
                (times[i] - i as f64 * 10.0).abs() < 0.01,
                "times[{i}] = {}, expected {}",
                times[i],
                i as f64 * 10.0
            );
        }

        cleanup_dir(&dir);
    }

    #[test]
    fn downsample_correctness() {
        let states: Vec<HistoryState> = (0..100).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);

        assert_eq!(ds.len(), 10);
        assert!((ds[0].t - 0.0).abs() < 1e-9);
        assert!((ds[9].t - 99.0).abs() < 1e-9);
    }

    #[test]
    fn downsample_preserves_all_when_small() {
        let states: Vec<HistoryState> = (0..5).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);
        assert_eq!(ds.len(), 5);
    }

    /// Typical measurement per size, in microseconds.
    ///
    /// `measure` does its own setup for the size it is handed and returns only
    /// the timed part, so growing inputs do not charge their construction to
    /// the measurement.
    ///
    /// Two things keep measurement order from masquerading as scale.
    ///
    /// Every repetition covers every size and the starting point rotates, so
    /// with `reps == sizes.len()` each size occupies each position exactly once
    /// — size 0 lands in positions 0, 2, 1 over three repetitions, and the
    /// others likewise. Without that, running every repetition of the smallest
    /// input before starting the largest would tie time order to input size,
    /// and a runner that slows down partway through would look exactly like
    /// cost climbing with size.
    ///
    /// The samples are then reduced by their median rather than their minimum.
    /// The minimum looks attractive — scheduling noise only ever adds time —
    /// but it undoes the balance the rotation just bought: under a monotonic
    /// slowdown the fastest sample for every size is its first occurrence, and
    /// the first repetition runs the sizes in ascending order, so the minima
    /// come back ordered by size again. The median draws on the whole balanced
    /// set and still discards a single spike.
    fn typical_per_size(
        sizes: &[usize],
        reps: usize,
        mut measure: impl FnMut(usize) -> u128,
    ) -> Vec<(usize, u128)> {
        let mut samples: Vec<Vec<u128>> = vec![Vec::with_capacity(reps); sizes.len()];
        for rep in 0..reps {
            for offset in 0..sizes.len() {
                let i = (offset + rep) % sizes.len();
                samples[i].push(measure(sizes[i]));
            }
        }
        sizes
            .iter()
            .copied()
            .zip(samples.into_iter().map(|mut s| {
                s.sort_unstable();
                s[s.len() / 2]
            }))
            .collect()
    }

    /// Default ratio the checks below allow between their worst and best
    /// sample. Callers pass their own where the work has a shape this does not
    /// fit — `overview()` across entity counts and `downsample` across input
    /// sizes both take 3.0, each with its measurements recorded at the call.
    ///
    /// Quadratic growth over a 4x size range shows up as ~4x in cost per unit;
    /// n log n over the same range is ~1.3x. 2.0 sits between them.
    const SCALING_BAR: f64 = 2.0;

    /// Assert the cost per unit of input does not climb as the input grows —
    /// that is, the work stays about linear.
    ///
    /// This says nothing about absolute speed, deliberately. The same flush
    /// measured 364-400ms on a Linux runner, 642-717ms on a Windows one, and
    /// 705-1610ms once the rest of the test suite was running alongside it;
    /// Windows also moved 2x between runs of the same image. A millisecond
    /// budget across that spread reports how busy the machine was. A ratio
    /// between sizes measured back to back on one machine does not: both halves
    /// absorb the same noise.
    ///
    /// The blind spot is a constant-factor regression — ten times slower per
    /// unit, still linear, still passes. Catching that needs a stable machine
    /// and a history to compare against rather than a single CI run.
    fn check_cost_per_unit_flat(samples: &[(usize, u128)], bar: f64) -> Result<(), String> {
        let per_unit: Vec<f64> = samples
            .iter()
            .map(|(n, us)| *us as f64 / *n as f64)
            .collect();

        let detail: Vec<String> = samples
            .iter()
            .zip(&per_unit)
            .map(|((n, us), c)| format!("n={n}: {us}us ({c:.4}us/unit)"))
            .collect();

        // Only a *rise* against an earlier, smaller size is a problem, and each
        // size is checked so a spike in the middle cannot hide between its
        // neighbours.
        //
        // The direction matters. A per-unit cost that falls as the input grows
        // is what amortising a fixed cost looks like, and `load_all` does
        // exactly that: it pays per-segment decode for a segment count this
        // fixture holds constant, so the smallest input carries the largest
        // share of it. Measured under deliberate CPU load, that legitimately
        // produced 215.6 / 139.0 / 71.0 us per state — a 3.04x spread with
        // nothing wrong. Comparing the extremes regardless of direction failed
        // it; comparing only upward movement passes it and still catches 1, 5, 1.
        let mut best_so_far = f64::INFINITY;
        for (i, &cost) in per_unit.iter().enumerate() {
            if cost / best_so_far > bar {
                return Err(format!(
                    "cost per unit rose {:.2}x at n={} against the cheapest smaller \
                     input (bar {bar:.1}x). Samples — {}",
                    cost / best_so_far,
                    samples[i].0,
                    detail.join(", ")
                ));
            }
            best_so_far = best_so_far.min(cost);
        }
        Ok(())
    }

    /// Run `attempt` until it reports no violation, failing only if every
    /// attempt does.
    ///
    /// This is what separates the two things a timing gate can see. Work that
    /// became super-linear trips the bar every time; a runner that stalled for a
    /// moment trips it once. Rotating the order and taking a median per size
    /// reduce the second without removing it — measured under 16-way saturation,
    /// one size came out at 340us/unit while its neighbours sat at 140 and 119,
    /// which is noise wearing the shape of a regression.
    ///
    /// Each attempt is reported as it happens, so a genuine regression leaves
    /// the full trail in the log rather than only its last measurement.
    fn assert_scaling_stable(
        label: &str,
        attempts: u32,
        mut attempt: impl FnMut() -> Result<(), String>,
    ) {
        let mut last = String::new();
        for i in 1..=attempts {
            match attempt() {
                Ok(()) => return,
                Err(msg) => {
                    eprintln!("{label}: attempt {i}/{attempts} tripped the bar — {msg}");
                    last = msg;
                }
            }
        }
        panic!(
            "{label}: all {attempts} attempts tripped the bar, so this is the shape of \
             the work rather than a busy machine. Last — {last}"
        );
    }

    /// Assert the raw time does not grow with the input — for work whose cost is
    /// set by something other than the input length.
    ///
    /// `downsample_states` is the case that needs this: it performs
    /// `max_points - 2` stride-indexed clones, so a larger input changes the
    /// index arithmetic and nothing else. Measured at a fixed `max_points` of
    /// 1000, it takes 164 / 167 / 165 / 180us for 100k / 200k / 400k / 800k
    /// states — 1.10x across an 8x size range.
    ///
    /// Normalising that by input length would make the check vacuous, and worse
    /// than vacuous: an accidental full-input scan would turn raw time linear,
    /// which flattens cost-per-state and looks like success. Holding raw time
    /// flat catches it, since such a regression grows with the size range.
    fn check_raw_time_flat(samples: &[(usize, u128)], bar: f64) -> Result<(), String> {
        let times: Vec<f64> = samples.iter().map(|(_, us)| *us as f64).collect();
        let best = times.iter().copied().fold(f64::INFINITY, f64::min);
        let worst = times.iter().copied().fold(0.0_f64, f64::max);

        let detail: Vec<String> = samples
            .iter()
            .map(|(n, us)| format!("n={n}: {us}us"))
            .collect();

        if worst / best > bar {
            return Err(format!(
                "raw time spread {:.2}x across sizes (bar {bar:.1}x) for work whose \
                 cost should not depend on input length. Samples — {}",
                worst / best,
                detail.join(", ")
            ));
        }
        Ok(())
    }

    // The gates above are only as good as these two functions, and the timing
    // tests exercise whichever branch the machine happens to produce. These feed
    // them fixed samples so a regression in the logic itself cannot pass
    // unnoticed.

    #[test]
    fn cost_per_unit_check_rejects_a_rise_at_any_size() {
        // Flat cost per unit: 1us per unit at every size.
        assert!(
            check_cost_per_unit_flat(&[(100, 100), (200, 200), (400, 400)], 2.0).is_ok(),
            "a flat series must pass"
        );

        // A spike in the middle. This is the case an endpoint comparison misses,
        // since the first and last samples are identical.
        let err = check_cost_per_unit_flat(&[(100, 100), (200, 1000), (400, 400)], 2.0)
            .expect_err("a 5x spike in the middle must fail");
        assert!(
            err.contains("n=200"),
            "the message must name the size: {err}"
        );

        // A rise only at the largest size.
        assert!(
            check_cost_per_unit_flat(&[(100, 100), (200, 200), (400, 1600)], 2.0).is_err(),
            "a 4x rise at the largest size must fail"
        );

        // Falling cost per unit is what amortising a fixed cost looks like, and
        // must pass however far it falls.
        assert!(
            check_cost_per_unit_flat(&[(100, 1000), (200, 800), (400, 400)], 2.0).is_ok(),
            "a decreasing series must pass"
        );

        // Right at the bar, and just past it.
        assert!(
            check_cost_per_unit_flat(&[(100, 100), (200, 400)], 2.0).is_ok(),
            "exactly 2.0x must pass at a 2.0 bar"
        );
        assert!(
            check_cost_per_unit_flat(&[(100, 100), (200, 420)], 2.0).is_err(),
            "2.1x must fail at a 2.0 bar"
        );
    }

    #[test]
    fn raw_time_check_rejects_growth_in_either_direction() {
        // Constant work: the input grows 8x and the time does not.
        assert!(
            check_raw_time_flat(&[(100, 500), (200, 510), (800, 520)], 3.0).is_ok(),
            "flat raw time must pass"
        );

        // The O(n) regression this guards against.
        let err = check_raw_time_flat(&[(100, 500), (200, 1000), (800, 4000)], 3.0)
            .expect_err("8x growth must fail");
        assert!(
            err.contains("n=800"),
            "the message must carry the samples: {err}"
        );

        // Unlike the per-unit check, this one is a spread: a dip is as
        // interesting as a rise, since either means the cost tracks the input.
        assert!(
            check_raw_time_flat(&[(100, 4000), (200, 1000), (800, 500)], 3.0).is_err(),
            "an 8x fall must fail too"
        );
    }

    #[test]
    fn scaling_retry_needs_every_attempt_to_trip() {
        // Trips once, then passes: the machine was busy, not the code.
        let mut calls = 0;
        assert_scaling_stable("transient", 3, || {
            calls += 1;
            if calls == 1 {
                Err("first attempt".to_string())
            } else {
                Ok(())
            }
        });
        assert_eq!(calls, 2, "must stop as soon as an attempt passes");

        // Passes first time: no repetition at all.
        let mut calls = 0;
        assert_scaling_stable("clean", 3, || {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 1);
    }

    #[test]
    #[should_panic(expected = "all 3 attempts tripped the bar")]
    fn scaling_retry_fails_when_every_attempt_trips() {
        assert_scaling_stable("persistent", 3, || Err("every time".to_string()));
    }

    #[test]
    fn typical_per_size_rotates_and_takes_the_median() {
        // Record the order the sizes are measured in, and hand back a value that
        // identifies which call it was, so the median is checkable.
        let mut order = Vec::new();
        let mut nth = 0u128;
        let samples = typical_per_size(&[10usize, 20, 30], 3, |n| {
            order.push(n);
            nth += 1;
            // size 20's three calls return 5, 1, 3 -> median 3
            match n {
                20 => [5u128, 1, 3][(order.iter().filter(|&&s| s == 20).count()) - 1],
                _ => nth,
            }
        });

        assert_eq!(
            order,
            vec![10, 20, 30, 20, 30, 10, 30, 10, 20],
            "each size must occupy each position exactly once"
        );
        let mid = samples.iter().find(|(n, _)| *n == 20).expect("size 20");
        assert_eq!(mid.1, 3, "the median of 5, 1, 3 is 3");
    }

    #[test]
    fn downsample_cost_stays_independent_of_input_size() {
        // Sizes span 8x. A 4x range leaves too little margin: an injected
        // full-input scan measured 3.06x against a 3.0x bar, because the
        // constant part of the work dilutes the ratio. Over 8x the same fault
        // lands far clear of the bar while the clean measurement stays at 1.10x.
        let sizes = [100_000usize, 200_000, 400_000, 800_000];

        // Built once up front: this operation only reads, so the same input can
        // be measured repeatedly, and construction stays out of the timings.
        let inputs: Vec<Vec<HistoryState>> = sizes
            .iter()
            .map(|&n| (0..n).map(|i| make_state(i as f64)).collect())
            .collect();

        // Looser than SCALING_BAR at 3.0: these samples are a few hundred
        // microseconds, where timer granularity and cache effects weigh more.
        // Measured 1.10x across an 8x range, and an injected full-input scan
        // came out at 5.88x — so the bar is 2.7x clear of the clean measurement
        // and 2.0x below the fault it has to catch.
        assert_scaling_stable("downsample", 3, || {
            let samples = typical_per_size(&sizes, sizes.len(), |n| {
                let states = &inputs[sizes.iter().position(|&s| s == n).expect("known size")];
                let start = std::time::Instant::now();
                let ds = HistoryBuffer::downsample(states, 1000);
                let elapsed = start.elapsed();
                assert_eq!(ds.len(), 1000, "downsample must hit its target size");
                elapsed.as_micros()
            });
            check_raw_time_flat(&samples, 3.0)
        });
    }

    #[test]
    fn flush_cost_per_row_stays_flat() {
        // `flush` drains half the buffer, so twice the target is pushed. Sizes
        // stay modest because each flush encodes and writes a real .rrd.
        let sizes = [625usize, 1250, 2500];

        assert_scaling_stable("flush", 3, || {
            let samples = typical_per_size(&sizes, sizes.len(), |rows| {
                let dir = temp_data_dir(&format!("flush-scale-{rows}"));
                let mut buf = HistoryBuffer::new(10_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
                for i in 0..(rows * 2) {
                    buf.states.push_back(make_state(i as f64));
                }

                let start = std::time::Instant::now();
                buf.flush();
                let elapsed = start.elapsed();

                assert_eq!(buf.segment_count, 1, "one flush must write one segment");
                cleanup_dir(&dir);
                elapsed.as_micros()
            });
            check_cost_per_unit_flat(&samples, SCALING_BAR)
        });
    }

    #[test]
    fn load_all_cost_per_state_stays_flat() {
        // The capacity scales with the input, which keeps two things fixed that
        // would otherwise move with it. Measured against this fixture: every
        // size ends with 18 segments on disk and exactly `capacity` states in
        // memory, i.e. 10% of the input.
        //
        // A fixed capacity fails this test on unchanged code. Measured at
        // capacity 2000, the in-memory share runs 60% / 40% / 20% across these
        // three sizes (1 / 3 / 8 segments), so the work migrates from the cheap
        // in-memory tail to the far more expensive per-segment rerun decode and
        // the cost per state climbs for reasons that have nothing to do with
        // complexity. Holding the mix still leaves rows-per-segment as the only
        // thing varying.
        let sizes = [2_500usize, 5_000, 10_000];

        assert_scaling_stable("load_all", 3, || {
            let samples = typical_per_size(&sizes, sizes.len(), |n| {
                let dir = temp_data_dir(&format!("load-scale-{n}"));
                let mut buf = HistoryBuffer::new(n / 10, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
                for i in 0..n {
                    buf.push(make_state(i as f64));
                }

                let start = std::time::Instant::now();
                let all = buf.load_all();
                let elapsed = start.elapsed();

                assert_eq!(all.len(), n, "load_all must return every pushed state");
                cleanup_dir(&dir);
                elapsed.as_micros()
            });
            check_cost_per_unit_flat(&samples, SCALING_BAR)
        });
    }

    #[test]
    fn query_range_filters_by_time() {
        let dir = temp_data_dir("qr-filter");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(20.0, 60.0, None, None);
        assert!(result.len() >= 4, "should include t=20,30,40,50,60");
        for s in &result {
            assert!(s.t >= 20.0 && s.t <= 60.0, "t={} out of range", s.t);
        }

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_with_downsample() {
        let dir = temp_data_dir("qr-ds");
        let mut buf = HistoryBuffer::new(200, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..100 {
            buf.push(make_state(i as f64));
        }

        let result = buf.query_range(0.0, 99.0, Some(10), None);
        assert_eq!(result.len(), 10);
        assert!((result[0].t - 0.0).abs() < 1e-9);
        assert!((result[9].t - 99.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_empty_range() {
        let dir = temp_data_dir("qr-empty");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(200.0, 300.0, None, None);
        assert!(result.is_empty());

        cleanup_dir(&dir);
    }

    #[test]
    fn flush_preserves_attitude() {
        let dir = temp_data_dir("flush-attitude");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..5 {
            let t = i as f64 * 10.0;
            let pos = nalgebra::Vector3::new(6778.0, 0.0, 0.0);
            let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
            let attitude = Some(AttitudePayload {
                quaternion_wxyz: [0.707, 0.0, 0.707, 0.0],
                angular_velocity_body: [0.01 * t, 0.0, 0.0],
                source: AttitudeSource::Propagated,
                rw_momentum: None,
            });
            let hs = make_history_state(
                EntityPath::parse("/world/sat/att-sat"),
                t,
                &pos,
                &vel,
                TEST_MU,
                TEST_BODY_RADIUS,
                HashMap::new(),
                attitude,
            );
            buf.push(hs);
        }

        assert!(buf.segment_count > 0, "should have flushed");

        let all = buf.load_all();
        assert_eq!(all.len(), 5);
        for hs in &all {
            let att = hs
                .attitude
                .as_ref()
                .expect("attitude should survive flush/load round-trip");
            assert!(
                (att.quaternion_wxyz[0] - 0.707).abs() < 1e-9,
                "quaternion should be preserved"
            );
        }

        cleanup_dir(&dir);
    }

    /// A state carrying every optional payload field: the per-force
    /// acceleration breakdown and reaction-wheel momentum are the parts the
    /// old `.rrd` spill silently dropped.
    fn make_state_full(t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        let mut accels = HashMap::new();
        accels.insert("gravity".to_string(), 8.68e-3 + t * 1e-9);
        accels.insert("drag".to_string(), -1.234e-9);
        accels.insert("j2".to_string(), 1.1e-5);
        make_history_state(
            EntityPath::parse("/world/sat/full"),
            t,
            &pos,
            &vel,
            TEST_MU,
            TEST_BODY_RADIUS,
            accels,
            Some(AttitudePayload {
                quaternion_wxyz: [0.5, 0.5, 0.5, 0.5],
                angular_velocity_body: [0.01, -0.02, 0.03 + t * 1e-6],
                source: AttitudeSource::Propagated,
                // Four wheels: a variable-length payload the fixed 3-vector
                // rrd components could not express either.
                rw_momentum: Some(vec![0.1, -0.2, 0.3, 4.0e-3]),
            }),
        )
    }

    /// A `data_dir` that can never hold a segment: the path is an existing
    /// *file*, so both `create_dir_all` and `File::create` under it fail.
    fn unwritable_data_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("orts-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::write(&path, b"not a directory").expect("failed to create blocking file");
        path
    }

    /// A failed spill must not cost history: the buffer is the only source of
    /// truth for states that are not on disk, so it keeps them.
    #[test]
    fn flush_failure_keeps_states_in_memory() {
        let dir = unwritable_data_dir("flush-fail-retain");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..5 {
            buf.push(make_state(i as f64 * 10.0));
        }

        assert_eq!(buf.segment_count, 0, "no segment can have been written");
        let all = buf.load_all();
        assert_eq!(all.len(), 5, "every pushed state must still be readable");
        let times: Vec<f64> = all.iter().map(|s| s.t).collect();
        assert_eq!(times, vec![0.0, 10.0, 20.0, 30.0, 40.0]);

        let _ = std::fs::remove_file(&dir);
    }

    /// A permanently failing spill must not grow memory without bound: past
    /// `capacity * MAX_RETAINED_BUFFERS` the oldest states are discarded (with
    /// a warning) rather than retained forever.
    #[test]
    fn flush_failure_bounds_memory() {
        let dir = unwritable_data_dir("flush-fail-bounded");
        let capacity = 4;
        let mut buf = HistoryBuffer::new(capacity, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..500 {
            buf.push(make_state(i as f64));
        }

        let cap = capacity * MAX_RETAINED_BUFFERS;
        assert_eq!(buf.segment_count, 0);
        assert!(
            buf.states.len() <= cap,
            "in-memory buffer grew to {} states, cap is {cap}",
            buf.states.len()
        );
        // The newest states are the ones that survive.
        assert!((buf.states.back().unwrap().t - 499.0).abs() < 1e-9);
        // The overview still covers the discarded span, so a reconnecting
        // client sees the whole run at reduced fidelity.
        let overview = buf.overview();
        assert!(overview.first().unwrap().t < 100.0);

        let _ = std::fs::remove_file(&dir);
    }

    /// Full round-trip through the spill: accelerations and RW momentum come
    /// back too. Losing them made the viewer chart a gravity acceleration of
    /// 0 for every flushed window.
    #[test]
    fn flush_round_trip_preserves_full_payload() {
        let dir = temp_data_dir("flush-full-payload");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        let pushed: Vec<HistoryState> = (0..10).map(|i| make_state_full(i as f64 * 10.0)).collect();
        for hs in &pushed {
            buf.push(hs.clone());
        }
        assert!(buf.segment_count > 0, "precondition: must have spilled");

        let all = buf.load_all();
        assert_eq!(all.len(), pushed.len());
        for (expected, actual) in pushed.iter().zip(all.iter()) {
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::to_value(actual).unwrap(),
                "state at t = {} changed across the spill",
                expected.t
            );
        }

        cleanup_dir(&dir);
    }

    /// Two-path consistency: the same states read through the in-memory tail
    /// and through the spill must produce identical payloads.
    #[test]
    fn in_memory_and_spilled_paths_agree() {
        let mem_dir = temp_data_dir("two-path-mem");
        let disk_dir = temp_data_dir("two-path-disk");
        // Same states, one buffer large enough to never spill and one that
        // spills almost everything.
        let mut in_memory = HistoryBuffer::new(1000, mem_dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let mut spilled = HistoryBuffer::new(4, disk_dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..40 {
            let hs = make_state_full(i as f64 * 5.0);
            in_memory.push(hs.clone());
            spilled.push(hs);
        }

        assert_eq!(in_memory.segment_count, 0, "must stay in memory");
        assert!(spilled.segment_count > 0, "must have spilled");

        let from_memory = in_memory.query_range(0.0, 200.0, None, None);
        let from_disk = spilled.query_range(0.0, 200.0, None, None);
        assert_eq!(from_memory.len(), 40);
        assert_eq!(from_disk.len(), 40);
        for (mem, disk) in from_memory.iter().zip(from_disk.iter()) {
            assert_eq!(
                serde_json::to_value(mem).unwrap(),
                serde_json::to_value(disk).unwrap(),
                "payload at t = {} differs between the in-memory and spilled paths",
                mem.t
            );
        }

        cleanup_dir(&mem_dir);
        cleanup_dir(&disk_dir);
    }

    /// A diverged run is exactly what an operator wants to read back, so
    /// non-finite values must survive the spill instead of taking their row
    /// down with them.
    #[test]
    fn flush_round_trip_preserves_non_finite_values() {
        let dir = temp_data_dir("flush-non-finite");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..5 {
            let t = i as f64 * 10.0;
            let pos = nalgebra::Vector3::new(6778.0, f64::INFINITY, 0.0);
            let vel = nalgebra::Vector3::new(f64::NAN, 7.669, f64::NEG_INFINITY);
            let mut accels = HashMap::new();
            accels.insert("gravity".to_string(), f64::NAN);
            accels.insert("drag".to_string(), f64::NEG_INFINITY);
            buf.push(make_history_state(
                EntityPath::parse("/world/sat/diverged"),
                t,
                &pos,
                &vel,
                TEST_MU,
                TEST_BODY_RADIUS,
                accels,
                Some(AttitudePayload {
                    quaternion_wxyz: [f64::NAN, 0.0, 0.0, 0.0],
                    angular_velocity_body: [f64::INFINITY, 0.0, 0.0],
                    source: AttitudeSource::Propagated,
                    rw_momentum: Some(vec![f64::NAN, 1.0]),
                }),
            ));
        }
        assert!(buf.segment_count > 0, "precondition: must have spilled");

        let all = buf.load_all();
        assert_eq!(all.len(), 5, "non-finite rows must not be dropped");
        for hs in &all {
            assert_eq!(hs.position[1].to_bits(), f64::INFINITY.to_bits());
            assert!(hs.velocity[0].is_nan());
            assert_eq!(hs.velocity[2].to_bits(), f64::NEG_INFINITY.to_bits());
            assert!(hs.accelerations["gravity"].is_nan());
            assert_eq!(
                hs.accelerations["drag"].to_bits(),
                f64::NEG_INFINITY.to_bits()
            );
            let att = hs.attitude.as_ref().expect("attitude must survive");
            assert!(att.quaternion_wxyz[0].is_nan());
            assert_eq!(
                att.angular_velocity_body[0].to_bits(),
                f64::INFINITY.to_bits()
            );
            let rw = att.rw_momentum.as_ref().expect("rw momentum must survive");
            assert!(rw[0].is_nan());
            assert_eq!(rw[1], 1.0);
        }

        cleanup_dir(&dir);
    }

    /// Every value that a spilled float must survive as: finite values bit
    /// for bit, non-finite values as themselves. (`NaN` payload and sign are
    /// deliberately outside the contract, see [`F64`].)
    #[test]
    fn f64_json_round_trip() {
        // Bit-for-bit, including the sign of zero and the infinities.
        for value in [
            0.0,
            -0.0,
            1.0,
            -7.669,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            1e-300,
            0.03 + 20.0 * 1e-6,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let json = serde_json::to_string(&F64(value)).unwrap();
            let back: F64 = serde_json::from_str(&json).unwrap();
            assert_eq!(
                back.0.to_bits(),
                value.to_bits(),
                "{value} did not round-trip through {json}"
            );
        }
        // NaN comes back as a NaN. Asserting the bit pattern would claim more
        // than the format carries: `Display` writes every NaN as "NaN", so
        // the sign and payload do not survive (and are not wanted — what a
        // reader needs is "this row diverged").
        for value in [f64::NAN, -f64::NAN] {
            let json = serde_json::to_string(&F64(value)).unwrap();
            let back: F64 = serde_json::from_str(&json).unwrap();
            assert!(back.0.is_nan(), "NaN did not survive as {json}");
        }
    }

    /// The buffer spills once it holds `capacity` states, not one more.
    ///
    /// `capacity` is documented as the most it keeps in memory before
    /// flushing, and the retry after a failed spill as happening once the
    /// buffer has grown by another `capacity`. Both were reached one push
    /// late: measured with `capacity = 4`, the first spill came on push 5.
    #[test]
    fn the_buffer_spills_at_capacity() {
        let dir = std::env::temp_dir().join(format!(
            "hist_cap_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut buf = HistoryBuffer::new(4, dir.clone(), 398600.4418, 6378.137);

        for i in 0..3 {
            buf.push(make_state(i as f64));
        }
        assert_eq!(buf.segment_count, 0, "three of four states is not full");

        buf.push(make_state(3.0));
        assert_eq!(buf.segment_count, 1, "the fourth state fills it");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The memory cap holds on every push, not only when a write is attempted.
    ///
    /// A failed write moves the next attempt `capacity` pushes out. Measured
    /// with the cap enforced only there: capacity 4 reached 35 states against a
    /// cap of 32, and 63 of 200 pushes sat above it.
    #[test]
    fn a_failing_spill_never_leaves_the_buffer_over_the_cap() {
        let dir = temp_data_dir("cap-every-push");
        cleanup_dir(&dir);
        // A regular file where the directory should be: every write fails.
        std::fs::write(&dir, b"not a directory").expect("place the blocker");

        let capacity = 4usize;
        let cap = capacity * MAX_RETAINED_BUFFERS;
        let mut buf = HistoryBuffer::new(capacity, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let mut high = 0usize;
        for i in 0..200 {
            buf.push(make_state(i as f64));
            high = high.max(buf.states.len());
            assert!(
                buf.states.len() <= cap,
                "push {i} left {} states, over the cap of {cap}",
                buf.states.len()
            );
        }
        assert!(
            high > cap - capacity,
            "the run has to reach the cap for this to test anything: high water {high}"
        );
        assert_eq!(buf.segment_count, 0, "every write failed");

        std::fs::remove_file(&dir).ok();
    }

    /// A directory that becomes writable again is written to again.
    ///
    /// The retry threshold is `len + capacity`, and the memory cap holds the
    /// length at or below `capacity * MAX_RETAINED_BUFFERS`. A threshold above
    /// the cap is therefore never reached: measured without the clamp, after
    /// 100 failing pushes at capacity 4 the threshold sat at 36 against a cap
    /// of 32, and 100 further pushes to a writable directory wrote nothing.
    #[test]
    fn a_writable_directory_is_used_again_after_the_cap_is_reached() {
        let dir = temp_data_dir("recover-after-cap");
        cleanup_dir(&dir);
        // A regular file where the directory should be: every write fails.
        std::fs::write(&dir, b"not a directory").expect("place the blocker");

        let capacity = 4usize;
        let mut buf = HistoryBuffer::new(capacity, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..100 {
            buf.push(make_state(i as f64));
        }
        assert_eq!(buf.segment_count, 0, "every write failed");
        assert!(
            buf.failed_spills > 1,
            "the run has to keep retrying for this to test anything: {} attempts",
            buf.failed_spills
        );
        assert!(
            buf.flush_at <= capacity * MAX_RETAINED_BUFFERS,
            "the threshold has to stay reachable: {} against a cap of {}",
            buf.flush_at,
            capacity * MAX_RETAINED_BUFFERS
        );

        std::fs::remove_file(&dir).expect("remove the blocker");
        std::fs::create_dir_all(&dir).expect("make the directory");
        for i in 100..200 {
            buf.push(make_state(i as f64));
        }
        assert!(
            buf.segment_count > 0,
            "writes resume once the directory takes them"
        );
        assert!(
            buf.states.len() <= capacity,
            "and the buffer drains back to the normal band: {}",
            buf.states.len()
        );

        cleanup_dir(&dir);
    }

    /// A failing spill costs one write per `capacity` pushes, not two.
    ///
    /// The doc promises that cadence, and it is what keeps a permanently
    /// unwritable directory cheap. Measured while the cap only trimmed *past*
    /// itself: attempts landed on pushes 31 and 32, then 36 and 37 — a length
    /// sitting exactly at the cap with the threshold there too spent a second
    /// write on the same cycle.
    #[test]
    fn a_failing_spill_attempts_one_write_per_capacity_pushes() {
        let dir = temp_data_dir("cadence");
        cleanup_dir(&dir);
        // A regular file where the directory should be: every write fails.
        std::fs::write(&dir, b"not a directory").expect("place the blocker");

        let capacity = 4usize;
        let cap = capacity * MAX_RETAINED_BUFFERS;
        let mut buf = HistoryBuffer::new(capacity, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        let mut attempts_at = Vec::new();
        let mut seen = 0u32;
        for i in 0..80 {
            buf.push(make_state(i as f64));
            if buf.failed_spills != seen {
                attempts_at.push(i);
                seen = buf.failed_spills;
            }
        }

        // Past the cap the buffer is in its steady state, so the gaps are the
        // cadence. Before it the buffer is still growing into the cap.
        let steady: Vec<usize> = attempts_at.into_iter().filter(|i| *i > cap).collect();
        assert!(
            steady.len() > 5,
            "enough cycles to read a cadence: {steady:?}"
        );
        let gaps: Vec<usize> = steady.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.iter().all(|g| *g == capacity),
            "one attempt per {capacity} pushes: gaps {gaps:?} at {steady:?}"
        );

        std::fs::remove_file(&dir).ok();
    }
}
