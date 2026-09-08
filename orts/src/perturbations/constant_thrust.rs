use arika::epoch::Epoch;
use arika::frame::{self, Eci, Vec3};

use crate::model::ExternalLoads;
use crate::model::{EvalSegment, HasFrame, HasOrbit, Model};

/// Constant-thrust force model active over a fixed epoch interval.
///
/// Applies a uniform acceleration (in ECI) from `start` to `end`
/// (inclusive on both ends), and zero acceleration outside that window.
/// Acceleration is stored pre-computed as `total_dv / duration` so the
/// hot-path `eval()` is branch-and-lookup only.
///
/// ## Use case
///
/// Replaces the "impulsive Δv at a single epoch" approximation for
/// spacecraft maneuvers. The impulsive model has an irreducible
/// position-error floor proportional to `|Δv| · burn_duration²` (the
/// finite burn's mean-time-of-thrust differs from the geometric
/// midpoint when the thrust profile is asymmetric, and the position
/// trajectory through the burn differs from a single jump). Modelling
/// the burn as a continuous force lets the integrator smoothly
/// integrate through the burn window, eliminating the impulsive
/// residual for uniform-thrust burns and reducing it significantly
/// for real asymmetric profiles.
///
/// ## Limitations (v1)
///
/// - **Uniform thrust only**: real OMS-E-class burns have ramp-up and
///   ramp-down phases that are not modelled here. For Orion DRI/DRDI-
///   sized burns (≤ 200 m/s, ≤ 2 min) the residual asymmetry is a few
///   percent; for longer burns a piecewise-constant or ramped profile
///   would be more accurate.
/// - **Fixed ECI direction**: the thrust direction is constant in the
///   inertial frame for the whole burn. If the spacecraft rotates
///   during the burn to change thrust pointing, this model cannot
///   track it. For orbital-injection burns where the guidance holds a
///   fixed inertial attitude, this is fine.
/// - **No mass depletion**: the constant acceleration assumes constant
///   mass. For small Δv fractions of total mass (Orion: ~5 % of wet
///   mass for a 200 m/s burn at Isp ~316 s) this is acceptable.
/// - **Hard on/off at boundaries**: the force value jumps from zero
///   to `acceleration` at `start` and back to zero at `end`. The
///   Dop853 integrator handles this cleanly **only if the integration
///   interval does not straddle a `start`/`end` boundary** — when a
///   single fixed-step call crosses the boundary, Dop853's 12-stage
///   cluster evaluates some stages inside the burn (force = `a`) and
///   some outside (force = `0`), producing a polynomial that matches
///   neither ODE and gives wildly wrong results. The artemis1 example
///   hit this empirically at 1812 km / 73,706 km errors before the
///   trap was diagnosed. **Callers must** segment their integration
///   so each `integrate()` call sees a uniform force model: e.g.,
///   propagate coast → `burn.start`, then a fresh `integrate()` from
///   `burn.start` → `burn.end` with this `ConstantThrust` installed,
///   then another fresh call for the post-burn coast. See
///   `verify_burn_chain_continuous` in the artemis1 example for the
///   reference pattern. Adaptive integrators with event detection
///   could in principle drop the burden from the caller, but the
///   current orts `Dop853::integrate` is fixed-step.
#[derive(Debug, Clone, Copy)]
pub struct ConstantThrust<F: Eci = frame::SimpleEci> {
    /// Human-readable name (e.g. `"DRI"`, `"thrust_burn3"`).
    pub name: &'static str,
    /// First epoch at which the thrust is active (inclusive).
    pub start: Epoch,
    /// Last epoch at which the thrust is active (inclusive).
    pub end: Epoch,
    /// Pre-computed constant acceleration vector [km/s²], in the inertial
    /// frame `F` the Δv was given in.
    ///
    /// Equal to `total_dv / (end − start in seconds)`.
    acceleration: Vec3<F>,
}

impl<F: Eci> ConstantThrust<F> {
    /// Build a constant-thrust model from a total Δv vector and a burn
    /// window. The required acceleration is `total_dv / duration`.
    ///
    /// * `name` — diagnostic label, stored by reference (static string).
    /// * `start`, `end` — burn window epochs. `end > start` required.
    /// * `total_dv_kms` — integrated propulsive Δv [km/s] that should be
    ///   imparted over the window (equivalent to what an impulsive model would
    ///   apply). Its frame `F` is the frame the model must be propagated in:
    ///   the direction is held fixed in `F` for the whole burn, so a Δv given
    ///   in one inertial frame cannot be reused in another (`SimpleEci` and
    ///   `Gcrs` differ by ~484 arcsec at 2024).
    pub fn new(name: &'static str, start: Epoch, end: Epoch, total_dv_kms: Vec3<F>) -> Self {
        let duration_s = end.duration_since(&start).as_si_seconds();
        assert!(
            duration_s > 0.0,
            "ConstantThrust {name:?}: end epoch must strictly follow start"
        );
        Self {
            name,
            start,
            end,
            acceleration: total_dv_kms / duration_s,
        }
    }

    /// Returns the pre-computed constant acceleration vector [km/s²].
    /// Exposed for tests / diagnostics; the integrator consumes it via
    /// [`Model::eval`].
    pub fn acceleration_kms2(&self) -> Vec3<F> {
        self.acceleration
    }

    /// Returns the burn duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.end.duration_since(&self.start).as_si_seconds()
    }

    /// Returns the total Δv that this thrust model integrates to over
    /// `[start, end)` (= acceleration × duration).
    pub fn total_dv_kms(&self) -> Vec3<F> {
        self.acceleration * self.duration_seconds()
    }

    /// Returns `true` if `epoch` falls within `[start, end)` — the start
    /// counts, the end does not.
    ///
    /// The half-open interval is the one
    /// [`BurnWindow`](crate::spacecraft::BurnWindow) uses, and it is what makes
    /// abutting burns add up to their own lengths rather than sharing an
    /// instant. It also matters beyond that instant once a propagation loop
    /// splits its span at the edges this model reports and holds the burn's
    /// state over each segment: with the end counted as active, the segment
    /// starting at `end` would thrust for its whole length. Measured on a 0.1 s
    /// burn split into segments, RK4 with `dt = 1` applied 4/3 of the Δv asked
    /// for, the stages on the two edges leaking a sixth apiece into the
    /// segments on either side.
    ///
    /// Measured on the canonical TAI timeline, as [`duration_seconds`] and the
    /// edges reported by [`next_edge_after`] are. UTC Julian Dates do not
    /// advance uniformly across a leap second, so comparing them would put the
    /// switch a second away from the boundary this model reports, and would
    /// spread the Δv over a duration one second short.
    ///
    /// [`duration_seconds`]: Self::duration_seconds
    /// [`next_edge_after`]: Self::next_edge_after
    fn is_active(&self, epoch: &Epoch) -> bool {
        epoch.duration_since(&self.start).as_si_seconds() >= 0.0
            && self.end.duration_since(epoch).as_si_seconds() > 0.0
    }
}

impl<F: Eci> ConstantThrust<F> {
    /// Integration time of the next edge of the burn after `t`.
    ///
    /// The burn is bounded by two epochs while a propagation loop works in
    /// integration time, so this converts through `epoch_at_t`: the offset from
    /// there to an edge is the same in both. Without an epoch there is no
    /// mapping and no edge to report.
    ///
    /// `start` and `end` are where the burn begins and ends. Which one-sided
    /// value a stage landing exactly on an edge should take is the propagation
    /// loop's to decide (#446); this reports the times, not that rule.
    fn next_edge_after(&self, t: f64, epoch_at_t: Option<&Epoch>) -> Option<f64> {
        let now = epoch_at_t?;
        [self.start, self.end]
            .iter()
            .map(|edge| t + edge.duration_since(now).as_si_seconds())
            .filter(|edge_t| *edge_t > t && edge_t.is_finite())
            .min_by(f64::total_cmp)
    }

    /// Shared body of [`Model::eval`] for the frames this model supports.
    fn loads(&self, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        // The stored acceleration is already a `Vec3<F>` and `F` is the state's
        // frame, so it goes into the loads without a re-tag.
        let acceleration_inertial = match epoch {
            // No epoch → no way to know whether the burn is active → zero.
            // This matches the convention used by ThirdBodyGravity for
            // consistency across force models.
            Some(epoch) if self.is_active(epoch) => self.acceleration,
            _ => Vec3::zeros(),
        };
        ExternalLoads {
            acceleration_inertial,
            torque_body: Vec3::zeros(),
            mass_rate: 0.0,
        }
    }
}

// Implemented per frame rather than over `F: Eci`, because holding a direction
// fixed for the whole burn is a claim about the frame's *axes*, and the `Eci`
// category does not make it: `Cirs` axes are the celestial intermediate pole and
// origin **of date**, and `Teme`'s are the true equator of date, so components
// held constant in either drift inertially over the burn. `SimpleEci` ignores
// precession and nutation by construction and `Gcrs` is the GCRF realization, so
// both have axes fixed to the precision this model works at. arika states the
// same rule for its frame categories: write concrete types where the precision
// matters, and keep `<F: Eci>` for precision-agnostic math.
//
// A capability trait in arika (`Eci` frames whose axes are inertially fixed)
// would let this be one impl again; see the follow-up noted in the PR.
impl<S: HasFrame<Frame = frame::SimpleEci> + HasOrbit> Model<S>
    for ConstantThrust<frame::SimpleEci>
{
    fn name(&self) -> &str {
        self.name
    }

    fn eval(&self, _t: f64, _state: &S, epoch: Option<&Epoch>) -> ExternalLoads<S::Frame> {
        self.loads(epoch)
    }

    /// The burn's state at the segment's start, held for the whole segment.
    ///
    /// A propagation loop splits its span at the edges
    /// [`next_discontinuity_after`](Self::next_discontinuity_after) reports, so
    /// no edge falls strictly inside a segment and the start speaks for all of
    /// it. What this changes is the stage on the segment's end, which reading
    /// the epoch of the stage would find already outside a burn ending there.
    fn eval_in_segment(
        &self,
        segment: &EvalSegment<'_>,
        _t: f64,
        _state: &S,
        _epoch: Option<&Epoch>,
    ) -> ExternalLoads<S::Frame> {
        self.loads(segment.start_epoch)
    }

    fn next_discontinuity_after(&self, t: f64, epoch: Option<&Epoch>) -> Option<f64> {
        self.next_edge_after(t, epoch)
    }
}

impl<S: HasFrame<Frame = frame::Gcrs> + HasOrbit> Model<S> for ConstantThrust<frame::Gcrs> {
    fn name(&self) -> &str {
        self.name
    }

    fn eval(&self, _t: f64, _state: &S, epoch: Option<&Epoch>) -> ExternalLoads<S::Frame> {
        self.loads(epoch)
    }

    /// The burn's state at the segment's start, held for the whole segment.
    ///
    /// A propagation loop splits its span at the edges
    /// [`next_discontinuity_after`](Self::next_discontinuity_after) reports, so
    /// no edge falls strictly inside a segment and the start speaks for all of
    /// it. What this changes is the stage on the segment's end, which reading
    /// the epoch of the stage would find already outside a burn ending there.
    fn eval_in_segment(
        &self,
        segment: &EvalSegment<'_>,
        _t: f64,
        _state: &S,
        _epoch: Option<&Epoch>,
    ) -> ExternalLoads<S::Frame> {
        self.loads(segment.start_epoch)
    }

    fn next_discontinuity_after(&self, t: f64, epoch: Option<&Epoch>) -> Option<f64> {
        self.next_edge_after(t, epoch)
    }
}

// Static assertion that `ConstantThrust` can cross thread boundaries
// (same requirement as `ThirdBodyGravity` — `OrbitalSystem` must stay
// `Send + Sync` when it holds force models).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ConstantThrust>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use nalgebra::{Vector3, vector};

    fn test_state() -> OrbitalState {
        OrbitalState::new(vector![7000.0, 0.0, 0.0], vector![0.0, 7.5, 0.0])
    }

    fn gcrs_test_state() -> OrbitalState<frame::Gcrs> {
        OrbitalState::new_in_frame(vector![7000.0, 0.0, 0.0], vector![0.0, 7.5, 0.0])
    }

    fn epoch_seconds_from_j2000(seconds: f64) -> Epoch {
        // Arbitrary anchor; we only care about relative offsets for these tests.
        Epoch::from_jd(2_451_545.0 + seconds / 86_400.0)
    }

    #[test]
    fn new_computes_correct_acceleration() {
        let start = epoch_seconds_from_j2000(0.0);
        let end = epoch_seconds_from_j2000(100.0);
        // 50 m/s total Δv in +x over 100 s → 0.5 m/s² = 5e-4 km/s²
        // The tolerance (1e-8) accounts for JD round-trip precision: at
        // modern epochs (~J2000), f64 JD ULP is ~5e-10 days ≈ 50 µs, so
        // the recovered duration and hence the acceleration have a
        // relative error ~5e-7 for a 100-s interval.
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("test", start, end, Vec3::new(0.05, 0.0, 0.0));
        let a = thrust.acceleration_kms2();
        assert!((a.x() - 5e-4).abs() < 1e-8);
        assert_eq!(a.y(), 0.0);
        assert_eq!(a.z(), 0.0);
    }

    #[test]
    fn total_dv_round_trip_matches_constructor_input() {
        let start = epoch_seconds_from_j2000(0.0);
        let end = epoch_seconds_from_j2000(120.0);
        let total = Vec3::<frame::SimpleEci>::new(0.1, -0.02, 0.05);
        let thrust = ConstantThrust::new("rt", start, end, total);
        let recovered = thrust.total_dv_kms();
        assert!((recovered - total).magnitude() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "end epoch must strictly follow start")]
    fn new_panics_on_reversed_interval() {
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(0.0);
        let _ =
            ConstantThrust::<frame::SimpleEci>::new("bad", start, end, Vec3::new(0.1, 0.0, 0.0));
    }

    #[test]
    fn eval_is_zero_before_start() {
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("bf", start, end, Vec3::new(0.05, 0.0, 0.0));
        let probe = epoch_seconds_from_j2000(50.0);
        let loads = thrust.eval(0.0, &test_state(), Some(&probe));
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
    }

    #[test]
    fn eval_is_zero_after_end() {
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("af", start, end, Vec3::new(0.05, 0.0, 0.0));
        let probe = epoch_seconds_from_j2000(300.0);
        let loads = thrust.eval(0.0, &test_state(), Some(&probe));
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
    }

    #[test]
    fn eval_returns_constant_acceleration_inside_window() {
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        // 10 m/s over 100 s → 0.1 m/s² = 1e-4 km/s²
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("mid", start, end, Vec3::new(0.01, 0.0, 0.0));
        let expected = vector![1e-4, 0.0, 0.0];
        // 199.99 rather than 200.0: the window is half-open, so its own end is
        // outside it, and `epoch_seconds_from_j2000` goes through a Julian Date
        // whose resolution here is about 40 us — a probe a microsecond short of
        // the end rounds onto it. `eval_at_the_start_thrusts_and_at_the_end_does_not`
        // pins the end itself, and `eval_just_before_the_end_thrusts` the
        // instant before it, on the SI timeline where that is representable.
        for probe_sec in [100.0, 120.0, 150.0, 180.0, 199.99] {
            let probe = epoch_seconds_from_j2000(probe_sec);
            let loads = thrust.eval(0.0, &test_state(), Some(&probe));
            let a = loads.acceleration_inertial.into_inner();
            // 1e-8 tolerance accounts for JD round-trip precision in
            // the 100-s duration (see `new_computes_correct_acceleration`).
            assert!(
                (a - expected).magnitude() < 1e-8,
                "at probe={probe_sec}s: expected {expected:?} got {a:?}"
            );
        }
    }

    #[test]
    fn eval_with_no_epoch_returns_zero() {
        let thrust = ConstantThrust::new(
            "noep",
            epoch_seconds_from_j2000(0.0),
            epoch_seconds_from_j2000(100.0),
            Vec3::<frame::SimpleEci>::new(0.05, 0.0, 0.0),
        );
        let loads = thrust.eval(0.0, &test_state(), None);
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
    }

    #[test]
    fn eval_at_the_start_thrusts_and_at_the_end_does_not() {
        // Half-open, as `BurnWindow` is: the start counts, the end does not, so
        // a segment beginning where the burn ends does not thrust.
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("bd", start, end, Vec3::new(0.01, 0.0, 0.0));
        let expected = vector![1e-4, 0.0, 0.0];

        let loads_start = thrust.eval(0.0, &test_state(), Some(&start));
        let loads_end = thrust.eval(0.0, &test_state(), Some(&end));
        assert!((loads_start.acceleration_inertial.into_inner() - expected).magnitude() < 1e-8);
        assert_eq!(
            loads_end.acceleration_inertial.into_inner(),
            Vector3::zeros()
        );
    }

    /// The last instant before the end still thrusts: the interval is
    /// half-open, not short of its own length.
    #[test]
    fn eval_just_before_the_end_thrusts() {
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("bd", start, end, Vec3::new(0.01, 0.0, 0.0));
        let expected = vector![1e-4, 0.0, 0.0];

        let loads = thrust.eval(0.0, &test_state(), Some(&end.add_si_seconds(-1e-9)));
        assert!((loads.acceleration_inertial.into_inner() - expected).magnitude() < 1e-8);
    }

    #[test]
    fn constant_thrust_is_clone_and_copy() {
        let t1 = ConstantThrust::new(
            "cc",
            epoch_seconds_from_j2000(0.0),
            epoch_seconds_from_j2000(50.0),
            Vec3::<frame::SimpleEci>::new(0.02, 0.0, 0.0),
        );
        let t2 = t1;
        let t3 = t1.clone();
        assert_eq!(t1.acceleration_kms2(), t2.acceleration_kms2());
        assert_eq!(t1.acceleration_kms2(), t3.acceleration_kms2());
    }

    /// The `Gcrs` impl is separate from `SimpleEci`'s, so it needs its own
    /// evaluation: the compile-fail cases only prove which frames are rejected,
    /// so without this, deleting the `Gcrs` impl would leave every test green.
    #[test]
    fn gcrs_burn_delivers_its_acceleration_in_gcrs() {
        let start = epoch_seconds_from_j2000(0.0);
        let end = epoch_seconds_from_j2000(100.0);
        let dv = Vec3::<frame::Gcrs>::new(0.05, -0.02, 0.01);
        let thrust = ConstantThrust::new("gcrs-burn", start, end, dv);

        let inside = thrust.eval(
            0.0,
            &gcrs_test_state(),
            Some(&epoch_seconds_from_j2000(50.0)),
        );
        let expected = dv.into_inner() / 100.0;
        // 1e-8, matching `eval_returns_constant_acceleration_inside_window`: the
        // 100-s duration is recovered through a JD round-trip, so it is not
        // exactly 100.
        assert!(
            (inside.acceleration_inertial.into_inner() - expected).magnitude() < 1e-8,
            "in-window acceleration should be dv/duration, got {:?}",
            inside.acceleration_inertial.into_inner()
        );
        assert_eq!(inside.torque_body.into_inner(), Vector3::zeros());
        assert_eq!(inside.mass_rate, 0.0);

        let outside = thrust.eval(
            0.0,
            &gcrs_test_state(),
            Some(&epoch_seconds_from_j2000(200.0)),
        );
        assert_eq!(
            outside.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "outside the burn window the acceleration is zero"
        );
    }
}
