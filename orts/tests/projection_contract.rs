//! A saturating reaction wheel stays on its bound, and the spacecraft keeps
//! the angular momentum the wheel stopped taking — whichever integrator runs.
//!
//! The bound used to be enforced by the `aux_bounds` projection of
//! `AugmentedState`: the wheel model zeroed the torque in the saturating
//! direction, which bounded the overshoot within a step without removing it,
//! and the projection then clamped whatever was left. Clamping is what lost the
//! angular momentum measured in [#446]: the body had already integrated the
//! reaction that carried the wheel past its bound, so moving the wheel back
//! without returning it destroyed that much of a conserved total.
//!
//! Reaching the bound is now a boundary the propagation locates, and the
//! boundary handling is what puts the momentum on it — giving the overshoot
//! back to the body. These cases run a saturating wheel through a group, which
//! is the loop that handles boundaries, under each integrator the configuration
//! offers.
//!
//! [#446]: https://github.com/sksat/orts/issues/446

use nalgebra::{Matrix3, Vector3};
use utsuroi::{Integrator, Rk4, Tolerances};

use orts::effector::{AugmentedState, ConstraintMode};
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{ReactionWheelAssembly, RwCommand, SpacecraftDynamics, SpacecraftState};

const MAX_MOMENTUM: f64 = 0.53;
const MAX_TORQUE: f64 = 0.1;
const WHEEL_INERTIA: f64 = 0.01;
/// Isotropic, and only the z wheel is driven, so the body's rate stays along z
/// with the total. `dH_body/dt = -ω × H_body` then vanishes and the body-frame
/// vector itself is constant, not just its magnitude — which is what lets these
/// cases compare it component by component. There is no external torque: the
/// gravity-gradient model is not installed.
const BODY_INERTIA: f64 = 10.0;
/// A torque about z is allocated to the z wheel alone, which therefore spins
/// down to `-MAX_MOMENTUM` at `t = MAX_MOMENTUM / MAX_TORQUE = 5.3 s`: between
/// the ticks of the grid below, which is the case the boundary handling exists
/// for. On a tick the fixed-step walk would land on the bound by itself.
const T_END: f64 = 20.0;
const DT: f64 = 0.25;

type Dynamics = SpacecraftDynamics<PointMass>;

fn saturating_system() -> Dynamics {
    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
    rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
    SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
}

/// The same spacecraft with its wheels commanded to hold still.
fn idle_system() -> Dynamics {
    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
    rw.command = RwCommand::Torques(vec![0.0; 3]);
    SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
}

fn initial_plant() -> SpacecraftState {
    SpacecraftState::from_orbit(
        orts::orbital::OrbitalState::new(
            Vector3::new(7000.0, 0.0, 0.0),
            Vector3::new(0.0, 7.546, 0.0),
        ),
        100.0,
    )
}

/// Body-frame angular momentum of the whole spacecraft: the body's own plus
/// every wheel's. `three_axis` spins its wheels about x, y and z, so the
/// wheels' contribution is their momenta read as a vector.
fn total_momentum(state: &AugmentedState<SpacecraftState>) -> Vector3<f64> {
    BODY_INERTIA * state.plant.attitude.angular_velocity
        + Vector3::new(state.aux[0], state.aux[1], state.aux[2])
}

/// The worst any wheel's momentum ran past its bound over a walk.
fn worst_overshoot(state: &AugmentedState<SpacecraftState>) -> f64 {
    state
        .aux
        .iter()
        .take(3)
        .map(|h| h.abs() - MAX_MOMENTUM)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn integrators() -> Vec<(&'static str, IntegratorConfig)> {
    let tight = Tolerances {
        // cli/src/config.rs defaults.
        atol: 1e-10,
        rtol: 1e-8,
    };
    let loose = Tolerances {
        // Where the adaptive step grows, and the overshoot with it.
        atol: 1e-3,
        rtol: 1e-3,
    };
    vec![
        ("rk4", IntegratorConfig::Rk4 { dt: DT }),
        (
            "dp45",
            IntegratorConfig::Dp45 {
                dt: DT,
                tolerances: tight.clone(),
            },
        ),
        (
            "dp45 (loose)",
            IntegratorConfig::Dp45 {
                dt: DT,
                tolerances: loose.clone(),
            },
        ),
        (
            "dop853",
            IntegratorConfig::Dop853 {
                dt: DT,
                tolerances: tight,
            },
        ),
        (
            "dop853 (loose)",
            IntegratorConfig::Dop853 {
                dt: DT,
                tolerances: loose,
            },
        ),
    ]
}

/// Walk the saturating system to `T_END` and answer the worst overshoot seen,
/// the body-frame momentum it started and ended with, and the final state.
fn walk(
    config: IntegratorConfig,
) -> (
    f64,
    Vector3<f64>,
    Vector3<f64>,
    AugmentedState<SpacecraftState>,
) {
    let system = saturating_system();
    let initial = system.initial_augmented_state(initial_plant());
    let started_with = total_momentum(&initial);

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(config).add_satellite("sat", initial, saturating_system());
    let mut worst = f64::NEG_INFINITY;
    group
        .propagate_to_with(T_END, |_id, _t, state| {
            worst = worst.max(worst_overshoot(state));
        })
        .expect("the walk succeeds");

    let final_state = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    let ended_with = total_momentum(&final_state);
    (worst, started_with, ended_with, final_state)
}

#[test]
fn a_saturating_wheel_stays_on_its_bound() {
    for (name, config) in integrators() {
        let (worst, _, _, final_state) = walk(config);
        assert!(
            worst <= 1e-9,
            "{name}: a wheel ran {worst:.3e} past its bound"
        );
        assert!(
            (final_state.aux[2] + MAX_MOMENTUM).abs() < 1e-6,
            "{name}: the z wheel ends held at its lower bound, not at {}",
            final_state.aux[2]
        );
    }
}

#[test]
fn the_spacecraft_keeps_the_momentum_the_wheel_stopped_taking() {
    for (name, config) in integrators() {
        let (_, started_with, ended_with, _) = walk(config);
        let lost = (ended_with - started_with).magnitude();
        assert!(
            lost < 1e-9,
            "{name}: {lost:.3e} N·m·s of the body-frame total went missing \
             (started {started_with:?}, ended {ended_with:?})"
        );
    }
}

/// The limit is the boundary handling's to keep, and a walk that handles no
/// boundaries keeps nothing: `Integrator::integrate` steps to the end of the
/// span with no root search in it, so the mode of every wheel stays `Free` and
/// the motor drives the wheel as far as the span allows. A caller who wants the
/// limit enforced propagates through a path that runs the walk — a group, the
/// CLI's controlled path, or `orts::boundary::walk_to_target` directly.
#[test]
fn a_direct_integration_has_no_boundary_to_stop_at() {
    let system = saturating_system();
    let initial = system.initial_augmented_state(initial_plant());
    let ended = Rk4.integrate(&system, initial, 0.0, T_END, DT, |_, _| {});

    // MAX_TORQUE for the whole span, unimpeded: 2.0 N·m·s, which is 3.8 times
    // the wheel's own limit.
    let expected = -MAX_TORQUE * T_END;
    assert!(
        (ended.aux[2] - expected).abs() < 1e-9,
        "the wheel ends at {}, not at the {expected} the motor asks for",
        ended.aux[2]
    );
}

/// A wheel resting on its bound with the motor asking for nothing has two
/// margins at zero at once: the bound it is sitting on, and the release that a
/// torque turning inward would bring. Neither is a crossing — the state is on
/// the boundary, not past it — so the walk leaves the mode where it is and
/// reports nothing but its steps. Read as crossings, the two would hand the
/// mode back and forth for as long as the settling loop runs, once per walk.
#[test]
fn a_wheel_resting_on_its_bound_is_not_a_crossing() {
    let system = idle_system();
    let mut initial = system.initial_augmented_state(initial_plant());
    // Exactly on the lower bound, and still running free.
    initial.aux[2] = -MAX_MOMENTUM;

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT }).add_satellite(
            "sat",
            initial,
            idle_system(),
        );
    let mut samples: Vec<(f64, ConstraintMode, f64)> = Vec::new();
    group
        .propagate_to_with(4.0 * DT, |_id, t, state| {
            samples.push((t, state.modes[2], state.aux[2]));
        })
        .expect("the walk succeeds");

    assert_eq!(
        samples.len(),
        4,
        "four steps and nothing else, at {samples:?}"
    );
    for (t, mode, h) in samples {
        assert_eq!(mode, ConstraintMode::Free, "the mode stays put, at t = {t}");
        assert!(
            (h + MAX_MOMENTUM).abs() < 1e-12,
            "and so does the wheel, at {h} at t = {t}"
        );
    }
}

/// The body-frame cases above are the degenerate ones: an isotropic inertia
/// with one wheel axis driven keeps `ω` parallel to the total, so the vector in
/// body axes is constant and the gyroscopic term `-ω × H` never contributes.
/// With a non-isotropic inertia and a tumbling body the body-frame vector
/// turns, and only the inertial one is conserved — which is where [#446] asked
/// for the check. This walks a wheel to its limit under those conditions and
/// reads `R_bi (Iω + Σ aᵢ hᵢ)` at every step.
///
/// [#446]: https://github.com/sksat/orts/issues/446
#[test]
fn a_tumbling_spacecraft_keeps_its_inertial_momentum_across_the_bound() {
    // Distinct principal moments [kg·m²], so the body-frame total turns.
    const INERTIA: [f64; 3] = [8.0, 12.0, 20.0];
    let inertia = Matrix3::from_diagonal(&Vector3::new(INERTIA[0], INERTIA[1], INERTIA[2]));

    let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
    rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
    let system = SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw);

    let mut plant = initial_plant();
    // Tumbling on all three axes, fast enough that the body-frame total turns
    // through a large angle over the span.
    plant.attitude.angular_velocity = Vector3::new(0.05, -0.03, 0.02);
    let initial = system.initial_augmented_state(plant);

    let body_total = |state: &AugmentedState<SpacecraftState>| {
        Vector3::new(
            INERTIA[0] * state.plant.attitude.angular_velocity[0],
            INERTIA[1] * state.plant.attitude.angular_velocity[1],
            INERTIA[2] * state.plant.attitude.angular_velocity[2],
        ) + Vector3::new(state.aux[0], state.aux[1], state.aux[2])
    };
    let inertial_total = |state: &AugmentedState<SpacecraftState>| {
        state.plant.attitude.orientation() * body_total(state)
    };
    let started_with = inertial_total(&initial);
    let started_in_body = body_total(&initial);

    let rebuilt = || {
        let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
        rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };
    let mut group: IndependentGroup<Dynamics> = IndependentGroup::new(IntegratorConfig::Dop853 {
        dt: DT,
        tolerances: Tolerances {
            atol: 1e-12,
            rtol: 1e-12,
        },
    })
    .add_satellite("sat", initial, rebuilt());

    let mut worst_drift: f64 = 0.0;
    let mut turned_by: f64 = 0.0;
    let mut held = false;
    group
        .propagate_to_with(T_END, |_id, _t, state| {
            worst_drift = worst_drift.max((inertial_total(state) - started_with).magnitude());
            // How far the body-frame vector has moved since the start, which
            // is what makes the inertial comparison the meaningful one here.
            turned_by = turned_by.max((body_total(state) - started_in_body).magnitude());
            held |= state.modes[2] == ConstraintMode::Lower;
        })
        .expect("the walk succeeds");

    assert!(held, "the z wheel reaches its limit inside the span");
    assert!(
        turned_by > 1e-3,
        "the body-frame total moves by {turned_by:.3e} N·m·s, so this is not the \
         degenerate case"
    );
    // Measured 7.3e-16 N·m·s over the span, against a body-frame vector that
    // moves 1.4e-1: the inertial total is conserved to rounding, and the
    // threshold leaves room for the platform's own last bits.
    const DRIFT_ALLOWED: f64 = 1e-12;
    assert!(
        worst_drift < DRIFT_ALLOWED,
        "the inertial total drifted {worst_drift:.3e} N·m·s, more than the \
         {DRIFT_ALLOWED:.0e} rounding accounts for"
    );
}

/// What the search can and cannot see is decided by the step: a root is found
/// from the sign of the value at the step's ends, so a step holding two sign
/// changes of the same margin holds none the search can report.
///
/// A wheel with motor lag can produce exactly that. Braking a wheel that is
/// still accelerating outward, from just inside its limit, sends it past the
/// limit and brings it back as the realized torque decays through zero: with
/// `h = 0.999`, a limit of 1, a realized torque of +0.1 N·m against a command
/// of -0.1, and a time constant of 50 ms, the momentum peaks at 1.0005 N·m·s
/// after 35 ms and is back under the limit by 100 ms. A 100 ms step sees
/// 0.9957 at its end and reports nothing; a 10 ms step catches the crossing
/// and holds the wheel.
///
/// The obligation is the caller's, and `DESIGN.md` says so: the search cannot
/// check what a step contains. It is the same step the lag itself needs — two
/// steps per time constant is not a resolution the exponential is integrated
/// at either.
#[test]
fn a_step_coarser_than_the_motor_lag_misses_a_brief_excursion() {
    const LIMIT: f64 = 1.0;
    const T_M: f64 = 0.05;
    const TORQUE: f64 = 0.1;

    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let build = || {
        let wheel =
            orts::spacecraft::reaction_wheel::Rw::new(Vector3::z(), WHEEL_INERTIA, LIMIT, TORQUE)
                .with_motor_lag(T_M);
        let mut rw = ReactionWheelAssembly::new(vec![wheel]);
        // The motor is asked to brake, from a state where it is still pushing
        // the wheel out.
        rw.command = RwCommand::Torques(vec![-TORQUE]);
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };

    let walk_with =
        |dt: f64| {
            let system = build();
            let mut initial = system.initial_augmented_state(initial_plant());
            initial.aux[0] = 0.999;
            // Still accelerating outward when the reversal arrives.
            initial.aux[1] = TORQUE;

            let mut group: IndependentGroup<Dynamics> = IndependentGroup::new(
                IntegratorConfig::Rk4 { dt },
            )
            .add_satellite("sat", initial, build());
            let mut peak = f64::NEG_INFINITY;
            let mut held = false;
            group
                .propagate_to_with(0.2, |_id, _t, state| {
                    peak = peak.max(state.aux[0]);
                    held |= state.modes[0] != ConstraintMode::Free;
                })
                .expect("the walk succeeds");
            (peak, held)
        };

    let (coarse_peak, coarse_held) = walk_with(0.1);
    assert!(
        !coarse_held,
        "a 100 ms step reports no crossing, so the wheel is never held"
    );
    assert!(
        (coarse_peak - 0.995_667).abs() < 1e-6,
        "and the states it does report stay under the limit, at {coarse_peak}"
    );

    let (fine_peak, fine_held) = walk_with(0.01);
    assert!(fine_held, "a 10 ms step catches the crossing");
    assert!(
        (fine_peak - LIMIT).abs() < 1e-9,
        "and holds the wheel on its bound, not at {fine_peak}"
    );
}

/// A held wheel comes off its bound when the motor turns around, and the walk
/// is what hands the mode back.
///
/// Holding is half of a one-sided constraint; a wheel that never releases is an
/// actuator lost for the rest of the run. Two motors exercise the two ways the
/// release can arrive. One reverses at once, so the release margin is already
/// negative when the walk starts and settling before the first step is what
/// catches it. The other has a 50 ms lag, so its realized torque passes zero
/// inside a step and the search is what locates it.
#[test]
fn a_held_wheel_is_released_when_the_motor_turns_around() {
    // Driven onto its bound first, by the same walk the cases above measure.
    let (_, started_with, _, held) = walk(IntegratorConfig::Rk4 { dt: DT });
    assert_eq!(
        held.modes[2],
        ConstraintMode::Lower,
        "the z wheel ends this walk held on its lower bound"
    );

    // The command reverses: a torque about +z drove the wheel to its lower
    // bound, so one about -z drives it back inward.
    let inward = || {
        let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
        let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
        rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, -MAX_TORQUE)));
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT }).add_satellite(
            "sat",
            held.clone(),
            inward(),
        );
    let mut released_at = None;
    group
        .propagate_to_with(5.0, |_id, t, state| {
            if released_at.is_none() && state.modes[2] == ConstraintMode::Free {
                released_at = Some(t);
            }
        })
        .expect("the walk succeeds");
    let after = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();

    assert_eq!(
        released_at,
        Some(0.0),
        "an instant reversal is already past its release when the walk starts, \
         so settling before the first step is what reports it"
    );
    assert_eq!(
        after.modes[2],
        ConstraintMode::Free,
        "and the wheel runs free for the rest of the walk"
    );
    // Five seconds of MAX_TORQUE inward from the bound.
    let expected = -MAX_MOMENTUM + MAX_TORQUE * 5.0;
    assert!(
        (after.aux[2] - expected).abs() < 1e-9,
        "the momentum moves inward at the commanded rate: {} against {expected}",
        after.aux[2]
    );
    let lost = (total_momentum(&after) - started_with).magnitude();
    assert!(
        lost < 1e-9,
        "and the total is still conserved across hold and release: {lost:.3e} N·m·s"
    );

    // A motor that takes 50 ms to reverse: the realized torque still pushes
    // outward when the command arrives, so the release is a crossing inside a
    // step rather than a state the walk starts past.
    const LIMIT: f64 = 1.0;
    const T_M: f64 = 0.05;
    let lagged = || {
        let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
        let wheel = orts::spacecraft::reaction_wheel::Rw::new(
            Vector3::z(),
            WHEEL_INERTIA,
            LIMIT,
            MAX_TORQUE,
        )
        .with_motor_lag(T_M);
        let mut rw = ReactionWheelAssembly::new(vec![wheel]);
        rw.command = RwCommand::Torques(vec![MAX_TORQUE]);
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };
    let system = lagged();
    let mut initial = system.initial_augmented_state(initial_plant());
    // On the lower bound, with the motor still driving it outward.
    initial.aux[0] = -LIMIT;
    initial.aux[1] = -MAX_TORQUE;
    initial.modes[0] = ConstraintMode::Lower;

    // A 50 ms motor asks for a step to match: RK4 on this lag is unstable past
    // about 28 ms, and a quarter-second step leaves the realized torque pinned
    // to its own bound instead of following the command.
    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: 0.01 }).add_satellite(
            "sat",
            initial,
            lagged(),
        );
    let mut released_at = None;
    group
        .propagate_to_with(1.0, |_id, t, state| {
            if released_at.is_none() && state.modes[0] == ConstraintMode::Free {
                released_at = Some(t);
            }
        })
        .expect("the walk succeeds");
    let released_at = released_at.expect("the lagged motor releases the wheel");
    // The realized torque decays from -MAX_TORQUE toward +MAX_TORQUE with a
    // 50 ms time constant, so it passes zero at t_m * ln 2 = 34.7 ms.
    let analytic = T_M * 2.0_f64.ln();
    assert!(
        (released_at - analytic).abs() < 1e-3,
        "the release is located where the realized torque turns around: \
         {released_at} against {analytic}"
    );
    let after = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    assert_eq!(after.modes[0], ConstraintMode::Free);
    assert!(
        after.aux[0] > -LIMIT,
        "and the wheel has moved inward off the bound, to {}",
        after.aux[0]
    );
}

/// A state can arrive a hair past a boundary — a caller restoring a state it
/// saved, a command applied between walks — and however small the overshoot,
/// the search cannot find it: the margin is already negative and only goes
/// further negative, so there is no sign change left to report. Such a wheel
/// would run on past its limit for the whole span.
///
/// Settling before the first step is what catches it, and the test to be sure
/// is the one whose overshoot is smaller than the tolerance the wheel declares
/// for sitting on its bound (1e-12 N·m·s): the tolerance suppresses a second
/// report of a crossing already located, and being on the crossed side is not
/// that.
#[test]
fn a_wheel_a_hair_past_its_bound_is_settled_before_it_runs_further() {
    let system = saturating_system();
    let mut initial = system.initial_augmented_state(initial_plant());
    // Past the lower bound by less than the wheel's own boundary tolerance,
    // and still running free, with the motor driving it further out.
    initial.aux[2] = -MAX_MOMENTUM - 1e-13;

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT }).add_satellite(
            "sat",
            initial,
            saturating_system(),
        );
    group
        .propagate_to_with(T_END, |_id, _t, _state| {})
        .expect("the walk succeeds");

    let ended = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    assert!(
        (ended.aux[2] + MAX_MOMENTUM).abs() < 1e-12,
        "the wheel is put on its bound and held there, not left at {}",
        ended.aux[2]
    );
    assert_eq!(
        ended.modes[2],
        ConstraintMode::Lower,
        "and its mode says which bound holds it"
    );
}
