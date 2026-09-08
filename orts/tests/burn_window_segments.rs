//! Propagating a group through a scheduled burn.
//!
//! A solver evaluates the right-hand side at stage times of its own choosing,
//! so a burn window narrower than the gap between two of them contributes
//! nothing, and one that covers a whole step loses the weight of the stage
//! sitting on its exclusive end. Both are measured below against the same
//! oracle: the propellant a constant-thrust burn spends is `thrust / (Isp *
//! g0)` times the burn's length, whatever the trajectory does, because the mass
//! flow of a thruster at full throttle does not depend on the state.
//!
//! The propagation loop ends its steps at the window edges the thruster
//! reports, and hands the solver a system bound to each segment, which is what
//! makes these agree with the oracle rather than with the stage times.

use nalgebra::{Matrix3, Vector3};
use orts::OrbitalState;
use orts::attitude::AttitudeState;
use orts::effector::AugmentedState;
use orts::group::IntegratorConfig;
use orts::group::independent::IndependentGroup;
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{
    BurnWindow, G0, ScheduledBurn, SpacecraftDynamics, SpacecraftState, Thruster,
};
use utsuroi::Tolerances;

const THRUST_N: f64 = 10.0;
const ISP_S: f64 = 300.0;
const MASS_KG: f64 = 100.0;
const SPAN_S: f64 = 10.0;

/// How sharply a spent-propellant figure can be compared, in kg.
///
/// The propellant is the difference of two masses near `MASS_KG`, so it carries
/// the resolution of f64 there — `100 * f64::EPSILON` is 2.2e-14 — however
/// exactly the mass flow was integrated. Four of those is loose enough for the
/// subtraction and tight enough to see the failures these tests are about: a
/// missed window spends nothing, and a window whose end stage reads off spends
/// 5/6.
const PROPELLANT_TOL_KG: f64 = 4.0 * MASS_KG * f64::EPSILON;

/// Propellant a full-throttle burn of `length` seconds spends, in kg.
fn propellant_for(length: f64) -> f64 {
    THRUST_N / (ISP_S * G0) * length
}

fn dynamics_with(windows: Vec<BurnWindow>) -> SpacecraftDynamics<PointMass> {
    let thruster = Thruster::new(THRUST_N, ISP_S, Vector3::x())
        .with_profile(Box::new(ScheduledBurn { windows }));
    SpacecraftDynamics::new(arika::earth::MU, PointMass, Matrix3::identity()).with_model(thruster)
}

fn initial_state() -> AugmentedState<SpacecraftState> {
    // 400 km circular orbit, thrusting along +x of the body frame.
    let r = arika::earth::R + 400.0;
    let v = (arika::earth::MU / r).sqrt();
    AugmentedState {
        plant: SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0)),
            attitude: AttitudeState::identity(),
            mass: MASS_KG,
        },
        aux: vec![],
        aux_bounds: vec![],
    }
}

/// Propellant spent over `SPAN_S` seconds, propagating with `integrator`.
fn propellant_spent(windows: Vec<BurnWindow>, integrator: IntegratorConfig) -> f64 {
    let mut group = IndependentGroup::new(integrator).add_satellite(
        "sat",
        initial_state(),
        dynamics_with(windows),
    );
    let mut final_mass = MASS_KG;
    group
        .propagate_to_with(SPAN_S, |_, _, state| final_mass = state.plant.mass)
        .expect("the burn and the orbit are finite everywhere");
    MASS_KG - final_mass
}

fn integrators() -> Vec<(&'static str, IntegratorConfig)> {
    vec![
        ("RK4 dt=1", IntegratorConfig::Rk4 { dt: 1.0 }),
        (
            "DP45",
            IntegratorConfig::Dp45 {
                dt: 1.0,
                tolerances: Tolerances::default(),
            },
        ),
        (
            "DOP853",
            IntegratorConfig::Dop853 {
                dt: 1.0,
                tolerances: Tolerances::default(),
            },
        ),
    ]
}

/// A window narrower than the largest gap between adjacent stage times. With
/// `dt = 1` the RK4 stages fall at 0, 0.5, 0.5 and 1, so `[0.1, 0.2)` used to
/// be sampled at none of them and the burn spent nothing at all.
#[test]
fn a_window_shorter_than_a_step_spends_its_propellant() {
    let expected = propellant_for(0.1);
    for (name, integrator) in integrators() {
        let spent = propellant_spent(vec![BurnWindow::full(0.1, 0.2)], integrator);
        assert!(
            (spent - expected).abs() < PROPELLANT_TOL_KG,
            "{name} spent {spent} kg, expected {expected} kg"
        );
    }
}

/// A window covering a whole step. The stage on its exclusive end reads the
/// throttle as off, and RK4 weights that stage 1/6, so this used to spend 5/6
/// of the propellant.
#[test]
fn a_window_covering_a_whole_step_spends_all_of_its_propellant() {
    let expected = propellant_for(1.0);
    for (name, integrator) in integrators() {
        let spent = propellant_spent(vec![BurnWindow::full(0.0, 1.0)], integrator);
        assert!(
            (spent - expected).abs() < PROPELLANT_TOL_KG,
            "{name} spent {spent} kg, expected {expected} kg"
        );
    }
}

/// Two windows that abut share an edge, and the burn runs across it without
/// the shared time counting twice or dropping out.
#[test]
fn abutting_windows_burn_as_one() {
    let expected = propellant_for(2.0);
    for (name, integrator) in integrators() {
        let spent = propellant_spent(
            vec![BurnWindow::full(1.0, 2.0), BurnWindow::full(2.0, 3.0)],
            integrator,
        );
        assert!(
            (spent - expected).abs() < PROPELLANT_TOL_KG,
            "{name} spent {spent} kg, expected {expected} kg"
        );
    }
}

/// A burn whose window ends exactly where the propagation does. The last
/// segment ends at the span's end, and the throttle holds to it.
#[test]
fn a_window_ending_with_the_span_burns_to_the_end() {
    let expected = propellant_for(1.0);
    for (name, integrator) in integrators() {
        let spent = propellant_spent(vec![BurnWindow::full(SPAN_S - 1.0, SPAN_S)], integrator);
        assert!(
            (spent - expected).abs() < PROPELLANT_TOL_KG,
            "{name} spent {spent} kg, expected {expected} kg"
        );
    }
}

/// Splitting the call must not change the answer: segment modes and stepper
/// state belong to a segment, not to a call.
#[test]
fn splitting_the_propagation_call_spends_the_same_propellant() {
    let expected = propellant_for(0.1);
    for (name, integrator) in integrators() {
        let mut group = IndependentGroup::new(integrator).add_satellite(
            "sat",
            initial_state(),
            dynamics_with(vec![BurnWindow::full(0.1, 0.2)]),
        );
        let mut final_mass = MASS_KG;
        for target in [0.15, 1.0, SPAN_S] {
            group
                .propagate_to_with(target, |_, _, state| final_mass = state.plant.mass)
                .expect("the burn and the orbit are finite everywhere");
        }
        let spent = MASS_KG - final_mass;
        assert!(
            (spent - expected).abs() < PROPELLANT_TOL_KG,
            "{name} spent {spent} kg across three calls, expected {expected} kg"
        );
    }
}

/// A burn shorter than a step, in the group that propagates satellites
/// together. `CoupledGroup` needs a state it can add an acceleration to, so
/// this flies `OrbitalSystem` with `ConstantThrust` — the orbital model whose
/// schedule is written in epochs — rather than the thruster above.
///
/// With no interactions the two group loops are the same problem, so the
/// independent one is the oracle: it splits the span at the same edges, and a
/// coupled loop that integrated straight through them would come out
/// elsewhere: measured, integrating straight through leaves the position
/// 2.8e-5 km away with RK4. The burn's own displacement over the rest of the
/// span is 9.9e-6 km with DOP853, and the RK4 figure is larger because
/// splitting the span at the edges also changes its step grid.
#[test]
fn a_coupled_group_flies_a_short_burn_like_an_independent_one() {
    use arika::epoch::Epoch;
    use arika::frame::Vec3;
    use orts::group::coupled::CoupledGroup;
    use orts::orbital::OrbitalSystem;
    use orts::perturbations::ConstantThrust;

    let epoch_0 = Epoch::j2000();
    let burn = || {
        ConstantThrust::new(
            "burn",
            epoch_0.add_si_seconds(0.1),
            epoch_0.add_si_seconds(0.2),
            Vec3::new(1e-6, 0.0, 0.0),
        )
    };
    let system = || {
        OrbitalSystem::new(arika::earth::MU, Box::new(PointMass))
            .with_model(burn())
            .with_epoch(epoch_0)
    };
    let r = arika::earth::R + 400.0;
    let v = (arika::earth::MU / r).sqrt();
    let state = || OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0));

    let coast = || OrbitalSystem::new(arika::earth::MU, Box::new(PointMass)).with_epoch(epoch_0);

    for (name, integrator) in integrators() {
        let integrator_for_baseline = integrator.clone();
        let mut independent =
            IndependentGroup::new(integrator.clone()).add_satellite("sat", state(), system());
        independent
            .propagate_to(SPAN_S)
            .expect("the burn and the orbit are finite everywhere");

        let mut coupled = CoupledGroup::new(integrator).add_satellite("sat", state(), system());
        coupled
            .propagate_to(SPAN_S)
            .expect("the burn and the orbit are finite everywhere");

        // Comparing the two loops alone would pass if both missed the burn,
        // so this also measures the burn against a run without one.
        let mut coasting =
            IndependentGroup::new(integrator_for_baseline).add_satellite("sat", state(), coast());
        coasting
            .propagate_to(SPAN_S)
            .expect("the orbit is finite everywhere");

        let alone = independent.snapshot().positions[0].1;
        let together = coupled.snapshot().positions[0].1;
        let unburnt = coasting.snapshot().positions[0].1;
        let gap = (alone - together).norm();
        let moved = (alone - unburnt).norm();
        println!("{name}: gap {gap:e} km, burn moved {moved:e} km");
        assert!(gap < 1e-15, "{name}: the two loops differ by {gap:e} km");
        assert!(
            moved > 1e-6,
            "{name}: the burn moved the satellite only {moved:e} km, so this \
             comparison cannot tell a missed burn from a flown one"
        );
    }
}

/// The Δv an epoch-scheduled burn applies, against the Δv it was given.
///
/// `ConstantThrust` spreads a Δv over `[start, end)`, so what a propagation
/// applies is that Δv and nothing more. Comparing the two group loops cannot
/// see this: both would be wrong together. Measured before the burn's interval
/// became half-open, RK4 with `dt = 1` applied 4/3 of it — the stages on the
/// two edges each leaked a sixth of a step into the segment beyond.
///
/// The span stops just after the burn so the orbit's own evolution stays small,
/// and the run is compared against a coasting one. The 1e-7 tolerance covers
/// the gravity the two runs no longer share once the burn has moved one of
/// them: RK4 and DOP853 agree on the excess to 1e-12, so it is not truncation.
///
/// The burn sits at `[0.15, 0.25)` rather than `[0.1, 0.2)` so that the
/// segment before it is longer than the burn itself. With both 0.1 s long, a
/// stage-time reading leaks `h/6` of thrust into the segment before the burn
/// and drops the same `h/6` from the burn's own last stage, and the two cancel
/// exactly under RK4 — measured, that made RK4 agree with the oracle for the
/// wrong reason.
#[test]
fn an_epoch_scheduled_burn_applies_the_delta_v_it_was_given() {
    use arika::epoch::Epoch;
    use arika::frame::Vec3;
    use orts::orbital::OrbitalSystem;
    use orts::perturbations::ConstantThrust;

    let epoch_0 = Epoch::j2000();
    let asked = 1e-6;
    let system = || {
        OrbitalSystem::new(arika::earth::MU, Box::new(PointMass))
            .with_model(ConstantThrust::new(
                "burn",
                epoch_0.add_si_seconds(0.15),
                epoch_0.add_si_seconds(0.25),
                Vec3::new(asked, 0.0, 0.0),
            ))
            .with_epoch(epoch_0)
    };
    let coast = || OrbitalSystem::new(arika::earth::MU, Box::new(PointMass)).with_epoch(epoch_0);
    let r = arika::earth::R + 400.0;
    let v = (arika::earth::MU / r).sqrt();
    let state = || OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0));

    for (name, integrator) in integrators() {
        let mut burning =
            IndependentGroup::new(integrator.clone()).add_satellite("sat", state(), system());
        let mut coasting = IndependentGroup::new(integrator).add_satellite("sat", state(), coast());
        let mut burnt_velocity = Vector3::zeros();
        let mut coast_velocity = Vector3::zeros();
        burning
            .propagate_to_with(0.35, |_, _, s| burnt_velocity = *s.velocity())
            .expect("the burn and the orbit are finite everywhere");
        coasting
            .propagate_to_with(0.35, |_, _, s| coast_velocity = *s.velocity())
            .expect("the orbit is finite everywhere");

        let applied = (burnt_velocity - coast_velocity).norm();
        assert!(
            (applied / asked - 1.0).abs() < 1e-7,
            "{name} applied {applied:e} km/s of the {asked:e} km/s asked for"
        );
    }
}
