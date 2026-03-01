use std::f64::consts::PI;
use std::ops::ControlFlow;

use nalgebra::{Matrix3, UnitQuaternion, Vector3, Vector4};
use orts_integrator::{
    DormandPrince, IntegrationOutcome, Integrator, Rk4, Tolerances,
};

use orts_attitude::{AttitudeState, AttitudeSystem, GravityGradientTorque};

fn diagonal_inertia(ix: f64, iy: f64, iz: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(ix, iy, iz))
}

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    diagonal_inertia(i, i, i)
}

/// Rotational kinetic energy T = 0.5 * ω · (I · ω)
fn rotational_energy(state: &AttitudeState, inertia: &Matrix3<f64>) -> f64 {
    0.5 * state.angular_velocity.dot(&(inertia * state.angular_velocity))
}

/// Angular momentum in inertial frame: L_inertial = R_bi^T * (I * ω)
fn angular_momentum_inertial(state: &AttitudeState, inertia: &Matrix3<f64>) -> Vector3<f64> {
    let l_body = inertia * state.angular_velocity;
    state.rotation_matrix() * l_body
}

// ──────────────────────────────────────────────────────
// Torque-free motion
// ──────────────────────────────────────────────────────

#[test]
fn torque_free_symmetric_body_constant_omega() {
    // Spherically symmetric body: ω × (I·ω) = I(ω × ω) = 0
    // → ω remains constant, quaternion evolves kinematically
    let i_val = 10.0;
    let system = AttitudeSystem::new(symmetric_inertia(i_val));

    let omega0 = Vector3::new(0.1, 0.2, 0.3);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    // Integrate for 100 seconds with RK4
    let dt = 0.01;
    let t_end = 100.0;
    let mut max_omega_error = 0.0_f64;

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let err = (state.angular_velocity - omega0).magnitude();
        max_omega_error = max_omega_error.max(err);
    });

    assert!(
        max_omega_error < 1e-12,
        "ω should remain constant for symmetric body, max error: {max_omega_error:.2e}"
    );

    // Quaternion should still be unit
    let q_norm = final_state.quaternion.magnitude();
    assert!(
        (q_norm - 1.0).abs() < 1e-10,
        "Quaternion should remain normalized, norm: {q_norm}"
    );
}

#[test]
fn torque_free_axisymmetric_precession() {
    // Axisymmetric body (Ix = Iy ≠ Iz): spin about z with small transverse component
    // The angular velocity precesses about the symmetry axis (z-body) at rate:
    //   Ω_prec = ωz * (Iz - Ix) / Ix
    let ix = 10.0;
    let iz = 15.0;
    let system = AttitudeSystem::new(diagonal_inertia(ix, ix, iz));

    let wz = 1.0; // spin rate about symmetry axis
    let wx0 = 0.1; // small transverse component
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: Vector3::new(wx0, 0.0, wz),
    };

    // ωz should remain constant (torque-free axisymmetric)
    let dt = 0.001;
    let t_end = 10.0;
    let mut max_wz_error = 0.0_f64;
    let mut max_transverse_error = 0.0_f64;
    let transverse_mag = wx0; // |ω_transverse| is conserved

    let _ = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let wz_err = (state.angular_velocity[2] - wz).abs();
        max_wz_error = max_wz_error.max(wz_err);

        let wt = (state.angular_velocity[0].powi(2) + state.angular_velocity[1].powi(2)).sqrt();
        let wt_err = (wt - transverse_mag).abs();
        max_transverse_error = max_transverse_error.max(wt_err);
    });

    assert!(
        max_wz_error < 1e-10,
        "ωz should be constant, max error: {max_wz_error:.2e}"
    );
    assert!(
        max_transverse_error < 1e-10,
        "|ω_transverse| should be constant, max error: {max_transverse_error:.2e}"
    );
}

#[test]
fn torque_free_energy_conservation() {
    // Asymmetric body: energy must be conserved
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);
    let system = AttitudeSystem::new(inertia);

    let omega0 = Vector3::new(0.5, 0.3, 0.1);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    let e0 = rotational_energy(&initial, &inertia);
    let dt = 0.01;
    let t_end = 100.0;
    let mut max_rel_error = 0.0_f64;

    let _ = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let e = rotational_energy(state, &inertia);
        let rel_err = ((e - e0) / e0).abs();
        max_rel_error = max_rel_error.max(rel_err);
    });

    assert!(
        max_rel_error < 1e-10,
        "Energy should be conserved, max relative error: {max_rel_error:.2e}"
    );
}

#[test]
fn torque_free_angular_momentum_conservation() {
    // Angular momentum in inertial frame must be conserved (torque-free)
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);
    let system = AttitudeSystem::new(inertia);

    let omega0 = Vector3::new(0.5, 0.3, 0.1);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    let l0 = angular_momentum_inertial(&initial, &inertia);
    let dt = 0.01;
    let t_end = 100.0;
    let mut max_error = 0.0_f64;

    let _ = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let l = angular_momentum_inertial(state, &inertia);
        let err = (l - l0).magnitude() / l0.magnitude();
        max_error = max_error.max(err);
    });

    assert!(
        max_error < 1e-10,
        "Angular momentum should be conserved, max relative error: {max_error:.2e}"
    );
}

// ──────────────────────────────────────────────────────
// Gravity gradient libration
// ──────────────────────────────────────────────────────

#[test]
fn gravity_gradient_libration_frequency() {
    // Small-angle pitch libration about radial equilibrium with fixed position.
    //
    // Use a fixed position r_eci = (r, 0, 0) so the body oscillates about the
    // equilibrium orientation (body x-axis along radial) without needing orbit
    // co-rotation. The restoring torque for small pitch angle θ about z is:
    //   τ_z ≈ -3(μ/r³)(Iy - Ix) θ
    //
    // giving libration frequency:
    //   ω_lib = sqrt(3 n² (Iy - Ix) / Iz)
    //
    // where n² = μ/r³.
    let mu: f64 = 398600.4418; // km³/s²
    let r: f64 = 7000.0; // km
    let n_sq = mu / r.powi(3); // n² = μ/r³

    // Inertia: Ix < Iy (so gravity gradient is stabilizing in pitch)
    let ix = 10.0;
    let iy = 30.0;
    let iz = 25.0;
    let inertia = diagonal_inertia(ix, iy, iz);

    // Expected libration frequency
    let omega_lib = (3.0 * n_sq * (iy - ix) / iz).sqrt();
    let period_lib = 2.0 * PI / omega_lib;

    // Fixed position — no orbital motion, pure gravity gradient oscillation
    let gg = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r, 0.0, 0.0));
    let system = AttitudeSystem::new(inertia).with_torque(Box::new(gg));

    // Initial condition: small pitch angle about z-body, zero angular velocity
    let pitch0 = 0.01; // rad (small angle)
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
    let uq = UnitQuaternion::from_axis_angle(&axis, pitch0);
    let initial = AttitudeState::new(uq, Vector3::zeros());

    // Integrate for 3 libration periods with DP45
    let tol = Tolerances {
        atol: 1e-12,
        rtol: 1e-10,
    };
    let t_end = 3.0 * period_lib;
    let dt0 = period_lib / 1000.0;

    // Collect pitch angle time series
    let mut times = vec![0.0];
    let mut pitches = vec![pitch0];

    let outcome: IntegrationOutcome<AttitudeState, ()> =
        DormandPrince.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            dt0,
            &tol,
            |t, state| {
                // Pitch angle ≈ 2 * q_z for small angles (scalar-first: q = [w, x, y, z])
                let pitch = 2.0 * state.quaternion[3];
                times.push(t);
                pitches.push(pitch);
            },
            |_, _| ControlFlow::Continue(()),
        );

    match outcome {
        IntegrationOutcome::Completed(_) => {}
        other => panic!("Integration failed: {other:?}"),
    }

    // Find zero-crossings to measure the period
    let mut zero_crossings = Vec::new();
    for i in 1..pitches.len() {
        if pitches[i - 1] * pitches[i] < 0.0 {
            // Linear interpolation for zero crossing
            let t_zero =
                times[i - 1] + (times[i] - times[i - 1]) * pitches[i - 1].abs()
                    / (pitches[i - 1].abs() + pitches[i].abs());
            zero_crossings.push(t_zero);
        }
    }

    assert!(
        zero_crossings.len() >= 4,
        "Expected at least 4 zero crossings in 3 periods, got {} \
         (period_lib={period_lib:.1}s, t_end={t_end:.1}s)",
        zero_crossings.len()
    );

    // Half-period = time between consecutive zero crossings
    // Average over multiple half-periods for robustness
    let n_half = zero_crossings.len() - 1;
    let total_half_periods = zero_crossings[n_half] - zero_crossings[0];
    let measured_half_period = total_half_periods / n_half as f64;
    let measured_period = 2.0 * measured_half_period;
    let freq_error = ((measured_period - period_lib) / period_lib).abs();

    assert!(
        freq_error < 0.01,
        "Libration frequency error: {:.2}% (measured period: {measured_period:.2}s, \
         expected: {period_lib:.2}s)",
        freq_error * 100.0
    );
}

// ──────────────────────────────────────────────────────
// Integration method tests
// ──────────────────────────────────────────────────────

#[test]
fn dp45_attitude_integration() {
    // Verify DP45 works with AttitudeState (torque-free asymmetric body)
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);
    let system = AttitudeSystem::new(inertia);

    let omega0 = Vector3::new(0.5, 0.3, 0.1);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    let tol = Tolerances {
        atol: 1e-12,
        rtol: 1e-10,
    };
    let t_end = 100.0;
    let dt0 = 0.1;

    let e0 = rotational_energy(&initial, &inertia);
    let l0 = angular_momentum_inertial(&initial, &inertia);

    let outcome: IntegrationOutcome<AttitudeState, ()> =
        DormandPrince.integrate_adaptive_with_events(
            &system,
            initial,
            0.0,
            t_end,
            dt0,
            &tol,
            |_, _| {},
            |_, _| ControlFlow::Continue(()),
        );

    let final_state = match outcome {
        IntegrationOutcome::Completed(s) => s,
        other => panic!("DP45 failed: {other:?}"),
    };

    // Energy conservation
    let e_final = rotational_energy(&final_state, &inertia);
    let energy_err = ((e_final - e0) / e0).abs();
    assert!(
        energy_err < 1e-9,
        "DP45 energy error: {energy_err:.2e}"
    );

    // Angular momentum conservation
    let l_final = angular_momentum_inertial(&final_state, &inertia);
    let l_err = (l_final - l0).magnitude() / l0.magnitude();
    assert!(
        l_err < 1e-9,
        "DP45 angular momentum error: {l_err:.2e}"
    );

    // Quaternion should be close to unit (project() renormalizes each step,
    // but adaptive stepping may accumulate small drift)
    let q_norm = final_state.quaternion.magnitude();
    assert!(
        (q_norm - 1.0).abs() < 1e-8,
        "Quaternion not normalized: {q_norm}"
    );
}

#[test]
fn dt_convergence_rk4() {
    // RK4 is 4th-order: halving dt should reduce error by ~16x
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);
    let system = AttitudeSystem::new(inertia);

    let omega0 = Vector3::new(0.5, 0.3, 0.1);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    let t_end = 10.0;

    // Use a very fine reference solution
    let ref_state = Rk4.integrate(&system, initial.clone(), 0.0, t_end, 0.0001, |_, _| {});

    let mut errors = Vec::new();
    for &dt in &[0.1, 0.05, 0.025] {
        let state = Rk4.integrate(&system, initial.clone(), 0.0, t_end, dt, |_, _| {});
        let err = (state.angular_velocity - ref_state.angular_velocity).magnitude();
        errors.push(err);
    }

    // Check 4th-order convergence: error ratio should be ~16 when dt halved
    for i in 0..errors.len() - 1 {
        let ratio = errors[i] / errors[i + 1];
        assert!(
            ratio > 14.0 && ratio < 18.0,
            "Expected error ratio ~16, got {ratio:.2} (errors: {:.2e}, {:.2e})",
            errors[i],
            errors[i + 1]
        );
    }
}

#[test]
fn tolerance_convergence_dp45() {
    // DP45 is adaptive: tightening tolerances should improve accuracy
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);
    let system = AttitudeSystem::new(inertia);

    let omega0 = Vector3::new(0.5, 0.3, 0.1);
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: omega0,
    };

    let t_end = 50.0;
    let dt0 = 0.1;

    // Reference: very tight tolerance
    let ref_tol = Tolerances {
        atol: 1e-14,
        rtol: 1e-14,
    };
    let ref_outcome: IntegrationOutcome<AttitudeState, ()> =
        DormandPrince.integrate_adaptive_with_events(
            &system,
            initial.clone(),
            0.0,
            t_end,
            dt0,
            &ref_tol,
            |_, _| {},
            |_, _| ControlFlow::Continue(()),
        );
    let ref_state = match ref_outcome {
        IntegrationOutcome::Completed(s) => s,
        other => panic!("Reference integration failed: {other:?}"),
    };

    // Test with progressively tighter tolerances
    let tol_levels = [1e-6, 1e-8, 1e-10];
    let mut errors = Vec::new();
    for &tol_val in &tol_levels {
        let tol = Tolerances {
            atol: tol_val,
            rtol: tol_val,
        };
        let outcome: IntegrationOutcome<AttitudeState, ()> =
            DormandPrince.integrate_adaptive_with_events(
                &system,
                initial.clone(),
                0.0,
                t_end,
                dt0,
                &tol,
                |_, _| {},
                |_, _| ControlFlow::Continue(()),
            );
        let state = match outcome {
            IntegrationOutcome::Completed(s) => s,
            other => panic!("DP45 failed at tol={tol_val}: {other:?}"),
        };
        let err = (state.angular_velocity - ref_state.angular_velocity).magnitude();
        errors.push(err);
    }

    // Each 100x tightening of tolerance should reduce error significantly
    for i in 0..errors.len() - 1 {
        assert!(
            errors[i + 1] < errors[i],
            "Tighter tolerance should give smaller error: tol={:.0e} err={:.2e}, \
             tol={:.0e} err={:.2e}",
            tol_levels[i],
            errors[i],
            tol_levels[i + 1],
            errors[i + 1]
        );
    }

    // Tightest tolerance should give very good accuracy
    assert!(
        errors[2] < 1e-8,
        "DP45 with tol=1e-10 should achieve <1e-8 error, got {:.2e}",
        errors[2]
    );
}
