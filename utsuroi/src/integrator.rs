use core::ops::ControlFlow;

use crate::error::{validate_step_size, validate_time_span};
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
    ///
    /// # Panics
    ///
    /// Panics if the arguments cannot produce a terminating integration —
    /// `dt <= 0`, a non-finite `dt`, `t_end < t0`, or a `dt` so small relative
    /// to `t` that `t + dt` rounds back to `t` — and if a step produces a
    /// non-finite state. Use [`try_integrate`](Integrator::try_integrate) to
    /// handle those cases without panicking. Previously the invalid arguments
    /// spun forever and the non-finite state ran to the end of the span.
    fn integrate<S, F>(
        &self,
        system: &S,
        initial: S::State,
        t0: f64,
        t_end: f64,
        dt: f64,
        callback: F,
    ) -> S::State
    where
        S: DynamicalSystem,
        F: FnMut(f64, &S::State),
    {
        match self.try_integrate(system, initial, t0, t_end, dt, callback) {
            Ok(state) => state,
            Err(e) => panic!("utsuroi: integrate() cannot run: {e}"),
        }
    }

    /// Fallible variant of [`integrate`](Integrator::integrate).
    ///
    /// Returns `Err` instead of panicking when the step size or time span
    /// cannot produce a terminating integration, and stops with
    /// [`IntegrationError::NonFiniteState`] at the first step whose result is
    /// not finite — the state after that step is not a trajectory, and every
    /// step taken from it is wasted.
    fn try_integrate<S, F>(
        &self,
        system: &S,
        initial: S::State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
    ) -> Result<S::State, IntegrationError>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &S::State),
    {
        validate_step_size(dt)?;
        validate_time_span(t0, t_end)?;

        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            let h = dt.min(t_end - t);
            // `h > 0` holds, but for large `|t|` it can still be below the
            // spacing of representable f64 values around `t`.
            if t + h == t {
                return Err(IntegrationError::TimeStagnated { t, dt: h });
            }
            state = self.step(system, t, &state, h);
            t += h;

            // The same check `integrate_with_events` makes. Without it the
            // controlled path — the one caller that uses `try_integrate` — read
            // sensors off a `NaN` state and fed it to a controller, and the run
            // reported success.
            if !state.is_finite() {
                return Err(IntegrationError::NonFiniteState { t });
            }

            callback(t, &state);
        }

        Ok(state)
    }

    /// Integrate a dynamical system with event detection and NaN/Inf checking.
    ///
    /// `event_check(t0, &initial)` runs first, before any step: a
    /// level-triggered event can already hold at `t0`, and reporting it after
    /// a step would name a state the predicate itself rejects. Terminating
    /// there returns the initial state at `t0` and calls no callback, since
    /// the callback reports accepted steps and none was taken.
    ///
    /// Then, after each step:
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
        if let Err(e) = validate_step_size(dt).and_then(|()| validate_time_span(t0, t_end)) {
            return IntegrationOutcome::Error(e);
        }

        let mut state = initial;
        let mut t = t0;

        // The predicate is asked about the state it was given, before any
        // step. A level-triggered event — "below the surface", "past this
        // altitude" — can already hold at `t0`, and stepping first reports it
        // one step late with a state the caller's own predicate calls invalid.
        // Only for a span that advances: an empty span takes no step, so it
        // reports nothing and asks nothing, and comes back as it went in.
        if t0 < t_end
            && let ControlFlow::Break(reason) = event_check(t0, &state)
        {
            return IntegrationOutcome::Terminated {
                state,
                t: t0,
                reason,
            };
        }

        while t < t_end {
            let h = dt.min(t_end - t);
            if t + h == t {
                return IntegrationOutcome::Error(IntegrationError::TimeStagnated { t, dt: h });
            }
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

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;

    use nalgebra::vector;

    use crate::test_systems::UniformMotion;
    use crate::{IntegrationError, IntegrationOutcome, Rk4, State};

    use super::*;

    fn system() -> UniformMotion {
        UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        }
    }

    fn initial() -> State<3, 2> {
        State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0])
    }

    // Each of these inputs used to spin the `while t < t_end` loop forever
    // instead of terminating.

    #[test]
    fn try_integrate_rejects_zero_step() {
        let err = Rk4
            .try_integrate(&system(), initial(), 0.0, 1.0, 0.0, |_, _| {})
            .unwrap_err();
        assert_eq!(err, IntegrationError::InvalidStepSize { dt: 0.0 });
    }

    #[test]
    fn try_integrate_rejects_negative_step() {
        let err = Rk4
            .try_integrate(&system(), initial(), 0.0, 1.0, -0.1, |_, _| {})
            .unwrap_err();
        assert_eq!(err, IntegrationError::InvalidStepSize { dt: -0.1 });
    }

    #[test]
    fn try_integrate_rejects_non_finite_step() {
        let err = Rk4
            .try_integrate(&system(), initial(), 0.0, 1.0, f64::NAN, |_, _| {})
            .unwrap_err();
        assert!(matches!(err, IntegrationError::InvalidStepSize { dt } if dt.is_nan()));
    }

    /// `t_end < t0` used to return the initial state unchanged, silently
    /// reporting success for an integration that never ran.
    #[test]
    fn try_integrate_rejects_backward_span() {
        let err = Rk4
            .try_integrate(&system(), initial(), 1.0, 0.0, 0.1, |_, _| {})
            .unwrap_err();
        assert_eq!(
            err,
            IntegrationError::InvalidTimeSpan {
                t0: 1.0,
                t_end: 0.0
            }
        );
    }

    #[test]
    fn try_integrate_accepts_empty_span() {
        // t_end == t0 is a legitimate no-op, not an error.
        let state = Rk4
            .try_integrate(&system(), initial(), 1.0, 1.0, 0.1, |_, _| {})
            .expect("empty span should be a no-op, not an error");
        assert_eq!(state.y()[0], 0.0);
    }

    /// `try_integrate` stops where `integrate_with_events` does. It used to run
    /// the whole span on a non-finite state and return `Ok`, so the controlled
    /// simulation — its one caller — read sensors off `NaN` and reported success.
    #[test]
    fn try_integrate_stops_at_a_non_finite_state() {
        struct Exploding;
        impl DynamicalSystem for Exploding {
            type State = State<3, 2>;
            fn derivatives(&self, t: f64, state: &Self::State) -> Self::State {
                let accel = if t > 0.3 {
                    vector![f64::INFINITY, 0.0, 0.0]
                } else {
                    vector![0.0, 0.0, 0.0]
                };
                State::<3, 2>::from_derivative(*state.dy(), accel)
            }
        }

        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let err = Rk4
            .try_integrate(&Exploding, initial, 0.0, 10.0, 0.1, |_, _| {})
            .expect_err("a non-finite state should stop the integration");
        match err {
            IntegrationError::NonFiniteState { t } => {
                assert!(t > 0.3, "should stop after the blow-up, got t={t}");
                assert!(
                    t < 10.0,
                    "should stop before the end of the span, got t={t}"
                );
            }
            other => panic!("expected NonFiniteState, got {other:?}"),
        }
    }

    /// At `t = 2^53` the f64 spacing exceeds 1, so `t + 0.5 == t`: `dt` is
    /// positive yet time cannot advance.
    #[test]
    fn try_integrate_detects_time_stagnation() {
        let t0 = 9007199254740992.0_f64; // 2^53
        let err = Rk4
            .try_integrate(&system(), initial(), t0, t0 + 2.0, 0.5, |_, _| {})
            .unwrap_err();
        assert!(matches!(err, IntegrationError::TimeStagnated { t, .. } if t == t0));
    }

    #[test]
    #[should_panic(expected = "step size must be positive and finite")]
    fn integrate_panics_on_zero_step() {
        Rk4.integrate(&system(), initial(), 0.0, 1.0, 0.0, |_, _| {});
    }

    #[test]
    fn integrate_with_events_rejects_zero_step() {
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Rk4.integrate_with_events(
            &system(),
            initial(),
            0.0,
            1.0,
            0.0,
            |_, _| {},
            |_, _| ControlFlow::Continue(()),
        );
        assert!(matches!(
            outcome,
            IntegrationOutcome::Error(IntegrationError::InvalidStepSize { dt }) if dt == 0.0
        ));
    }

    #[test]
    fn integrate_with_events_rejects_backward_span() {
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Rk4.integrate_with_events(
            &system(),
            initial(),
            1.0,
            0.0,
            0.1,
            |_, _| {},
            |_, _| ControlFlow::Continue(()),
        );
        assert!(matches!(
            outcome,
            IntegrationOutcome::Error(IntegrationError::InvalidTimeSpan { .. })
        ));
    }

    /// The guards must not change behaviour for well-formed inputs.
    #[test]
    fn try_integrate_matches_integrate_for_valid_input() {
        let a = Rk4.try_integrate(&system(), initial(), 0.0, 1.0, 0.1, |_, _| {});
        let b = Rk4.integrate(&system(), initial(), 0.0, 1.0, 0.1, |_, _| {});
        assert_eq!(a.expect("valid input"), b);
    }
}
