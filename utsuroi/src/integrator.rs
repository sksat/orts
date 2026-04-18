use core::ops::ControlFlow;

use crate::{DynamicalSystem, IntegrationError, IntegrationOutcome, OdeState};

/// Common interface for fixed-step numerical integrators.
///
/// Implementors provide [`step`](Integrator::step), which advances the state
/// by a single time step. Default implementations of [`integrate`](Integrator::integrate)
/// and [`integrate_with_events`](Integrator::integrate_with_events) build on `step`
/// to provide multi-step integration with optional event detection.
pub trait Integrator {
    /// Perform a single integration step, advancing the state from `t` by `dt`.
    fn step<S: DynamicalSystem>(&self, system: &S, t: f64, state: &S::State, dt: f64) -> S::State;

    /// Integrate a dynamical system from `t0` to `t_end` using fixed step size `dt`.
    ///
    /// Calls `callback(t, &state)` after each step, allowing the caller to
    /// record intermediate states (e.g., for energy monitoring or trajectory output).
    ///
    /// Returns the final state at `t_end`.
    fn integrate<S, F>(
        &self,
        system: &S,
        initial: S::State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
    ) -> S::State
    where
        S: DynamicalSystem,
        F: FnMut(f64, &S::State),
    {
        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            let h = dt.min(t_end - t);
            state = self.step(system, t, &state, h);
            t += h;
            callback(t, &state);
        }

        state
    }

    /// Integrate a dynamical system with event detection and NaN/Inf checking.
    ///
    /// After each step:
    /// 1. Checks for NaN/Inf in state → returns `IntegrationOutcome::Error`
    /// 2. Calls `callback(t, &state)`
    /// 3. Calls `event_check(t, &state)` → if `Break(reason)`, returns `Terminated`
    #[allow(clippy::too_many_arguments)]
    fn integrate_with_events<S, F, E, B>(
        &self,
        system: &S,
        initial: S::State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
        event_check: E,
    ) -> IntegrationOutcome<S::State, B>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &S::State),
        E: Fn(f64, &S::State) -> ControlFlow<B>,
    {
        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            let h = dt.min(t_end - t);
            state = self.step(system, t, &state, h);
            t += h;

            if !state.is_finite() {
                return IntegrationOutcome::Error(IntegrationError::NonFiniteState { t });
            }

            callback(t, &state);

            if let ControlFlow::Break(reason) = event_check(t, &state) {
                return IntegrationOutcome::Terminated { state, t, reason };
            }
        }

        IntegrationOutcome::Completed(state)
    }
}
