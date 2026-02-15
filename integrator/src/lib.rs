use std::ops::ControlFlow;

use nalgebra::Vector3;

/// State of a dynamical system with position and velocity vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub position: Vector3<f64>,
    pub velocity: Vector3<f64>,
}

/// Time derivative of a state: velocity and acceleration vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct StateDerivative {
    pub velocity: Vector3<f64>,
    pub acceleration: Vector3<f64>,
}

/// A dynamical system that can compute state derivatives at a given time.
pub trait DynamicalSystem {
    fn derivatives(&self, t: f64, state: &State) -> StateDerivative;
}

/// Classic 4th-order Runge-Kutta integrator.
pub struct Rk4;

impl Rk4 {
    /// Perform a single RK4 integration step.
    ///
    /// Classic RK4:
    /// k1 = f(t, y)
    /// k2 = f(t + dt/2, y + dt/2 * k1)
    /// k3 = f(t + dt/2, y + dt/2 * k2)
    /// k4 = f(t + dt, y + dt * k3)
    /// y_next = y + dt/6 * (k1 + 2*k2 + 2*k3 + k4)
    pub fn step<S: DynamicalSystem>(system: &S, t: f64, state: &State, dt: f64) -> State {
        let k1 = system.derivatives(t, state);

        let state2 = State {
            position: state.position + dt / 2.0 * k1.velocity,
            velocity: state.velocity + dt / 2.0 * k1.acceleration,
        };
        let k2 = system.derivatives(t + dt / 2.0, &state2);

        let state3 = State {
            position: state.position + dt / 2.0 * k2.velocity,
            velocity: state.velocity + dt / 2.0 * k2.acceleration,
        };
        let k3 = system.derivatives(t + dt / 2.0, &state3);

        let state4 = State {
            position: state.position + dt * k3.velocity,
            velocity: state.velocity + dt * k3.acceleration,
        };
        let k4 = system.derivatives(t + dt, &state4);

        State {
            position: state.position
                + dt / 6.0
                    * (k1.velocity + 2.0 * k2.velocity + 2.0 * k3.velocity + k4.velocity),
            velocity: state.velocity
                + dt / 6.0
                    * (k1.acceleration
                        + 2.0 * k2.acceleration
                        + 2.0 * k3.acceleration
                        + k4.acceleration),
        }
    }
    /// Integrate a dynamical system from t0 to t_end using fixed step size dt.
    ///
    /// Calls `callback(t, &state)` after each step, allowing the caller to
    /// record intermediate states (e.g., for energy monitoring or trajectory output).
    ///
    /// Returns the final state at t_end.
    pub fn integrate<S, F>(
        system: &S,
        initial: State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
    ) -> State
    where
        S: DynamicalSystem,
        F: FnMut(f64, &State),
    {
        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            // Adjust the last step to land exactly on t_end
            let h = dt.min(t_end - t);
            state = Self::step(system, t, &state, h);
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
    ///
    /// The existing `integrate` method is unchanged for backward compatibility.
    pub fn integrate_with_events<S, F, E, B>(
        system: &S,
        initial: State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
        event_check: E,
    ) -> IntegrationOutcome<B>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &State),
        E: Fn(f64, &State) -> ControlFlow<B>,
    {
        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            let h = dt.min(t_end - t);
            state = Self::step(system, t, &state, h);
            t += h;

            // Check for NaN/Inf
            if !state
                .position
                .iter()
                .chain(state.velocity.iter())
                .all(|v| v.is_finite())
            {
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

/// Tolerance configuration for adaptive step-size integrators.
#[derive(Debug, Clone)]
pub struct Tolerances {
    /// Absolute tolerance (applied uniformly to all state components).
    pub atol: f64,
    /// Relative tolerance (applied uniformly to all state components).
    pub rtol: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            atol: 1e-10,
            rtol: 1e-8,
        }
    }
}

/// Reason the integration was stopped by the integrator itself.
#[derive(Debug, Clone, PartialEq)]
pub enum IntegrationError {
    /// A NaN or Inf was detected in the state after a step.
    NonFiniteState { t: f64 },
    /// Step size became smaller than minimum threshold.
    StepSizeTooSmall { t: f64, dt: f64 },
}

/// Outcome of an integration with event detection.
#[derive(Debug, Clone)]
pub enum IntegrationOutcome<B> {
    /// Integration completed normally (reached t_end).
    Completed(State),
    /// Integration was terminated early by the event checker.
    Terminated { state: State, t: f64, reason: B },
    /// Integration was aborted due to a numerical error.
    Error(IntegrationError),
}

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

impl DormandPrince {
    /// Perform a single Dormand-Prince step.
    ///
    /// Returns `(y5, error_pos, error_vel, k7)` where:
    /// - `y5`: 5th-order solution (to propagate)
    /// - `error_pos`, `error_vel`: difference between 5th and 4th order solutions
    /// - `k7`: 7th-stage derivative (reusable as k1 of next step via FSAL)
    pub fn step<S: DynamicalSystem>(
        system: &S,
        t: f64,
        state: &State,
        dt: f64,
    ) -> (State, Vector3<f64>, Vector3<f64>, StateDerivative) {
        let k1 = system.derivatives(t, state);
        Self::step_with_k1(system, t, state, dt, &k1)
    }

    /// Perform a single Dormand-Prince step with pre-computed k1 (FSAL).
    pub fn step_with_k1<S: DynamicalSystem>(
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

    /// Integrate a dynamical system from t0 to t_end using fixed step size dt.
    ///
    /// Uses the 5th-order Dormand-Prince solution (more accurate than RK4).
    pub fn integrate<S, F>(
        system: &S,
        initial: State,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
    ) -> State
    where
        S: DynamicalSystem,
        F: FnMut(f64, &State),
    {
        let mut state = initial;
        let mut t = t0;

        while t < t_end {
            let h = dt.min(t_end - t);
            let (y5, _err_pos, _err_vel, _k7) = Self::step(system, t, &state, h);
            state = y5;
            t += h;
            callback(t, &state);
        }

        state
    }

    /// Integrate adaptively with event detection and NaN/Inf checking.
    ///
    /// Uses the Dormand-Prince RK5(4) method with automatic step-size control.
    /// The `dt_initial` parameter is used as the initial step size guess.
    #[allow(clippy::too_many_arguments)]
    pub fn integrate_adaptive_with_events<S, F, E, B>(
        system: &S,
        initial: State,
        t0: f64,
        t_end: f64,
        dt_initial: f64,
        tol: &Tolerances,
        mut callback: F,
        event_check: E,
    ) -> IntegrationOutcome<B>
    where
        S: DynamicalSystem,
        F: FnMut(f64, &State),
        E: Fn(f64, &State) -> ControlFlow<B>,
    {
        let mut state = initial;
        let mut t = t0;
        let mut dt = dt_initial.min(t_end - t0);
        let dt_min = 1e-12 * (t_end - t0).abs().max(1.0);

        // Compute k1 for the first step
        let mut k1 = system.derivatives(t, &state);

        while t < t_end {
            // Clamp dt to not overshoot t_end
            let h = dt.min(t_end - t);

            let (y5, err_pos, err_vel, k7) = Self::step_with_k1(system, t, &state, h, &k1);

            // Check for NaN/Inf in the new state
            if !y5
                .position
                .iter()
                .chain(y5.velocity.iter())
                .all(|v| v.is_finite())
            {
                return IntegrationOutcome::Error(IntegrationError::NonFiniteState { t: t + h });
            }

            // Compute error norm
            let err = error_norm(&state, &y5, &err_pos, &err_vel, tol);

            if err <= 1.0 {
                // Accept step
                state = y5;
                t += h;
                k1 = k7; // FSAL: reuse k7 as k1 for next step

                callback(t, &state);

                if let ControlFlow::Break(reason) = event_check(t, &state) {
                    return IntegrationOutcome::Terminated { state, t, reason };
                }

                // Compute new step size
                let factor = if err < 1e-15 {
                    DP_MAX_FACTOR
                } else {
                    (DP_SAFETY * err.powf(-0.2)).clamp(DP_MIN_FACTOR, DP_MAX_FACTOR)
                };
                dt = h * factor;
            } else {
                // Reject step, reduce dt
                let factor = (DP_SAFETY * err.powf(-0.2)).clamp(DP_MIN_FACTOR, 1.0);
                dt = h * factor;

                if dt < dt_min {
                    return IntegrationOutcome::Error(IntegrationError::StepSizeTooSmall {
                        t,
                        dt,
                    });
                }

                // Re-evaluate k1 since state hasn't changed but dt will differ
                // Actually k1 is still valid since it's f(t, state) which hasn't changed
            }
        }

        IntegrationOutcome::Completed(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    #[test]
    fn state_clone_and_debug() {
        let state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let cloned = state.clone();
        assert_eq!(state, cloned);
        // Debug should not panic
        let _debug = format!("{:?}", state);
    }

    #[test]
    fn state_derivative_clone_and_debug() {
        let deriv = StateDerivative {
            velocity: vector![1.0, 0.0, 0.0],
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let cloned = deriv.clone();
        assert_eq!(deriv, cloned);
        let _debug = format!("{:?}", deriv);
    }

    struct UniformMotion {
        constant_velocity: Vector3<f64>,
    }

    impl DynamicalSystem for UniformMotion {
        fn derivatives(&self, _t: f64, _state: &State) -> StateDerivative {
            StateDerivative {
                velocity: self.constant_velocity,
                acceleration: vector![0.0, 0.0, 0.0],
            }
        }
    }

    #[test]
    fn dynamical_system_trait_usage() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);
        assert_eq!(deriv.velocity, vector![1.0, 0.0, 0.0]);
        assert_eq!(deriv.acceleration, vector![0.0, 0.0, 0.0]);
    }

    // --- RK4 test helper systems ---

    /// Constant acceleration system: dv/dt = constant acceleration
    struct ConstantAcceleration {
        acceleration: Vector3<f64>,
    }

    impl DynamicalSystem for ConstantAcceleration {
        fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
            StateDerivative {
                velocity: state.velocity,
                acceleration: self.acceleration,
            }
        }
    }

    /// Simple harmonic oscillator: dv/dt = -x (ω = 1)
    struct HarmonicOscillator;

    impl DynamicalSystem for HarmonicOscillator {
        fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
            StateDerivative {
                velocity: state.velocity,
                acceleration: -state.position,
            }
        }
    }

    // --- RK4 step tests ---

    #[test]
    fn test_rk4_uniform_motion() {
        // dx/dt = v, dv/dt = 0
        // With v = (1,0,0), after t=1.0 position should be (1,0,0)
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let result = Rk4::step(&system, 0.0, &state, 1.0);
        let eps = 1e-12;
        assert!((result.position.x - 1.0).abs() < eps, "x: {}", result.position.x);
        assert!(result.position.y.abs() < eps);
        assert!(result.position.z.abs() < eps);
        assert!((result.velocity.x - 1.0).abs() < eps);
        assert!(result.velocity.y.abs() < eps);
        assert!(result.velocity.z.abs() < eps);
    }

    #[test]
    fn test_rk4_constant_acceleration() {
        // dv/dt = (0, -9.8, 0)
        // After t=1.0: v = v0 + a*t, x = x0 + v0*t + 0.5*a*t^2
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![10.0, 20.0, 0.0],
        };
        let dt = 1.0;
        let result = Rk4::step(&system, 0.0, &state, dt);

        // Analytical solution:
        // position = x0 + v0*t + 0.5*a*t^2
        let expected_px = 10.0;
        let expected_py = 20.0 + 0.5 * (-9.8) * 1.0;
        // velocity = v0 + a*t
        let expected_vy = 20.0 + (-9.8) * 1.0;

        let eps = 1e-12;
        assert!((result.position.x - expected_px).abs() < eps, "px: {}", result.position.x);
        assert!((result.position.y - expected_py).abs() < eps, "py: {}", result.position.y);
        assert!((result.velocity.x - 10.0).abs() < eps);
        assert!((result.velocity.y - expected_vy).abs() < eps);
    }

    #[test]
    fn test_rk4_harmonic_oscillator() {
        // dv/dt = -x (ω=1)
        // Analytical: x(t) = cos(t), v(t) = -sin(t) with x(0)=1, v(0)=0
        let system = HarmonicOscillator;
        let mut state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };

        let dt = 0.001;
        let steps = 1000; // integrate to t=1.0
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4::step(&system, t, &state, dt);
            t += dt;
        }

        let expected_x = t.cos();
        let expected_vx = -t.sin();
        let eps = 1e-10;
        assert!(
            (state.position.x - expected_x).abs() < eps,
            "position.x: {} expected: {} error: {}",
            state.position.x,
            expected_x,
            (state.position.x - expected_x).abs()
        );
        assert!(
            (state.velocity.x - expected_vx).abs() < eps,
            "velocity.x: {} expected: {} error: {}",
            state.velocity.x,
            expected_vx,
            (state.velocity.x - expected_vx).abs()
        );
    }

    // --- dt precision / convergence tests ---

    /// Helper: integrate harmonic oscillator for a given number of steps and return position error.
    /// Uses a fixed number of steps so that the total integration time is exactly steps * dt.
    /// We compare against the analytical solution at the actual end time.
    fn harmonic_oscillator_error_with_steps(dt: f64, steps: usize) -> f64 {
        let system = HarmonicOscillator;
        let mut state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4::step(&system, t, &state, dt);
            t += dt;
        }
        // Compare against analytical solution at t: x(t) = cos(t), v(t) = -sin(t)
        let x_error = (state.position.x - t.cos()).abs();
        let v_error = (state.velocity.x + t.sin()).abs();
        x_error.max(v_error)
    }

    #[test]
    fn test_rk4_order_of_accuracy() {
        // RK4 is 4th order: halving dt should reduce the global error by ~2^4 = 16.
        // Integrate to the same end time with dt and dt/2 (doubling steps).
        // Use 100 steps at dt=0.1 (t_end=10.0) and 200 steps at dt=0.05 (t_end=10.0).
        let error_coarse = harmonic_oscillator_error_with_steps(0.1, 100);
        let error_fine = harmonic_oscillator_error_with_steps(0.05, 200);

        let ratio = error_coarse / error_fine;

        // For a 4th order method, expected ratio ~16.
        // Allow some tolerance due to finite precision and higher-order terms.
        assert!(
            ratio > 12.0 && ratio < 20.0,
            "Error ratio should be approximately 16 for 4th-order method, got {ratio:.2} \
             (errors: coarse={error_coarse:.2e}, fine={error_fine:.2e})"
        );
    }

    #[test]
    fn test_rk4_convergence() {
        // Verify error decreases as O(dt^4) across multiple dt values.
        // All integrate to the same end time t=10.0.
        let base_steps = 50;
        let refinements = [1, 2, 4, 8]; // multipliers for step count
        let dts_and_steps: Vec<(f64, usize)> = refinements
            .iter()
            .map(|&m| {
                let steps = base_steps * m;
                let dt = 10.0 / steps as f64;
                (dt, steps)
            })
            .collect();

        let errors: Vec<f64> = dts_and_steps
            .iter()
            .map(|&(dt, steps)| harmonic_oscillator_error_with_steps(dt, steps))
            .collect();

        // Check that each successive halving of dt gives ~16x error reduction
        for i in 0..errors.len() - 1 {
            let ratio = errors[i] / errors[i + 1];
            assert!(
                ratio > 12.0 && ratio < 20.0,
                "Convergence ratio at dt={:.4} -> dt={:.4} should be ~16, got {ratio:.2} \
                 (errors: {:.2e} -> {:.2e})",
                dts_and_steps[i].0,
                dts_and_steps[i + 1].0,
                errors[i],
                errors[i + 1]
            );
        }

        // Also verify the error actually decreases monotonically
        for i in 0..errors.len() - 1 {
            assert!(
                errors[i] > errors[i + 1],
                "Error should decrease with smaller dt: error[dt={:.4}]={:.2e} > error[dt={:.4}]={:.2e}",
                dts_and_steps[i].0,
                errors[i],
                dts_and_steps[i + 1].0,
                errors[i + 1]
            );
        }
    }

    // --- Multi-step integration tests ---

    #[test]
    fn test_rk4_integrate_harmonic_oscillator() {
        // Integrate harmonic oscillator for one full period (2*pi).
        // Should return close to the initial state.
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };

        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = Rk4::integrate(&system, initial, 0.0, t_end, dt, |_t, _state| {});

        let eps = 1e-8;
        assert!(
            (final_state.position.x - 1.0).abs() < eps,
            "After one period, x should return to 1.0, got {} (error: {:.2e})",
            final_state.position.x,
            (final_state.position.x - 1.0).abs()
        );
        assert!(
            final_state.velocity.x.abs() < eps,
            "After one period, vx should return to 0.0, got {} (error: {:.2e})",
            final_state.velocity.x,
            final_state.velocity.x.abs()
        );
    }

    #[test]
    fn test_rk4_energy_conservation() {
        // For harmonic oscillator, total energy E = 0.5*(|v|^2 + |x|^2) is conserved.
        // Track energy at each step and verify max drift is below threshold.
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };

        let initial_energy =
            0.5 * (initial.velocity.norm_squared() + initial.position.norm_squared());

        let mut max_energy_drift: f64 = 0.0;

        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.01;

        Rk4::integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
            let energy = 0.5 * (state.velocity.norm_squared() + state.position.norm_squared());
            let drift = (energy - initial_energy).abs();
            max_energy_drift = max_energy_drift.max(drift);
        });

        // With dt=0.01, energy drift should be very small for RK4
        let threshold = 1e-8;
        assert!(
            max_energy_drift < threshold,
            "Energy drift {max_energy_drift:.2e} exceeds threshold {threshold:.2e}"
        );
    }

    // --- integrate_with_events tests ---

    #[test]
    fn integrate_with_events_completes_normally() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let outcome: IntegrationOutcome<()> = Rk4::integrate_with_events(
            &system,
            initial,
            0.0,
            1.0,
            0.1,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        match outcome {
            IntegrationOutcome::Completed(state) => {
                assert!((state.position.x - 1.0).abs() < 1e-12);
            }
            _ => panic!("Expected Completed, got other variant"),
        }
    }

    #[test]
    fn integrate_with_events_terminates_on_event() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let outcome = Rk4::integrate_with_events(
            &system,
            initial,
            0.0,
            10.0,
            0.1,
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
                assert!(t < 10.0, "Should terminate early, got t={t}");
                // Position crosses 0.5 at t≈0.5; with dt=0.1, detected at t=0.6
                assert!(
                    t > 0.4 && t < 0.7,
                    "Should terminate around t=0.5-0.6, got t={t}"
                );
                assert_eq!(reason, "crossed threshold");
            }
            _ => panic!("Expected Terminated"),
        }
    }

    #[test]
    fn integrate_with_events_detects_nan() {
        // System that produces NaN after a few steps by returning Inf acceleration
        struct ExplodingSystem;
        impl DynamicalSystem for ExplodingSystem {
            fn derivatives(&self, t: f64, state: &State) -> StateDerivative {
                // After t > 0.3, return Inf to simulate numerical blow-up
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
        let outcome: IntegrationOutcome<()> = Rk4::integrate_with_events(
            &ExplodingSystem,
            initial,
            0.0,
            10.0,
            0.1,
            |_t, _state| {},
            |_t, _state| ControlFlow::Continue(()),
        );
        match outcome {
            IntegrationOutcome::Error(IntegrationError::NonFiniteState { t }) => {
                assert!(t > 0.3, "NaN should be detected after blow-up, got t={t}");
            }
            _ => panic!("Expected NonFiniteState error"),
        }
    }

    #[test]
    fn integrate_with_events_callback_fires_on_termination_step() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let mut callback_count = 0;
        let outcome = Rk4::integrate_with_events(
            &system,
            initial,
            0.0,
            10.0,
            1.0,
            |_t, _state| {
                callback_count += 1;
            },
            |_t, state| {
                if state.position.x > 2.5 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        // Callback fires for t=1, t=2, t=3 (terminated at t=3)
        assert_eq!(callback_count, 3);
        assert!(matches!(outcome, IntegrationOutcome::Terminated { .. }));
    }

    // ===================================================================
    // Dormand-Prince tests
    // ===================================================================

    // --- Phase 1: Single step tests ---

    #[test]
    fn dp_step_uniform_motion_exact() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let (y5, _err_pos, _err_vel, _k7) = DormandPrince::step(&system, 0.0, &state, 1.0);
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
        let (y5, _err_pos, _err_vel, _k7) = DormandPrince::step(&system, 0.0, &state, dt);

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
        // The embedded error estimate should be a reasonable predictor:
        // for a single step of the harmonic oscillator, the error estimate
        // magnitude should be within an order of magnitude of the actual y5 error.
        let system = HarmonicOscillator;
        let state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let dt = 0.5;
        let (y5, err_pos, _err_vel, _k7) = DormandPrince::step(&system, 0.0, &state, dt);

        let analytical_x = dt.cos();
        let actual_err = (y5.position.x - analytical_x).abs();
        let estimated_err = err_pos.x.abs();

        // Both should be on similar order of magnitude
        assert!(actual_err > 0.0, "Actual error should be nonzero");
        assert!(estimated_err > 0.0, "Estimated error should be nonzero");

        let ratio = actual_err / estimated_err;
        // Ratio should be reasonable (within 2 orders of magnitude)
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
        let (y5, _err_pos, _err_vel, k7) = DormandPrince::step(&system, 0.0, &state, dt);

        // k7 should equal derivatives(t+dt, y5)
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
        // For 5th-order method, local truncation error is O(dt^6).
        // Halving dt should reduce single-step error by ~2^6 = 64.
        let system = HarmonicOscillator;
        let state = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };

        let dt1 = 0.1;
        let dt2 = 0.05;

        let (y5_coarse, _, _, _) = DormandPrince::step(&system, 0.0, &state, dt1);
        let (y5_fine, _, _, _) = DormandPrince::step(&system, 0.0, &state, dt2);

        let err_coarse = (y5_coarse.position.x - dt1.cos()).abs();
        let err_fine = (y5_fine.position.x - dt2.cos()).abs();

        let ratio = err_coarse / err_fine;
        // Expected ~64 for O(dt^6) local truncation
        assert!(
            ratio > 40.0 && ratio < 100.0,
            "Local truncation order ratio = {ratio:.2}, expected ~64 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Phase 2: Error norm tests ---

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

        // Doubling atol should halve the norm
        let ratio = norm1 / norm2;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Expected ratio 2.0, got {ratio:.4} (norm1={norm1:.4e}, norm2={norm2:.4e})"
        );
    }

    // --- Phase 3: Fixed-step integration tests ---

    #[test]
    fn dp_integrate_uniform_motion() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let final_state = DormandPrince::integrate(&system, initial, 0.0, 1.0, 0.1, |_, _| {});
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
        let final_state = DormandPrince::integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        // 5th order should be very accurate with dt=0.01
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
        // Global convergence should be 5th order: halving dt → 2^5 = 32x error reduction.
        fn dp_harmonic_error(dt: f64, steps: usize) -> f64 {
            let system = HarmonicOscillator;
            let mut state = State {
                position: vector![1.0, 0.0, 0.0],
                velocity: vector![0.0, 0.0, 0.0],
            };
            let mut t = 0.0;
            for _ in 0..steps {
                let (y5, _, _, _) = DormandPrince::step(&system, t, &state, dt);
                state = y5;
                t += dt;
            }
            let x_error = (state.position.x - t.cos()).abs();
            let v_error = (state.velocity.x + t.sin()).abs();
            x_error.max(v_error)
        }

        // 100 steps of dt=0.1 (t=10), 200 steps of dt=0.05 (t=10)
        let err_coarse = dp_harmonic_error(0.1, 100);
        let err_fine = dp_harmonic_error(0.05, 200);

        let ratio = err_coarse / err_fine;
        // For 5th-order global: expected ~32
        assert!(
            ratio > 20.0 && ratio < 50.0,
            "DP global convergence ratio = {ratio:.2}, expected ~32 (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
        );
    }

    // --- Phase 4: Adaptive integration tests ---

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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        // Adaptive stepping trades exact energy conservation for accuracy control.
        // The drift should be small but may exceed fixed-step RK4's symplectic-like behavior.
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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        let outcome = DormandPrince::integrate_adaptive_with_events(
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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        // System with extremely stiff dynamics that forces step size below minimum.
        // omega^2 = 1e20, so omega = 1e10, period ≈ 6.28e-10 s.
        // Required dt for tight tolerances is ~1e-13, below dt_min ≈ 1e-11.
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
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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
        // For a smooth problem, adaptive should use fewer derivative evaluations
        // than fixed-step for comparable accuracy.
        let system = HarmonicOscillator;
        let initial = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let t_end = 2.0 * std::f64::consts::PI;

        // Count adaptive steps
        let mut adaptive_steps = 0u64;
        let tol = Tolerances { atol: 1e-10, rtol: 1e-8 };
        let outcome: IntegrationOutcome<()> = DormandPrince::integrate_adaptive_with_events(
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

        // Compare with fixed-step RK4 at dt=0.01 (628 steps for comparable accuracy)
        let rk4_steps = (t_end / 0.01).ceil() as u64;

        assert!(
            adaptive_steps < rk4_steps,
            "Adaptive should use fewer steps: adaptive={adaptive_steps}, rk4={rk4_steps}"
        );
    }
}
