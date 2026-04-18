use core::ops::ControlFlow;

use nalgebra::SVector;

use crate::{DynamicalSystem, IntegrationError, IntegrationOutcome, OdeState, State};

/// Störmer-Verlet (velocity Verlet) symplectic integrator.
///
/// A 2nd-order symplectic method for separable Hamiltonian systems
/// (where acceleration depends only on position, not velocity).
/// Exactly preserves phase-space volume and has excellent long-term
/// energy conservation properties.
///
/// Only works with 2nd-order ODE states (`State<DIM, 2>`).
pub struct StormerVerlet;

impl StormerVerlet {
    /// Perform a single Störmer-Verlet (velocity Verlet) step.
    ///
    /// Kick-drift-kick form:
    /// 1. v_{1/2} = v_n + (dt/2) * a(t_n, q_n)
    /// 2. q_{n+1} = q_n + dt * v_{1/2}
    /// 3. v_{n+1} = v_{1/2} + (dt/2) * a(t_{n+1}, q_{n+1})
    pub fn step<const DIM: usize, S>(
        &self,
        system: &S,
        t: f64,
        state: &State<DIM, 2>,
        dt: f64,
    ) -> State<DIM, 2>
    where
        S: DynamicalSystem<State = State<DIM, 2>>,
    {
        // Evaluate acceleration at current state
        let deriv = system.derivatives(t, state);
        let a_n = *deriv.dy();

        // Half-kick
        let v_half: SVector<f64, DIM> = *state.dy() + (dt / 2.0) * a_n;

        // Full drift
        let q_next: SVector<f64, DIM> = *state.y() + dt * v_half;

        // Evaluate acceleration at new position
        let temp = State::<DIM, 2>::new(q_next, v_half);
        let deriv_next = system.derivatives(t + dt, &temp);
        let a_next = *deriv_next.dy();

        // Second half-kick
        let v_next: SVector<f64, DIM> = v_half + (dt / 2.0) * a_next;

        let mut result = State::<DIM, 2>::new(q_next, v_next);
        result.project(t + dt);
        result
    }

    /// Integrate from `t0` to `t_end` with fixed step size `dt`.
    pub fn integrate<const DIM: usize, S, F>(
        &self,
        system: &S,
        initial: State<DIM, 2>,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
    ) -> State<DIM, 2>
    where
        S: DynamicalSystem<State = State<DIM, 2>>,
        F: FnMut(f64, &State<DIM, 2>),
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

    /// Integrate with event detection and NaN/Inf checking.
    #[allow(clippy::too_many_arguments)]
    pub fn integrate_with_events<const DIM: usize, S, F, E, B>(
        &self,
        system: &S,
        initial: State<DIM, 2>,
        t0: f64,
        t_end: f64,
        dt: f64,
        mut callback: F,
        event_check: E,
    ) -> IntegrationOutcome<State<DIM, 2>, B>
    where
        S: DynamicalSystem<State = State<DIM, 2>>,
        F: FnMut(f64, &State<DIM, 2>),
        E: Fn(f64, &State<DIM, 2>) -> ControlFlow<B>,
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

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;

    use nalgebra::vector;

    use crate::test_systems::*;
    use crate::{IntegrationError, IntegrationOutcome, State};

    use super::*;

    // --- Basic correctness ---

    #[test]
    fn verlet_uniform_motion_exact() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let result = StormerVerlet.step(&system, 0.0, &state, 1.0);
        let eps = 1e-12;
        assert!((result.y().x - 1.0).abs() < eps, "x: {}", result.y().x);
        assert!((result.dy().x - 1.0).abs() < eps, "vx: {}", result.dy().x);
    }

    #[test]
    fn verlet_constant_acceleration_exact() {
        // Verlet is 2nd-order, so quadratic motion should be exact.
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![10.0, 20.0, 0.0]);
        let dt = 1.0;
        let result = StormerVerlet.step(&system, 0.0, &state, dt);

        let expected_py = 20.0 + 0.5 * (-9.8) * 1.0;
        let expected_vy = 20.0 + (-9.8) * 1.0;

        let eps = 1e-12;
        assert!((result.y().x - 10.0).abs() < eps);
        assert!((result.y().y - expected_py).abs() < eps);
        assert!((result.dy().y - expected_vy).abs() < eps);
    }

    // --- Order of accuracy ---

    fn harmonic_error_with_steps(dt: f64, steps: usize) -> f64 {
        let system = HarmonicOscillator;
        let mut state = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let mut t = 0.0;
        for _ in 0..steps {
            state = StormerVerlet.step(&system, t, &state, dt);
            t += dt;
        }
        let x_error = (state.y().x - t.cos()).abs();
        let v_error = (state.dy().x + t.sin()).abs();
        x_error.max(v_error)
    }

    #[test]
    fn verlet_2nd_order_accuracy() {
        let error_coarse = harmonic_error_with_steps(0.1, 100);
        let error_fine = harmonic_error_with_steps(0.05, 200);

        let ratio = error_coarse / error_fine;
        // 2nd-order method: halving dt should reduce error by ~4
        assert!(
            ratio > 3.0 && ratio < 5.0,
            "Error ratio should be ~4 for 2nd-order method, got {ratio:.2} \
             (errors: coarse={error_coarse:.2e}, fine={error_fine:.2e})"
        );
    }

    #[test]
    fn verlet_convergence() {
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
            .map(|&(dt, steps)| harmonic_error_with_steps(dt, steps))
            .collect();

        for i in 0..errors.len() - 1 {
            let ratio = errors[i] / errors[i + 1];
            assert!(
                ratio > 3.0 && ratio < 5.0,
                "Convergence ratio at dt={:.4} -> dt={:.4} should be ~4, got {ratio:.2} \
                 (errors: {:.2e} -> {:.2e})",
                dts_and_steps[i].0,
                dts_and_steps[i + 1].0,
                errors[i],
                errors[i + 1]
            );
        }
    }

    // --- Trade-offs ---
    //
    // Symplectic property holds only for separable Hamiltonians
    // (a = f(q) only). Velocity-dependent forces (drag, Lorentz)
    // break separability and the method degrades to a generic 2nd-order scheme.
    //
    // Per-step accuracy and energy drift comparisons with RK4 are in comparison.rs.

    #[test]
    fn verlet_not_symplectic_for_velocity_dependent_forces() {
        // Damped harmonic oscillator: dv/dt = -x - γv (non-separable).
        // Verlet's second derivatives call uses the intermediate velocity,
        // so the symplectic structure is broken. It still works as a plain
        // 2nd-order integrator but loses the bounded-energy guarantee.
        use crate::DynamicalSystem;

        struct DampedOscillator;
        impl DynamicalSystem for DampedOscillator {
            type State = State<1, 2>;
            fn derivatives(&self, _t: f64, state: &State<1, 2>) -> State<1, 2> {
                let x = state.y()[0];
                let v = state.dy()[0];
                State::from_derivative(
                    nalgebra::Vector1::new(v),
                    nalgebra::Vector1::new(-x - 0.01 * v),
                )
            }
        }

        let initial = State::<1, 2>::new(vector![1.0], vector![0.0]);
        let dt = 0.01;
        let t_end = 200.0 * std::f64::consts::PI; // ~628s, 100 periods

        let final_state =
            StormerVerlet.integrate(&DampedOscillator, initial, 0.0, t_end, dt, |_, _| {});

        // Analytical: amplitude ∝ e^(-γ/2 · t), γ=0.01
        // → e^(-0.005 · 628.3) ≈ 0.043
        let amplitude = (final_state.y()[0].powi(2) + final_state.dy()[0].powi(2)).sqrt();
        let expected_decay = (-0.005 * t_end).exp();

        // Qualitatively correct (amplitude decays), but splitting error from
        // non-separable force means accuracy is worse than for separable case.
        assert!(
            amplitude < 0.5,
            "Damped oscillator amplitude should decay significantly, got {amplitude:.4}"
        );
        let relative_error = (amplitude - expected_decay).abs() / expected_decay;
        assert!(
            relative_error < 0.5,
            "Non-separable splitting error: amplitude={amplitude:.4}, \
             expected={expected_decay:.4}, relative_error={relative_error:.2}"
        );
    }

    // --- Full period integration ---

    #[test]
    fn verlet_integrate_harmonic_full_period() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = StormerVerlet.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let eps = 1e-5; // 2nd-order, so less accurate than RK4 per step
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

    // --- 1D tests ---

    #[test]
    fn verlet_1d_harmonic_oscillator() {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(vector![1.0], vector![0.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = StormerVerlet.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let eps = 1e-5;
        assert!(
            (final_state.y()[0] - 1.0).abs() < eps,
            "1D SHO full period: x={} (error: {:.2e})",
            final_state.y()[0],
            (final_state.y()[0] - 1.0).abs()
        );
    }

    // --- 2D tests ---

    #[test]
    fn verlet_2d_harmonic_oscillator() {
        let system = HarmonicOscillator2D;
        let initial = State::<2, 2>::new(vector![1.0, 0.0], vector![0.0, 1.0]);
        let t_end = 2.0 * std::f64::consts::PI;
        let dt = 0.001;

        let final_state = StormerVerlet.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let eps = 1e-5;
        assert!(
            (final_state.y()[0] - 1.0).abs() < eps,
            "2D SHO x={} (error: {:.2e})",
            final_state.y()[0],
            (final_state.y()[0] - 1.0).abs()
        );
        assert!(
            final_state.y()[1].abs() < eps,
            "2D SHO y={} (error: {:.2e})",
            final_state.y()[1],
            final_state.y()[1].abs()
        );
    }

    #[test]
    fn verlet_2d_energy_conservation() {
        let system = HarmonicOscillator2D;
        let initial = State::<2, 2>::new(vector![1.0, 0.0], vector![0.0, 1.0]);
        let initial_energy = 0.5 * (initial.dy().norm_squared() + initial.y().norm_squared());
        let dt = 0.01;
        let t_end = 20.0 * std::f64::consts::PI;

        let mut max_drift: f64 = 0.0;
        StormerVerlet.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
            let energy = 0.5 * (state.dy().norm_squared() + state.y().norm_squared());
            max_drift = max_drift.max((energy - initial_energy).abs());
        });

        assert!(max_drift < 1e-4, "2D energy drift: {max_drift:.2e}");
    }

    // --- Event detection ---

    #[test]
    fn verlet_integrate_with_events_completes() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome: IntegrationOutcome<State<3, 2>, ()> = StormerVerlet.integrate_with_events(
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
            _ => panic!("Expected Completed"),
        }
    }

    #[test]
    fn verlet_integrate_with_events_terminates() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome = StormerVerlet.integrate_with_events(
            &system,
            initial,
            0.0,
            10.0,
            0.1,
            |_t, _state| {},
            |_t, state| {
                if state.y().x > 0.5 {
                    ControlFlow::Break("crossed")
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        match outcome {
            IntegrationOutcome::Terminated { t, reason, .. } => {
                assert!(t < 10.0);
                assert!(t > 0.4 && t < 0.7, "t={t}");
                assert_eq!(reason, "crossed");
            }
            _ => panic!("Expected Terminated"),
        }
    }

    #[test]
    fn verlet_detects_nan() {
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
        let outcome: IntegrationOutcome<State<3, 2>, ()> = StormerVerlet.integrate_with_events(
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
                assert!(t > 0.3, "NaN detected at t={t}");
            }
            _ => panic!("Expected NonFiniteState error"),
        }
    }

    // --- Property-based tests (proptest) ---
    //
    // These verify universal mathematical properties that must hold for ANY
    // valid initial conditions and step sizes, not just hand-picked examples.

    use nalgebra::SVector;
    use proptest::prelude::*;

    proptest! {
        /// Verlet is time-reversible: N steps forward + N steps backward = identity.
        /// This is a fundamental property of symplectic integrators that holds
        /// to machine precision (not just to truncation order).
        #[test]
        fn verlet_time_reversible(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.01f64..0.2,
            n_steps in 10u32..100,
        ) {
            let system = HarmonicOscillator1D;
            let initial = State::<1, 2>::new(SVector::from([x0]), SVector::from([v0]));

            // Forward integration: N steps with +dt
            let mut state = initial.clone();
            let mut t = 0.0;
            for _ in 0..n_steps {
                state = StormerVerlet.step(&system, t, &state, dt);
                t += dt;
            }

            // Backward integration: N steps with -dt
            for _ in 0..n_steps {
                t -= dt;
                state = StormerVerlet.step(&system, t, &state, -dt);
            }

            // Should return to initial state to near machine precision
            let x_err = (state.y()[0] - x0).abs();
            let v_err = (state.dy()[0] - v0).abs();
            let scale = x0.abs().max(v0.abs()).max(1.0);
            prop_assert!(
                x_err < 1e-10 * scale,
                "Time-reversal x error: {x_err:.2e} (x0={x0}, v0={v0}, dt={dt}, steps={n_steps})"
            );
            prop_assert!(
                v_err < 1e-10 * scale,
                "Time-reversal v error: {v_err:.2e} (x0={x0}, v0={v0}, dt={dt}, steps={n_steps})"
            );
        }

        /// For any initial condition, doubling the step count (halving dt)
        /// should yield noticeably better accuracy (Störmer-Verlet is 2nd
        /// order, so ~4×). We force integer step counts to avoid partial-step
        /// artifacts and allow generous slack for phase-error effects.
        #[test]
        fn verlet_error_decreases_with_smaller_dt(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.02f64..0.1,
        ) {
            let system = HarmonicOscillator1D;
            let t_end = 2.0 * std::f64::consts::PI;
            let expected_x = x0 * t_end.cos() + v0 * t_end.sin();

            // Force integer step counts so both runs reach t_end exactly,
            // eliminating partial-step truncation differences.
            let n_coarse = (t_end / dt).ceil() as usize;
            let dt_coarse = t_end / n_coarse as f64;
            let dt_fine = t_end / (n_coarse * 2) as f64;

            let coarse = StormerVerlet.integrate(
                &system,
                State::<1, 2>::new(SVector::from([x0]), SVector::from([v0])),
                0.0, t_end, dt_coarse, |_, _| {},
            );
            let err_coarse = (coarse.y()[0] - expected_x).abs();

            // Skip cases where the leading error coefficient is too small
            // for reliable convergence comparison (phase-error regime).
            prop_assume!(err_coarse > 1e-3);

            let fine = StormerVerlet.integrate(
                &system,
                State::<1, 2>::new(SVector::from([x0]), SVector::from([v0])),
                0.0, t_end, dt_fine, |_, _| {},
            );
            let err_fine = (fine.y()[0] - expected_x).abs();

            // SV is 2nd order → expect ~4× improvement when halving dt.
            // Allow generous slack for phase-error effects: require ≥2×.
            prop_assert!(
                err_fine < err_coarse * 0.5,
                "Expected ≥2× convergence: ratio={:.2}, coarse={err_coarse:.2e}, fine={err_fine:.2e}",
                err_coarse / err_fine
            );
        }

        /// For any initial condition and step size, Verlet's energy drift
        /// does not grow over time (2nd half max |ΔE| ≈ 1st half).
        #[test]
        fn verlet_symplectic_for_any_initial_condition(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.01f64..0.1,
        ) {
            let system = HarmonicOscillator1D;
            let initial = State::<1, 2>::new(SVector::from([x0]), SVector::from([v0]));
            let initial_energy = 0.5 * (x0 * x0 + v0 * v0);

            // Skip near-zero amplitude (energy ≈ 0, ratios are meaningless)
            prop_assume!(initial_energy > 1e-4);

            let t_end = 100.0 * 2.0 * std::f64::consts::PI; // 100 periods
            let t_mid = t_end / 2.0;

            let mut first_half = 0.0f64;
            let mut second_half = 0.0f64;

            StormerVerlet.integrate(&system, initial, 0.0, t_end, dt, |t, state| {
                let energy = 0.5 * (state.y()[0].powi(2) + state.dy()[0].powi(2));
                let drift = (energy - initial_energy).abs();
                if t < t_mid {
                    first_half = first_half.max(drift);
                } else {
                    second_half = second_half.max(drift);
                }
            });

            // Symplectic: ratio should be close to 1.0
            prop_assume!(first_half > 1e-15); // avoid division by zero
            let ratio = second_half / first_half;
            prop_assert!(
                ratio < 1.5,
                "Verlet energy drift ratio={ratio:.2} (expected ~1.0), \
                 x0={x0}, v0={v0}, dt={dt}, first={first_half:.2e}, second={second_half:.2e}"
            );
        }
    }
}
