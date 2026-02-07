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
}
