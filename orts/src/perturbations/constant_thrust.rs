use arika::epoch::Epoch;
use arika::frame::{self, Eci, Vec3};

use crate::model::ExternalLoads;
use crate::model::{HasFrame, HasOrbit, Model};

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
        let duration_s = (end.jd() - start.jd()) * 86_400.0;
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
        (self.end.jd() - self.start.jd()) * 86_400.0
    }

    /// Returns the total Δv that this thrust model integrates to over
    /// `[start, end]` (= acceleration × duration).
    pub fn total_dv_kms(&self) -> Vec3<F> {
        self.acceleration * self.duration_seconds()
    }

    /// Returns `true` if `epoch` falls within `[start, end]` (inclusive).
    fn is_active(&self, epoch: &Epoch) -> bool {
        epoch.jd() >= self.start.jd() && epoch.jd() <= self.end.jd()
    }
}

impl<F: Eci> ConstantThrust<F> {
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
}

impl<S: HasFrame<Frame = frame::Gcrs> + HasOrbit> Model<S> for ConstantThrust<frame::Gcrs> {
    fn name(&self) -> &str {
        self.name
    }

    fn eval(&self, _t: f64, _state: &S, epoch: Option<&Epoch>) -> ExternalLoads<S::Frame> {
        self.loads(epoch)
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
        for probe_sec in [100.0, 120.0, 150.0, 180.0, 200.0] {
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
    fn eval_at_exact_boundaries_is_active() {
        // Inclusive on both ends: start and end epochs return thrust, not zero.
        let start = epoch_seconds_from_j2000(100.0);
        let end = epoch_seconds_from_j2000(200.0);
        let thrust =
            ConstantThrust::<frame::SimpleEci>::new("bd", start, end, Vec3::new(0.01, 0.0, 0.0));
        let expected = vector![1e-4, 0.0, 0.0];

        let loads_start = thrust.eval(0.0, &test_state(), Some(&start));
        let loads_end = thrust.eval(0.0, &test_state(), Some(&end));
        assert!((loads_start.acceleration_inertial.into_inner() - expected).magnitude() < 1e-8);
        assert!((loads_end.acceleration_inertial.into_inner() - expected).magnitude() < 1e-8);
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
