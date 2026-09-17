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

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nalgebra::{Matrix3, Vector3};
use utsuroi::{RootSearch, Tolerances};

use orts::effector::{AugmentedState, ConstraintMode};
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::model::{ExternalLoads, Model};
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

/// A model that does nothing and counts how often it was asked.
///
/// One evaluation of the right-hand side evaluates every model once, so its
/// count is the number of derivative evaluations: accepted steps, the trial
/// steps of a search, and the stages of each.
struct EvalCounter(Arc<AtomicUsize>);

impl Model<SpacecraftState> for EvalCounter {
    fn name(&self) -> &str {
        "eval_counter"
    }

    fn eval(
        &self,
        _t: f64,
        _state: &SpacecraftState,
        _epoch: Option<&arika::epoch::Epoch>,
    ) -> ExternalLoads {
        self.0.fetch_add(1, Ordering::Relaxed);
        ExternalLoads::zeros()
    }
}

/// The same spacecraft, counting its derivative evaluations into `counter`.
fn counted_system(counter: &Arc<AtomicUsize>) -> SpacecraftDynamics<PointMass> {
    system().with_model(EvalCounter(Arc::clone(counter)))
}

fn initial() -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        attitude: orts::attitude::AttitudeState::identity(),
        mass: DRY_MASS + PROPELLANT,
    }
}

/// The three integrators the configuration offers, at a tolerance tight enough
/// that the propellant figures are the boundary handling's and not the
/// solver's.
fn integrators() -> Vec<(&'static str, IntegratorConfig)> {
    vec![
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
    ]
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
    for (name, config) in integrators() {
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

/// A step wide enough to take a stage below zero mass still shows the crossing.
///
/// The domain guard is what keeps such a stage finite, and what it suppresses
/// matters: `F/m` is the singular half, while the mass rate is not — it is
/// `T / (I_sp g_0)`, the same at any mass. Suppressing the rate too would leave
/// both ends of the step above the floor, and the crossing inside it would go
/// unreported until a later step, which is the defect this PR is about.
///
/// 99 kg of propellant at 1 kg/s crosses the floor at 99 s, inside a
/// hundred-second step whose last stage sits at zero mass.
#[test]
fn a_step_whose_stage_reaches_zero_mass_still_locates_the_floor() {
    // 1 kg/s at Isp 200 s.
    const THRUST: f64 = 1.0 * ISP_S * G0;
    const FLOOR: f64 = 1.0;
    const PROPELLANT_HERE: f64 = 99.0;
    const STEP: f64 = 100.0;

    let built = || {
        SpacecraftDynamics::new(1e-30, PointMass, Matrix3::identity())
            .with_propellant(PropellantPool::new(FLOOR))
            .with_propulsion(Thruster::new(THRUST, ISP_S, Vector3::x()))
    };
    let system = built();
    let plant = SpacecraftState {
        mass: FLOOR + PROPELLANT_HERE,
        ..initial()
    };
    let start = system.initial_augmented_state(plant);

    let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(
        IntegratorConfig::Rk4 { dt: STEP },
    )
    .add_satellite("sat", start, built());
    let mut settled_at = None;
    group
        .propagate_to_with(STEP, |_id, t, state| {
            if settled_at.is_none() && state.modes[0] != ConstraintMode::Free {
                settled_at = Some(t);
            }
        })
        .expect("the walk succeeds");

    let settled_at = settled_at.expect("the floor is located inside the first step");
    assert!(
        (settled_at - PROPELLANT_HERE).abs() < 1e-2,
        "the floor is crossed at {PROPELLANT_HERE} s and located there, not at {settled_at}"
    );
    let ended = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .clone();
    assert!(
        (ended.plant.mass - FLOOR).abs() < 1e-6,
        "and the mass ends on the floor, not at {}",
        ended.plant.mass
    );
}

/// What locating the floor costs, counted rather than timed.
///
/// The propellant is what decides whether the walk meets the boundary at all,
/// so the comparison is the same burn with a tank that empties inside the span
/// against one that outlasts it. The difference is the search: the bracket is
/// the step the crossing falls in, halved until it is inside the tolerance,
/// and each halving re-steps the solver's stages.
///
/// Run with `cargo test -p orts --test propellant_floor -- --nocapture`.
#[test]
fn what_locating_the_floor_costs() {
    /// Propellant that outlasts the span: 0.1 kg/s for 5 s needs 0.5 kg.
    const PLENTY: f64 = 1.0;

    fn evaluations(propellant: f64, config: IntegratorConfig) -> (usize, ConstraintMode) {
        let counter = Arc::new(AtomicUsize::new(0));
        let built = counted_system(&counter);
        let plant = SpacecraftState {
            mass: DRY_MASS + propellant,
            ..initial()
        };
        let start = built.initial_augmented_state(plant);
        let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> =
            IndependentGroup::new(config).add_satellite("sat", start, counted_system(&counter));
        group.propagate_to(T_END).expect("the walk succeeds");
        let mode = group
            .satellites()
            .next()
            .expect("one satellite")
            .state
            .modes[0];
        (counter.load(Ordering::Relaxed), mode)
    }

    println!("\nA {T_END} s burn in {DT} s steps, t_tolerance = 1 ms (the default)");
    println!(
        "{:<10} {:>22} {:>20} {:>10}",
        "integrator", "evals (tank outlasts)", "evals (tank empties)", "ratio"
    );
    for (name, config) in integrators() {
        let (plenty, plenty_mode) = evaluations(PLENTY, config.clone());
        let (empty, empty_mode) = evaluations(PROPELLANT, config);
        assert_eq!(
            plenty_mode,
            ConstraintMode::Free,
            "{name}: the larger tank outlasts the span"
        );
        assert_eq!(
            empty_mode,
            ConstraintMode::Lower,
            "{name}: the smaller tank empties inside it"
        );
        println!(
            "{name:<10} {plenty:>22} {empty:>20} {:>10.3}",
            empty as f64 / plenty as f64
        );
        // Ten halvings of a one-second step at a millisecond, times the
        // solver's stages, is the order of it — a regression that searched on
        // every step would cost that much per step instead of once.
        let searched = empty.saturating_sub(plenty);
        assert!(
            searched < 300,
            "{name}: locating the floor cost {searched} evaluations"
        );
    }
}

/// What the tolerance buys, in the units the issue measured: the excess ΔV is
/// the impulse for the propellant burned past the floor, which is `ṁ` times
/// the width the search narrowed the crossing to.
#[test]
fn the_excess_delta_v_follows_the_tolerance() {
    let mass_rate = THRUST_N / (ISP_S * G0);

    println!("\nRK4 dt = 1 s, the floor crossed 0.4 s in");
    println!(
        "{:<14} {:>18} {:>18}",
        "t_tolerance", "excess dv [m/s]", "budget T/m_dry*e"
    );
    for t_tolerance in [1e-3, 1e-6, 1e-9] {
        let (dv, _, _) = walk(IntegratorConfig::Rk4 { dt: DT }, t_tolerance);
        let excess = dv - analytical_dv();
        // ΔV for the propellant spent past the floor, at the floor's mass.
        let budget = THRUST_N / DRY_MASS * t_tolerance;
        println!("{t_tolerance:<14e} {excess:>18.3e} {budget:>18.3e}");
        assert!(
            excess >= -1e-9 && excess <= budget * 1.5,
            "at {t_tolerance:e} the excess is {excess:.3e} m/s, and the budget \
             ṁ·ε/m = {:.3e} kg worth is {budget:.3e} m/s",
            mass_rate * t_tolerance
        );
    }
}
