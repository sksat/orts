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
use utsuroi::{Integrator, Rk4, RootSearch, SegmentContext, Tolerances};

use orts::boundary::{DeclaredBoundary, HasBoundaries};
use orts::effector::{AugmentedState, BoundaryKind, ConstraintMode, EffectorBoundary};
use std::sync::Arc;

use orts::group::coupled::{InterSatelliteForce, PairContext};
use orts::group::{
    CoupledGroup, IndependentGroup, IntegratorConfig, PairRegime, RegimeConfig, Scheduler,
};
use orts::orbital::OrbitalState;
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

/// A wheel whose realized torque starts at zero is starting to move, so the
/// walk does not stop there.
///
/// `initial_augmented_state` zeroes the auxiliary state, which is the state
/// every run starts from: a realized torque of zero, with a command for it to
/// follow. The torque leaves zero at once, and counting that as a turn of the
/// momentum would stop the walk a root tolerance in — a step split where the
/// momentum is monotone, and a sample the caller never asked for. Measured
/// before `Crossing::Reversal`: a stop at 0.78 ms, with the samples after it at
/// 100.78 ms and 200 ms instead of the 100 ms grid.
#[test]
fn a_realized_torque_starting_at_zero_is_no_turning_point() {
    const LIMIT: f64 = 1.0;
    const TORQUE: f64 = 0.1;
    const DT: f64 = 0.1;

    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let build = || {
        let wheel =
            orts::spacecraft::reaction_wheel::Rw::new(Vector3::z(), WHEEL_INERTIA, LIMIT, TORQUE)
                .with_motor_lag(0.05);
        let mut rw = ReactionWheelAssembly::new(vec![wheel]);
        rw.command = RwCommand::Torques(vec![-TORQUE]);
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };
    let system = build();
    let initial = system.initial_augmented_state(initial_plant());
    assert_eq!(
        initial.aux,
        vec![0.0, 0.0],
        "the state every run starts from: no momentum, no realized torque"
    );

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT }).add_satellite(
            "sat",
            initial,
            build(),
        );
    let mut samples = Vec::new();
    group
        .propagate_to_with(0.2, |_id, t, _state| samples.push(t))
        .expect("the walk succeeds");

    assert_eq!(
        samples.len(),
        2,
        "only the two step boundaries are reported: {samples:?}"
    );
    for (i, t) in samples.iter().enumerate() {
        let expected = DT * (i + 1) as f64;
        assert!(
            (t - expected).abs() < 1e-12,
            "sample {i} is on the step grid at {expected} s, not {t} s"
        );
    }
}

/// Two wheels whose motors turn around at different times are each handled at
/// their own time, and one wheel's turn leaves the other's constraint alone.
///
/// The z wheel starts just inside its limit, so its excursion is held; the x
/// wheel starts at half its limit and only turns around. Their time constants
/// are 50 ms and 45 ms, so the turns are at 34.7 ms and 31.2 ms — two zeros of
/// two different rates in the one 100 ms step, and the x turn falls while z is
/// held (13.7 ms to 34.8 ms). A split that cleared another wheel's mode would
/// release z at the x turn, which the release time below would show.
#[test]
fn two_lagging_wheels_are_handled_at_their_own_times() {
    const LIMIT: f64 = 1.0;
    const TORQUE: f64 = 0.1;
    const T_Z: f64 = 0.05;
    const T_X: f64 = 0.045;

    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let build = || {
        let z =
            orts::spacecraft::reaction_wheel::Rw::new(Vector3::z(), WHEEL_INERTIA, LIMIT, TORQUE)
                .with_motor_lag(T_Z);
        let x =
            orts::spacecraft::reaction_wheel::Rw::new(Vector3::x(), WHEEL_INERTIA, LIMIT, TORQUE)
                .with_motor_lag(T_X);
        let mut rw = ReactionWheelAssembly::new(vec![z, x]);
        rw.command = RwCommand::Torques(vec![-TORQUE, -TORQUE]);
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };
    let system = build();
    let mut initial = system.initial_augmented_state(initial_plant());
    // Momenta, then the realized torques: z is nearly saturated, x is not.
    initial.aux[0] = 0.999;
    initial.aux[1] = 0.5;
    initial.aux[2] = TORQUE;
    initial.aux[3] = TORQUE;

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: 0.1 }).add_satellite(
            "sat",
            initial,
            build(),
        );
    let mut z_held_from = None;
    let mut z_released_at = None;
    let mut x_ever_held = false;
    let mut x_peak = f64::NEG_INFINITY;
    let mut stops = Vec::new();
    group
        .propagate_to_with(0.2, |_id, t, state| {
            stops.push(t);
            x_peak = x_peak.max(state.aux[1]);
            x_ever_held |= state.modes[1] != ConstraintMode::Free;
            let z_held = state.modes[0] != ConstraintMode::Free;
            match (z_held, z_held_from, z_released_at) {
                (true, None, _) => z_held_from = Some(t),
                (false, Some(_), None) => z_released_at = Some(t),
                _ => {}
            }
        })
        .expect("the walk succeeds");

    let held_from = z_held_from.expect("the z wheel is held");
    let released_at = z_released_at.expect("the z wheel is released again");
    assert!(
        (0.0132..=0.0142).contains(&held_from),
        "the z wheel is held at its crossing (13.2 ms), not at {held_from} s"
    );
    assert!(
        (0.0347..=0.0357).contains(&released_at),
        "and released where its own torque turns (34.7 ms), not at {released_at} s"
    );
    assert!(!x_ever_held, "the x wheel reaches no bound");
    assert!(
        x_peak < 0.51,
        "and stays near where it started, peaking at {x_peak}"
    );
    let x_turn = T_X * 2.0_f64.ln();
    assert!(
        stops.iter().any(|t| (t - x_turn).abs() < 2e-3),
        "the walk also stops where the x wheel's torque turns ({x_turn:.6} s), among {stops:?}"
    );
}

/// A wheel that turns around short of its limit stops the walk at the turn,
/// and nothing there moves.
///
/// The turning point is declared so that a step can be cut at it; it is not a
/// constraint. With the limit at 1.0 and the momentum at 0.9, the same reversal
/// as below passes the realized torque through zero at `T_M ln 2` = 34.7 ms
/// without reaching a bound. The walk stops there once, the mode stays `Free`,
/// and the momentum keeps what the integration gave it.
#[test]
fn a_turning_point_short_of_the_limit_moves_nothing() {
    const LIMIT: f64 = 1.0;
    const T_M: f64 = 0.05;
    const TORQUE: f64 = 0.1;
    const START: f64 = 0.9;

    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let build = || {
        let wheel =
            orts::spacecraft::reaction_wheel::Rw::new(Vector3::z(), WHEEL_INERTIA, LIMIT, TORQUE)
                .with_motor_lag(T_M);
        let mut rw = ReactionWheelAssembly::new(vec![wheel]);
        rw.command = RwCommand::Torques(vec![-TORQUE]);
        SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia).with_effector(rw)
    };
    let system = build();
    let mut initial = system.initial_augmented_state(initial_plant());
    initial.aux[0] = START;
    initial.aux[1] = TORQUE;

    let mut group: IndependentGroup<Dynamics> =
        IndependentGroup::new(IntegratorConfig::Rk4 { dt: 0.1 }).add_satellite(
            "sat",
            initial,
            build(),
        );
    let mut stops = Vec::new();
    let mut peak = f64::NEG_INFINITY;
    let mut ever_held = false;
    group
        .propagate_to_with(0.2, |_id, t, state| {
            stops.push(t);
            peak = peak.max(state.aux[0]);
            ever_held |= state.modes[0] != ConstraintMode::Free;
        })
        .expect("the walk succeeds");

    assert!(!ever_held, "no bound is reached, so the wheel stays free");
    assert!(
        peak < LIMIT,
        "the momentum stays under the limit, peaking at {peak}"
    );
    let turn = T_M * 2.0_f64.ln();
    assert!(
        stops.iter().any(|t| (t - turn).abs() < 2e-3),
        "the walk stops where the torque turns ({turn:.6} s), among {stops:?}"
    );
    assert_eq!(
        stops.len(),
        3,
        "the turn is reported once, then the two step boundaries: {stops:?}"
    );
}

/// A brief excursion past the limit is held even when the step is coarser than
/// the motor lag, because the wheel declares where its momentum turns around.
///
/// Braking a wheel that is still accelerating outward, from just inside its
/// limit, sends the momentum past the limit and brings it back as the realized
/// torque decays through zero. With `h = 0.999`, a limit of 1, a realized
/// torque of +0.1 N·m against a command of -0.1, and a time constant of 50 ms,
/// the margin `limit - h` falls through zero at 13.2 ms and rises back through
/// it at 59.7 ms (analytic). Both zeros sit inside a 100 ms step, so the step's
/// two ends — 0.0010 and 0.0043 — are the same sign, and reading only those two
/// reported nothing at all.
///
/// The momentum turns around where the realized torque passes zero, at 34.7 ms,
/// and `BoundaryKind::TurningPoint` declares that. Locating it takes the search
/// through widths that end inside the excursion, which is what puts the upper
/// bound among the candidates: a 100 ms step now holds the wheel at 13.7 ms and
/// releases it at 34.8 ms, within the millisecond the default search asks for.
#[test]
fn a_step_coarser_than_the_motor_lag_still_holds_a_brief_excursion() {
    const LIMIT: f64 = 1.0;
    const T_M: f64 = 0.05;
    const TORQUE: f64 = 0.1;
    /// The default `RootSearch::t_tolerance`, which is how far a located time
    /// can sit past the crossing.
    const LOCATED_WITHIN: f64 = 1e-3;

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

    /// Peak momentum, and when the wheel went onto its bound and came off it.
    struct Walk {
        peak: f64,
        held_from: Option<f64>,
        released_at: Option<f64>,
    }

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
            let mut walk = Walk {
                peak: f64::NEG_INFINITY,
                held_from: None,
                released_at: None,
            };
            group
                .propagate_to_with(0.2, |_id, t, state| {
                    walk.peak = walk.peak.max(state.aux[0]);
                    let held = state.modes[0] != ConstraintMode::Free;
                    match (held, walk.held_from, walk.released_at) {
                        (true, None, _) => walk.held_from = Some(t),
                        (false, Some(_), None) => walk.released_at = Some(t),
                        _ => {}
                    }
                })
                .expect("the walk succeeds");
            walk
        };

    for (dt, label) in [(0.1, "100 ms"), (0.01, "10 ms")] {
        let walk = walk_with(dt);
        let held_from = walk
            .held_from
            .unwrap_or_else(|| panic!("a {label} step holds the wheel"));
        let released_at = walk
            .released_at
            .unwrap_or_else(|| panic!("a {label} step releases the wheel again"));
        assert!(
            (walk.peak - LIMIT).abs() < 1e-9,
            "a {label} step keeps the momentum on its bound, not at {}",
            walk.peak
        );
        assert!(
            (0.0132..=0.0132 + LOCATED_WITHIN).contains(&held_from),
            "a {label} step holds it at the crossing (13.2 ms), not at {held_from} s"
        );
        assert!(
            (0.0347..=0.0347 + LOCATED_WITHIN).contains(&released_at),
            "a {label} step releases it where the torque turns (34.7 ms), not at \
             {released_at} s"
        );
    }
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

/// The search's time tolerance is what the located time can be late by, and it
/// is the caller's to choose: the z wheel reaches its limit at exactly 5.3 s,
/// inside the step from 5.25 to 5.5. Asked for a micro-second, the walk halves
/// that step until it has the instant; asked for a fifth of a second, one
/// halving already brings the interval inside the tolerance and the crossing
/// is reported at 5.375 — 0.075 s late, and the momentum for those 75 ms goes
/// back to the body.
#[test]
fn the_tolerance_decides_how_closely_the_time_is_located() {
    const REACHED_AT: f64 = MAX_MOMENTUM / MAX_TORQUE;

    // The first instant the walk reported the z wheel sitting on its bound.
    let held_at = |t_tolerance: f64| -> f64 {
        let system = saturating_system();
        let initial = system.initial_augmented_state(initial_plant());
        let mut group: IndependentGroup<Dynamics> =
            IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT })
                .with_root_search(RootSearch {
                    t_tolerance,
                    ..RootSearch::default()
                })
                .add_satellite("sat", initial, saturating_system());

        let mut first = f64::INFINITY;
        group
            .propagate_to_with(T_END, |_id, t, state| {
                if (state.aux[2] + MAX_MOMENTUM).abs() < 1e-12 && t < first {
                    first = t;
                }
            })
            .expect("the walk succeeds");
        first
    };

    let tight = held_at(1e-6);
    assert!(
        (tight - REACHED_AT).abs() <= 1e-6,
        "asked for a micro-second, the bound is reported at {tight}, not {REACHED_AT}"
    );

    let loose = held_at(0.2);
    assert!(
        (loose - 5.375).abs() < 1e-12,
        "asked for a fifth of a second, one halving of the step from 5.25 gives \
         5.375, not {loose}"
    );
    assert!(
        loose - REACHED_AT > 0.05,
        "which is later than the tight answer, by more than the wheel's own \
         tolerance"
    );
}

/// A particle that bounces off a wall, for the paths a spacecraft with
/// effectors cannot reach: a coupled group and a scheduler keep one state per
/// satellite and need `FromAcceleration`, which `AugmentedState` does not
/// have, so nothing they can propagate declares a boundary of its own yet.
/// They do forward the search, and this is a system that shows it — the
/// bounce turns *when* the wall was met into where the particle ends up, which
/// the final state carries on its own.
struct Wall;

const WALL: f64 = 7000.0;
const WALL_SPEED: f64 = -100.0;
/// 31 km out at 100 km/s meets the wall at 310 ms — inside the first step of
/// the grid below, whose end is at 2.5 s.
const WALL_START: f64 = 7031.0;
const DT_WALL: f64 = 2.5;
const WALL_SPAN: f64 = 10.0;

impl utsuroi::DynamicalSystem for Wall {
    type State = OrbitalState;
    fn derivatives(&self, _t: f64, state: &OrbitalState) -> OrbitalState {
        OrbitalState::from_derivative(*state.velocity(), Vector3::zeros())
    }
}

impl HasBoundaries for Wall {
    fn boundaries(&self) -> Vec<DeclaredBoundary> {
        vec![DeclaredBoundary {
            satellite: 0,
            effector: 0,
            boundary: EffectorBoundary {
                kind: BoundaryKind::ReachedLower { index: 0 },
                boundary_tolerance: 0.0,
            },
            aux_offset: 0,
            aux_dim: 0,
            mode_offset: 0,
            mode_dim: 0,
        }]
    }

    fn boundary_value(
        &self,
        _declared: &DeclaredBoundary,
        _segment: Option<&SegmentContext>,
        _t: f64,
        state: &OrbitalState,
    ) -> f64 {
        state.position().x - WALL
    }

    fn settle_boundary(&self, _declared: &DeclaredBoundary, state: &mut OrbitalState) {
        // On the wall, and on its way back: the time it turned decides where
        // it ends up.
        *state = OrbitalState::new(
            Vector3::new(WALL, 0.0, 0.0),
            Vector3::new(-WALL_SPEED, 0.0, 0.0),
        );
    }

    fn boundary_is_active(&self, _declared: &DeclaredBoundary, state: &OrbitalState) -> bool {
        // On its way to the wall, which is the mode this system has instead of
        // a stored one.
        state.velocity().x < 0.0
    }
}

/// Each path locates the bounce to the tolerance it was given, and the
/// position at the end of the span says which: a micro-second puts the turn at
/// 0.31 s, a second needs two halvings of the 2.5 s step and puts it at 0.625,
/// and the particle ends 31 km apart between the two. A path that dropped the
/// forwarding would answer the same thing for both.
#[test]
fn every_path_that_forwards_the_tolerance_uses_it() {
    // Where the particle ends up if it turned at `t_hit`.
    fn ends_at(t_hit: f64) -> f64 {
        WALL - WALL_SPEED * (WALL_SPAN - t_hit)
    }
    let met_at = (WALL_START - WALL) / -WALL_SPEED;

    let search = |t_tolerance: f64| RootSearch {
        t_tolerance,
        ..RootSearch::default()
    };
    let start = || {
        OrbitalState::new(
            Vector3::new(WALL_START, 0.0, 0.0),
            Vector3::new(WALL_SPEED, 0.0, 0.0),
        )
    };

    let independent = |t_tolerance: f64| -> f64 {
        let mut group: IndependentGroup<Wall> =
            IndependentGroup::new(IntegratorConfig::Rk4 { dt: DT_WALL })
                .with_root_search(search(t_tolerance))
                .add_satellite("a", start(), Wall);
        group.propagate_to(WALL_SPAN).expect("the walk succeeds");
        group
            .satellites()
            .next()
            .expect("one satellite")
            .state
            .position()
            .x
    };

    let coupled = |t_tolerance: f64| -> f64 {
        let mut group: CoupledGroup<Wall> =
            CoupledGroup::new(IntegratorConfig::Rk4 { dt: DT_WALL })
                .with_root_search(search(t_tolerance))
                .add_satellite("a", start(), Wall);
        group.propagate_to(WALL_SPAN).expect("the walk succeeds");
        group.group_state().states[0].position().x
    };

    let scheduled = |t_tolerance: f64| -> f64 {
        let mut sched: Scheduler<Wall> = Scheduler::new(
            RegimeConfig {
                couple_enter: 1.0,
                couple_exit: 2.0,
                sync_enter: 2.0,
                sync_exit: 3.0,
                // No regrouping inside the span, so this walks the same
                // 2.5 s steps the groups above do: a sync interval shorter
                // than the tolerance would satisfy it with a whole step.
                sync_interval: 10.0,
                min_dwell_time: 0.0,
            },
            IntegratorConfig::Rk4 { dt: DT_WALL },
        )
        .with_root_search(search(t_tolerance))
        .add_satellite("a", start(), Wall);
        sched.propagate_to(WALL_SPAN).expect("the walk succeeds");
        sched
            .satellite_state(&"a".into())
            .expect("one satellite")
            .position()
            .x
    };

    // The scheduler has two branches and one satellite only reaches the
    // independent one. A declared interaction held at `Coupled` puts a pair in
    // one component, so the walk goes through `CoupledGroup` — with a force of
    // zero, so both particles still fly the trajectory the cases above expect.
    struct NoForce;

    impl InterSatelliteForce for NoForce {
        fn name(&self) -> &str {
            "no_force"
        }
        fn acceleration_pair(&self, _ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>) {
            (Vector3::zeros(), Vector3::zeros())
        }
    }

    let scheduled_coupled = |t_tolerance: f64| -> f64 {
        let mut sched: Scheduler<Wall> = Scheduler::new(
            RegimeConfig {
                couple_enter: 1.0,
                couple_exit: 2.0,
                sync_enter: 2.0,
                sync_exit: 3.0,
                sync_interval: 10.0,
                min_dwell_time: 0.0,
            },
            IntegratorConfig::Rk4 { dt: DT_WALL },
        )
        .with_root_search(search(t_tolerance))
        .add_satellite("a", start(), Wall)
        .add_satellite(
            "b",
            OrbitalState::new(
                Vector3::new(WALL_START, 0.5, 0.0),
                Vector3::new(WALL_SPEED, 0.0, 0.0),
            ),
            Wall,
        )
        .add_interaction_fixed("a", "b", PairRegime::Coupled, Arc::new(NoForce));
        sched.propagate_to(WALL_SPAN).expect("the walk succeeds");
        sched
            .satellite_state(&"a".into())
            .expect("the first satellite")
            .position()
            .x
    };

    for (name, at) in [
        ("independent group", independent(1e-6)),
        ("coupled group", coupled(1e-6)),
        ("scheduler", scheduled(1e-6)),
        ("scheduler, coupled component", scheduled_coupled(1e-6)),
    ] {
        assert!(
            (at - ends_at(met_at)).abs() < 1e-3,
            "{name}: a micro-second turns it at {met_at} and ends at {}, not {at}",
            ends_at(met_at)
        );
    }
    for (name, at) in [
        ("independent group", independent(1.0)),
        ("coupled group", coupled(1.0)),
        ("scheduler", scheduled(1.0)),
        ("scheduler, coupled component", scheduled_coupled(1.0)),
    ] {
        // Two halvings of the 2.5 s step: [0, 1.25] is still wider than the
        // tolerance, [0, 0.625] is not.
        assert!(
            (at - ends_at(0.625)).abs() < 1e-9,
            "{name}: a second turns it at 0.625 and ends at {}, not {at}",
            ends_at(0.625)
        );
    }
}
