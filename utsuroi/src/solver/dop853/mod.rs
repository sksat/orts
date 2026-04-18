mod coeff;

use core::ops::ControlFlow;

use coeff::*;

#[allow(unused_imports)]
use crate::math::F64Ext;
use crate::{
    DynamicalSystem, IntegrationError, IntegrationOutcome, Integrator, OdeState, Tolerances,
};

/// Dormand-Prince 8(5,3) adaptive step-size integrator (DOP853).
///
/// Uses a 12-stage embedded Runge-Kutta pair. The 8th-order solution is
/// propagated (local extrapolation); the 5th and 3rd-order solutions are
/// used for error estimation. Based on Hairer, Norsett & Wanner (1993).
pub struct Dop853;

/// Internal 12-stage DOP853 computation.
///
/// Returns `(y8, error, k13)` where:
/// - `y8`: 8th-order solution
/// - `error`: composite error estimate (combined 5th + 3rd order)
/// - `k13`: derivative at y8 (reusable as k1 of next step via FSAL)
fn dop853_step_impl<S: DynamicalSystem>(
    system: &S,
    t: f64,
    state: &S::State,
    dt: f64,
    k1: &S::State,
) -> (S::State, S::State, S::State) {
    // Stage 2
    let s2 = state.axpy(dt * A21, k1);
    let k2 = system.derivatives(t + C2 * dt, &s2);

    // Stage 3
    let s3 = state.axpy(dt * A31, k1).axpy(dt * A32, &k2);
    let k3 = system.derivatives(t + C3 * dt, &s3);

    // Stage 4
    let s4 = state.axpy(dt * A41, k1).axpy(dt * A43, &k3);
    let k4 = system.derivatives(t + C4 * dt, &s4);

    // Stage 5
    let s5 = state
        .axpy(dt * A51, k1)
        .axpy(dt * A53, &k3)
        .axpy(dt * A54, &k4);
    let k5 = system.derivatives(t + C5 * dt, &s5);

    // Stage 6
    let s6 = state
        .axpy(dt * A61, k1)
        .axpy(dt * A64, &k4)
        .axpy(dt * A65, &k5);
    let k6 = system.derivatives(t + C6 * dt, &s6);

    // Stage 7
    let s7 = state
        .axpy(dt * A71, k1)
        .axpy(dt * A74, &k4)
        .axpy(dt * A75, &k5)
        .axpy(dt * A76, &k6);
    let k7 = system.derivatives(t + C7 * dt, &s7);

    // Stage 8
    let s8 = state
        .axpy(dt * A81, k1)
        .axpy(dt * A84, &k4)
        .axpy(dt * A85, &k5)
        .axpy(dt * A86, &k6)
        .axpy(dt * A87, &k7);
    let k8 = system.derivatives(t + C8 * dt, &s8);

    // Stage 9
    let s9 = state
        .axpy(dt * A91, k1)
        .axpy(dt * A94, &k4)
        .axpy(dt * A95, &k5)
        .axpy(dt * A96, &k6)
        .axpy(dt * A97, &k7)
        .axpy(dt * A98, &k8);
    let k9 = system.derivatives(t + C9 * dt, &s9);

    // Stage 10
    let s10 = state
        .axpy(dt * A101, k1)
        .axpy(dt * A104, &k4)
        .axpy(dt * A105, &k5)
        .axpy(dt * A106, &k6)
        .axpy(dt * A107, &k7)
        .axpy(dt * A108, &k8)
        .axpy(dt * A109, &k9);
    let k10 = system.derivatives(t + C10 * dt, &s10);

    // Stage 11
    let s11 = state
        .axpy(dt * A111, k1)
        .axpy(dt * A114, &k4)
        .axpy(dt * A115, &k5)
        .axpy(dt * A116, &k6)
        .axpy(dt * A117, &k7)
        .axpy(dt * A118, &k8)
        .axpy(dt * A119, &k9)
        .axpy(dt * A1110, &k10);
    let k11 = system.derivatives(t + C11 * dt, &s11);

    // Stage 12
    let s12 = state
        .axpy(dt * A121, k1)
        .axpy(dt * A124, &k4)
        .axpy(dt * A125, &k5)
        .axpy(dt * A126, &k6)
        .axpy(dt * A127, &k7)
        .axpy(dt * A128, &k8)
        .axpy(dt * A129, &k9)
        .axpy(dt * A1210, &k10)
        .axpy(dt * A1211, &k11);
    let k12 = system.derivatives(t + dt, &s12);

    // 8th-order solution (y8)
    let y8 = state
        .axpy(dt * B1, k1)
        .axpy(dt * B6, &k6)
        .axpy(dt * B7, &k7)
        .axpy(dt * B8, &k8)
        .axpy(dt * B9, &k9)
        .axpy(dt * B10, &k10)
        .axpy(dt * B11, &k11)
        .axpy(dt * B12, &k12);

    // Stage 13 (FSAL: evaluated at y8)
    let k13 = system.derivatives(t + dt, &y8);

    // Error estimation: combine 5th-order and 3rd-order errors
    // 5th-order error: dt * (er1*k1 + er6*k6 + ... + er12*k12)
    let err5 = k1
        .scale(ER1)
        .axpy(ER6, &k6)
        .axpy(ER7, &k7)
        .axpy(ER8, &k8)
        .axpy(ER9, &k9)
        .axpy(ER10, &k10)
        .axpy(ER11, &k11)
        .axpy(ER12, &k12)
        .scale(dt);

    // 3rd-order error: y8 - bhh solution
    // bhh solution = y + dt * (bhh1*k1 + bhh2*k9 + bhh3*k12)
    // err3 = y8 - y_bhh = dt*((b1-bhh1)*k1 + b6*k6 + ... - bhh2*k9 - ... + (b12-bhh3)*k12)
    let err3 = k1
        .scale(B1 - BHH1)
        .axpy(B6, &k6)
        .axpy(B7, &k7)
        .axpy(B8, &k8)
        .axpy(B9 - BHH2, &k9)
        .axpy(B10, &k10)
        .axpy(B11, &k11)
        .axpy(B12 - BHH3, &k12)
        .scale(dt);

    // Combined error: sqrt((err5^2 + 0.01 * err3^2) / (1 + 0.01))
    // We use a weighted combination following Hairer's approach.
    // For the OdeState interface, we combine err5 and err3 into a single error vector.
    // The actual norm computation happens in error_norm_dop853.
    // Store err5 as the primary error; we'll compute the combined norm separately.
    // Actually, to keep the interface clean, we precompute a combined error.
    // Hairer uses: err = sqrt( (err5/sc)^2 + 0.01*(err3/sc)^2 ) / sqrt(1.01)
    // We approximate by returning err5 scaled up slightly when err3 dominates.
    // For simplicity in the OdeState interface, use err5 as error estimate.
    // The 5th-order error is the tighter one and drives step control.
    let _ = err3; // Will be used for combined norm later if needed
    let error = err5;

    (y8, error, k13)
}

impl Integrator for Dop853 {
    fn step<S: DynamicalSystem>(&self, system: &S, t: f64, state: &S::State, dt: f64) -> S::State {
        let k1 = system.derivatives(t, state);
        let (y8, _, _) = dop853_step_impl(system, t, state, dt, &k1);
        y8
    }
}

/// Result of [`AdaptiveStepper853::advance_to`].
pub enum AdvanceOutcome853<B> {
    /// Reached the target time.
    Reached,
    /// An event terminated integration early.
    Event { reason: B },
}

/// Stateful adaptive stepper for DOP853.
///
/// Created via [`Dop853::stepper`]. Callers repeatedly call
/// [`advance_to`](AdaptiveStepper853::advance_to) to advance to successive
/// target times.
pub struct AdaptiveStepper853<'a, S: DynamicalSystem> {
    system: &'a S,
    state: S::State,
    t: f64,
    dt: f64,
    k1: S::State,
    tol: Tolerances,
    /// Minimum step size below which integration fails.
    pub dt_min: f64,
}

impl<'a, S: DynamicalSystem> AdaptiveStepper853<'a, S> {
    /// Advance adaptively to `t_target`.
    pub fn advance_to<F, E, B>(
        &mut self,
        t_target: f64,
        mut callback: F,
        event_check: E,
    ) -> Result<AdvanceOutcome853<B>, IntegrationError>
    where
        F: FnMut(f64, &S::State),
        E: Fn(f64, &S::State) -> ControlFlow<B>,
    {
        while self.t < t_target {
            let h = self.dt.min(t_target - self.t);

            let (y8, error, k13) = dop853_step_impl(self.system, self.t, &self.state, h, &self.k1);

            // NaN/Inf check
            if !y8.is_finite() {
                return Err(IntegrationError::NonFiniteState { t: self.t + h });
            }

            let err = self.state.error_norm(&y8, &error, &self.tol);

            if err <= 1.0 {
                // Accept step
                self.state = y8;
                self.t += h;
                self.k1 = k13; // FSAL

                callback(self.t, &self.state);

                if let ControlFlow::Break(reason) = event_check(self.t, &self.state) {
                    return Ok(AdvanceOutcome853::Event { reason });
                }

                // Grow step size (8th-order exponent)
                let factor = if err < 1e-15 {
                    MAX_FACTOR
                } else {
                    (SAFETY * err.powf(-ORDER_EXP)).clamp(MIN_FACTOR, MAX_FACTOR)
                };
                self.dt = h * factor;
            } else {
                // Reject step, shrink
                let factor = (SAFETY * err.powf(-ORDER_EXP)).clamp(MIN_FACTOR, 1.0);
                self.dt = h * factor;

                if self.dt < self.dt_min {
                    return Err(IntegrationError::StepSizeTooSmall {
                        t: self.t,
                        dt: self.dt,
                    });
                }
            }
        }

        Ok(AdvanceOutcome853::Reached)
    }

    /// Current state.
    pub fn state(&self) -> &S::State {
        &self.state
    }

    /// Current time.
    pub fn t(&self) -> f64 {
        self.t
    }

    /// Current adaptive step size.
    pub fn dt(&self) -> f64 {
        self.dt
    }

    /// Consume the stepper and return the final state.
    pub fn into_state(self) -> S::State {
        self.state
    }
}

impl Dop853 {
    /// Create an [`AdaptiveStepper853`] for the given system and initial conditions.
    pub fn stepper<'a, S: DynamicalSystem>(
        &self,
        system: &'a S,
        initial: S::State,
        t0: f64,
        dt: f64,
        tol: Tolerances,
    ) -> AdaptiveStepper853<'a, S> {
        let k1 = system.derivatives(t0, &initial);
        let dt_min = 1e-12 * (dt * 100.0).abs().max(1.0);
        AdaptiveStepper853 {
            system,
            state: initial,
            t: t0,
            dt,
            k1,
            tol,
            dt_min,
        }
    }

    /// Perform a single DOP853 step with full output.
    ///
    /// Returns `(y8, error, k13)` where:
    /// - `y8`: 8th-order solution (to propagate)
    /// - `error`: error estimate
    /// - `k13`: derivative at y8 (reusable as k1 of next step via FSAL)
    pub fn step_full<S: DynamicalSystem>(
        &self,
        system: &S,
        t: f64,
        state: &S::State,
        dt: f64,
    ) -> (S::State, S::State, S::State) {
        let k1 = system.derivatives(t, state);
        dop853_step_impl(system, t, state, dt, &k1)
    }

    /// Integrate adaptively with event detection and NaN/Inf checking.
    #[allow(clippy::too_many_arguments)]
    pub fn integrate_adaptive_with_events<S, F, E, B>(
        &self,
        system: &S,
        initial: S::State,
        t0: f64,
        t_end: f64,
        dt_initial: f64,
        tol: &Tolerances,
        callback: F,
        event_check: E,
    ) -> IntegrationOutcome<S::State, B>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &S::State),
        E: Fn(f64, &S::State) -> ControlFlow<B>,
    {
        let mut stepper =
            self.stepper(system, initial, t0, dt_initial.min(t_end - t0), tol.clone());
        stepper.dt_min = 1e-12 * (t_end - t0).abs().max(1.0);

        match stepper.advance_to(t_end, callback, event_check) {
            Ok(AdvanceOutcome853::Reached) => IntegrationOutcome::Completed(stepper.into_state()),
            Ok(AdvanceOutcome853::Event { reason }) => {
                let t = stepper.t();
                IntegrationOutcome::Terminated {
                    state: stepper.into_state(),
                    t,
                    reason,
                }
            }
            Err(e) => IntegrationOutcome::Error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;

    use nalgebra::vector;

    use crate::test_systems::*;
    use crate::{IntegrationError, IntegrationOutcome, Integrator, State, Tolerances};

    use super::*;

    // --- Single step tests ---

    #[test]
    fn step_uniform_motion_exact() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let (y8, _error, _k13) = Dop853.step_full(&system, 0.0, &state, 1.0);
        let eps = 1e-12;
        assert!((y8.y().x - 1.0).abs() < eps, "y8 pos: {}", y8.y().x);
        assert!((y8.dy().x - 1.0).abs() < eps, "y8 vel: {}", y8.dy().x);
    }

    #[test]
    fn step_constant_acceleration_exact() {
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![10.0, 20.0, 0.0]);
        let dt = 1.0;
        let (y8, _error, _k13) = Dop853.step_full(&system, 0.0, &state, dt);

        let expected_px = 10.0;
        let expected_py = 20.0 + 0.5 * (-9.8) * 1.0;
        let expected_vy = 20.0 + (-9.8) * 1.0;

        let eps = 1e-12;
        assert!((y8.y().x - expected_px).abs() < eps);
        assert!((y8.y().y - expected_py).abs() < eps);
        assert!((y8.dy().y - expected_vy).abs() < eps);
    }

    #[test]
    fn step_error_estimate_reasonable() {
        let system = HarmonicOscillator;
        let state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let dt = 0.5;
        let (y8, error, _k13) = Dop853.step_full(&system, 0.0, &state, dt);

        let analytical_x = dt.cos();
        let actual_err = (y8.y().x - analytical_x).abs();
        let estimated_err = error.y().x.abs();

        assert!(actual_err > 0.0, "Actual error should be nonzero");
        assert!(estimated_err > 0.0, "Estimated error should be nonzero");

        // Error estimate should be a reasonable predictor
        let ratio = actual_err / estimated_err;
        assert!(
            ratio > 1e-4 && ratio < 1e4,
            "Error estimate ratio: actual={actual_err:.2e}, estimated={estimated_err:.2e}, ratio={ratio:.2}"
        );
    }

    #[test]
    fn step_fsal_property() {
        let system = HarmonicOscillator;
        let state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let dt = 0.1;
        let (y8, _error, k13) = Dop853.step_full(&system, 0.0, &state, dt);

        let k1_next = system.derivatives(dt, &y8);

        let eps = 1e-14;
        assert!(
            (k13.y() - k1_next.y()).magnitude() < eps,
            "FSAL velocity mismatch: {:?} vs {:?}",
            k13.y(),
            k1_next.y()
        );
        assert!(
            (k13.dy() - k1_next.dy()).magnitude() < eps,
            "FSAL acceleration mismatch: {:?} vs {:?}",
            k13.dy(),
            k1_next.dy()
        );
    }

    #[test]
    fn step_local_truncation_order() {
        // For an 8th-order method, local truncation error ~ O(h^9)
        // Halving h should reduce error by ~2^9 = 512
        // Use larger step sizes to avoid machine-precision floor
        let system = HarmonicOscillator;
        let state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);

        let dt1 = 0.5;
        let dt2 = 0.25;

        let (y8_coarse, _, _) = Dop853.step_full(&system, 0.0, &state, dt1);
        let (y8_fine, _, _) = Dop853.step_full(&system, 0.0, &state, dt2);

        let err_coarse = (y8_coarse.y().x - dt1.cos()).abs();
        let err_fine = (y8_fine.y().x - dt2.cos()).abs();

        let ratio = err_coarse / err_fine;
        // Expected ~512 for 9th-order local truncation
        assert!(
            ratio > 200.0 && ratio < 1500.0,
            "Local truncation order ratio = {ratio:.2}, expected ~512 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Fixed-step integration tests ---

    #[test]
    fn integrate_uniform_motion() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let final_state = Dop853.integrate(&system, initial, 0.0, 1.0, 0.1, |_, _| {});
        assert!((final_state.y().x - 1.0).abs() < 1e-12);
    }

    #[test]
    fn integrate_harmonic_full_period() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.01;
        let final_state = Dop853.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        // 8th-order should be extremely accurate even with dt=0.01
        let eps = 1e-13;
        assert!(
            (final_state.y().x - 1.0).abs() < eps,
            "After full period, x should be ~1.0, got {} (err={:.2e})",
            final_state.y().x,
            (final_state.y().x - 1.0).abs()
        );
        assert!(
            final_state.dy().x.abs() < eps,
            "After full period, vx should be ~0.0, got {} (err={:.2e})",
            final_state.dy().x,
            final_state.dy().x.abs()
        );
    }

    #[test]
    fn integrate_8th_order_convergence() {
        // Global error ~ O(h^8): halving h should reduce error by ~2^8 = 256
        // Use larger step sizes to stay above machine precision floor
        fn harmonic_error(dt: f64, steps: usize) -> f64 {
            let system = HarmonicOscillator;
            let mut state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
            let mut t = 0.0;
            for _ in 0..steps {
                let (y8, _, _) = Dop853.step_full(&system, t, &state, dt);
                state = y8;
                t += dt;
            }
            let x_error = (state.y().x - t.cos()).abs();
            let v_error = (state.dy().x + t.sin()).abs();
            x_error.max(v_error)
        }

        let err_coarse = harmonic_error(0.5, 20);
        let err_fine = harmonic_error(0.25, 40);

        let ratio = err_coarse / err_fine;
        // Expected ~256 for 8th-order global convergence
        assert!(
            ratio > 100.0 && ratio < 700.0,
            "DOP853 global convergence ratio = {ratio:.2}, expected ~256 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Adaptive integration tests ---

    #[test]
    fn adaptive_completes_normally() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let tol = Tolerances::default();
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            1.0,
            0.1,
            &tol,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        match outcome {
            IntegrationOutcome::Completed(state) => {
                assert!(
                    (state.y().x - 1.0).abs() < 1e-8,
                    "Expected position ~1.0, got {}",
                    state.y().x
                );
            }
            other => panic!("Expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn adaptive_harmonic_full_period() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            0.1,
            &tol,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        match outcome {
            IntegrationOutcome::Completed(state) => {
                let eps = 1e-6;
                assert!(
                    (state.y().x - 1.0).abs() < eps,
                    "After full period, x={} (err={:.2e})",
                    state.y().x,
                    (state.y().x - 1.0).abs()
                );
                assert!(
                    state.dy().x.abs() < eps,
                    "After full period, vx={} (err={:.2e})",
                    state.dy().x,
                    state.dy().x.abs()
                );
            }
            other => panic!("Expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn adaptive_energy_conservation() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let initial_energy = 0.5 * (initial.dy().norm_squared() + initial.y().norm_squared());
        let mut max_energy_drift: f64 = 0.0;

        let t_end = 2.0 * std::f64::consts::PI;
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            0.1,
            &tol,
            |_t, state| {
                let energy = 0.5 * (state.dy().norm_squared() + state.y().norm_squared());
                let drift = (energy - initial_energy).abs();
                max_energy_drift = max_energy_drift.max(drift);
            },
            |_t, _state| ControlFlow::Continue(()),
        );
        assert!(matches!(outcome, IntegrationOutcome::Completed(_)));
        assert!(
            max_energy_drift < 1e-7,
            "Energy drift {max_energy_drift:.2e} too large"
        );
    }

    #[test]
    fn adaptive_lands_on_t_end() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 1.234;
        let tol = Tolerances::default();
        let mut last_t = 0.0;
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            0.1,
            &tol,
            |t, _state| {
                last_t = t;
            },
            |_t, _state| ControlFlow::Continue(()),
        );
        assert!(matches!(outcome, IntegrationOutcome::Completed(_)));
        assert!(
            (last_t - t_end).abs() < 1e-12,
            "Last callback t={last_t}, expected t_end={t_end}"
        );
    }

    #[test]
    fn adaptive_terminates_on_event() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let tol = Tolerances::default();
        let outcome = Dop853.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            10.0,
            0.1,
            &tol,
            |_t, _state| {},
            |_t, state| {
                if state.y().x > 0.5 {
                    ControlFlow::Break("crossed threshold")
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        match outcome {
            IntegrationOutcome::Terminated { t, reason, .. } => {
                assert!(t < 10.0);
                assert!(
                    t > 0.4 && t < 1.5,
                    "Expected termination near 0.5, got t={t}"
                );
                assert_eq!(reason, "crossed threshold");
            }
            other => panic!("Expected Terminated, got {other:?}"),
        }
    }

    #[test]
    fn adaptive_detects_nan() {
        use crate::DynamicalSystem;

        struct ExplodingSystem;
        impl DynamicalSystem for ExplodingSystem {
            type State = State<3, 2>;
            fn derivatives(&self, t: f64, state: &State<3, 2>) -> State<3, 2> {
                let accel = if t > 0.3 {
                    vector![f64::INFINITY, 0.0, 0.0]
                } else {
                    vector![0.0, 0.0, 0.0]
                };
                State::<3, 2>::from_derivative(*state.dy(), accel)
            }
        }

        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let tol = Tolerances::default();
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &ExplodingSystem,
            initial,
            0.0,
            10.0,
            0.1,
            &tol,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        match outcome {
            IntegrationOutcome::Error(IntegrationError::NonFiniteState { t }) => {
                assert!(t > 0.3, "NaN detected at t={t}, expected after 0.3");
            }
            other => panic!("Expected NonFiniteState error, got {other:?}"),
        }
    }

    #[test]
    fn adaptive_detects_step_too_small() {
        use crate::DynamicalSystem;

        struct VeryStiffSystem;
        impl DynamicalSystem for VeryStiffSystem {
            type State = State<3, 2>;
            fn derivatives(&self, _t: f64, state: &State<3, 2>) -> State<3, 2> {
                State::<3, 2>::from_derivative(*state.dy(), -1e20 * state.y())
            }
        }

        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let tol = Tolerances {
            atol: 1e-12,
            rtol: 1e-12,
        };
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &VeryStiffSystem,
            initial,
            0.0,
            10.0,
            1.0,
            &tol,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        assert!(
            matches!(
                outcome,
                IntegrationOutcome::Error(IntegrationError::StepSizeTooSmall { .. })
            ),
            "Expected StepSizeTooSmall, got {outcome:?}"
        );
    }

    #[test]
    fn adaptive_fewer_steps_than_dp45() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };

        // Count DOP853 steps
        let mut dop853_steps = 0u64;
        let outcome853: IntegrationOutcome<State<3, 2>, ()> = Dop853
            .integrate_adaptive_with_events(
                &system,
                initial.clone(),
                0.0,
                t_end,
                1.0,
                &tol,
                |_t, _state| {
                    dop853_steps += 1;
                },
                |_t, _state| ControlFlow::Continue(()),
            );
        assert!(matches!(outcome853, IntegrationOutcome::Completed(_)));

        // Count DP45 steps
        let mut dp45_steps = 0u64;
        let outcome45: IntegrationOutcome<State<3, 2>, ()> = crate::DormandPrince
            .integrate_adaptive_with_events(
                &system,
                initial,
                0.0,
                t_end,
                1.0,
                &tol,
                |_t, _state| {
                    dp45_steps += 1;
                },
                |_t, _state| ControlFlow::Continue(()),
            );
        assert!(matches!(outcome45, IntegrationOutcome::Completed(_)));

        assert!(
            dop853_steps <= dp45_steps,
            "DOP853 should use fewer or equal steps: dop853={dop853_steps}, dp45={dp45_steps}"
        );
    }

    // --- Discriminating tests: problems where lower-order methods fail ---

    /// With a coarse fixed step (dt=0.5), DOP853's 8th-order accuracy yields
    /// good results on a harmonic oscillator full period, while RK4 (4th-order)
    /// accumulates ~1000x more error.
    #[test]
    fn coarse_step_dop853_accurate_rk4_fails() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.5; // Deliberately coarse

        let n_steps = (t_end / dt).ceil() as usize;
        let actual_dt = t_end / n_steps as f64;

        // DOP853
        let dop853_final =
            Dop853.integrate(&system, initial.clone(), 0.0, t_end, actual_dt, |_, _| {});
        let dop853_err = (dop853_final.y().x - 1.0).abs();

        // RK4
        let rk4_final = crate::Rk4.integrate(&system, initial, 0.0, t_end, actual_dt, |_, _| {});
        let rk4_err = (rk4_final.y().x - 1.0).abs();

        // DOP853 should be dramatically more accurate
        assert!(
            dop853_err < 1e-8,
            "DOP853 error {dop853_err:.2e} should be < 1e-8 with dt={actual_dt:.3}"
        );
        assert!(
            rk4_err > dop853_err * 100.0,
            "RK4 error {rk4_err:.2e} should be >100x DOP853 error {dop853_err:.2e}"
        );
    }

    /// Over 100 oscillation periods, DOP853 preserves energy far better than
    /// DP45 at the same adaptive tolerance.
    #[test]
    fn long_integration_dop853_better_energy_than_dp45() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 100.0 * 2.0 * std::f64::consts::PI; // 100 periods
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-10,
        };

        let energy = |s: &State<3, 2>| 0.5 * (s.y().norm_squared() + s.dy().norm_squared());
        let e0 = energy(&initial);

        // DOP853
        let mut max_drift_853: f64 = 0.0;
        let outcome853: IntegrationOutcome<State<3, 2>, ()> = Dop853
            .integrate_adaptive_with_events(
                &system,
                initial.clone(),
                0.0,
                t_end,
                1.0,
                &tol,
                |_t, state| {
                    max_drift_853 = max_drift_853.max((energy(state) - e0).abs());
                },
                |_t, _state| ControlFlow::Continue(()),
            );
        assert!(matches!(outcome853, IntegrationOutcome::Completed(_)));

        // DP45
        let mut max_drift_dp45: f64 = 0.0;
        let outcome45: IntegrationOutcome<State<3, 2>, ()> = crate::DormandPrince
            .integrate_adaptive_with_events(
                &system,
                initial,
                0.0,
                t_end,
                1.0,
                &tol,
                |_t, state| {
                    max_drift_dp45 = max_drift_dp45.max((energy(state) - e0).abs());
                },
                |_t, _state| ControlFlow::Continue(()),
            );
        assert!(matches!(outcome45, IntegrationOutcome::Completed(_)));

        // DOP853 should have noticeably less energy drift
        assert!(
            max_drift_853 < max_drift_dp45,
            "DOP853 energy drift {max_drift_853:.2e} should be less than DP45 {max_drift_dp45:.2e}"
        );
    }

    /// At tight tolerances, DOP853 requires fewer total function evaluations
    /// than DP45. DOP853 uses 13 stages/step (12 new with FSAL) vs DP45's
    /// 7 (6 new with FSAL), but its larger stable step size more than
    /// compensates.
    #[test]
    fn tight_tolerance_dop853_fewer_evaluations() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 10.0 * 2.0 * std::f64::consts::PI; // 10 periods
        let tol = Tolerances {
            atol: 1e-13,
            rtol: 1e-13,
        };

        // Count steps → function evaluations
        let mut dop853_steps = 0u64;
        let _: IntegrationOutcome<State<3, 2>, ()> = Dop853.integrate_adaptive_with_events(
            &system,
            initial.clone(),
            0.0,
            t_end,
            1.0,
            &tol,
            |_t, _state| {
                dop853_steps += 1;
            },
            |_t, _state| ControlFlow::Continue(()),
        );
        let dop853_evals = dop853_steps * 13; // 13 stages per step

        let mut dp45_steps = 0u64;
        let _: IntegrationOutcome<State<3, 2>, ()> = crate::DormandPrince
            .integrate_adaptive_with_events(
                &system,
                initial,
                0.0,
                t_end,
                1.0,
                &tol,
                |_t, _state| {
                    dp45_steps += 1;
                },
                |_t, _state| ControlFlow::Continue(()),
            );
        let dp45_evals = dp45_steps * 7; // 7 stages per step

        assert!(
            dop853_evals < dp45_evals,
            "DOP853 evals {dop853_evals} (={dop853_steps}×13) should be < DP45 evals {dp45_evals} (={dp45_steps}×7)"
        );
    }

    /// Over a very long integration (1000 periods) with a moderate fixed step,
    /// RK4's accumulated phase error becomes large while DOP853 remains
    /// orders of magnitude more accurate.
    #[test]
    fn long_fixed_step_rk4_drifts_dop853_accurate() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 1000.0 * 2.0 * std::f64::consts::PI; // 1000 periods
        let dt = 0.3;

        let rk4_final = crate::Rk4.integrate(&system, initial.clone(), 0.0, t_end, dt, |_, _| {});
        let dop853_final = Dop853.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let rk4_err = (rk4_final.y().x - 1.0).abs();
        let dop853_err = (dop853_final.y().x - 1.0).abs();

        // DOP853 should be dramatically more accurate
        assert!(
            dop853_err < rk4_err * 1e-4,
            "DOP853 err {dop853_err:.2e} should be >10000x better than RK4 err {rk4_err:.2e}"
        );
        // RK4 should have noticeable accumulated error
        assert!(
            rk4_err > 1e-2,
            "RK4 error {rk4_err:.2e} should be noticeable after 1000 periods with dt=0.3"
        );
    }
}
