use std::f64::consts::PI;

use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use utsuroi::{Integrator, Rk4};

use orts::attitude::{
    AttitudeState, AttitudeSystem, DecoupledAttitudeSystem, GravityGradientTorque,
    InertialPdController, NadirPointing, TrackingPdController,
};

fn diagonal_inertia(ix: f64, iy: f64, iz: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(ix, iy, iz))
}

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    diagonal_inertia(i, i, i)
}

/// Quaternion angle error in degrees between current orientation and target.
fn angle_error_deg(state: &AttitudeState, target_q: &UnitQuaternion<f64>) -> f64 {
    let q_err = state.orientation() * target_q.inverse();
    q_err.angle().to_degrees()
}

/// Rotational kinetic energy T = 0.5 * ω · (I · ω)
fn rotational_energy(state: &AttitudeState, inertia: &Matrix3<f64>) -> f64 {
    0.5 * state
        .angular_velocity
        .dot(&(inertia * state.angular_velocity))
}

/// Lyapunov function for inertial PD control:
/// V = 0.5 * ω·(I·ω) + kp * |q_err_vec|²
///
/// For small angles this reduces to V ≈ 0.5 * ω·(I·ω) + kp * θ²/4
fn lyapunov_pd(
    state: &AttitudeState,
    inertia: &Matrix3<f64>,
    kp: f64,
    target_q: &UnitQuaternion<f64>,
) -> f64 {
    let kinetic = rotational_energy(state, inertia);
    let mut q_err = target_q.inverse() * state.orientation();
    if q_err.w < 0.0 {
        q_err = UnitQuaternion::new_unchecked(-q_err.into_inner());
    }
    let q_vec = q_err.as_ref().vector();
    let potential = kp * q_vec.norm_squared();
    kinetic + potential
}

// ──────────────────────────────────────────────────────
// Test 1: Inertial PD convergence (underdamped)
// ──────────────────────────────────────────────────────

#[test]
fn inertial_pd_convergence_underdamped() {
    // Symmetric inertia I = 10 kg·m²
    // Kp = 1.0, Kd = 2.0
    // ζ = Kd / (2√(Kp·I)) = 2 / (2√10) ≈ 0.316 (underdamped)
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let kp = 1.0;
    let kd = 2.0;
    let target_q = UnitQuaternion::identity();

    let ctrl = InertialPdController::diagonal(kp, kd, target_q);
    let system = AttitudeSystem::new(inertia).with_model(ctrl);

    // Initial condition: 10° rotation about Z from identity
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
    let uq = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial = AttitudeState::new(uq, Vector3::zeros());

    // Integrate with RK4, dt=0.1s, for 100s
    let dt = 0.1;
    let t_end = 100.0;

    // Track Lyapunov function
    let mut lyapunov_values = Vec::new();
    lyapunov_values.push(lyapunov_pd(&initial, &inertia, kp, &target_q));

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        lyapunov_values.push(lyapunov_pd(state, &inertia, kp, &target_q));
    });

    // Check 1: final angle error < 0.01°
    let final_error = angle_error_deg(&final_state, &target_q);
    assert!(
        final_error < 0.01,
        "Final angle error should be < 0.01°, got {final_error:.6}°"
    );

    // Check 2: Lyapunov function should decrease significantly from initial
    let v_initial = lyapunov_values[0];
    let v_final = *lyapunov_values.last().unwrap();
    assert!(
        v_final < v_initial * 0.001,
        "Lyapunov function should decrease significantly: V_0={v_initial:.6e}, V_f={v_final:.6e}"
    );

    // Check 3: second half should have smaller average Lyapunov value (no sustained growth)
    let n = lyapunov_values.len();
    let first_half_avg: f64 = lyapunov_values[..n / 2].iter().sum::<f64>() / (n / 2) as f64;
    let second_half_avg: f64 = lyapunov_values[n / 2..].iter().sum::<f64>() / (n - n / 2) as f64;
    assert!(
        second_half_avg < first_half_avg,
        "Second half avg should be smaller: first={first_half_avg:.6e}, second={second_half_avg:.6e}"
    );
}

// ──────────────────────────────────────────────────────
// Test 2: Overdamped response (no overshoot)
// ──────────────────────────────────────────────────────

#[test]
fn inertial_pd_overdamped_no_overshoot() {
    // I = 10, Kp = 1.0, Kd = 10.0
    // ζ = 10 / (2√10) ≈ 1.58 (overdamped)
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let kp = 1.0;
    let kd = 10.0;
    let target_q = UnitQuaternion::identity();

    let ctrl = InertialPdController::diagonal(kp, kd, target_q);
    let system = AttitudeSystem::new(inertia).with_model(ctrl);

    // Initial condition: 10° rotation about Z
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
    let uq = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial = AttitudeState::new(uq, Vector3::zeros());

    let dt = 0.1;
    let t_end = 100.0;

    // Track angle errors for monotonic decrease check
    let mut prev_error = angle_error_deg(&initial, &target_q);
    let mut monotonic = true;

    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_t, state| {
        let err = angle_error_deg(state, &target_q);
        if err > prev_error + 1e-10 {
            monotonic = false;
        }
        prev_error = err;
    });

    // Check: angle error decreases monotonically (no overshoot)
    assert!(
        monotonic,
        "Overdamped response should have monotonically decreasing error"
    );

    // Also verify convergence
    let final_error = angle_error_deg(&final_state, &target_q);
    assert!(
        final_error < 0.01,
        "Final angle error should be < 0.01°, got {final_error:.6}°"
    );
}

// ──────────────────────────────────────────────────────
// Test 3: DecoupledAttitudeSystem equivalence with AttitudeSystem
// ──────────────────────────────────────────────────────

#[test]
fn decoupled_system_equivalence_gravity_gradient() {
    // Both systems should produce identical results for gravity gradient
    // since GravityGradientTorque uses its internal position_fn (not HasOrbit).
    let mu: f64 = 398600.4418;
    let r: f64 = 7000.0;
    let inertia = diagonal_inertia(10.0, 20.0, 30.0);

    // AttitudeSystem with GravityGradientTorque
    let gg1 = GravityGradientTorque::circular_orbit(mu, r, inertia);
    let system1 = AttitudeSystem::new(inertia).with_model(gg1);

    // DecoupledAttitudeSystem with the same GravityGradientTorque
    let gg2 = GravityGradientTorque::circular_orbit(mu, r, inertia);
    let system2 = DecoupledAttitudeSystem::circular_orbit(inertia, mu, r, 100.0).with_model(gg2);

    // Initial condition: small pitch angle
    let pitch0 = 0.01; // rad
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
    let uq = UnitQuaternion::from_axis_angle(&axis, pitch0);
    let initial = AttitudeState::new(uq, Vector3::zeros());

    let dt = 0.01;
    let t_end = 50.0;

    // Integrate both systems
    let final1 = Rk4.integrate(&system1, initial.clone(), 0.0, t_end, dt, |_, _| {});
    let final2 = Rk4.integrate(&system2, initial, 0.0, t_end, dt, |_, _| {});

    // Compare quaternions
    let q_diff = (final1.quaternion - final2.quaternion).magnitude();
    assert!(
        q_diff < 1e-12,
        "Quaternion difference should be < 1e-12, got {q_diff:.2e}"
    );

    // Compare angular velocities
    let w_diff = (final1.angular_velocity - final2.angular_velocity).magnitude();
    assert!(
        w_diff < 1e-12,
        "Angular velocity difference should be < 1e-12, got {w_diff:.2e}"
    );
}

// ──────────────────────────────────────────────────────
// Test 4: Nadir tracking with PD control + gravity gradient
// ──────────────────────────────────────────────────────

#[test]
fn decoupled_nadir_tracking_bounded_error() {
    // Nadir pointing with TrackingPdController + GravityGradientTorque.
    // The attitude error should stay bounded during one orbit.
    use orts::attitude::AttitudeReference;

    let mu: f64 = 398600.4418; // km^3/s^2
    let r: f64 = 7000.0; // km
    let n = (mu / r.powi(3)).sqrt(); // mean motion [rad/s]
    let v_circ = (mu / r).sqrt();
    let period = 2.0 * PI / n;

    // Symmetric inertia to avoid cross-axis coupling
    let inertia = symmetric_inertia(20.0);

    // PD gains: natural frequency well above orbital rate
    let kp = 0.5;
    let kd = 3.0;

    // Gravity gradient torque
    let gg = GravityGradientTorque::circular_orbit(mu, r, inertia);

    // Tracking PD controller
    let pd = TrackingPdController::diagonal(kp, kd, NadirPointing);

    let system = DecoupledAttitudeSystem::circular_orbit(inertia, mu, r, 100.0)
        .with_model(gg)
        .with_model(pd);

    // Get initial LVLH target
    let orbit0 = orts::OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v_circ, 0.0));
    let (q_target_0, omega_target_0) = NadirPointing.target(0.0, &orbit0, None);

    // Start with 5° perturbation from nadir
    let perturb_axis = nalgebra::Unit::new_normalize(Vector3::new(1.0, 1.0, 1.0));
    let perturb = UnitQuaternion::from_axis_angle(&perturb_axis, 5.0_f64.to_radians());
    let initial_q = perturb * q_target_0;

    // Initial angular velocity: LVLH co-rotation rate (transformed to perturbed body frame)
    let q_target_to_current = initial_q.inverse() * q_target_0;
    let omega_init = q_target_to_current * omega_target_0;
    let initial = AttitudeState::new(initial_q, omega_init);

    // Integrate for one orbit
    let dt = 0.1;
    let t_end = period;
    let mut max_error_deg = 0.0_f64;

    let _ = Rk4.integrate(&system, initial, 0.0, t_end, dt, |t, state| {
        let orbit_t = {
            let theta = n * t;
            orts::OrbitalState::new(
                Vector3::new(r * theta.cos(), r * theta.sin(), 0.0),
                Vector3::new(-v_circ * theta.sin(), v_circ * theta.cos(), 0.0),
            )
        };
        let (q_target_t, _) = NadirPointing.target(t, &orbit_t, None);
        let q_err = state.orientation() * q_target_t.inverse();
        let error_deg = q_err.angle().to_degrees();
        max_error_deg = max_error_deg.max(error_deg);
    });

    assert!(
        max_error_deg < 10.0,
        "Attitude error should stay bounded during one orbit, max was {max_error_deg:.2}°"
    );
}

// ──────────────────────────────────────────────────────
// Test 5: InertialPdController with non-identity target converges
// ──────────────────────────────────────────────────────

#[test]
fn inertial_pd_non_identity_target_converges() {
    // Verify the PD controller works correctly for an arbitrary (non-identity) target quaternion.
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let kp = 1.0;
    let kd = 2.0;

    // Use a 90° rotation about X as target
    let target_q = UnitQuaternion::from_axis_angle(
        &nalgebra::Unit::new_normalize(Vector3::new(1.0, 0.0, 0.0)),
        PI / 2.0,
    );
    let system =
        AttitudeSystem::new(inertia).with_model(InertialPdController::diagonal(kp, kd, target_q));

    // Start with 10° body-frame perturbation about Z
    let perturb = UnitQuaternion::from_axis_angle(
        &nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0)),
        10.0_f64.to_radians(),
    );
    let initial_q = target_q * perturb; // body-frame perturbation
    let initial = AttitudeState::new(initial_q, Vector3::zeros());

    let dt = 0.1;
    let t_end = 100.0;
    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    let final_error = angle_error_deg(&final_state, &target_q);
    assert!(
        final_error < 0.01,
        "InertialPd with 90° target should converge, got {final_error:.4}°"
    );
}

// ──────────────────────────────────────────────────────
// Test 6: TrackingPdController with InertialPointing converges
// ──────────────────────────────────────────────────────

#[test]
fn tracking_pd_with_inertial_pointing_converges() {
    // Use TrackingPdController with InertialPointing (omega_target = 0)
    // This should behave identically to InertialPdController.
    use orts::attitude::InertialPointing;

    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let target_q = UnitQuaternion::identity();
    let kp = 1.0;
    let kd = 2.0;

    let ref_point = InertialPointing { target_q };
    let ctrl = TrackingPdController::diagonal(kp, kd, ref_point);

    let mu = 398600.4418;
    let r = 7000.0;
    let system = DecoupledAttitudeSystem::circular_orbit(inertia, mu, r, 100.0).with_model(ctrl);

    // Initial condition: 10° rotation about Z
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
    let uq = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial = AttitudeState::new(uq, Vector3::zeros());

    let dt = 0.1;
    let t_end = 100.0;
    let final_state = Rk4.integrate(&system, initial, 0.0, t_end, dt, |_, _| {});

    let final_error = angle_error_deg(&final_state, &target_q);
    assert!(
        final_error < 0.01,
        "TrackingPd + InertialPointing should converge, got {final_error:.6}°"
    );
}

// ──────────────────────────────────────────────────────
// Test 7: PD controller model names
// ──────────────────────────────────────────────────────

#[test]
fn model_names_reported_correctly() {
    let inertia = symmetric_inertia(10.0);
    let target_q = UnitQuaternion::identity();

    let ctrl = InertialPdController::diagonal(1.0, 2.0, target_q);
    let system = AttitudeSystem::new(inertia).with_model(ctrl);
    assert_eq!(system.model_names(), vec!["pd_inertial"]);

    let pd = TrackingPdController::diagonal(1.0, 2.0, NadirPointing);
    let system2 =
        DecoupledAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0).with_model(pd);
    assert_eq!(system2.model_names(), vec!["pd_tracking"]);
}
