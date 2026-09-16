//! Running dry is a boundary the propagation locates, so a burn spends the
//! propellant that exists and no more.
//!
//! Issue [#446] measured the alternative. A thruster asked whether the mass was
//! at its floor and answered from the state it was handed, so a step that
//! crossed the floor burned at full thrust for the whole step: with 0.04 kg of
//! propellant left, a thrust of 196.133 N and a one-second step, the spacecraft
//! ended 0.01 kg *below* its floor and gained 0.980273 m/s where Tsiolkovsky
//! gives 0.784375 m/s for the propellant it had — 25% too much.
//!
//! [#446]: https://github.com/sksat/orts/issues/446

use nalgebra::{Matrix3, Vector3};
use utsuroi::{RootSearch, Tolerances};

use orts::effector::{AugmentedState, ConstraintMode};
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital::OrbitalState;
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{G0, PropellantPool, SpacecraftDynamics, SpacecraftState, Thruster};

/// The case from the issue: 0.4 s of burning left, in steps of a second.
const THRUST_N: f64 = 196.133;
const ISP_S: f64 = 200.0;
const DRY_MASS: f64 = 100.0;
const PROPELLANT: f64 = 0.04;
const DT: f64 = 1.0;
const T_END: f64 = 5.0;

fn system() -> SpacecraftDynamics<PointMass> {
    // Free space: the ΔV to compare against is the burn's alone.
    SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
        .with_propellant(PropellantPool::new(DRY_MASS))
        .with_propulsion(Thruster::new(THRUST_N, ISP_S, Vector3::x()))
}

fn initial() -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        attitude: orts::attitude::AttitudeState::identity(),
        mass: DRY_MASS + PROPELLANT,
    }
}

/// Tsiolkovsky for the propellant the spacecraft actually carried [m/s].
fn analytical_dv() -> f64 {
    ISP_S * G0 * ((DRY_MASS + PROPELLANT) / DRY_MASS).ln()
}

/// Walk to `T_END` and answer the ΔV [m/s], the final mass and the pool's mode.
fn walk(config: IntegratorConfig, t_tolerance: f64) -> (f64, f64, ConstraintMode) {
    let system = system();
    let start = system.initial_augmented_state(initial());
    let v0 = *start.plant.orbit.velocity();

    let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(config)
        .with_root_search(RootSearch {
            t_tolerance,
            ..RootSearch::default()
        })
        .add_satellite("sat", start, system);
    group.propagate_to(T_END).expect("the walk succeeds");

    let ended: AugmentedState<SpacecraftState> = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    let dv = (*ended.plant.orbit.velocity() - v0).magnitude() * 1000.0;
    (dv, ended.plant.mass, ended.modes[0])
}

/// The burn spends the propellant that exists: the mass lands on the floor, the
/// mode says the tank is empty, and the ΔV is Tsiolkovsky's for that propellant
/// rather than for the whole step the crossing fell in.
#[test]
fn a_burn_across_the_floor_spends_the_propellant_it_has() {
    for (name, config) in [
        ("rk4", IntegratorConfig::Rk4 { dt: DT }),
        (
            "dp45",
            IntegratorConfig::Dp45 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-12,
                },
            },
        ),
        (
            "dop853",
            IntegratorConfig::Dop853 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-12,
                },
            },
        ),
    ] {
        let (dv, mass, mode) = walk(config, 1e-9);

        assert_eq!(mode, ConstraintMode::Lower, "{name}: the tank ends empty");
        assert!(
            (mass - DRY_MASS).abs() < 1e-9,
            "{name}: the mass ends on the floor, not at {mass}"
        );

        // A nanosecond of localization leaves the burn `ṁ · t_tolerance` past
        // the floor, and the impulse for that propellant stays: with
        // ṁ = 0.1 kg/s it is 1e-10 kg, worth about 2e-9 m/s.
        let excess = dv - analytical_dv();
        assert!(
            excess.abs() < 1e-6,
            "{name}: {dv} m/s against Tsiolkovsky's {}, {excess:.3e} too much",
            analytical_dv()
        );
    }
}

/// What the tolerance buys, in the units the issue measured: the excess ΔV is
/// the impulse for the propellant burned past the floor, which is `ṁ` times
/// the width the search narrowed the crossing to.
#[test]
fn the_excess_delta_v_follows_the_tolerance() {
    let mass_rate = THRUST_N / (ISP_S * G0);

    for t_tolerance in [1e-3, 1e-6] {
        let (dv, _, _) = walk(IntegratorConfig::Rk4 { dt: DT }, t_tolerance);
        let excess = dv - analytical_dv();
        // ΔV for the propellant spent past the floor, at the floor's mass.
        let budget = THRUST_N / DRY_MASS * t_tolerance;
        assert!(
            excess >= -1e-9 && excess <= budget * 1.5,
            "at {t_tolerance:e} the excess is {excess:.3e} m/s, and the budget \
             ṁ·ε/m = {:.3e} kg worth is {budget:.3e} m/s",
            mass_rate * t_tolerance
        );
    }
}
