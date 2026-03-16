//! Cross-method comparison tests.
//!
//! Tests here compare behaviors across different integrators to validate
//! that measured properties (e.g., time-reversibility, energy conservation)
//! genuinely distinguish symplectic from non-symplectic methods.
//! These tests would not belong in any single integrator's module.

use nalgebra::{SVector, vector};
use proptest::prelude::*;

use crate::test_systems::*;
use crate::{Integrator, Rk4, State, StormerVerlet};

// ---------------------------------------------------------------------------
// Time-reversibility comparison
// ---------------------------------------------------------------------------

proptest! {
    /// RK4 is NOT time-reversible: forward+backward does NOT recover initial state.
    /// This contrast validates that the time-reversibility test in verlet.rs
    /// is actually measuring a meaningful property — if RK4 also passed,
    /// the Verlet test would be vacuous.
    #[test]
    fn rk4_not_time_reversible(
        x0 in -10.0f64..10.0,
        v0 in -5.0f64..5.0,
    ) {
        let system = HarmonicOscillator1D;
        let initial = State::<1, 2>::new(SVector::from([x0]), SVector::from([v0]));
        let dt = 0.1;
        let n_steps = 50;

        let mut state = initial.clone();
        let mut t = 0.0;
        for _ in 0..n_steps {
            state = Rk4.step(&system, t, &state, dt);
            t += dt;
        }
        for _ in 0..n_steps {
            t -= dt;
            state = Rk4.step(&system, t, &state, -dt);
        }

        let x_err = (state.y()[0] - x0).abs();
        let v_err = (state.dy()[0] - v0).abs();
        let total_err = x_err + v_err;
        // RK4's irreversibility should produce measurable error
        prop_assert!(
            total_err > 1e-14,
            "RK4 should NOT be time-reversible, but error={total_err:.2e}"
        );
    }
}

// ---------------------------------------------------------------------------
// Energy drift comparison (symplectic vs non-symplectic)
// ---------------------------------------------------------------------------
//
// Symplectic integrators preserve a modified Hamiltonian H̃ = H + O(dt^p),
// so the true energy H oscillates with bounded amplitude O(dt^p) for ALL time.
// Non-symplectic methods (RK4 etc.) have secular energy drift that grows
// linearly with integration time.
//
// We verify this by splitting a long integration into halves and comparing
// the max |ΔE| in each half. For symplectic methods the ratio ≈ 1.0 (bounded);
// for non-symplectic the ratio ≈ 2.0 (linear growth).

/// Measure max energy deviation in the first and second halves of an integration.
fn energy_drift_halves<F>(integrator: F, t_end: f64, dt: f64) -> (f64, f64)
where
    F: Fn(
        &HarmonicOscillator,
        State<3, 2>,
        f64,
        f64,
        f64,
        &mut dyn FnMut(f64, &State<3, 2>),
    ) -> State<3, 2>,
{
    let system = HarmonicOscillator;
    let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
    let initial_energy = 0.5;
    let t_mid = t_end / 2.0;

    let mut first_half: f64 = 0.0;
    let mut second_half: f64 = 0.0;

    integrator(&system, initial, 0.0, t_end, dt, &mut |t,
                                                       state: &State<
        3,
        2,
    >| {
        let energy = 0.5 * (state.dy().norm_squared() + state.y().norm_squared());
        let drift = (energy - initial_energy).abs();
        if t < t_mid {
            first_half = first_half.max(drift);
        } else {
            second_half = second_half.max(drift);
        }
    });

    (first_half, second_half)
}

#[test]
fn verlet_no_secular_energy_drift() {
    // Symplectic: max |ΔE| in the second half ≈ first half (ratio ≈ 1.0).
    let dt = 0.05;
    let t_end = 1000.0 * 2.0 * std::f64::consts::PI; // 1000 periods

    let (first, second) = energy_drift_halves(
        |sys, init, t0, te, dt, cb| StormerVerlet.integrate(sys, init, t0, te, dt, cb),
        t_end,
        dt,
    );

    let ratio = second / first;
    assert!(
        ratio < 1.2,
        "Verlet energy drift ratio (2nd/1st half) should be ~1.0, got {ratio:.2} \
         (first={first:.2e}, second={second:.2e})"
    );
}

#[test]
fn rk4_has_secular_energy_drift() {
    // Contrast: RK4 (non-symplectic) has secular drift (ratio ≈ 2.0).
    // This test exists to confirm the measurement methodology —
    // if RK4 also showed ratio ≈ 1.0, the test above would be meaningless.
    let dt = 0.05;
    let t_end = 1000.0 * 2.0 * std::f64::consts::PI;

    let (first, second) = energy_drift_halves(
        |sys, init, t0, te, dt, cb| Rk4.integrate(sys, init, t0, te, dt, cb),
        t_end,
        dt,
    );

    let ratio = second / first;
    assert!(
        ratio > 1.5,
        "RK4 energy drift ratio (2nd/1st half) should be ~2.0, got {ratio:.2} \
         (first={first:.2e}, second={second:.2e})"
    );
}

// ---------------------------------------------------------------------------
// Per-step accuracy comparison
// ---------------------------------------------------------------------------

#[test]
fn verlet_less_accurate_per_step_than_rk4() {
    // At the same dt, RK4 (4th-order) is orders of magnitude more accurate.
    // The symplectic advantage only appears over many periods.
    let system = HarmonicOscillator;
    let initial = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
    let dt = 0.01;
    let t_end = 2.0 * std::f64::consts::PI;

    let verlet_final = StormerVerlet.integrate(&system, initial.clone(), 0.0, t_end, dt, |_, _| {});
    let rk4_final = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    let verlet_err = (verlet_final.y().x - 1.0).abs();
    let rk4_err = (rk4_final.y().x - 1.0).abs();

    assert!(
        rk4_err < verlet_err,
        "RK4 ({rk4_err:.2e}) should be more accurate than Verlet ({verlet_err:.2e}) at same dt"
    );
    // 2nd-order vs 4th-order → error ratio should be O(dt^{-2}) ≈ 10000x at dt=0.01
    assert!(
        verlet_err / rk4_err > 100.0,
        "Verlet/RK4 error ratio should be large (2nd vs 4th order): {:.0}x",
        verlet_err / rk4_err
    );
}
