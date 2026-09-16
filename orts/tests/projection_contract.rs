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
use utsuroi::Tolerances;

use orts::effector::AugmentedState;
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{ReactionWheelAssembly, RwCommand, SpacecraftDynamics, SpacecraftState};

const MAX_MOMENTUM: f64 = 0.53;
const MAX_TORQUE: f64 = 0.1;
const WHEEL_INERTIA: f64 = 0.01;
/// Isotropic, so the body-frame total is the conserved one: with no external
/// torque and no off-diagonal terms, `I·ω + Σ aᵢ hᵢ` holds still.
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
