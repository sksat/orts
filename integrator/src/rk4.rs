use crate::{DynamicalSystem, Integrator, OdeState};

/// Classic 4th-order Runge-Kutta integrator.
pub struct Rk4;

impl Integrator for Rk4 {
    /// Perform a single RK4 integration step.
    ///
    /// Classic RK4:
    /// k1 = f(t, y)
    /// k2 = f(t + dt/2, y + dt/2 * k1)
    /// k3 = f(t + dt/2, y + dt/2 * k2)
    /// k4 = f(t + dt, y + dt * k3)
    /// y_next = y + dt/6 * (k1 + 2*k2 + 2*k3 + k4)
    fn step<S: DynamicalSystem>(&self, system: &S, t: f64, state: &S::State, dt: f64) -> S::State {
        let k1 = system.derivatives(t, state);

        let s2 = state.axpy(dt / 2.0, &k1);
        let k2 = system.derivatives(t + dt / 2.0, &s2);

        let s3 = state.axpy(dt / 2.0, &k2);
        let k3 = system.derivatives(t + dt / 2.0, &s3);

        let s4 = state.axpy(dt, &k3);
        let k4 = system.derivatives(t + dt, &s4);

        // y + dt/6 * (k1 + 2*k2 + 2*k3 + k4)
        let k_sum = k1.axpy(2.0, &k2).axpy(2.0, &k3).axpy(1.0, &k4);
        let mut result = state.axpy(dt / 6.0, &k_sum);
        result.project(t + dt);
        result
    }
}

#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;

    use nalgebra::vector;

    use crate::test_systems::*;
    use crate::{IntegrationError, IntegrationOutcome, Integrator, State};

    use super::*;

    #[test]
    fn test_rk4_uniform_motion() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let result = Rk4.step(&system, 0.0, &state, 1.0);
        let eps = 1e-12;
        assert!((result.y().x - 1.0).abs() < eps, "x: {}", result.y().x);
        assert!(result.y().y.abs() < eps);
        assert!(result.y().z.abs() < eps);
        assert!((result.dy().x - 1.0).abs() < eps);
        assert!(result.dy().y.abs() < eps);
        assert!(result.dy().z.abs() < eps);
    }

    #[test]
    fn test_rk4_constant_acceleration() {
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![10.0, 20.0, 0.0]);
        let dt = 1.0;
        let result = Rk4.step(&system, 0.0, &state, dt);

        let expected_px = 10.0;
        let expected_py = 20.0 + 0.5 * (-9.8) * 1.0;
        let expected_vy = 20.0 + (-9.8) * 1.0;

        let eps = 1e-12;
        assert!(
            (result.y().x - expected_px).abs() < eps,
            "px: {}",
            result.y().x
        );
        assert!(
            (result.y().y - expected_py).abs() < eps,
            "py: {}",
            result.y().y
        );
        assert!((result.dy().x - 10.0).abs() < eps);
        assert!((result.dy().y - expected_vy).abs() < eps);
    }

    #[test]
    fn test_rk4_harmonic_oscillator() {
        let system = HarmonicOscillator;
        let mut state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);

        let dt = 0.001;
        let steps = 1000;
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }

        let expected_x = t.cos();
        let expected_vx = -t.sin();
        let eps = 1e-10;
        assert!(
            (state.y().x - expected_x).abs() < eps,
            "y().x: {} expected: {} error: {}",
            state.y().x,
            expected_x,
            (state.y().x - expected_x).abs()
        );
        assert!(
            (state.dy().x - expected_vx).abs() < eps,
            "dy().x: {} expected: {} error: {}",
            state.dy().x,
            expected_vx,
            (state.dy().x - expected_vx).abs()
        );
    }

    fn harmonic_oscillator_error_with_steps(dt: f64, steps: usize) -> f64 {
        let system = HarmonicOscillator;
        let mut state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }
        let x_error = (state.y().x - t.cos()).abs();
        let v_error = (state.dy().x + t.sin()).abs();
        x_error.max(v_error)
    }

    #[test]
    fn test_rk4_order_of_accuracy() {
        let error_coarse = harmonic_oscillator_error_with_steps(0.1, 100);
        let error_fine = harmonic_oscillator_error_with_steps(0.05, 200);

        let ratio = error_coarse / error_fine;

        assert!(
            ratio > 12.0 && ratio < 20.0,
            "Error ratio should be approximately 16 for 4th-order method, got {ratio:.2} \
             (errors: coarse={error_coarse:.2e}, fine={error_fine:.2e})"
        );
    }

    #[test]
    fn test_rk4_convergence() {
        let base_steps = 50;
        let refinements = [1, 2, 4, 8];
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

    #[test]
    fn test_rk4_integrate_harmonic_oscillator() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);

        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, _state| {});

        let eps = 1e-8;
        assert!(
            (final_state.y().x - 1.0).abs() < eps,
            "After one period, x should return to 1.0, got {} (error: {:.2e})",
            final_state.y().x,
            (final_state.y().x - 1.0).abs()
        );
        assert!(
            final_state.dy().x.abs() < eps,
            "After one period, vx should return to 0.0, got {} (error: {:.2e})",
            final_state.dy().x,
            final_state.dy().x.abs()
        );
    }

    #[test]
    fn test_rk4_energy_conservation() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);

        let initial_energy = 0.5 * (initial.dy().norm_squared() + initial.y().norm_squared());

        let mut max_energy_drift: f64 = 0.0;

        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.01;

        Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
            let energy = 0.5 * (state.dy().norm_squared() + state.y().norm_squared());
            let drift = (energy - initial_energy).abs();
            max_energy_drift = max_energy_drift.max(drift);
        });

        let threshold = 1e-8;
        assert!(
            max_energy_drift < threshold,
            "Energy drift {max_energy_drift:.2e} exceeds threshold {threshold:.2e}"
        );
    }

    #[test]
    fn integrate_with_events_completes_normally() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Rk4.integrate_with_events(
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
                assert!((state.y().x - 1.0).abs() < 1e-12);
            }
            _ => panic!("Expected Completed, got other variant"),
        }
    }

    #[test]
    fn integrate_with_events_terminates_on_event() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome = Rk4.integrate_with_events(
            &system,
            initial,
            0.0,
            10.0,
            0.1,
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
                assert!(t < 10.0, "Should terminate early, got t={t}");
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
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Rk4.integrate_with_events(
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

    // --- 1D tests ---

    #[test]
    fn test_rk4_1d_harmonic_oscillator() {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(vector![1.0], vector![0.0]);

        let dt = 0.001;
        let steps = 1000;
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }

        let expected_x = t.cos();
        let expected_vx = -t.sin();
        let eps = 1e-10;
        assert!(
            (state.y()[0] - expected_x).abs() < eps,
            "1D SHO x: {} expected: {} error: {:.2e}",
            state.y()[0],
            expected_x,
            (state.y()[0] - expected_x).abs()
        );
        assert!(
            (state.dy()[0] - expected_vx).abs() < eps,
            "1D SHO v: {} expected: {} error: {:.2e}",
            state.dy()[0],
            expected_vx,
            (state.dy()[0] - expected_vx).abs()
        );
    }

    #[test]
    fn test_rk4_1d_exponential_decay() {
        let k = 0.5;
        let system = ExponentialDecay { k };
        let y0 = 2.0;
        let mut state = State {
            components: [nalgebra::Vector1::new(y0)],
        };

        let dt = 0.001;
        let steps = 1000;
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }

        let expected = y0 * (-k * t).exp();
        let eps = 1e-10;
        assert!(
            (state.components[0][0] - expected).abs() < eps,
            "Exponential decay: {} expected: {} error: {:.2e}",
            state.components[0][0],
            expected,
            (state.components[0][0] - expected).abs()
        );
    }

    #[test]
    fn test_rk4_1d_integrate_full_period() {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(vector![1.0], vector![0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, _state| {});

        let eps = 1e-8;
        assert!(
            (final_state.y()[0] - 1.0).abs() < eps,
            "1D SHO full period: x={} (error: {:.2e})",
            final_state.y()[0],
            (final_state.y()[0] - 1.0).abs()
        );
    }

    // --- 2D tests ---

    #[test]
    fn test_rk4_2d_harmonic_oscillator() {
        let system = HarmonicOscillator2D;
        // x-component: cos(t), y-component: sin(t)
        let mut state = State::<2, 2>::new(vector![1.0, 0.0], vector![0.0, 1.0]);

        let dt = 0.001;
        let steps = 1000;
        let mut t = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }

        let eps = 1e-10;
        assert!(
            (state.y()[0] - t.cos()).abs() < eps,
            "2D SHO x: {} expected: {} error: {:.2e}",
            state.y()[0],
            t.cos(),
            (state.y()[0] - t.cos()).abs()
        );
        assert!(
            (state.y()[1] - t.sin()).abs() < eps,
            "2D SHO y: {} expected: {} error: {:.2e}",
            state.y()[1],
            t.sin(),
            (state.y()[1] - t.sin()).abs()
        );
    }

    #[test]
    fn test_rk4_lotka_volterra_invariant() {
        // Classic Lotka-Volterra with α=β=δ=γ=1 for simplicity.
        // Conserved quantity: H(x,y) = δx - γ ln(x) + βy - α ln(y)
        let alpha = 1.0;
        let beta = 1.0;
        let delta = 1.0;
        let gamma = 1.0;
        let system = LotkaVolterra {
            alpha,
            beta,
            delta,
            gamma,
        };
        let x0 = 1.5;
        let y0 = 1.0;
        let mut state = State {
            components: [nalgebra::Vector2::new(x0, y0)],
        };

        let h0 = delta * x0 - gamma * x0.ln() + beta * y0 - alpha * y0.ln();

        let dt = 0.001;
        let steps = 10000; // 10 time units
        let mut t = 0.0;
        let mut max_drift: f64 = 0.0;
        for _ in 0..steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
            let x = state.components[0][0];
            let y = state.components[0][1];
            let h = delta * x - gamma * x.ln() + beta * y - alpha * y.ln();
            max_drift = max_drift.max((h - h0).abs());
        }

        assert!(
            max_drift < 1e-6,
            "Lotka-Volterra invariant drift: {max_drift:.2e}"
        );
    }

    #[test]
    fn integrate_with_events_callback_fires_on_termination_step() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let mut callback_count = 0;
        let outcome = Rk4.integrate_with_events(
            &system,
            initial,
            0.0,
            10.0,
            1.0,
            |_t, _state| {
                callback_count += 1;
            },
            |_t, state| {
                if state.y().x > 2.5 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        assert_eq!(callback_count, 3);
        assert!(matches!(outcome, IntegrationOutcome::Terminated { .. }));
    }
}
