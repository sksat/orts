use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

use orts::record::archetypes::OrbitalState;
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;

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

/// Bounded buffer that accumulates history states and periodically flushes to .rrd segments.
pub struct HistoryBuffer {
    /// Recent states kept in memory.
    pub states: VecDeque<HistoryState>,
    /// Maximum number of states to keep in memory before flushing.
    pub capacity: usize,
    /// Directory for .rrd segment files.
    pub data_dir: PathBuf,
    /// Number of segment files written so far.
    pub segment_count: u32,
    /// Gravitational parameter (for computing Keplerian elements from loaded data).
    pub mu: f64,
    /// Central body radius [km] (for computing derived values from loaded data).
    pub body_radius: f64,

    // --- Incremental per-entity overview -----------------------------------
    //
    // Maintained in O(1) amortized per `push()` call, read in
    // O(num_entities * OVERVIEW_MAX_POINTS_PER_ENTITY) with no disk I/O.
    // This lets re-connects to long-running simulations return the history
    // overview instantly, without re-reading every .rrd segment from disk
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
            mu,
            body_radius,
            overview_per_entity: HashMap::new(),
        }
    }

    /// Push a state into the buffer. Flushes to .rrd if capacity is exceeded,
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
        if self.states.len() > self.capacity {
            self.flush();
        }
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

    /// Flush the oldest half of the buffer to a .rrd segment file.
    pub fn flush(&mut self) {
        let flush_count = self.states.len() / 2;
        if flush_count == 0 {
            return;
        }

        let to_flush: Vec<HistoryState> = self.states.drain(..flush_count).collect();

        let mut rec = Recording::new();

        for (i, hs) in to_flush.iter().enumerate() {
            let sat_path = hs.entity_path.clone();
            let tp = TimePoint::new().with_sim_time(hs.t).with_step(i as u64);
            let os = OrbitalState::new(
                nalgebra::Vector3::new(hs.position[0], hs.position[1], hs.position[2]),
                nalgebra::Vector3::new(hs.velocity[0], hs.velocity[1], hs.velocity[2]),
            );
            let (q, w) = if let Some(att) = &hs.attitude {
                (
                    Some(orts::record::components::Quaternion4D(
                        nalgebra::Vector4::from_row_slice(&att.quaternion_wxyz),
                    )),
                    Some(orts::record::components::AngularVelocity3D(
                        nalgebra::Vector3::from_row_slice(&att.angular_velocity_body),
                    )),
                )
            } else {
                (None, None)
            };
            rec.log_orbital_state_with_attitude(&sat_path, &tp, &os, q.as_ref(), w.as_ref());
        }

        let seg_path = self
            .data_dir
            .join(format!("seg_{:04}.rrd", self.segment_count));
        if let Err(e) =
            orts::record::rerun_export::save_as_rrd(&rec, "orts", seg_path.to_str().unwrap())
        {
            eprintln!("Warning: failed to flush segment: {e}");
            return;
        }

        self.segment_count += 1;
    }

    /// Load all data: .rrd segments + in-memory buffer, sorted by time.
    pub fn load_all(&self) -> Vec<HistoryState> {
        let mut all = Vec::new();

        // Read .rrd segment files in order
        for i in 0..self.segment_count {
            let seg_path = self.data_dir.join(format!("seg_{i:04}.rrd"));
            match orts::record::rerun_export::load_from_rrd(seg_path.to_str().unwrap()) {
                Ok(rows) => {
                    for row in rows {
                        let pos = nalgebra::Vector3::new(row.x, row.y, row.z);
                        let vel = nalgebra::Vector3::new(row.vx, row.vy, row.vz);
                        let entity_path = row
                            .entity_path
                            .as_deref()
                            .map(EntityPath::parse)
                            .unwrap_or_else(|| EntityPath::parse("/world/sat/default"));
                        let attitude = row.quaternion.map(|q| AttitudePayload {
                            quaternion_wxyz: q,
                            angular_velocity_body: row.angular_velocity.unwrap_or([0.0; 3]),
                            source: AttitudeSource::Propagated,
                            rw_momentum: None,
                        });
                        all.push(make_history_state(
                            entity_path,
                            row.t,
                            &pos,
                            &vel,
                            self.mu,
                            self.body_radius,
                            HashMap::new(),
                            attitude,
                        ));
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to read segment {i}: {e}");
                }
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
    ///   tail and skip reading any `.rrd` segments.
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

    // --- Incremental overview buffer -----------------------------------
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
        // Regression gate. With the old `load_all()` based implementation
        // this test fails because the cost scales with the number of
        // flushed segments (disk I/O + decode + sort). The incremental
        // overview buffer must answer from memory in ~O(OVERVIEW_MAX_POINTS_PER_ENTITY)
        // time regardless of how many segments exist.
        let dir = temp_data_dir("overview-perf");
        let mut buf = HistoryBuffer::new(1_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);
        for i in 0..20_000 {
            buf.push(make_state(i as f64));
        }
        assert!(
            buf.segment_count >= 10,
            "precondition: enough flushes to make load_all expensive"
        );

        let start = std::time::Instant::now();
        let ov = buf.overview();
        let elapsed = start.elapsed();

        assert!(ov.len() <= OVERVIEW_MAX_POINTS_PER_ENTITY);
        assert!(
            elapsed.as_millis() < 20,
            "overview() took {}ms with {} flushed segments; expected < 20ms \
             (must not touch disk, must not call load_all)",
            elapsed.as_millis(),
            buf.segment_count
        );
        cleanup_dir(&dir);
    }

    #[test]
    fn overview_multi_entity_cost_is_bounded() {
        // The per-entity overview design flattens every entity buffer into
        // a Vec and sorts by `t` on each read. For realistic constellation
        // sizes (10+ sats) the sort cost must stay comfortably under the
        // perf gate — this test guards against accidental O(N^2) or
        // disk-touching regressions if `OVERVIEW_MAX_POINTS_PER_ENTITY` is
        // bumped, or if `overview()` grows auxiliary computation.
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

        let start = std::time::Instant::now();
        let ov = buf.overview();
        let elapsed = start.elapsed();

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
        // Perf gate: flatten + sort of ~10k points in a Vec is expected
        // to run in a few ms. 50ms is a loose CI-safe ceiling that still
        // catches order-of-magnitude regressions.
        assert!(
            elapsed.as_millis() < 50,
            "multi-entity overview() took {}ms for {} sats, expected < 50ms",
            elapsed.as_millis(),
            sats.len()
        );
        cleanup_dir(&dir);
    }

    // --- query_range in-memory fast path ---------------------------------
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
        assert!(dir.join("seg_0000.rrd").exists());
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

    #[test]
    fn downsample_performance() {
        let states: Vec<HistoryState> = (0..100_000).map(|i| make_state(i as f64)).collect();
        let start = std::time::Instant::now();
        let ds = HistoryBuffer::downsample(&states, 1000);
        let elapsed = start.elapsed();

        assert_eq!(ds.len(), 1000);
        assert!(
            elapsed.as_millis() < 10,
            "downsample took {}ms, expected <10ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn flush_performance() {
        let dir = temp_data_dir("flush-perf");
        let mut buf = HistoryBuffer::new(10_000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..5000 {
            buf.states.push_back(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        buf.flush();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 2000,
            "flush took {}ms, expected <2000ms",
            elapsed.as_millis()
        );
        assert_eq!(buf.segment_count, 1);

        cleanup_dir(&dir);
    }

    #[test]
    fn load_all_performance() {
        let dir = temp_data_dir("load-perf");
        let mut buf = HistoryBuffer::new(2000, dir.clone(), TEST_MU, TEST_BODY_RADIUS);

        for i in 0..10_000 {
            buf.push(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        let all = buf.load_all();
        let elapsed = start.elapsed();

        assert_eq!(all.len(), 10_000);
        assert!(
            elapsed.as_millis() < 2000,
            "load_all took {}ms, expected <2000ms",
            elapsed.as_millis()
        );

        cleanup_dir(&dir);
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
}
