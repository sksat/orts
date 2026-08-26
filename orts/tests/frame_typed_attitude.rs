//! Why the attitude surfaces carry their frame in the type (#332).
//!
//! A quaternion's components depend on the frame it is expressed against. These
//! tests show that the difference is real and operationally large — not a
//! rounding-level concern — for one physical attitude expressed against two
//! inertial frames the simulator actually supports. The compile-fail companions
//! in `tests/trybuild/` pin the type-level side: that such a value cannot be
//! consumed as the wrong frame.
//!
//! The two frames are reached from a common third frame, TEME, because `arika`
//! deliberately provides no direct `SimpleEci` ↔ `Gcrs` conversion (the point of
//! the distinction is that the approximate and the precise frame are not
//! interchangeable). `Rotation<Teme, Gcrs>` (IAU-76/FK5 reduction) and
//! `Rotation<Teme, SimpleEci>` (GMST-aligned z-rotation) give the same physical
//! orientation its components in each.

use arika::epoch::Epoch;
use arika::frame::{Body, Gcrs, Rotation, SimpleEci, Teme};
use nalgebra::{UnitQuaternion, Vector3};
use orts::attitude::{AttitudeState, InertialPointing, TrackingPdController};
use orts::model::{HasAttitude, HasOrbit, Model};
use orts::orbital::OrbitalState;
use orts::plugin::tick_input::AttitudeBodyToInertial;

/// 2024-03-20T12:00:00 UTC — 24 years of precession/nutation past J2000.
fn epoch() -> Epoch {
    Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
}

/// One physical spacecraft orientation, expressed as body→TEME.
fn body_to_teme() -> Rotation<Body, Teme> {
    Rotation::from_raw(UnitQuaternion::from_axis_angle(
        &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
        0.7,
    ))
}

fn body_to_gcrs() -> Rotation<Body, Gcrs> {
    body_to_teme().then(&Rotation::<Teme, Gcrs>::teme_to_gcrs(&epoch().to_tt()))
}

fn body_to_simple_eci() -> Rotation<Body, SimpleEci> {
    body_to_teme().then(&Rotation::<Teme, SimpleEci>::teme_to_simple_eci(
        &epoch().to_ut1_naive(),
    ))
}

/// The angle [rad] between two rotations of the same physical orientation,
/// compared through their raw components — i.e. the error someone would commit
/// by reading one frame's numbers as if they were the other's.
fn component_angle(a: &UnitQuaternion<f64>, b: &UnitQuaternion<f64>) -> f64 {
    (a.inverse() * b).angle()
}

#[test]
fn the_same_attitude_has_different_components_in_gcrs_and_simple_eci() {
    let gcrs = AttitudeBodyToInertial::new(body_to_gcrs());
    let simple = AttitudeBodyToInertial::new(body_to_simple_eci());

    let angle = component_angle(gcrs.inner().inner(), simple.inner().inner());
    let arcsec = angle.to_degrees() * 3600.0;

    // ~484 arcsec at this epoch: an order of magnitude above a good star
    // tracker's accuracy (1–30 arcsec), so mistaking one frame for the other is
    // a pointing error, not a rounding difference.
    assert!(
        arcsec > 300.0,
        "the frames must differ operationally, got {arcsec:.1} arcsec"
    );
    let expected_deg = 0.13436522507125545;
    let got_deg = angle.to_degrees();
    assert!(
        (got_deg - expected_deg).abs() <= 1e-12 * expected_deg,
        "GCRS/simple-ECI attitude difference changed: {got_deg} deg"
    );
}

struct TestState<F: arika::frame::Eci> {
    attitude: AttitudeState,
    orbit: OrbitalState<F>,
}

impl<F: arika::frame::Eci> HasAttitude for TestState<F> {
    fn attitude(&self) -> &AttitudeState {
        &self.attitude
    }
}

impl<F: arika::frame::Eci> HasOrbit for TestState<F> {
    type Frame = F;
    fn orbit(&self) -> &OrbitalState<F> {
        &self.orbit
    }
}

fn test_state<F: arika::frame::Eci>() -> TestState<F> {
    TestState {
        attitude: AttitudeState::new(
            UnitQuaternion::from_axis_angle(
                &nalgebra::Unit::new_normalize(Vector3::new(0.1, 0.2, -0.4)),
                0.3,
            ),
            Vector3::new(0.01, -0.02, 0.03),
        ),
        orbit: OrbitalState::<F>::new_in_frame(
            Vector3::new(4000.0, -5000.0, 2500.0),
            Vector3::new(1.0, 2.0, 7.0),
        ),
    }
}

/// The frame of an inertial-hold target changes the commanded torque, so
/// carrying it in the type is load-bearing rather than decorative: the same
/// physical target, tagged with the two frames, steers the spacecraft
/// differently.
#[test]
fn an_inertial_hold_target_commands_a_different_torque_per_frame() {
    let epoch = epoch();

    let in_gcrs = TrackingPdController::diagonal(
        1.0,
        2.0,
        InertialPointing {
            target_q: body_to_gcrs(),
        },
    )
    .eval(0.0, &test_state::<Gcrs>(), Some(&epoch))
    .torque_body
    .into_inner();

    let in_simple = TrackingPdController::diagonal(
        1.0,
        2.0,
        InertialPointing {
            target_q: body_to_simple_eci(),
        },
    )
    .eval(0.0, &test_state::<SimpleEci>(), Some(&epoch))
    .torque_body
    .into_inner();

    // Both are real commands of order 1 N·m, and they disagree by ~2e-3 N·m —
    // the proportional term's response to the ~484 arcsec frame difference.
    assert!(in_simple.magnitude() > 0.1, "fixture must command a torque");
    let diff = (in_gcrs - in_simple).magnitude();
    assert!(
        diff > 1e-3,
        "the frame of the target must change the command, got {diff:.3e} N·m"
    );
    // Pinned: the difference is the frame difference, not an unrelated change.
    let expected = 2.2670773348014944e-3;
    assert!(
        (diff - expected).abs() <= 1e-9 * expected,
        "torque difference between frames changed: {diff:e} N·m"
    );
}
