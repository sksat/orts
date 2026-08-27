//! Cross-solver tests for the [`OdeState::project`] contract.
//!
//! Every integrator in this crate must project the state it publishes, and
//! must not pay for the projection when the state type does not use it. Both
//! halves are pinned here with spy state types that record what the solvers
//! actually do, so the contract does not depend on any particular domain type
//! (attitude quaternions, effector bounds) implementing it.

use core::cell::Cell;
use core::ops::ControlFlow;

use crate::{
    Dop853, DormandPrince, DynamicalSystem, Integrator, OdeState, Projection, Rk4, Tolerances,
};

// Bounded scalar: project() clamps into [0, 1]

/// Scalar state whose projection clamps it into `[0, 1]`.
///
/// `y' = 1` starting from `y = 0` overshoots the bound in a single step of
/// `dt = 2`, so any solver that skips `project` is visible immediately.
#[derive(Debug, Clone, PartialEq)]
struct Bounded {
    y: f64,
}

impl OdeState for Bounded {
    fn zero_like(&self) -> Self {
        Bounded { y: 0.0 }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Bounded {
            y: self.y + scale * other.y,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Bounded { y: factor * self.y }
    }

    fn is_finite(&self) -> bool {
        self.y.is_finite()
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let sc = tol.atol + tol.rtol * self.y.abs().max(y_next.y.abs());
        (error.y / sc).abs()
    }

    fn project(&mut self, _t: f64) -> Projection {
        let clamped = self.y.clamp(0.0, 1.0);
        if clamped == self.y {
            Projection::Unchanged
        } else {
            self.y = clamped;
            Projection::Changed
        }
    }
}

/// `y' = 1`: the exact solution leaves the bound behind linearly.
struct Ramp;

impl DynamicalSystem for Ramp {
    type State = Bounded;

    fn derivatives(&self, _t: f64, _state: &Bounded) -> Bounded {
        Bounded { y: 1.0 }
    }
}

fn tol() -> Tolerances {
    // The CLI defaults.
    Tolerances {
        atol: 1e-10,
        rtol: 1e-8,
    }
}

#[test]
fn rk4_step_projects() {
    let y = Rk4.step(&Ramp, 0.0, &Bounded { y: 0.0 }, 2.0);
    assert_eq!(y.y, 1.0, "RK4 step must publish a projected state");
}

#[test]
fn dp45_fixed_step_projects() {
    let y = DormandPrince.step(&Ramp, 0.0, &Bounded { y: 0.0 }, 2.0);
    assert_eq!(y.y, 1.0, "DP45 step must publish a projected state");
}

#[test]
fn dop853_fixed_step_projects() {
    let y = Dop853.step(&Ramp, 0.0, &Bounded { y: 0.0 }, 2.0);
    assert_eq!(y.y, 1.0, "DOP853 step must publish a projected state");
}

#[test]
fn dp45_adaptive_projects_every_published_state() {
    let mut stepper = DormandPrince.stepper(&Ramp, Bounded { y: 0.0 }, 0.0, 0.5, tol());
    let mut worst = 0.0_f64;
    let outcome = stepper
        .advance_to::<_, _, ()>(
            2.0,
            |_t, s| worst = worst.max(s.y),
            |_t, s| {
                // The event check must see the projected state too.
                assert!(s.y <= 1.0, "event check saw unprojected y = {}", s.y);
                ControlFlow::Continue(())
            },
        )
        .expect("integration should reach the target");
    assert!(matches!(outcome, crate::AdvanceOutcome::Reached));
    assert!(
        worst <= 1.0,
        "DP45 published a state outside the bound: y = {worst}"
    );
    assert_eq!(stepper.state().y, 1.0);
}

#[test]
fn dop853_adaptive_projects_every_published_state() {
    let mut stepper = Dop853.stepper(&Ramp, Bounded { y: 0.0 }, 0.0, 0.5, tol());
    let mut worst = 0.0_f64;
    let outcome = stepper
        .advance_to::<_, _, ()>(
            2.0,
            |_t, s| worst = worst.max(s.y),
            |_t, s| {
                assert!(s.y <= 1.0, "event check saw unprojected y = {}", s.y);
                ControlFlow::Continue(())
            },
        )
        .expect("integration should reach the target");
    assert!(matches!(outcome, crate::AdvanceOutcome853::Reached));
    assert!(
        worst <= 1.0,
        "DOP853 published a state outside the bound: y = {worst}"
    );
    assert_eq!(stepper.state().y, 1.0);
}

/// `step_full` is the low-level entry point: it returns the raw candidate so
/// that a caller running its own step-size control can weigh it against the
/// error estimate that belongs to it. Projecting an accepted candidate is that
/// caller's job, and this pins the split.
#[test]
fn step_full_returns_the_unprojected_candidate() {
    let (y5, _, _) = DormandPrince.step_full(&Ramp, 0.0, &Bounded { y: 0.0 }, 2.0);
    assert!(
        (y5.y - 2.0).abs() < 1e-12,
        "step_full must not project, got y = {}",
        y5.y
    );
    let mut accepted = y5;
    assert_eq!(accepted.project(2.0), Projection::Changed);
    assert_eq!(accepted.y, 1.0);

    let (y8, _, _) = Dop853.step_full(&Ramp, 0.0, &Bounded { y: 0.0 }, 2.0);
    assert!(
        (y8.y - 2.0).abs() < 1e-12,
        "step_full must not project, got y = {}",
        y8.y
    );
    let mut accepted = y8;
    assert_eq!(accepted.project(2.0), Projection::Changed);
    assert_eq!(accepted.y, 1.0);
}

// Evaluation counting: how much does honouring the contract cost?

/// Counts calls to [`DynamicalSystem::derivatives`].
///
/// Step-count callbacks fire only on accepted steps, so they cannot measure
/// what a solver spends: they miss every rejected trial and every derivative
/// reused through FSAL. This wrapper counts the real thing.
pub(crate) struct Counting<'a, S> {
    inner: &'a S,
    count: Cell<u64>,
}

impl<'a, S> Counting<'a, S> {
    pub(crate) fn new(inner: &'a S) -> Self {
        Self {
            inner,
            count: Cell::new(0),
        }
    }

    pub(crate) fn count(&self) -> u64 {
        self.count.get()
    }
}

impl<S: DynamicalSystem> DynamicalSystem for Counting<'_, S> {
    type State = S::State;

    fn derivatives(&self, t: f64, state: &Self::State) -> Self::State {
        self.count.set(self.count.get() + 1);
        self.inner.derivatives(t, state)
    }
}

/// Counters read from inside a spy state.
///
/// `error_norm` is called exactly once per accept/reject decision, so counting
/// it counts *trials* — a quantity independent of the derivative count under
/// test. Deriving the trial count from the derivative count instead would only
/// confirm that the latter is divisible by the stage count, and would accept a
/// solver that spent twice as many evaluations per trial.
#[derive(Debug, Default)]
struct Spy {
    trials: Cell<u64>,
}

/// State with a decaying value plus a "marker" component that only exists to
/// make `project` report [`Projection::Changed`].
///
/// The marker is excluded from `error_norm`, and nothing in the dynamics reads
/// it, so switching `reset_marker` on changes *only* whether the projection
/// reports a change — the accepted step sequence stays identical. That makes
/// the difference in derivative evaluations attributable to the projection
/// alone.
#[derive(Debug, Clone)]
struct Marked<'a> {
    y: f64,
    marker: f64,
    reset_marker: bool,
    spy: &'a Spy,
}

impl OdeState for Marked<'_> {
    fn zero_like(&self) -> Self {
        Marked {
            y: 0.0,
            marker: 0.0,
            reset_marker: self.reset_marker,
            spy: self.spy,
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Marked {
            y: self.y + scale * other.y,
            marker: self.marker + scale * other.marker,
            reset_marker: self.reset_marker,
            spy: self.spy,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Marked {
            y: factor * self.y,
            marker: factor * self.marker,
            reset_marker: self.reset_marker,
            spy: self.spy,
        }
    }

    fn is_finite(&self) -> bool {
        self.y.is_finite() && self.marker.is_finite()
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        // One call per accept/reject decision, i.e. one per trial.
        self.spy.trials.set(self.spy.trials.get() + 1);
        let sc = tol.atol + tol.rtol * self.y.abs().max(y_next.y.abs());
        (error.y / sc).abs()
    }

    fn project(&mut self, _t: f64) -> Projection {
        if self.reset_marker && self.marker != 0.0 {
            self.marker = 0.0;
            Projection::Changed
        } else {
            Projection::Unchanged
        }
    }
}

/// `y' = -y`, `marker' = 1`.
struct Decay<'a> {
    spy: &'a Spy,
}

impl<'a> DynamicalSystem for Decay<'a> {
    type State = Marked<'a>;

    fn derivatives(&self, _t: f64, state: &Marked<'a>) -> Marked<'a> {
        Marked {
            y: -state.y,
            marker: 1.0,
            reset_marker: state.reset_marker,
            spy: self.spy,
        }
    }
}

fn initial(reset_marker: bool, spy: &Spy) -> Marked<'_> {
    Marked {
        y: 1.0,
        marker: 0.0,
        reset_marker,
        spy,
    }
}

/// What one integration spent.
struct Cost {
    /// Calls to [`DynamicalSystem::derivatives`].
    evals: u64,
    /// Accepted steps (callback invocations).
    accepted: u64,
    /// Accept/reject decisions, counted inside the state.
    trials: u64,
}

fn dp45_cost(reset_marker: bool) -> Cost {
    let spy = Spy::default();
    let decay = Decay { spy: &spy };
    let system = Counting::new(&decay);
    let mut stepper = DormandPrince.stepper(&system, initial(reset_marker, &spy), 0.0, 0.1, tol());
    let mut accepted = 0_u64;
    stepper
        .advance_to::<_, _, ()>(
            5.0,
            |_t, _s| accepted += 1,
            |_t, _s| ControlFlow::Continue(()),
        )
        .expect("integration should reach the target");
    Cost {
        evals: system.count(),
        accepted,
        trials: spy.trials.get(),
    }
}

fn dop853_cost(reset_marker: bool) -> Cost {
    let spy = Spy::default();
    let decay = Decay { spy: &spy };
    let system = Counting::new(&decay);
    let mut stepper = Dop853.stepper(&system, initial(reset_marker, &spy), 0.0, 0.1, tol());
    let mut accepted = 0_u64;
    stepper
        .advance_to::<_, _, ()>(
            5.0,
            |_t, _s| accepted += 1,
            |_t, _s| ControlFlow::Continue(()),
        )
        .expect("integration should reach the target");
    Cost {
        evals: system.count(),
        accepted,
        trials: spy.trials.get(),
    }
}

/// A state that never reports a change must cost exactly what the FSAL scheme
/// promises: 6 evaluations per trial plus the single initial derivative.
#[test]
fn dp45_no_op_projection_costs_nothing() {
    let cost = dp45_cost(false);
    assert!(
        cost.accepted > 5,
        "expected several steps, got {}",
        cost.accepted
    );
    assert_eq!(
        cost.trials, cost.accepted,
        "y' = -y at these tolerances should not need a retry \
         ({} trials for {} accepted steps)",
        cost.trials, cost.accepted
    );
    assert_eq!(
        cost.evals,
        1 + 6 * cost.trials,
        "DP45 with a no-op projection: {} evaluations for {} trials",
        cost.evals,
        cost.trials
    );
}

/// Only a projection that actually changes the state costs an extra
/// evaluation, and only one per changed step — the last one is never paid,
/// because the invalidated derivative is re-evaluated lazily by the *next*
/// step.
#[test]
fn dp45_changed_projection_costs_one_extra_evaluation_per_step() {
    let noop = dp45_cost(false);
    let changed = dp45_cost(true);
    assert_eq!(
        (noop.accepted, noop.trials),
        (changed.accepted, changed.trials),
        "the marker must not perturb the step sequence"
    );
    assert_eq!(
        changed.evals - noop.evals,
        changed.accepted - 1,
        "expected one extra evaluation per changed step except the last \
         ({} -> {} over {} steps)",
        noop.evals,
        changed.evals,
        changed.accepted
    );
}

/// DOP853's 13th stage *is* the next step's first stage, so it is evaluated at
/// the projected state by the next step. The projection is therefore free, and
/// the count is the same whether or not it changes the state.
#[test]
fn dop853_projection_is_free() {
    let noop = dop853_cost(false);
    let changed = dop853_cost(true);
    assert_eq!(
        (noop.accepted, noop.trials, noop.evals),
        (changed.accepted, changed.trials, changed.evals),
        "the projection must cost nothing for DOP853"
    );
    assert!(
        noop.accepted > 3,
        "expected several steps, got {}",
        noop.accepted
    );
    assert_eq!(
        noop.trials, noop.accepted,
        "y' = -y at these tolerances should not need a retry"
    );
    assert_eq!(
        noop.evals,
        11 * noop.trials + noop.accepted,
        "DOP853: 11 stages per trial plus one first-stage evaluation per step \
         ({} evaluations for {} trials, {} accepted)",
        noop.evals,
        noop.trials,
        noop.accepted
    );
}

/// A rejected trial must not cost the 13th stage: DOP853 spends 11
/// evaluations on a candidate it throws away, not 12.
#[test]
fn dop853_rejected_trial_costs_eleven_evaluations() {
    let spy = Spy::default();
    let decay = Decay { spy: &spy };
    let system = Counting::new(&decay);
    let tol = Tolerances {
        atol: 1e-14,
        rtol: 1e-14,
    };
    // A far too large first step is rejected, then retried smaller.
    let mut stepper = Dop853.stepper(&system, initial(false, &spy), 0.0, 5.0, tol);
    let mut accepted = 0_u64;
    stepper
        .advance_to::<_, _, ()>(
            5.0,
            |_t, _s| accepted += 1,
            |_t, _s| ControlFlow::Continue(()),
        )
        .expect("integration should reach the target");
    let evals = system.count();
    let trials = spy.trials.get();
    assert!(
        trials > accepted,
        "this setup should reject at least one trial (trials={trials}, accepted={accepted})"
    );
    // Each trial costs 11 evaluations; each step start additionally pays the
    // single first-stage evaluation, exactly once per accepted step.
    assert_eq!(
        evals,
        11 * trials + accepted,
        "{evals} evaluations for {trials} trials and {accepted} accepted steps"
    );
}
