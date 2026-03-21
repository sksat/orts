//! Oracle tests for reaction wheel dynamics via AugmentedAttitudeSystem.
//!
//! These tests verify physical invariants (angular momentum conservation,
//! torque-spin coupling, saturation, and rate limiting) for the reaction
//! wheel assembly integrated alongside attitude dynamics.

use nalgebra::{Matrix3, Vector3, Vector4};
use utsuroi::{Integrator, Rk4};

use orts::attitude::{AttitudeState, AugmentedAttitudeSystem};
use orts::effector::AugmentedState;
use orts::spacecraft::ReactionWheelAssembly;

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

/// Compute total angular momentum: spacecraft body + reaction wheels.
///
/// L_total = R_ib * (I * ω) + Σ h_i * R_ib * axis_i
///
/// where R_ib is the body-to-inertial rotation matrix.
fn total_angular_momentum(
    state: &AugmentedState<AttitudeState>,
    inertia: &Matrix3<f64>,
    wheel_axes: &[Vector3<f64>],
) -> Vector3<f64> {
    // Body angular momentum in body frame
    let l_body_sc = inertia * state.plant.angular_velocity;

    // Wheel angular momentum in body frame
    let mut l_body_rw = Vector3::zeros();
    for (i, axis) in wheel_axes.iter().enumerate() {
        l_body_rw += state.aux[i] * axis;
    }

    // Transform total body-frame angular momentum to inertial frame
    let r_ib = state.plant.rotation_matrix();
    r_ib * (l_body_sc + l_body_rw)
}

// ──────────────────────────────────────────────────────
// Test 1: Angular momentum conservation
// ──────────────────────────────────────────────────────

#[test]
fn angular_momentum_conservation_with_rw() {
    // AugmentedAttitudeSystem with 3-axis RW, no external torques.
    // Apply commanded_torque to spin up wheels.
    // Total angular momentum (spacecraft body + RW) must be conserved.
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);

    let mut rw = ReactionWheelAssembly::three_axis(0.01, 10.0, 0.5);
    rw.commanded_torque = Vector3::new(0.1, 0.05, 0.2);

    let wheel_axes = vec![Vector3::x(), Vector3::y(), Vector3::z()];

    let system = AugmentedAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0)
        .with_effector(rw);

    let initial = AugmentedState {
        plant: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.01, -0.02, 0.03),
        },
        aux: system.initial_aux_state(),
    };

    let l0 = total_angular_momentum(&initial, &inertia, &wheel_axes);

    let dt = 0.01;
    let t_end = 50.0;
    let mut max_rel_error = 0.0_f64;

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let l = total_angular_momentum(state, &inertia, &wheel_axes);
        let err = (l - l0).magnitude() / l0.magnitude();
        max_rel_error = max_rel_error.max(err);
    });

    // Verify final angular momentum is conserved
    let l_final = total_angular_momentum(&final_state, &inertia, &wheel_axes);
    let final_err = (l_final - l0).magnitude() / l0.magnitude();

    assert!(
        max_rel_error < 1e-8,
        "Total angular momentum should be conserved, max relative error: {max_rel_error:.2e}"
    );
    assert!(
        final_err < 1e-8,
        "Final angular momentum error: {final_err:.2e}"
    );
}

// ──────────────────────────────────────────────────────
// Test 2: RW torque produces spacecraft rotation
// ──────────────────────────────────────────────────────

#[test]
fn rw_torque_produces_opposite_spacecraft_rotation() {
    // Start at rest, command torque about Z.
    // RW Z-wheel accelerates (h_z increases).
    // Spacecraft rotates in opposite direction (ω_z negative).
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);

    let mut rw = ReactionWheelAssembly::three_axis(0.01, 10.0, 0.5);
    rw.commanded_torque = Vector3::new(0.0, 0.0, 0.1); // desired +Z body torque

    let system = AugmentedAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0)
        .with_effector(rw);

    let initial = AugmentedState {
        plant: AttitudeState::identity(), // at rest
        aux: system.initial_aux_state(),  // wheels at zero momentum
    };

    let dt = 0.01;
    let t_end = 10.0;

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    // Z-wheel absorbs negative momentum (reaction to +Z body torque)
    assert!(
        final_state.aux[2] < 0.0,
        "Z-wheel momentum should be negative (reaction), got {}",
        final_state.aux[2]
    );

    // Spacecraft should rotate in commanded direction (positive omega_z)
    assert!(
        final_state.plant.angular_velocity[2] > 0.0,
        "Spacecraft omega_z should be positive, got {}",
        final_state.plant.angular_velocity[2]
    );

    // X and Y wheels and spacecraft components should be near zero
    assert!(
        final_state.aux[0].abs() < 1e-12,
        "X-wheel momentum should be ~0, got {}",
        final_state.aux[0]
    );
    assert!(
        final_state.aux[1].abs() < 1e-12,
        "Y-wheel momentum should be ~0, got {}",
        final_state.aux[1]
    );
    assert!(
        final_state.plant.angular_velocity[0].abs() < 1e-12,
        "omega_x should be ~0, got {}",
        final_state.plant.angular_velocity[0]
    );
    assert!(
        final_state.plant.angular_velocity[1].abs() < 1e-12,
        "omega_y should be ~0, got {}",
        final_state.plant.angular_velocity[1]
    );

    // Verify quantitative relationship: h_z = -I * omega_z (momentum conservation)
    let h_z = final_state.aux[2];
    let i_omega_z = i_val * final_state.plant.angular_velocity[2];
    assert!(
        (h_z + i_omega_z).abs() < 1e-10,
        "h_z + I*omega_z should be ~0 (momentum conservation), got {:.2e}",
        h_z + i_omega_z
    );
}

// ──────────────────────────────────────────────────────
// Test 3: Momentum saturation
// ──────────────────────────────────────────────────────

#[test]
fn momentum_saturation_stops_acceleration() {
    // Command continuous torque until wheel reaches max_momentum.
    // Verify wheel stops accelerating and spacecraft angular velocity stabilizes.
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);

    let max_momentum = 0.5; // small limit for fast saturation
    let max_torque = 0.1;
    let mut rw = ReactionWheelAssembly::three_axis(0.01, max_momentum, max_torque);
    rw.commanded_torque = Vector3::new(0.0, 0.0, max_torque); // saturate Z-wheel

    let system = AugmentedAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0)
        .with_effector(rw);

    let initial = AugmentedState {
        plant: AttitudeState::identity(),
        aux: system.initial_aux_state(),
    };

    // Time to saturation: h_max / tau_max = 0.5 / 0.1 = 5.0 s
    let dt = 0.01;
    let t_end = 20.0; // well past saturation time

    let mut omega_at_saturation = None;

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |t, state| {
        // Record omega_z around saturation time
        if t > 6.0 && omega_at_saturation.is_none() {
            omega_at_saturation = Some(state.plant.angular_velocity[2]);
        }
    });

    // Z-wheel should be at -max_momentum (absorbs reaction to +Z body torque)
    assert!(
        (final_state.aux[2] + max_momentum).abs() < 0.01,
        "Z-wheel should be at -max_momentum {}, got {}",
        -max_momentum,
        final_state.aux[2]
    );

    // Spacecraft angular velocity should have stopped changing after saturation
    let omega_z_final = final_state.plant.angular_velocity[2];
    let omega_z_sat = omega_at_saturation.unwrap();
    assert!(
        (omega_z_final - omega_z_sat).abs() < 1e-10,
        "omega_z should stop changing after saturation: at sat={omega_z_sat:.6e}, final={omega_z_final:.6e}"
    );
}

// ──────────────────────────────────────────────────────
// Test 4: Torque rate limiting
// ──────────────────────────────────────────────────────

#[test]
fn torque_rate_limiting_clamps_acceleration() {
    // Command a very large torque. Verify actual torque is clamped to max_torque
    // and dh/dt does not exceed max_torque.
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);

    let max_torque = 0.1;
    let mut rw = ReactionWheelAssembly::three_axis(0.01, 100.0, max_torque);
    rw.commanded_torque = Vector3::new(0.0, 0.0, 100.0); // way above max_torque

    let system = AugmentedAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0)
        .with_effector(rw);

    let initial = AugmentedState {
        plant: AttitudeState::identity(),
        aux: system.initial_aux_state(),
    };

    let dt = 0.01;
    let t_end = 10.0;

    let mut prev_h_z = 0.0;
    let mut prev_t = 0.0;
    let mut max_dh_dt = 0.0_f64;

    let _ = Rk4.integrate(&system, initial, 0.0, t_end, dt, |t, state| {
        if t > 0.0 {
            let dh_dt = (state.aux[2] - prev_h_z) / (t - prev_t);
            max_dh_dt = max_dh_dt.max(dh_dt.abs());
        }
        prev_h_z = state.aux[2];
        prev_t = t;
    });

    // dh/dt should not exceed max_torque (with some numerical tolerance)
    assert!(
        max_dh_dt <= max_torque * 1.01,
        "dh/dt should be limited to {max_torque}, max observed: {max_dh_dt:.6e}"
    );
}
