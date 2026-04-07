use std::f64::consts::PI;

use nalgebra::{Matrix3, Vector3};
use utsuroi::{Integrator, Rk4};

use kaname::Eci;
use kaname::constants::{MU_EARTH, R_EARTH};
use kaname::epoch::Epoch;
use orts::attitude::{
    AttitudeState, BdotDetumbler, BdotFiniteDiff, CommandedMagnetorquer, DecoupledAttitudeSystem,
};
use orts::control::DiscreteController;
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

fn test_epoch() -> Epoch {
    Epoch::j2000()
}

/// Rotational kinetic energy T = 0.5 * omega . (I . omega)
fn rotational_energy(state: &AttitudeState, inertia: &Matrix3<f64>) -> f64 {
    0.5 * state
        .angular_velocity
        .dot(&(inertia * state.angular_velocity))
}

/// Segment-by-segment simulation loop for a discrete B-dot controller.
///
/// Rebuilds the `DecoupledAttitudeSystem` each control segment so the
/// `CommandedMagnetorquer` holds the latest command from the controller.
fn simulate_discrete_bdot(
    inertia: Matrix3<f64>,
    mu: f64,
    radius: f64,
    mass: f64,
    controller: &mut BdotFiniteDiff,
    initial: AttitudeState,
    t_end: f64,
    dt_ode: f64,
    epoch: Epoch,
    mut callback: impl FnMut(f64, &AttitudeState),
) -> AttitudeState {
    let dt_ctrl = controller.sample_period();
    let mut state = initial;
    let mut cmd = controller.initial_command();
    let mut t = 0.0;

    // Orbital parameters for circular orbit
    let n = (mu / radius.powi(3)).sqrt();
    let v = (mu / radius).sqrt();

    while t < t_end - 1e-12 {
        let t_next = (t + dt_ctrl).min(t_end);

        // Build system with current commanded moment frozen for this segment
        let actuator = CommandedMagnetorquer::new(cmd.clone(), TiltedDipole::earth());
        let system = DecoupledAttitudeSystem::circular_orbit(inertia, mu, radius, mass)
            .with_model(actuator)
            .with_epoch(epoch);

        // Integrate this segment
        state = Rk4.integrate(&system, state, t, t_next, dt_ode, |t_step, s| {
            callback(t_step, s);
        });

        t = t_next;

        // Controller samples at the end of each segment
        let theta = n * t;
        let orbit_at_t = orts::OrbitalState::new(
            Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
            Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
        );
        let current_epoch = epoch.add_seconds(t);
        cmd = controller.update(t, &state, &orbit_at_t, Some(&current_epoch));
    }

    state
}

// ------
// Test 1: BdotFiniteDiff first call returns zero
// ------

#[test]
fn bdot_finite_diff_first_call_returns_zero() {
    let mut ctrl = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        1.0,
    );

    let attitude = AttitudeState::new(
        nalgebra::UnitQuaternion::identity(),
        Vector3::new(0.1, 0.2, 0.05),
    );
    let orbit = orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros());
    let epoch = test_epoch();

    let cmd = ctrl.update(0.0, &attitude, &orbit, Some(&epoch));
    assert_eq!(
        cmd,
        Vector3::zeros(),
        "First call should return zero (no previous measurement)"
    );
}

// ------
// Test 2: BdotFiniteDiff second call returns non-zero
// ------

#[test]
fn bdot_finite_diff_second_call_nonzero() {
    let mut ctrl = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        1.0,
    );
    let epoch = test_epoch();

    let attitude1 = AttitudeState::new(
        nalgebra::UnitQuaternion::identity(),
        Vector3::new(0.1, 0.2, 0.05),
    );
    let orbit1 = orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros());
    let _ = ctrl.update(0.0, &attitude1, &orbit1, Some(&epoch));

    // Rotate a bit for the second call
    let axis = nalgebra::Unit::new_normalize(Vector3::new(0.1, 0.2, 0.05));
    let uq = nalgebra::UnitQuaternion::from_axis_angle(&axis, 0.1);
    let attitude2 = AttitudeState::new(uq, Vector3::new(0.1, 0.2, 0.05));
    let orbit2 = orts::OrbitalState::new(Vector3::new(6999.0, 100.0, 0.0), Vector3::zeros());
    let epoch2 = epoch.add_seconds(1.0);
    let cmd = ctrl.update(1.0, &attitude2, &orbit2, Some(&epoch2));

    assert!(
        cmd.magnitude() > 0.0,
        "Second call should produce non-zero command"
    );
}

// ------
// Test 3: BdotFiniteDiff moment clamping
// ------

#[test]
fn bdot_finite_diff_clamping() {
    let max_m = 0.001;
    let mut ctrl = BdotFiniteDiff::new(
        1e12, // huge gain to ensure clamping
        Vector3::new(max_m, max_m, max_m),
        TiltedDipole::earth(),
        1.0,
    );
    let epoch = test_epoch();

    let attitude1 = AttitudeState::new(
        nalgebra::UnitQuaternion::identity(),
        Vector3::new(0.5, 0.5, 0.5),
    );
    let orbit1 = orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros());
    let _ = ctrl.update(0.0, &attitude1, &orbit1, Some(&epoch));

    // Large rotation change for big dB/dt
    let axis = nalgebra::Unit::new_normalize(Vector3::new(1.0, 1.0, 1.0));
    let uq = nalgebra::UnitQuaternion::from_axis_angle(&axis, 1.0);
    let attitude2 = AttitudeState::new(uq, Vector3::new(0.5, 0.5, 0.5));
    let orbit2 = orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros());
    let epoch2 = epoch.add_seconds(1.0);
    let cmd = ctrl.update(1.0, &attitude2, &orbit2, Some(&epoch2));

    for i in 0..3 {
        assert!(
            cmd[i].abs() <= max_m + 1e-15,
            "Component {i} should be clamped to {max_m}, got {}",
            cmd[i].abs()
        );
    }
}

// ------
// Test 4: CommandedMagnetorquer produces correct torque direction
// ------

#[test]
fn commanded_magnetorquer_torque_is_m_cross_b() {
    use orts::model::Model;

    let m_cmd = Vector3::new(0.1, -0.2, 0.3);
    let actuator = CommandedMagnetorquer::new(m_cmd, TiltedDipole::earth());
    let epoch = test_epoch();

    let state = orts::attitude::DecoupledContext {
        attitude: AttitudeState::identity(),
        orbit: orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        mass: 100.0,
    };

    let loads = actuator.eval(0.0, &state, Some(&epoch));

    // Manually compute expected torque
    let b_eci = TiltedDipole::earth().field_eci(&Eci::new(7000.0, 0.0, 0.0), &epoch);
    let b_body = state
        .attitude
        .rotation_to_body()
        .transform(&b_eci)
        .into_inner();
    let expected_torque = m_cmd.cross(&b_body);

    let diff = (loads.torque_body.into_inner() - expected_torque).magnitude();
    assert!(
        diff < 1e-20,
        "Torque should be m x B, difference: {diff:.2e}"
    );
}

// ------
// Test 5: CommandedMagnetorquer zero command gives zero torque
// ------

#[test]
fn commanded_magnetorquer_zero_moment_zero_torque() {
    use orts::model::Model;

    let actuator = CommandedMagnetorquer::new(Vector3::zeros(), TiltedDipole::earth());
    let epoch = test_epoch();
    let state = orts::attitude::DecoupledContext {
        attitude: AttitudeState::identity(),
        orbit: orts::OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        mass: 100.0,
    };
    let loads = actuator.eval(0.0, &state, Some(&epoch));
    assert!(
        loads.torque_body.magnitude() < 1e-20,
        "Zero moment should give zero torque"
    );
}

// ------
// Test 6: Discrete B-dot reduces angular velocity (1 orbit)
// ------

#[test]
fn discrete_bdot_reduces_angular_velocity_one_orbit() {
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let altitude = 400.0;
    let radius = R_EARTH + altitude;
    let n = (MU_EARTH / radius.powi(3)).sqrt();
    let period = 2.0 * PI / n;

    let omega0 = Vector3::new(0.1, 0.2, 0.05);
    let omega0_mag = omega0.magnitude();
    let initial = AttitudeState::new(nalgebra::UnitQuaternion::identity(), omega0);

    let dt_ctrl = 1.0; // 1 Hz control
    let mut controller = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        dt_ctrl,
    );

    let e0 = rotational_energy(&initial, &inertia);
    let dt_ode = 0.5;
    let t_end = period; // 1 orbit

    let final_state = simulate_discrete_bdot(
        inertia,
        MU_EARTH,
        radius,
        500.0,
        &mut controller,
        initial,
        t_end,
        dt_ode,
        test_epoch(),
        |_, _| {},
    );

    let final_omega_mag = final_state.angular_velocity.magnitude();
    let final_energy = rotational_energy(&final_state, &inertia);

    // Energy should decrease
    assert!(
        final_energy < e0,
        "Energy should decrease: E_0={e0:.6e}, E_f={final_energy:.6e}"
    );

    // Angular velocity should decrease (at least somewhat after 1 orbit)
    assert!(
        final_omega_mag < omega0_mag,
        "Discrete B-dot should reduce |omega| after 1 orbit: \
         initial={omega0_mag:.4}, final={final_omega_mag:.4}"
    );
}

// ------
// Test 7: Discrete B-dot reduces angular velocity (3 orbits)
// ------

#[test]
fn discrete_bdot_reduces_angular_velocity_three_orbits() {
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let altitude = 400.0;
    let radius = R_EARTH + altitude;
    let n = (MU_EARTH / radius.powi(3)).sqrt();
    let period = 2.0 * PI / n;

    let omega0 = Vector3::new(0.1, 0.2, 0.05);
    let omega0_mag = omega0.magnitude();
    let initial = AttitudeState::new(nalgebra::UnitQuaternion::identity(), omega0);

    let dt_ctrl = 1.0;
    let mut controller = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        dt_ctrl,
    );

    let dt_ode = 0.5;
    let t_end = 3.0 * period;

    let final_state = simulate_discrete_bdot(
        inertia,
        MU_EARTH,
        radius,
        500.0,
        &mut controller,
        initial,
        t_end,
        dt_ode,
        test_epoch(),
        |_, _| {},
    );

    let final_omega_mag = final_state.angular_velocity.magnitude();

    // After 3 orbits, should reduce angular velocity by at least half
    // (discrete controller has a one-step delay so is slightly less efficient
    //  than the stateless analytical version)
    assert!(
        final_omega_mag < 0.5 * omega0_mag,
        "Discrete B-dot should reduce |omega| by at least half after 3 orbits: \
         initial={omega0_mag:.4}, final={final_omega_mag:.4}"
    );
}

// ------
// Test 8: Compare stateless vs finite-diff B-dot
// ------

#[test]
fn stateless_and_discrete_bdot_both_converge() {
    let i_val = 10.0;
    let inertia = symmetric_inertia(i_val);
    let altitude = 400.0;
    let radius = R_EARTH + altitude;
    let n = (MU_EARTH / radius.powi(3)).sqrt();
    let period = 2.0 * PI / n;

    let omega0 = Vector3::new(0.1, 0.2, 0.05);
    let omega0_mag = omega0.magnitude();
    let initial = AttitudeState::new(nalgebra::UnitQuaternion::identity(), omega0);

    let gain = 1e6;
    let max_moment = Vector3::new(10.0, 10.0, 10.0);
    let dt_ode = 0.5;
    let t_end = 3.0 * period; // 3 orbits for comfortable margin

    // --- Stateless (analytical) B-dot ---
    let bdot_stateless = BdotDetumbler::new(gain, max_moment, TiltedDipole::earth());
    let system = DecoupledAttitudeSystem::circular_orbit(inertia, MU_EARTH, radius, 500.0)
        .with_model(bdot_stateless)
        .with_epoch(test_epoch());
    let final_stateless = Rk4.integrate(&system, initial.clone(), 0.0, t_end, dt_ode, |_, _| {});
    let omega_stateless = final_stateless.angular_velocity.magnitude();

    // --- Discrete (finite-diff) B-dot ---
    let dt_ctrl = 1.0;
    let mut controller = BdotFiniteDiff::new(gain, max_moment, TiltedDipole::earth(), dt_ctrl);
    let final_discrete = simulate_discrete_bdot(
        inertia,
        MU_EARTH,
        radius,
        500.0,
        &mut controller,
        initial,
        t_end,
        dt_ode,
        test_epoch(),
        |_, _| {},
    );
    let omega_discrete = final_discrete.angular_velocity.magnitude();

    // Both should converge significantly (at least halved over 3 orbits)
    assert!(
        omega_stateless < 0.5 * omega0_mag,
        "Stateless B-dot should reduce |omega| by at least half: \
         initial={omega0_mag:.4}, final={omega_stateless:.4}"
    );
    assert!(
        omega_discrete < 0.5 * omega0_mag,
        "Discrete B-dot should reduce |omega| by at least half: \
         initial={omega0_mag:.4}, final={omega_discrete:.4}"
    );

    // The stateless version should be at least as efficient (it has no first-step delay)
    // But both should achieve meaningful detumbling
    println!(
        "Stateless final |omega|: {omega_stateless:.6}, Discrete final |omega|: {omega_discrete:.6}"
    );
}

// ------
// Test 9: DiscreteController trait sample_period
// ------

#[test]
fn discrete_controller_sample_period() {
    let ctrl = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        2.5,
    );
    assert!((ctrl.sample_period() - 2.5).abs() < 1e-15);
}

// ------
// Test 10: DiscreteController initial_command is zero
// ------

#[test]
fn discrete_controller_initial_command_zero() {
    let ctrl = BdotFiniteDiff::new(
        1e6,
        Vector3::new(10.0, 10.0, 10.0),
        TiltedDipole::earth(),
        1.0,
    );
    assert_eq!(ctrl.initial_command(), Vector3::zeros());
}
