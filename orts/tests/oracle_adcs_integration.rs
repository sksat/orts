//! Integration tests combining PD controller + Reaction Wheels + Gravity Gradient torque.
//!
//! These tests exercise the full ADCS chain:
//! - PD controller computes desired body torque from attitude error
//! - Reaction wheel assembly applies the commanded torque
//! - Gravity gradient provides a persistent disturbance torque
//!
//! The closed loop is implemented via segment-by-segment integration:
//! each control cycle evaluates the PD law at the current state, sets
//! the RW commanded torque, then integrates one segment with RW + GG.

use kaname::constants::{MU_EARTH, R_EARTH};
use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use utsuroi::{Integrator, Rk4};

use orts::attitude::{AttitudeState, AugmentedAttitudeSystem, GravityGradientTorque};
use orts::effector::AugmentedState;
use orts::spacecraft::ReactionWheelAssembly;

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

fn diagonal_inertia(ix: f64, iy: f64, iz: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(ix, iy, iz))
}

/// Compute PD control torque using the left-invariant quaternion error convention,
/// matching `InertialPdController::eval`.
///
/// Returns the body-frame torque command: tau = -Kp * theta_err - Kd * omega.
fn pd_control_law(
    att: &AttitudeState,
    target_q: &UnitQuaternion<f64>,
    kp: f64,
    kd: f64,
) -> Vector3<f64> {
    // Left-invariant error: q_err = q_target^{-1} * q_current
    let mut q_err = target_q.inverse() * att.orientation();

    // Hemisphere selection (shortest path)
    if q_err.w < 0.0 {
        q_err = UnitQuaternion::new_unchecked(-q_err.into_inner());
    }

    // Body-frame angular error: theta ~= 2 * q_err.vec for small angles
    let q_vec = q_err.as_ref().vector();
    let theta_error = 2.0 * Vector3::new(q_vec[0], q_vec[1], q_vec[2]);

    -kp * theta_error - kd * att.angular_velocity
}

/// Quaternion angle error in degrees between current orientation and target.
fn angle_error_deg(state: &AttitudeState, target_q: &UnitQuaternion<f64>) -> f64 {
    let q_err = target_q.inverse() * state.orientation();
    q_err.angle().to_degrees()
}

// ──────────────────────────────────────────────────────
// Test 1: PD + RW attitude stabilization with gravity gradient
// ──────────────────────────────────────────────────────

#[test]
fn pd_rw_stabilization_with_gravity_gradient() {
    // Full closed-loop: PD controller commands body torque -> RW applies it
    // -> gravity gradient disturbs. The attitude should converge to the target.
    //
    // Segment-by-segment loop:
    //   1. Evaluate PD at current state -> get commanded torque
    //   2. Set rw.commanded_torque = pd_torque
    //   3. Build system with GG + RW (no PD Model, since RW handles the torque)
    //   4. Integrate one segment

    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let radius = R_EARTH + 400.0; // LEO circular orbit [km]
    let mu = MU_EARTH;
    let mass = 500.0; // kg

    // PD gains
    let kp = 1.0;
    let kd = 2.0;
    let target_q = UnitQuaternion::identity();

    // Initial condition: 10 deg error about Z, at rest
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::z());
    let initial_q = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial_att = AttitudeState::new(initial_q, Vector3::zeros());

    // RW assembly: 3-axis, inertia=0.01, max_momentum=1.0, max_torque=0.5
    let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);

    // Integration parameters
    let dt_ctrl = 0.1; // control sample period [s]
    let dt_ode = 0.01; // ODE step size [s]
    let t_end = 60.0; // total simulation time [s]

    // Initial augmented state (3 RW momenta, all zero)
    let mut state = AugmentedState {
        plant: initial_att,
        aux: vec![0.0, 0.0, 0.0],
    };
    let mut t: f64 = 0.0;

    // Track error history for convergence verification
    let initial_error = angle_error_deg(&state.plant, &target_q);

    while t < t_end {
        let t_next = (t + dt_ctrl).min(t_end);

        // 1. PD control law (compute desired body torque)
        let tau_cmd = pd_control_law(&state.plant, &target_q, kp, kd);

        // 2. Set RW command
        let mut rw_seg = rw.clone();
        rw_seg.commanded_torque = tau_cmd;

        // 3. Build system with gravity gradient + RW (no PD Model)
        let gg = GravityGradientTorque::circular_orbit(mu, radius, inertia);
        let system = AugmentedAttitudeSystem::circular_orbit(inertia, mu, radius, mass)
            .with_model(gg)
            .with_effector(rw_seg);

        // 4. Integrate one segment
        state = Rk4.integrate(&system, state, t, t_next, dt_ode, |_, _| {});
        t = t_next;
    }

    // Final attitude should be close to target
    let final_error = angle_error_deg(&state.plant, &target_q);

    assert!(
        final_error < 1.0,
        "PD+RW should stabilize to <1 deg, got {final_error:.4} deg \
         (initial was {initial_error:.4} deg)"
    );

    // Angular velocity should be nearly zero (settled)
    let omega_mag = state.plant.angular_velocity.magnitude();
    assert!(
        omega_mag < 0.01,
        "Angular velocity should be small at steady state, got {omega_mag:.6} rad/s"
    );
}

// ──────────────────────────────────────────────────────
// Test 2: RW momentum buildup under constant gravity gradient disturbance
// ──────────────────────────────────────────────────────

#[test]
fn rw_momentum_buildup_under_gravity_gradient() {
    // With an asymmetric inertia tensor the gravity gradient produces a
    // persistent disturbance torque. The PD+RW controller maintains
    // pointing, but RW momentum accumulates (no dumping mechanism).
    //
    // We verify:
    //   a) Attitude error stays bounded (pointing maintained)
    //   b) RW momentum grows over time (no momentum dumping)

    let inertia = diagonal_inertia(10.0, 20.0, 30.0); // asymmetric
    let radius = R_EARTH + 400.0;
    let mu = MU_EARTH;
    let mass = 500.0;

    // Stronger PD gains for tighter pointing
    let kp = 2.0;
    let kd = 4.0;
    let target_q = UnitQuaternion::identity();

    // Start at target orientation (identity), at rest
    let initial_att = AttitudeState::new(target_q, Vector3::zeros());

    // RW assembly with generous momentum capacity
    let rw = ReactionWheelAssembly::three_axis(0.01, 10.0, 0.5);

    let dt_ctrl = 0.1;
    let dt_ode = 0.01;
    // Integrate for a significant fraction of the orbital period
    // Period ~= 2*pi*sqrt(r^3/mu) ~= 5554 s for 400 km altitude
    let t_end = 500.0; // ~9% of one orbit

    let mut state = AugmentedState {
        plant: initial_att,
        aux: vec![0.0, 0.0, 0.0],
    };
    let mut t: f64 = 0.0;

    let mut max_error_deg = 0.0_f64;
    let mut momentum_samples: Vec<f64> = Vec::new();

    while t < t_end {
        let t_next = (t + dt_ctrl).min(t_end);

        // PD control law
        let tau_cmd = pd_control_law(&state.plant, &target_q, kp, kd);

        // Set RW command
        let mut rw_seg = rw.clone();
        rw_seg.commanded_torque = tau_cmd;

        // Build system with GG + RW
        let gg = GravityGradientTorque::circular_orbit(mu, radius, inertia);
        let system = AugmentedAttitudeSystem::circular_orbit(inertia, mu, radius, mass)
            .with_model(gg)
            .with_effector(rw_seg);

        // Integrate one segment
        state = Rk4.integrate(&system, state, t, t_next, dt_ode, |_, _| {});
        t = t_next;

        // Track error and momentum
        let err = angle_error_deg(&state.plant, &target_q);
        max_error_deg = max_error_deg.max(err);

        // Total RW momentum magnitude
        let h_total = Vector3::new(state.aux[0], state.aux[1], state.aux[2]).magnitude();
        momentum_samples.push(h_total);
    }

    // a) Attitude error should stay bounded (controller active)
    assert!(
        max_error_deg < 5.0,
        "Attitude error should stay bounded, max was {max_error_deg:.4} deg"
    );

    // b) RW momentum should accumulate over time
    // Compare the average momentum in the last quarter vs the first quarter
    let n = momentum_samples.len();
    let first_quarter_avg: f64 = momentum_samples[..n / 4].iter().sum::<f64>() / (n / 4) as f64;
    let last_quarter_avg: f64 =
        momentum_samples[3 * n / 4..].iter().sum::<f64>() / (n - 3 * n / 4) as f64;

    assert!(
        last_quarter_avg > first_quarter_avg,
        "RW momentum should increase over time (gravity gradient absorbs into wheels): \
         first quarter avg = {first_quarter_avg:.6e}, last quarter avg = {last_quarter_avg:.6e}"
    );

    // Final momentum should be non-trivially positive
    let final_momentum = Vector3::new(state.aux[0], state.aux[1], state.aux[2]).magnitude();
    assert!(
        final_momentum > 1e-6,
        "Final RW momentum should be non-trivially positive, got {final_momentum:.6e} N*m*s"
    );
}

// ──────────────────────────────────────────────────────
// Test 3: PD+RW matches direct PD (ideal actuator) for symmetric body
// ──────────────────────────────────────────────────────

#[test]
fn pd_rw_matches_direct_pd_symmetric_body() {
    // For a symmetric body (no gravity gradient torque), the closed-loop
    // PD+RW system should produce attitude convergence comparable to the
    // ideal direct-torque PD controller, since the RW applies the
    // commanded torque exactly (within limits).
    //
    // We compare the final angle errors of both approaches and verify
    // they are both small.

    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let radius = R_EARTH + 400.0;
    let mu = MU_EARTH;
    let mass = 500.0;

    let kp = 1.0;
    let kd = 2.0;
    let target_q = UnitQuaternion::identity();

    // 10 deg initial error about Z
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::z());
    let initial_q = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial_att = AttitudeState::new(initial_q, Vector3::zeros());

    let dt_ctrl = 0.1;
    let dt_ode = 0.01;
    let t_end = 80.0;

    // ---- Path A: PD + RW (segment loop) ----
    let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
    let mut state_rw = AugmentedState {
        plant: initial_att.clone(),
        aux: vec![0.0, 0.0, 0.0],
    };
    let mut t: f64 = 0.0;

    while t < t_end {
        let t_next = (t + dt_ctrl).min(t_end);
        let tau_cmd = pd_control_law(&state_rw.plant, &target_q, kp, kd);

        let mut rw_seg = rw.clone();
        rw_seg.commanded_torque = tau_cmd;

        let system = AugmentedAttitudeSystem::circular_orbit(inertia, mu, radius, mass)
            .with_effector(rw_seg);

        state_rw = Rk4.integrate(&system, state_rw, t, t_next, dt_ode, |_, _| {});
        t = t_next;
    }

    let error_rw = angle_error_deg(&state_rw.plant, &target_q);

    // ---- Path B: Direct PD torque (ideal actuator) ----
    use orts::attitude::{AttitudeSystem, InertialPdController};

    let ctrl = InertialPdController::diagonal(kp, kd, target_q);
    let system_direct = AttitudeSystem::new(inertia).with_model(ctrl);
    let final_direct = Rk4.integrate(&system_direct, initial_att, 0.0, t_end, dt_ode, |_, _| {});
    let error_direct = angle_error_deg(&final_direct, &target_q);

    // Both should converge to well under 1 deg
    assert!(
        error_direct < 0.1,
        "Direct PD should converge, got {error_direct:.4} deg"
    );
    assert!(
        error_rw < 0.5,
        "PD+RW should converge comparably, got {error_rw:.4} deg"
    );

    // The RW path may have slightly more error due to the discrete control
    // update (ZOH at dt_ctrl), but should be in the same ballpark
    assert!(
        error_rw < error_direct * 100.0,
        "PD+RW error ({error_rw:.4} deg) should be within 100x of direct PD ({error_direct:.4} deg)"
    );
}
