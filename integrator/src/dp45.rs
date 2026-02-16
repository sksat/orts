use std::ops::ControlFlow;

use nalgebra::Vector3;

use crate::{
    DynamicalSystem, IntegrationError, IntegrationOutcome, Integrator, State, StateDerivative,
    Tolerances,
};

// ---------------------------------------------------------------------------
// Dormand-Prince RK5(4)7M coefficients (Dormand & Prince, 1980)
// ---------------------------------------------------------------------------

// Nodes (c_i)
const DP_C2: f64 = 1.0 / 5.0;
const DP_C3: f64 = 3.0 / 10.0;
const DP_C4: f64 = 4.0 / 5.0;
const DP_C5: f64 = 8.0 / 9.0;
// c6 = 1.0, c7 = 1.0 (used inline)

// a-matrix coefficients
const DP_A21: f64 = 1.0 / 5.0;

const DP_A31: f64 = 3.0 / 40.0;
const DP_A32: f64 = 9.0 / 40.0;

const DP_A41: f64 = 44.0 / 45.0;
const DP_A42: f64 = -56.0 / 15.0;
const DP_A43: f64 = 32.0 / 9.0;

const DP_A51: f64 = 19372.0 / 6561.0;
const DP_A52: f64 = -25360.0 / 2187.0;
const DP_A53: f64 = 64448.0 / 6561.0;
const DP_A54: f64 = -212.0 / 729.0;

const DP_A61: f64 = 9017.0 / 3168.0;
const DP_A62: f64 = -355.0 / 33.0;
const DP_A63: f64 = 46732.0 / 5247.0;
const DP_A64: f64 = 49.0 / 176.0;
const DP_A65: f64 = -5103.0 / 18656.0;

// 5th-order weights (b_i) — also row 7 of a-matrix (FSAL property)
const DP_B1: f64 = 35.0 / 384.0;
// DP_B2 = 0
const DP_B3: f64 = 500.0 / 1113.0;
const DP_B4: f64 = 125.0 / 192.0;
const DP_B5: f64 = -2187.0 / 6784.0;
const DP_B6: f64 = 11.0 / 84.0;
// DP_B7 = 0

// Error coefficients (e_i = b_i - b*_i)
const DP_E1: f64 = 71.0 / 57600.0;
// DP_E2 = 0
const DP_E3: f64 = -71.0 / 16695.0;
const DP_E4: f64 = 71.0 / 1920.0;
const DP_E5: f64 = -17253.0 / 339200.0;
const DP_E6: f64 = 22.0 / 525.0;
const DP_E7: f64 = -1.0 / 40.0;

// Step-size controller constants
const DP_SAFETY: f64 = 0.9;
const DP_MIN_FACTOR: f64 = 0.2;
const DP_MAX_FACTOR: f64 = 5.0;

/// Compute the RMS error norm for adaptive step-size control.
///
/// Uses the mixed absolute/relative tolerance formula from Hairer, Norsett, Wanner:
///   sc_i = atol + rtol * max(|y_n_i|, |y_{n+1}_i|)
///   err = sqrt(1/N * sum((delta_i / sc_i)^2))
pub fn error_norm(
    y_n: &State,
    y_next: &State,
    error_pos: &Vector3<f64>,
    error_vel: &Vector3<f64>,
    tol: &Tolerances,
) -> f64 {
    let mut sum_sq = 0.0;
    let n = 6; // 3 position + 3 velocity components

    for i in 0..3 {
        let sc = tol.atol + tol.rtol * y_n.position[i].abs().max(y_next.position[i].abs());
        let e = error_pos[i] / sc;
        sum_sq += e * e;
    }
    for i in 0..3 {
        let sc = tol.atol + tol.rtol * y_n.velocity[i].abs().max(y_next.velocity[i].abs());
        let e = error_vel[i] / sc;
        sum_sq += e * e;
    }

    (sum_sq / n as f64).sqrt()
}

/// Dormand-Prince RK5(4)7M adaptive step-size integrator.
///
/// Uses a 7-stage embedded Runge-Kutta pair. The 5th-order solution is
/// propagated (local extrapolation); the 4th-order solution is used only
/// for error estimation. The FSAL (First Same As Last) property allows
/// reuse of the 7th stage derivative as the 1st stage of the next step.
pub struct DormandPrince;

/// Internal 7-stage Dormand-Prince computation.
///
/// Returns `(y5, error_pos, error_vel, k7)`.
fn dp_step_impl<S: DynamicalSystem>(
    system: &S,
    t: f64,
    state: &State,
    dt: f64,
    k1: &StateDerivative,
) -> (State, Vector3<f64>, Vector3<f64>, StateDerivative) {
    // Stage 2
    let s2 = State {
        position: state.position + dt * DP_A21 * k1.velocity,
        velocity: state.velocity + dt * DP_A21 * k1.acceleration,
    };
    let k2 = system.derivatives(t + DP_C2 * dt, &s2);

    // Stage 3
    let s3 = State {
        position: state.position + dt * (DP_A31 * k1.velocity + DP_A32 * k2.velocity),
        velocity: state.velocity + dt * (DP_A31 * k1.acceleration + DP_A32 * k2.acceleration),
    };
    let k3 = system.derivatives(t + DP_C3 * dt, &s3);

    // Stage 4
    let s4 = State {
        position: state.position
            + dt * (DP_A41 * k1.velocity + DP_A42 * k2.velocity + DP_A43 * k3.velocity),
        velocity: state.velocity
            + dt * (DP_A41 * k1.acceleration
                + DP_A42 * k2.acceleration
                + DP_A43 * k3.acceleration),
    };
    let k4 = system.derivatives(t + DP_C4 * dt, &s4);

    // Stage 5
    let s5 = State {
        position: state.position
            + dt * (DP_A51 * k1.velocity
                + DP_A52 * k2.velocity
                + DP_A53 * k3.velocity
                + DP_A54 * k4.velocity),
        velocity: state.velocity
            + dt * (DP_A51 * k1.acceleration
                + DP_A52 * k2.acceleration
                + DP_A53 * k3.acceleration
                + DP_A54 * k4.acceleration),
    };
    let k5 = system.derivatives(t + DP_C5 * dt, &s5);

    // Stage 6
    let s6 = State {
        position: state.position
            + dt * (DP_A61 * k1.velocity
                + DP_A62 * k2.velocity
                + DP_A63 * k3.velocity
                + DP_A64 * k4.velocity
                + DP_A65 * k5.velocity),
        velocity: state.velocity
            + dt * (DP_A61 * k1.acceleration
                + DP_A62 * k2.acceleration
                + DP_A63 * k3.acceleration
                + DP_A64 * k4.acceleration
                + DP_A65 * k5.acceleration),
    };
    let k6 = system.derivatives(t + dt, &s6);

    // 5th-order solution (y5)
    let y5 = State {
        position: state.position
            + dt * (DP_B1 * k1.velocity
                + DP_B3 * k3.velocity
                + DP_B4 * k4.velocity
                + DP_B5 * k5.velocity
                + DP_B6 * k6.velocity),
        velocity: state.velocity
            + dt * (DP_B1 * k1.acceleration
                + DP_B3 * k3.acceleration
                + DP_B4 * k4.acceleration
                + DP_B5 * k5.acceleration
                + DP_B6 * k6.acceleration),
    };

    // Stage 7 (FSAL: evaluated at y5)
    let k7 = system.derivatives(t + dt, &y5);

    // Error estimate: dt * (e1*k1 + e3*k3 + e4*k4 + e5*k5 + e6*k6 + e7*k7)
    let error_pos = dt
        * (DP_E1 * k1.velocity
            + DP_E3 * k3.velocity
            + DP_E4 * k4.velocity
            + DP_E5 * k5.velocity
            + DP_E6 * k6.velocity
            + DP_E7 * k7.velocity);
    let error_vel = dt
        * (DP_E1 * k1.acceleration
            + DP_E3 * k3.acceleration
            + DP_E4 * k4.acceleration
            + DP_E5 * k5.acceleration
            + DP_E6 * k6.acceleration
            + DP_E7 * k7.acceleration);

    (y5, error_pos, error_vel, k7)
}

impl Integrator for DormandPrince {
    fn step<S: DynamicalSystem>(&self, system: &S, t: f64, state: &State, dt: f64) -> State {
        let k1 = system.derivatives(t, state);
        let (y5, _, _, _) = dp_step_impl(system, t, state, dt, &k1);
        y5
    }
}

/// Result of [`AdaptiveStepper::advance_to`].
pub enum AdvanceOutcome<B> {
    /// Reached the target time.
    Reached,
    /// An event terminated integration early.
    Event { reason: B },
}

/// Stateful adaptive stepper that encapsulates FSAL k1/dt management.
///
/// Created via [`DormandPrince::stepper`]. Callers repeatedly call
/// [`advance_to`](AdaptiveStepper::advance_to) to advance to successive
/// target times (e.g. output interval boundaries), without needing to
/// manage k1 reuse or step-size adaptation themselves.
pub struct AdaptiveStepper<'a, S: DynamicalSystem> {
    system: &'a S,
    state: State,
    t: f64,
    dt: f64,
    k1: StateDerivative,
    tol: Tolerances,
    /// Minimum step size below which integration fails. Can be overridden
    /// after construction to match the total integration interval.
    pub dt_min: f64,
}

impl<'a, S: DynamicalSystem> AdaptiveStepper<'a, S> {
    /// Advance adaptively to `t_target`.
    ///
    /// - Each accepted step calls `callback(t, &state)`.
    /// - If `event_check` returns `Break(reason)`, returns `Event { reason }`.
    /// - On success returns `Reached` with internal state updated to `t_target`.
    pub fn advance_to<F, E, B>(
        &mut self,
        t_target: f64,
        mut callback: F,
        event_check: E,
    ) -> Result<AdvanceOutcome<B>, IntegrationError>
    where
        F: FnMut(f64, &State),
        E: Fn(f64, &State) -> ControlFlow<B>,
    {
        while self.t < t_target {
            let h = self.dt.min(t_target - self.t);

            let (y5, err_pos, err_vel, k7) =
                dp_step_impl(self.system, self.t, &self.state, h, &self.k1);

            // NaN/Inf check
            if !y5
                .position
                .iter()
                .chain(y5.velocity.iter())
                .all(|v| v.is_finite())
            {
                return Err(IntegrationError::NonFiniteState { t: self.t + h });
            }

            let err = error_norm(&self.state, &y5, &err_pos, &err_vel, &self.tol);

            if err <= 1.0 {
                // Accept step
                self.state = y5;
                self.t += h;
                self.k1 = k7; // FSAL

                callback(self.t, &self.state);

                if let ControlFlow::Break(reason) = event_check(self.t, &self.state) {
                    return Ok(AdvanceOutcome::Event { reason });
                }

                // Grow step size
                let factor = if err < 1e-15 {
                    DP_MAX_FACTOR
                } else {
                    (DP_SAFETY * err.powf(-0.2)).clamp(DP_MIN_FACTOR, DP_MAX_FACTOR)
                };
                self.dt = h * factor;
            } else {
                // Reject step, shrink
                let factor = (DP_SAFETY * err.powf(-0.2)).clamp(DP_MIN_FACTOR, 1.0);
                self.dt = h * factor;

                if self.dt < self.dt_min {
                    return Err(IntegrationError::StepSizeTooSmall {
                        t: self.t,
                        dt: self.dt,
                    });
                }
            }
        }

        Ok(AdvanceOutcome::Reached)
    }

    /// Current state.
    pub fn state(&self) -> &State {
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
    pub fn into_state(self) -> State {
        self.state
    }
}

impl DormandPrince {
    /// Create an [`AdaptiveStepper`] for the given system and initial conditions.
    pub fn stepper<'a, S: DynamicalSystem>(
        &self,
        system: &'a S,
        initial: State,
        t0: f64,
        dt: f64,
        tol: Tolerances,
    ) -> AdaptiveStepper<'a, S> {
        let k1 = system.derivatives(t0, &initial);
        let dt_min = 1e-12 * (dt * 100.0).abs().max(1.0);
        AdaptiveStepper {
            system,
            state: initial,
            t: t0,
            dt,
            k1,
            tol,
            dt_min,
        }
    }

    /// Perform a single Dormand-Prince step with full output.
    ///
    /// Returns `(y5, error_pos, error_vel, k7)` where:
    /// - `y5`: 5th-order solution (to propagate)
    /// - `error_pos`, `error_vel`: difference between 5th and 4th order solutions
    /// - `k7`: 7th-stage derivative (reusable as k1 of next step via FSAL)
    pub fn step_full<S: DynamicalSystem>(
        &self,
        system: &S,
        t: f64,
        state: &State,
        dt: f64,
    ) -> (State, Vector3<f64>, Vector3<f64>, StateDerivative) {
        let k1 = system.derivatives(t, state);
        dp_step_impl(system, t, state, dt, &k1)
    }

    /// Integrate adaptively with event detection and NaN/Inf checking.
    ///
    /// Uses the Dormand-Prince RK5(4) method with automatic step-size control.
    /// The `dt_initial` parameter is used as the initial step size guess.
    #[allow(clippy::too_many_arguments)]
    pub fn integrate_adaptive_with_events<S, F, E, B>(
        &self,
        system: &S,
        initial: State,
        t0: f64,
        t_end: f64,
        dt_initial: f64,
        tol: &Tolerances,
        callback: F,
        event_check: E,
    ) -> IntegrationOutcome<B>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &State),
        E: Fn(f64, &State) -> ControlFlow<B>,
    {
        let mut stepper = self.stepper(system, initial, t0, dt_initial.min(t_end - t0), tol.clone());
        // Override dt_min to match the original behavior based on total interval
        stepper.dt_min = 1e-12 * (t_end - t0).abs().max(1.0);

        match stepper.advance_to(t_end, callback, event_check) {
            Ok(AdvanceOutcome::Reached) => IntegrationOutcome::Completed(stepper.into_state()),
            Ok(AdvanceOutcome::Event { reason }) => {
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
    use std::ops::ControlFlow;

    use nalgebra::vector;

    use crate::test_systems::*;
    use crate::{IntegrationError, IntegrationOutcome, Integrator, State, Tolerances};

    use super::*;

    // --- Single step tests ---

    #[test]
    fn dp_step_uniform_motion_exact() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let (y5, _err_pos, _err_vel, _k7) = DormandPrince.step_full(&system, 0.0, &state, 1.0);
        let eps = 1e-12;
        assert!((y5.position.x - 1.0).abs() < eps, "y5 pos: {}", y5.position.x);
        assert!((y5.velocity.x - 1.0).abs() < eps, "y5 vel: {}", y5.velocity.x);
    }

    #[test]
    fn dp_step_constant_acceleration_exact() {
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![10.0, 20.0, 0.0],
        };
        let dt = 1.0;
        let (y5, _err_pos, _err_vel, _k7) = DormandPrince.step_full(&system, 0.0, &state, dt);

        let expected_px = 10.0;
        let expected_py = 20.0 + 0.5 * (-9.8) * 1.0;
        let expected_vy = 20.0 + (-9.8) * 1.0;

        let eps = 1e-12;
        assert!((y5.position.x - expected_px).abs() < eps);
        assert!((y5.position.y - expected_py).abs() < eps);
        assert!((y5.velocity.y - expected_vy).abs() < eps);
    }

    #[test]
    fn dp_step_error_estimate_reasonable() {
        let system = HarmonicOscillator;
        let state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let dt = 0.5;
        let (y5, err_pos, _err_vel, _k7) = DormandPrince.step_full(&system, 0.0, &state, dt);

        let analytical_x = dt.cos();
        let actual_err = (y5.position.x - analytical_x).abs();
        let estimated_err = err_pos.x.abs();

        assert!(actual_err > 0.0, "Actual error should be nonzero");
        assert!(estimated_err > 0.0, "Estimated error should be nonzero");

        let ratio = actual_err / estimated_err;
        assert!(
            ratio > 0.01 && ratio < 100.0,
            "Error estimate should be reasonable predictor: actual={actual_err:.2e}, estimated={estimated_err:.2e}, ratio={ratio:.2}"
        );
    }

    #[test]
    fn dp_step_fsal_property() {
        let system = HarmonicOscillator;
        let state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let dt = 0.1;
        let (y5, _err_pos, _err_vel, k7) = DormandPrince.step_full(&system, 0.0, &state, dt);

        let k1_next = system.derivatives(dt, &y5);

        let eps = 1e-14;
        assert!(
            (k7.velocity - k1_next.velocity).magnitude() < eps,
            "FSAL velocity mismatch: {:?} vs {:?}",
            k7.velocity,
            k1_next.velocity
        );
        assert!(
            (k7.acceleration - k1_next.acceleration).magnitude() < eps,
            "FSAL acceleration mismatch: {:?} vs {:?}",
            k7.acceleration,
            k1_next.acceleration
        );
    }

    #[test]
    fn dp_step_local_truncation_order() {
        let system = HarmonicOscillator;
        let state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };

        let dt1 = 0.1;
        let dt2 = 0.05;

        let (y5_coarse, _, _, _) = DormandPrince.step_full(&system, 0.0, &state, dt1);
        let (y5_fine, _, _, _) = DormandPrince.step_full(&system, 0.0, &state, dt2);

        let err_coarse = (y5_coarse.position.x - dt1.cos()).abs();
        let err_fine = (y5_fine.position.x - dt2.cos()).abs();

        let ratio = err_coarse / err_fine;
        assert!(
            ratio > 40.0 && ratio < 100.0,
            "Local truncation order ratio = {ratio:.2}, expected ~64 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Error norm tests ---

    #[test]
    fn error_norm_zero_for_identical_states() {
        let state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let zero = vector![0.0, 0.0, 0.0];
        let tol = Tolerances::default();
        let norm = error_norm(&state, &state, &zero, &zero, &tol);
        assert!(norm == 0.0, "Expected 0.0, got {norm}");
    }

    #[test]
    fn error_norm_scales_with_atol() {
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let err = vector![1e-8, 0.0, 0.0];
        let zero = vector![0.0, 0.0, 0.0];

        let tol1 = Tolerances { atol: 1e-8, rtol: 0.0 };
        let tol2 = Tolerances { atol: 2e-8, rtol: 0.0 };

        let norm1 = error_norm(&state, &state, &err, &zero, &tol1);
        let norm2 = error_norm(&state, &state, &err, &zero, &tol2);

        let ratio = norm1 / norm2;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Expected ratio 2.0, got {ratio:.4} (norm1={norm1:.4e}, norm2={norm2:.4e})"
        );
    }

    // --- Fixed-step integration tests ---

    #[test]
    fn dp_integrate_uniform_motion() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let final_state = DormandPrince.integrate(&system, initial, 0.0, 1.0, 0.1, |_, _| {});
        assert!((final_state.position.x - 1.0).abs() < 1e-12);
    }

    #[test]
    fn dp_integrate_harmonic_full_period() {
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.01;
        let final_state = DormandPrince.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let eps = 1e-10;
        assert!(
            (final_state.position.x - 1.0).abs() < eps,
            "After full period, x should be ~1.0, got {} (err={:.2e})",
            final_state.position.x,
            (final_state.position.x - 1.0).abs()
        );
        assert!(
            final_state.velocity.x.abs() < eps,
            "After full period, vx should be ~0.0, got {} (err={:.2e})",
            final_state.velocity.x,
            final_state.velocity.x.abs()
        );
    }

    #[test]
    fn dp_integrate_5th_order_convergence() {
        fn dp_harmonic_error(dt: f64, steps: usize) -> f64 {
            let system = HarmonicOscillator;
            let mut state = State {
                position: vector![1.0, 0.0, 0.0],
                velocity: vector![0.0, 0.0, 0.0],
            };
            let mut t = 0.0;
            for _ in 0..steps {
                let (y5, _, _, _) = DormandPrince.step_full(&system, t, &state, dt);
                state = y5;
                t += dt;
            }
            let x_error = (state.position.x - t.cos()).abs();
            let v_error = (state.velocity.x + t.sin()).abs();
            x_error.max(v_error)
        }

        let err_coarse = dp_harmonic_error(0.1, 100);
        let err_fine = dp_harmonic_error(0.05, 200);

        let ratio = err_coarse / err_fine;
        assert!(
            ratio > 20.0 && ratio < 50.0,
            "DP global convergence ratio = {ratio:.2}, expected ~32 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Adaptive integration tests ---

    #[test]
    fn dp_adaptive_completes_normally() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let tol = Tolerances::default();
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
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
                    (state.position.x - 1.0).abs() < 1e-8,
                    "Expected position ~1.0, got {}",
                    state.position.x
                );
            }
            other => panic!("Expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn dp_adaptive_harmonic_full_period() {
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let t_end = 2.0 * std::f64::consts::PI;
        let tol = Tolerances { atol: 1e-10, rtol: 1e-8 };
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
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
                    (state.position.x - 1.0).abs() < eps,
                    "After full period, x={} (err={:.2e})",
                    state.position.x,
                    (state.position.x - 1.0).abs()
                );
                assert!(
                    state.velocity.x.abs() < eps,
                    "After full period, vx={} (err={:.2e})",
                    state.velocity.x,
                    state.velocity.x.abs()
                );
            }
            other => panic!("Expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn dp_adaptive_energy_conservation() {
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let initial_energy =
            0.5 * (initial.velocity.norm_squared() + initial.position.norm_squared());
        let mut max_energy_drift: f64 = 0.0;

        let t_end = 2.0 * std::f64::consts::PI;
        let tol = Tolerances { atol: 1e-10, rtol: 1e-8 };
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            0.1,
            &tol,
            |_t, state| {
                let energy =
                    0.5 * (state.velocity.norm_squared() + state.position.norm_squared());
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
    fn dp_adaptive_lands_on_t_end() {
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let t_end = 1.234;
        let tol = Tolerances::default();
        let mut last_t = 0.0;
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
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
    fn dp_adaptive_terminates_on_event() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let tol = Tolerances::default();
        let outcome = DormandPrince.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            10.0,
            0.1,
            &tol,
            |_t, _state| {},
            |_t, state| {
                if state.position.x > 0.5 {
                    ControlFlow::Break("crossed threshold")
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        match outcome {
            IntegrationOutcome::Terminated { t, reason, .. } => {
                assert!(t < 10.0);
                assert!(t > 0.4 && t < 1.5, "Expected termination near 0.5, got t={t}");
                assert_eq!(reason, "crossed threshold");
            }
            other => panic!("Expected Terminated, got {other:?}"),
        }
    }

    #[test]
    fn dp_adaptive_detects_nan() {
        use crate::DynamicalSystem;

        struct ExplodingSystem;
        impl DynamicalSystem for ExplodingSystem {
            fn derivatives(&self, t: f64, state: &State) -> StateDerivative {
                let accel = if t > 0.3 {
                    vector![f64::INFINITY, 0.0, 0.0]
                } else {
                    vector![0.0, 0.0, 0.0]
                };
                StateDerivative {
                    velocity: state.velocity,
                    acceleration: accel,
                }
            }
        }

        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let tol = Tolerances::default();
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
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
    fn dp_adaptive_detects_step_too_small() {
        use crate::DynamicalSystem;

        struct VeryStiffSystem;
        impl DynamicalSystem for VeryStiffSystem {
            fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
                StateDerivative {
                    velocity: state.velocity,
                    acceleration: -1e20 * state.position,
                }
            }
        }

        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let tol = Tolerances { atol: 1e-12, rtol: 1e-12 };
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
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
            matches!(outcome, IntegrationOutcome::Error(IntegrationError::StepSizeTooSmall { .. })),
            "Expected StepSizeTooSmall, got {outcome:?}"
        );
    }

    #[test]
    fn dp_adaptive_fewer_steps_for_smooth() {
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let t_end = 2.0 * std::f64::consts::PI;

        let mut adaptive_steps = 0u64;
        let tol = Tolerances { atol: 1e-10, rtol: 1e-8 };
        let outcome: IntegrationOutcome<()> = DormandPrince.integrate_adaptive_with_events(
            &system,
            initial.clone(),
            0.0,
            t_end,
            1.0,
            &tol,
            |_t, _state| {
                adaptive_steps += 1;
            },
            |_t, _state| ControlFlow::Continue(()),
        );
        assert!(matches!(outcome, IntegrationOutcome::Completed(_)));

        let rk4_steps = (t_end / 0.01).ceil() as u64;

        assert!(
            adaptive_steps < rk4_steps,
            "Adaptive should use fewer steps: adaptive={adaptive_steps}, rk4={rk4_steps}"
        );
    }
}
