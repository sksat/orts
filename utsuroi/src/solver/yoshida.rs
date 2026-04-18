// ---------------------------------------------------------------------------
// Yoshida symplectic integrators (Yoshida, 1990)
//
// Higher-order symplectic methods constructed by composing Störmer-Verlet
// steps with specific weight sequences. All methods preserve phase-space
// volume and have excellent long-term energy conservation.
//
// Reference: H. Yoshida, "Construction of higher order symplectic
// integrators", Physics Letters A 150(5-7), 262-268, 1990.
// ---------------------------------------------------------------------------

use core::ops::ControlFlow;

use crate::{DynamicalSystem, IntegrationError, IntegrationOutcome, OdeState, State};

use super::verlet::StormerVerlet;

// --- 4th order (3 substeps) ---
// Triple-jump: w1 = 1/(2 - 2^{1/3}), w0 = 1 - 2*w1

const Y4_W1: f64 = 1.3512071919596576;
const Y4_W0: f64 = -1.7024143839193153;
const Y4_WEIGHTS: [f64; 3] = [Y4_W1, Y4_W0, Y4_W1];

// --- 6th order (7 substeps) ---
// Yoshida (1990), Table 2, "Solution A"

const Y6_W1: f64 = -1.17767998417887;
const Y6_W2: f64 = 0.235573213359357;
const Y6_W3: f64 = 0.784513610477560;
const Y6_W0: f64 = 1.315186320683906;
const Y6_WEIGHTS: [f64; 7] = [Y6_W3, Y6_W2, Y6_W1, Y6_W0, Y6_W1, Y6_W2, Y6_W3];

// --- 8th order (15 substeps) ---
// Yoshida (1990), Table 3, "Solution D"

const Y8_W1: f64 = 0.311790812418427;
const Y8_W2: f64 = -1.55946803821447;
const Y8_W3: f64 = -1.67896928259640;
const Y8_W4: f64 = 1.66335809963315;
const Y8_W5: f64 = -1.06458714789183;
const Y8_W6: f64 = 1.36934946416871;
const Y8_W7: f64 = 0.629030650210433;
const Y8_W0: f64 = 1.65899088454396;
#[rustfmt::skip]
const Y8_WEIGHTS: [f64; 15] = [
    Y8_W7, Y8_W6, Y8_W5, Y8_W4, Y8_W3, Y8_W2, Y8_W1,
    Y8_W0,
    Y8_W1, Y8_W2, Y8_W3, Y8_W4, Y8_W5, Y8_W6, Y8_W7,
];

/// Compose a sequence of Störmer-Verlet substeps with given weights.
fn yoshida_step<const DIM: usize, S>(
    weights: &[f64],
    system: &S,
    t: f64,
    state: &State<DIM, 2>,
    dt: f64,
) -> State<DIM, 2>
where
    S: DynamicalSystem<State = State<DIM, 2>>,
{
    let mut current = state.clone();
    let mut t_current = t;
    for &w in weights {
        let sub_dt = w * dt;
        current = StormerVerlet.step(system, t_current, &current, sub_dt);
        t_current += sub_dt;
    }
    current
}

/// Yoshida 4th-order symplectic integrator.
///
/// Composes 3 Störmer-Verlet substeps using the triple-jump technique.
/// 4th-order accuracy with symplectic structure preservation.
/// Cost: 6 force evaluations per step (3 Verlet substeps × 2).
pub struct Yoshida4;

/// Yoshida 6th-order symplectic integrator.
///
/// Composes 7 Störmer-Verlet substeps.
/// 6th-order accuracy with symplectic structure preservation.
/// Cost: 14 force evaluations per step.
pub struct Yoshida6;

/// Yoshida 8th-order symplectic integrator.
///
/// Composes 15 Störmer-Verlet substeps.
/// 8th-order accuracy with symplectic structure preservation.
/// Cost: 30 force evaluations per step.
pub struct Yoshida8;

macro_rules! impl_yoshida {
    ($name:ident, $weights:expr) => {
        impl $name {
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
                yoshida_step(&$weights, system, t, state, dt)
            }

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
    };
}

impl_yoshida!(Yoshida4, Y4_WEIGHTS);
impl_yoshida!(Yoshida6, Y6_WEIGHTS);
impl_yoshida!(Yoshida8, Y8_WEIGHTS);

#[cfg(test)]
mod tests {
    use nalgebra::{SVector, vector};

    use crate::State;
    use crate::test_systems::*;

    use super::*;

    // --- Basic correctness ---

    #[test]
    fn yoshida4_uniform_motion_exact() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let result = Yoshida4.step(&system, 0.0, &state, 1.0);
        assert!((result.y().x - 1.0).abs() < 1e-12);
        assert!((result.dy().x - 1.0).abs() < 1e-12);
    }

    #[test]
    fn yoshida4_constant_acceleration_exact() {
        let system = ConstantAcceleration {
            acceleration: vector![0.0, -9.8, 0.0],
        };
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![10.0, 20.0, 0.0]);
        let result = Yoshida4.step(&system, 0.0, &state, 1.0);

        let expected_py = 20.0 + 0.5 * (-9.8);
        let expected_vy = 20.0 + (-9.8);

        assert!((result.y().x - 10.0).abs() < 1e-12);
        assert!((result.y().y - expected_py).abs() < 1e-12);
        assert!((result.dy().y - expected_vy).abs() < 1e-12);
    }

    // --- Order of accuracy ---

    fn harmonic_error(
        integrator_fn: impl Fn(f64, usize) -> State<1, 2>,
        dt: f64,
        steps: usize,
    ) -> f64 {
        let final_state = integrator_fn(dt, steps);
        let t = dt * steps as f64;
        let x_error = (final_state.y()[0] - t.cos()).abs();
        let v_error = (final_state.dy()[0] + t.sin()).abs();
        x_error.max(v_error)
    }

    fn run_yoshida4(dt: f64, steps: usize) -> State<1, 2> {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let mut t = 0.0;
        for _ in 0..steps {
            state = Yoshida4.step(&system, t, &state, dt);
            t += dt;
        }
        state
    }

    fn run_yoshida6(dt: f64, steps: usize) -> State<1, 2> {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let mut t = 0.0;
        for _ in 0..steps {
            state = Yoshida6.step(&system, t, &state, dt);
            t += dt;
        }
        state
    }

    fn run_yoshida8(dt: f64, steps: usize) -> State<1, 2> {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let mut t = 0.0;
        for _ in 0..steps {
            state = Yoshida8.step(&system, t, &state, dt);
            t += dt;
        }
        state
    }

    #[test]
    fn yoshida4_4th_order_accuracy() {
        let error_coarse = harmonic_error(run_yoshida4, 0.1, 100);
        let error_fine = harmonic_error(run_yoshida4, 0.05, 200);

        let ratio = error_coarse / error_fine;
        // 4th-order: halving dt should reduce error by ~16
        assert!(
            ratio > 12.0 && ratio < 20.0,
            "Error ratio should be ~16 for 4th-order, got {ratio:.2} \
             (errors: {error_coarse:.2e}, {error_fine:.2e})"
        );
    }

    #[test]
    fn yoshida6_6th_order_accuracy() {
        let error_coarse = harmonic_error(run_yoshida6, 0.1, 100);
        let error_fine = harmonic_error(run_yoshida6, 0.05, 200);

        let ratio = error_coarse / error_fine;
        // 6th-order: halving dt should reduce error by ~64
        assert!(
            ratio > 48.0 && ratio < 80.0,
            "Error ratio should be ~64 for 6th-order, got {ratio:.2} \
             (errors: {error_coarse:.2e}, {error_fine:.2e})"
        );
    }

    #[test]
    fn yoshida8_8th_order_accuracy() {
        let error_coarse = harmonic_error(run_yoshida8, 0.1, 100);
        let error_fine = harmonic_error(run_yoshida8, 0.05, 200);

        let ratio = error_coarse / error_fine;
        // 8th-order: halving dt should reduce error by ~256
        assert!(
            ratio > 180.0 && ratio < 350.0,
            "Error ratio should be ~256 for 8th-order, got {ratio:.2} \
             (errors: {error_coarse:.2e}, {error_fine:.2e})"
        );
    }

    // --- Symplecticity: bounded energy drift ---

    fn energy_drift<F>(integrator: F, dt: f64, t_end: f64) -> (f64, f64)
    where
        F: Fn(&HarmonicOscillator1D, f64, &State<1, 2>, f64) -> State<1, 2>,
    {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let initial_energy = 0.5;
        let t_mid = t_end / 2.0;

        let mut first_half: f64 = 0.0;
        let mut second_half: f64 = 0.0;

        let mut t = 0.0;
        while t < t_end {
            let h = dt.min(t_end - t);
            state = integrator(&system, t, &state, h);
            t += h;
            let energy = 0.5 * (state.y()[0].powi(2) + state.dy()[0].powi(2));
            let drift = (energy - initial_energy).abs();
            if t < t_mid {
                first_half = first_half.max(drift);
            } else {
                second_half = second_half.max(drift);
            }
        }

        (first_half, second_half)
    }

    #[test]
    fn yoshida4_no_secular_energy_drift() {
        let dt = 0.05;
        let t_end = 1000.0 * std::f64::consts::TAU;
        let (first, second) = energy_drift(|s, t, st, dt| Yoshida4.step(s, t, st, dt), dt, t_end);

        assert!(first > 0.0, "Should have some energy oscillation");
        let ratio = second / first;
        assert!(
            ratio < 1.5,
            "Yoshida4 energy drift ratio={ratio:.2} (first={first:.2e}, second={second:.2e})"
        );
    }

    #[test]
    fn yoshida6_no_secular_energy_drift() {
        let dt = 0.1;
        let t_end = 1000.0 * std::f64::consts::TAU;
        let (first, second) = energy_drift(|s, t, st, dt| Yoshida6.step(s, t, st, dt), dt, t_end);

        assert!(first > 0.0);
        let ratio = second / first;
        assert!(
            ratio < 1.5,
            "Yoshida6 energy drift ratio={ratio:.2} (first={first:.2e}, second={second:.2e})"
        );
    }

    #[test]
    fn yoshida8_no_secular_energy_drift() {
        let dt = 0.1;
        let t_end = 1000.0 * std::f64::consts::TAU;
        let (first, second) = energy_drift(|s, t, st, dt| Yoshida8.step(s, t, st, dt), dt, t_end);

        assert!(first > 0.0);
        let ratio = second / first;
        assert!(
            ratio < 1.5,
            "Yoshida8 energy drift ratio={ratio:.2} (first={first:.2e}, second={second:.2e})"
        );
    }

    // --- Accuracy comparison ---

    #[test]
    fn yoshida4_more_accurate_than_verlet() {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let dt = 0.01;
        let t_end = std::f64::consts::TAU;

        let verlet_final =
            StormerVerlet.integrate(&system, initial.clone(), 0.0, t_end, dt, |_, _| {});
        let yoshida4_final = Yoshida4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let verlet_err = (verlet_final.y()[0] - 1.0).abs();
        let yoshida4_err = (yoshida4_final.y()[0] - 1.0).abs();

        assert!(
            yoshida4_err < verlet_err,
            "Yoshida4 ({yoshida4_err:.2e}) should be more accurate than Verlet ({verlet_err:.2e})"
        );
    }

    #[test]
    fn yoshida6_more_accurate_than_yoshida4() {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let dt = 0.05;
        let t_end = std::f64::consts::TAU;

        let y4_final = Yoshida4.integrate(&system, initial.clone(), 0.0, t_end, dt, |_, _| {});
        let y6_final = Yoshida6.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

        let y4_err = (y4_final.y()[0] - 1.0).abs();
        let y6_err = (y6_final.y()[0] - 1.0).abs();

        assert!(
            y6_err < y4_err,
            "Yoshida6 ({y6_err:.2e}) should be more accurate than Yoshida4 ({y4_err:.2e})"
        );
    }

    // --- Time-reversibility ---

    use proptest::prelude::*;

    /// Helper: forward N steps then backward N steps, return (x_err, v_err).
    fn time_reversal_error(
        step_fn: impl Fn(&HarmonicOscillator1D, f64, &State<1, 2>, f64) -> State<1, 2>,
        x0: f64,
        v0: f64,
        dt: f64,
        n_steps: u32,
    ) -> (f64, f64) {
        let system = HarmonicOscillator1D;
        let mut state = State::<1, 2>::new(SVector::from([x0]), SVector::from([v0]));
        let mut t = 0.0;
        for _ in 0..n_steps {
            state = step_fn(&system, t, &state, dt);
            t += dt;
        }
        for _ in 0..n_steps {
            t -= dt;
            state = step_fn(&system, t, &state, -dt);
        }
        ((state.y()[0] - x0).abs(), (state.dy()[0] - v0).abs())
    }

    proptest! {
        #[test]
        fn yoshida4_time_reversible(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.01f64..0.2,
            n_steps in 10u32..50,
        ) {
            let (x_err, v_err) = time_reversal_error(
                |s, t, st, h| Yoshida4.step(s, t, st, h), x0, v0, dt, n_steps,
            );
            let scale = x0.abs().max(v0.abs()).max(1.0);
            prop_assert!(x_err < 1e-10 * scale, "Y4 x error: {x_err:.2e}");
            prop_assert!(v_err < 1e-10 * scale, "Y4 v error: {v_err:.2e}");
        }

        #[test]
        fn yoshida6_time_reversible(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.01f64..0.2,
            n_steps in 10u32..50,
        ) {
            let (x_err, v_err) = time_reversal_error(
                |s, t, st, h| Yoshida6.step(s, t, st, h), x0, v0, dt, n_steps,
            );
            let scale = x0.abs().max(v0.abs()).max(1.0);
            prop_assert!(x_err < 1e-9 * scale, "Y6 x error: {x_err:.2e}");
            prop_assert!(v_err < 1e-9 * scale, "Y6 v error: {v_err:.2e}");
        }

        #[test]
        fn yoshida8_time_reversible(
            x0 in -10.0f64..10.0,
            v0 in -5.0f64..5.0,
            dt in 0.01f64..0.2,
            n_steps in 10u32..50,
        ) {
            let (x_err, v_err) = time_reversal_error(
                |s, t, st, h| Yoshida8.step(s, t, st, h), x0, v0, dt, n_steps,
            );
            let scale = x0.abs().max(v0.abs()).max(1.0);
            prop_assert!(x_err < 1e-8 * scale, "Y8 x error: {x_err:.2e}");
            prop_assert!(v_err < 1e-8 * scale, "Y8 v error: {v_err:.2e}");
        }
    }

    // --- Event detection ---

    #[test]
    fn yoshida4_integrate_with_events_completes() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Yoshida4.integrate_with_events(
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
    fn yoshida4_integrate_with_events_terminates() {
        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let initial = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let outcome = Yoshida4.integrate_with_events(
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
    fn yoshida4_detects_nan() {
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
        let outcome: IntegrationOutcome<State<3, 2>, ()> = Yoshida4.integrate_with_events(
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

    // --- 1D / 2D / 3D ---

    #[test]
    fn yoshida4_1d_full_period() {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(SVector::from([1.0]), SVector::from([0.0]));
        let final_state = Yoshida4.integrate(
            &system,
            initial,
            0.0,
            std::f64::consts::TAU,
            0.01,
            |_, _| {},
        );

        assert!(
            (final_state.y()[0] - 1.0).abs() < 1e-8,
            "1D error: {:.2e}",
            (final_state.y()[0] - 1.0).abs()
        );
    }

    #[test]
    fn yoshida4_2d_full_period() {
        let system = HarmonicOscillator2D;
        let initial = State::<2, 2>::new(vector![1.0, 0.0], vector![0.0, 1.0]);
        let final_state = Yoshida4.integrate(
            &system,
            initial,
            0.0,
            std::f64::consts::TAU,
            0.01,
            |_, _| {},
        );

        let eps = 1e-8;
        assert!(
            (final_state.y()[0] - 1.0).abs() < eps,
            "2D x error: {:.2e}",
            (final_state.y()[0] - 1.0).abs()
        );
        assert!(
            final_state.y()[1].abs() < eps,
            "2D y error: {:.2e}",
            final_state.y()[1].abs()
        );
    }

    #[test]
    fn yoshida4_3d_full_period() {
        let system = HarmonicOscillator;
        let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let final_state = Yoshida4.integrate(
            &system,
            initial,
            0.0,
            std::f64::consts::TAU,
            0.01,
            |_, _| {},
        );

        assert!(
            (final_state.y().x - 1.0).abs() < 1e-8,
            "3D error: {:.2e}",
            (final_state.y().x - 1.0).abs()
        );
    }
}
