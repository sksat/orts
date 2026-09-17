//! What the boundary handling costs, counted rather than timed.
//!
//! A located boundary is paid for in re-stepped intervals: the search halves
//! the interval holding the crossing until it is inside the tolerance, and each
//! halving re-steps from the interval's start. The count is
//! `⌈log₂(bracket / t_tolerance)⌉` per boundary located, and every step of it
//! evaluates the right-hand side as many times as the solver's stages.
//!
//! These cases count the right-hand side evaluations of one walk, so the number
//! is the same on any machine. The comparison that matters is a walk whose
//! wheel saturates against one whose wheel does not: the difference is what
//! locating one boundary cost.
//!
//! The cost is per boundary located, so it amortizes over a run: measured over
//! one orbit it is under a percent of the walk. Run with
//! `-- --nocapture --test-threads=1` to read the tables (two of them print,
//! and parallel tests interleave their lines), and with `--release` to add the
//! timed one.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nalgebra::{Matrix3, Vector3};
use utsuroi::{RootSearch, Tolerances};

use orts::attitude::AttitudeState;
use orts::effector::ConstraintMode;
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::model::{ExternalLoads, Model};
use orts::orbital::OrbitalState;
use orts::orbital::gravity::PointMass;
use orts::spacecraft::{ReactionWheelAssembly, RwCommand, SpacecraftDynamics, SpacecraftState};

const MAX_MOMENTUM: f64 = 0.53;
const MAX_TORQUE: f64 = 0.1;
const WHEEL_INERTIA: f64 = 0.01;
const BODY_INERTIA: f64 = 10.0;
const T_END: f64 = 20.0;
const DT: f64 = 0.25;

/// A model that does nothing and counts how often it was asked.
///
/// Every evaluation of the right-hand side evaluates every model once, so its
/// count is the number of right-hand side evaluations — accepted steps, trial
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

/// A spacecraft whose z wheel is driven at `torque`, counting its right-hand
/// side evaluations into `counter`.
fn system(torque: f64, counter: &Arc<AtomicUsize>) -> SpacecraftDynamics<PointMass> {
    let inertia = Matrix3::from_diagonal(&Vector3::repeat(BODY_INERTIA));
    let mut rw = ReactionWheelAssembly::three_axis(WHEEL_INERTIA, MAX_MOMENTUM, MAX_TORQUE);
    rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, torque)));
    SpacecraftDynamics::new(arika::earth::MU, PointMass, inertia)
        .with_model(EvalCounter(Arc::clone(counter)))
        .with_effector(rw)
}

fn initial_plant() -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(
            Vector3::new(7000.0, 0.0, 0.0),
            Vector3::new(0.0, 7.546, 0.0),
        ),
        attitude: AttitudeState::identity(),
        mass: 100.0,
    }
}

/// Walk 20 s and answer how many times the right-hand side was evaluated, and
/// whether the wheel ended up held on its bound.
///
/// The search narrows a crossing to `RootSearch::default`'s 1 ms here, which is
/// what every propagation path uses.
fn evaluations(torque: f64, config: IntegratorConfig, search: RootSearch) -> Walk {
    let counter = Arc::new(AtomicUsize::new(0));
    let built = system(torque, &counter);
    let initial = built.initial_augmented_state(initial_plant());

    let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(config)
        .with_root_search(search)
        .add_satellite("sat", initial, system(torque, &counter));
    // The observer runs once per accepted step, and once more for the state a
    // boundary was settled on. That extra report is the settled state, so the
    // first one whose wheel is held is the time the search located.
    let mut reports = 0usize;
    let mut located = None;
    group
        .propagate_to_with(T_END, |_id, t, state| {
            reports += 1;
            if located.is_none() && state.modes[2] != ConstraintMode::Free {
                located = Some(t);
            }
        })
        .expect("the walk succeeds");

    let held = group
        .satellites()
        .next()
        .expect("one satellite")
        .state
        .modes[2]
        != ConstraintMode::Free;
    Walk {
        evaluations: counter.load(Ordering::Relaxed),
        reports,
        held,
        located,
    }
}

/// Walk 20 s in two calls that meet at `cut`, with a wheel that reaches no
/// bound.
///
/// The propagation resumes from a boundary with a fresh stepper, so an adaptive
/// solver re-grows the step it had grown. Cutting a walk in two at an arbitrary
/// time costs exactly that and nothing else, which is how much of the cost
/// above is the restart rather than the search.
fn evaluations_cut_at(cut: f64, config: IntegratorConfig, search: RootSearch) -> Walk {
    let counter = Arc::new(AtomicUsize::new(0));
    let torque = MAX_TORQUE / 10.0;
    let built = system(torque, &counter);
    let initial = built.initial_augmented_state(initial_plant());

    let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(config)
        .with_root_search(search)
        .add_satellite("sat", initial, system(torque, &counter));
    let mut reports = 0usize;
    for target in [cut, T_END] {
        group
            .propagate_to_with(target, |_id, _t, _state| reports += 1)
            .expect("the walk succeeds");
    }
    Walk {
        evaluations: counter.load(Ordering::Relaxed),
        reports,
        held: false,
        located: None,
    }
}

/// Walk one orbit and answer how long it took [ms], as the median of five.
///
/// A realistic span rather than the twenty seconds above: 5500 s is about one
/// low orbit, and the wheel saturates 9 s in and is held for the rest.
fn milliseconds_for_an_orbit(torque: f64, config: IntegratorConfig) -> f64 {
    const ORBIT: f64 = 5500.0;
    let mut samples = Vec::new();
    for _ in 0..5 {
        let counter = Arc::new(AtomicUsize::new(0));
        let built = system(torque, &counter);
        let initial = built.initial_augmented_state(initial_plant());
        let mut group: IndependentGroup<SpacecraftDynamics<PointMass>> = IndependentGroup::new(
            config.clone(),
        )
        .add_satellite("sat", initial, system(torque, &counter));
        let started = std::time::Instant::now();
        group.propagate_to(ORBIT).expect("the walk succeeds");
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// What one walk cost and what it ended up doing.
struct Walk {
    /// Right-hand side evaluations, including every trial step of a search.
    evaluations: usize,
    /// States the walk reported: one per accepted step, plus one per boundary.
    reports: usize,
    /// Whether the wheel ended held on a bound.
    held: bool,
    /// The time the search located the bound at, if it reached one.
    located: Option<f64>,
}

/// What a finer tolerance costs, and what it buys.
///
/// The search halves the bracket, so a tolerance ten times finer is
/// $\log_2 10 \approx 3.3$ more re-stepped intervals — four more RK4 stages
/// each — and the located time is within the tolerance either way. A cost that
/// grew with `1 / tolerance` rather than with its logarithm would make a
/// nanosecond unaffordable; it costs about nine times a tenth of a second.
///
/// Run with `cargo test -p orts --test boundary_cost -- --nocapture`.
#[test]
fn a_finer_tolerance_costs_its_logarithm() {
    // A step of 0.25 s brackets the crossing at 5.3 s, so the search starts
    // from a quarter second whatever the tolerance is.
    const T_STAR: f64 = MAX_MOMENTUM / MAX_TORQUE;
    let config = IntegratorConfig::Rk4 { dt: DT };
    let quiet = evaluations(MAX_TORQUE / 10.0, config.clone(), RootSearch::default()).evaluations;

    println!("\nRK4 dt = 0.25 s, the bound at t = {T_STAR} s; 320 evaluations reach no bound");
    println!(
        "{:<12} {:>8} {:>12} {:>12} {:>14}",
        "t_tolerance", "evals", "for the one", "halvings", "time error"
    );
    let mut previous = None;
    for tol in [1e-1, 1e-2, 1e-3, 1e-4, 1e-6, 1e-9] {
        let walk = evaluations(
            MAX_TORQUE,
            config.clone(),
            RootSearch {
                t_tolerance: tol,
                max_iterations: 60,
            },
        );
        let located = walk.located.expect("the driven wheel reaches its bound");
        let searched = walk.evaluations - quiet;
        // Four stages per re-stepped interval, and the restart costs 4 more.
        let halvings = (searched as f64 / 4.0) - 1.0;
        println!(
            "{tol:<12.0e} {:>8} {searched:>12} {halvings:>12.0} {:>14.2e}",
            walk.evaluations,
            (located - T_STAR).abs()
        );
        assert!(
            (located - T_STAR).abs() <= tol,
            "the located time is inside the tolerance it was given: {located} for {tol}"
        );
        if let Some((coarser_tol, coarser_halvings)) = previous {
            let decades: f64 = coarser_tol / tol;
            let grown = halvings - coarser_halvings;
            assert!(
                grown < 2.0 * decades.log2() + 2.0,
                "{tol}: {grown} more halvings than {coarser_tol}, not the \
                 {:.1} a halving search costs",
                decades.log2()
            );
        }
        previous = Some((tol, halvings));
    }
}

/// The cost of the walk, printed as a table for the pull request to quote.
///
/// Run with `cargo test -p orts --test boundary_cost -- --nocapture`.
#[test]
fn what_locating_a_boundary_costs() {
    let integrators = [
        ("rk4 (dt=0.25)", IntegratorConfig::Rk4 { dt: DT }),
        (
            "dp45 (1e-10/1e-8)",
            IntegratorConfig::Dp45 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-10,
                    rtol: 1e-8,
                },
            },
        ),
        (
            "dop853 (1e-10/1e-8)",
            IntegratorConfig::Dop853 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-10,
                    rtol: 1e-8,
                },
            },
        ),
    ];

    println!("\nA 20 s walk, t_tolerance = 1 ms (the default)");
    println!(
        "{:<22} {:>18} {:>18} {:>8}",
        "integrator", "evals (none/bound)", "steps (none/bound)", "ratio"
    );
    for (name, config) in integrators {
        // A tenth of the torque never reaches the bound in 20 s.
        let quiet = evaluations(MAX_TORQUE / 10.0, config.clone(), RootSearch::default());
        let busy = evaluations(MAX_TORQUE, config, RootSearch::default());
        assert!(!quiet.held, "{name}: the gentle case reaches no bound");
        assert!(busy.held, "{name}: the driven case reaches its bound");
        let ratio = busy.evaluations as f64 / quiet.evaluations as f64;
        println!(
            "{name:<22} {:>8}/{:<9} {:>8}/{:<9} {ratio:>8.3}",
            quiet.evaluations, busy.evaluations, quiet.reports, busy.reports
        );
        // The search costs `⌈log₂(bracket / t_tolerance)⌉` re-stepped
        // intervals for the one boundary it locates, each of them the solver's
        // stages: ten halvings of a 0.25 s step at 1 ms. A regression that
        // searched on every step instead of on the one that crosses would cost
        // that much per step.
        let searched = busy.evaluations - quiet.evaluations;
        assert!(
            searched < 200,
            "{name}: locating one boundary cost {searched} evaluations"
        );
    }

    println!("\nOf that, the part that is resuming rather than searching");
    println!(
        "{:<22} {:>10} {:>12} {:>10}",
        "integrator", "one call", "cut at 5.3", "restart"
    );
    for (name, config) in [
        ("rk4 (dt=0.25)", IntegratorConfig::Rk4 { dt: DT }),
        (
            "dp45 (1e-10/1e-8)",
            IntegratorConfig::Dp45 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-10,
                    rtol: 1e-8,
                },
            },
        ),
        (
            "dop853 (1e-10/1e-8)",
            IntegratorConfig::Dop853 {
                dt: DT,
                tolerances: Tolerances {
                    atol: 1e-10,
                    rtol: 1e-8,
                },
            },
        ),
    ] {
        let whole = evaluations(MAX_TORQUE / 10.0, config.clone(), RootSearch::default());
        // Where the driven wheel reaches its bound.
        let cut = evaluations_cut_at(MAX_MOMENTUM / MAX_TORQUE, config, RootSearch::default());
        println!(
            "{name:<22} {:>10} {:>12} {:>+10}",
            whole.evaluations,
            cut.evaluations,
            cut.evaluations as i64 - whole.evaluations as i64
        );
    }

    // Timed only where the build is the one a run uses; in debug the numbers
    // say more about the assertions than about the walk.
    if cfg!(not(debug_assertions)) {
        println!("\nOne orbit (5500 s), median of five [ms]");
        println!(
            "{:<22} {:>10} {:>12} {:>8}",
            "integrator", "no bound", "one located", "ratio"
        );
        for (name, config) in [
            ("rk4 (dt=0.25)", IntegratorConfig::Rk4 { dt: DT }),
            (
                "dp45 (1e-10/1e-8)",
                IntegratorConfig::Dp45 {
                    dt: DT,
                    tolerances: Tolerances {
                        atol: 1e-10,
                        rtol: 1e-8,
                    },
                },
            ),
            (
                "dop853 (1e-10/1e-8)",
                IntegratorConfig::Dop853 {
                    dt: DT,
                    tolerances: Tolerances {
                        atol: 1e-10,
                        rtol: 1e-8,
                    },
                },
            ),
        ] {
            let quiet = milliseconds_for_an_orbit(MAX_TORQUE / 10.0, config.clone());
            let busy = milliseconds_for_an_orbit(MAX_TORQUE, config);
            println!(
                "{name:<22} {quiet:>10.2} {busy:>12.2} {:>8.3}",
                busy / quiet
            );
        }
    }
}
