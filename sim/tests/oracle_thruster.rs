//! Oracle tests for the Thruster LoadModel.
//!
//! Validates against analytical solutions:
//! - Tsiolkovsky rocket equation (ΔV)
//! - Linear mass depletion
//! - Propellant exhaustion failsafe
//! - Torque spin-up
//! - RK4 dt convergence (4th order)

use nalgebra::{Matrix3, Vector3};
use orts_attitude::AttitudeState;
use orts_integrator::{Integrator, Rk4, State};
use orts_orbits::gravity::PointMass;
use orts_sim::spacecraft::{
    BurnWindow, ScheduledBurn, SpacecraftDynamics, SpacecraftState, Thruster, G0,
};

/// Free-space SpacecraftDynamics (negligible gravity).
///
/// Uses a tiny μ so gravity is effectively zero, letting us isolate thrust effects.
fn free_space_dynamics(
    inertia: Matrix3<f64>,
    thruster: Thruster,
) -> SpacecraftDynamics<PointMass> {
    SpacecraftDynamics::new(1e-30, PointMass, inertia).with_load(Box::new(thruster))
}

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

fn identity_spacecraft(mass: f64) -> SpacecraftState {
    SpacecraftState {
        orbit: State {
            position: Vector3::new(1e12, 0.0, 0.0), // far from origin → negligible gravity
            velocity: Vector3::zeros(),
        },
        attitude: AttitudeState::identity(),
        mass,
    }
}

// =====================================================================
// Tsiolkovsky rocket equation: ΔV = Isp × g₀ × ln(m₀/m_f)
// =====================================================================

#[test]
fn tsiolkovsky_constant_thrust() {
    let thrust = 10.0; // N
    let isp = 300.0; // s
    let m0 = 1000.0; // kg
    let burn_time = 1000.0; // s

    let mass_rate = thrust / (isp * G0); // kg/s (positive)
    let mf = m0 - mass_rate * burn_time;
    assert!(mf > 0.0, "sanity: final mass must be positive");

    // Analytical ΔV [km/s]
    let dv_analytical = isp * G0 * (m0 / mf).ln() / 1000.0;

    // Numerical: constant thrust along +X, identity attitude, no offset
    let thruster = Thruster::new(thrust, isp, Vector3::x());
    let dynamics = free_space_dynamics(symmetric_inertia(10.0), thruster);

    let dt = 0.1;
    let state0 = identity_spacecraft(m0);
    let result = Rk4.integrate(&dynamics, state0, 0.0, burn_time, dt, |_, _| {});

    let dv_numerical = result.orbit.velocity.magnitude();
    let rel_err = (dv_numerical - dv_analytical).abs() / dv_analytical;

    assert!(
        rel_err < 1e-4,
        "Tsiolkovsky ΔV: numerical={dv_numerical:.6e} km/s, analytical={dv_analytical:.6e} km/s, rel_err={rel_err:.3e}"
    );

    // Also verify final mass
    let mass_rel_err = (result.mass - mf).abs() / mf;
    assert!(
        mass_rel_err < 1e-8,
        "Final mass: numerical={:.4}, analytical={mf:.4}, rel_err={mass_rel_err:.3e}",
        result.mass
    );
}

// =====================================================================
// Mass depletion: m(t) = m₀ - F/(Isp×g₀)×t (linear for constant thrust)
// =====================================================================

#[test]
fn mass_depletion_linear() {
    let thrust = 5.0;
    let isp = 250.0;
    let m0 = 500.0;
    let mass_rate = thrust / (isp * G0); // kg/s

    let thruster = Thruster::new(thrust, isp, Vector3::x());
    let dynamics = free_space_dynamics(symmetric_inertia(10.0), thruster);

    let dt = 0.1;
    let state0 = identity_spacecraft(m0);

    // Check mass at several intermediate times
    for &check_t in &[10.0, 50.0, 100.0, 200.0] {
        let result = Rk4.integrate(&dynamics, state0.clone(), 0.0, check_t, dt, |_, _| {});
        let m_analytical = m0 - mass_rate * check_t;
        let rel_err = (result.mass - m_analytical).abs() / m_analytical;
        assert!(
            rel_err < 1e-8,
            "t={check_t}: mass numerical={:.6}, analytical={m_analytical:.6}, rel_err={rel_err:.3e}",
            result.mass
        );
    }
}

// =====================================================================
// Propellant exhaustion: dry_mass stops thrust
// =====================================================================

#[test]
fn propellant_exhaustion_stops_thrust() {
    let thrust = 100.0;
    let isp = 200.0;
    let m0 = 110.0;
    let dry_mass = 100.0;
    let mass_rate = thrust / (isp * G0);

    // Burn time to reach dry_mass: (m0 - dry_mass) / mass_rate
    let burn_time = (m0 - dry_mass) / mass_rate;

    let thruster = Thruster::new(thrust, isp, Vector3::x()).with_dry_mass(dry_mass);
    let dynamics = free_space_dynamics(symmetric_inertia(10.0), thruster);

    // Integrate well past the expected exhaustion time
    let total_time = burn_time * 3.0;
    let dt = 0.01;
    let state0 = identity_spacecraft(m0);
    let result = Rk4.integrate(&dynamics, state0, 0.0, total_time, dt, |_, _| {});

    // Mass should be near dry_mass (not below)
    assert!(
        result.mass >= dry_mass - 0.1,
        "Mass should not drop significantly below dry_mass: got {}, dry_mass={}",
        result.mass,
        dry_mass
    );

    // Velocity should stop increasing after burn — compare v at 2T vs 3T
    let dynamics2 = free_space_dynamics(
        symmetric_inertia(10.0),
        Thruster::new(thrust, isp, Vector3::x()).with_dry_mass(dry_mass),
    );
    let state0b = identity_spacecraft(m0);
    let r2 = Rk4.integrate(&dynamics2, state0b, 0.0, burn_time * 2.0, dt, |_, _| {});
    let v_diff = (result.orbit.velocity - r2.orbit.velocity).magnitude();
    assert!(
        v_diff < 1e-10,
        "Velocity should not increase after propellant exhaustion: |v(3T)-v(2T)| = {v_diff:.3e}"
    );
}

// =====================================================================
// Torque spin-up: offset thruster → linear angular velocity increase
// =====================================================================

#[test]
fn torque_spin_up() {
    // Thruster at offset [0, 1, 0] m, force along +X → τ = [0,0,-F] N·m
    // Symmetric inertia I=100 → α = τ/I = [0,0,-F/100] rad/s²
    // ω(t) = α × t
    let thrust = 1.0; // N
    let isp = 300.0;
    let inertia_val = 100.0;
    let t_final = 10.0;

    let thruster = Thruster::new(thrust, isp, Vector3::x())
        .with_offset(Vector3::new(0.0, 1.0, 0.0));
    let dynamics = free_space_dynamics(symmetric_inertia(inertia_val), thruster);

    let dt = 0.01;
    // Start with zero angular velocity
    let state0 = identity_spacecraft(500.0);
    let result = Rk4.integrate(&dynamics, state0, 0.0, t_final, dt, |_, _| {});

    // Expected: ωz = -F * t / I (constant torque, approximately — mass changes slightly)
    // For small mass change ratio, this is very close to linear
    let expected_omega_z = -thrust * t_final / inertia_val;
    let rel_err = (result.attitude.angular_velocity[2] - expected_omega_z).abs()
        / expected_omega_z.abs();

    assert!(
        rel_err < 1e-3,
        "ωz: numerical={:.6e}, expected={expected_omega_z:.6e}, rel_err={rel_err:.3e}",
        result.attitude.angular_velocity[2]
    );

    // ωx and ωy should be ~0
    assert!(
        result.attitude.angular_velocity[0].abs() < 1e-10,
        "ωx should be ~0"
    );
    assert!(
        result.attitude.angular_velocity[1].abs() < 1e-10,
        "ωy should be ~0"
    );
}

// =====================================================================
// dt convergence: RK4 4th-order accuracy (error ratio ≈ 16)
// =====================================================================

#[test]
fn dt_convergence_tsiolkovsky() {
    // Use a large mass-ratio burn so truncation error is visible at coarse dt.
    let thrust = 1000.0; // N (larger thrust → larger mass change → more nonlinearity)
    let isp = 200.0;
    let m0 = 500.0;
    let burn_time = 100.0;

    let mass_rate = thrust / (isp * G0);
    let mf = m0 - mass_rate * burn_time;
    assert!(mf > 0.0, "sanity: final mass must be positive");
    let dv_exact = isp * G0 * (m0 / mf).ln() / 1000.0; // km/s

    let mut errors = Vec::new();
    for &dt in &[8.0, 4.0, 2.0] {
        let thruster = Thruster::new(thrust, isp, Vector3::x());
        let dynamics = free_space_dynamics(symmetric_inertia(10.0), thruster);
        let state0 = identity_spacecraft(m0);
        let result = Rk4.integrate(&dynamics, state0, 0.0, burn_time, dt, |_, _| {});
        let dv = result.orbit.velocity.magnitude();
        errors.push((dv - dv_exact).abs());
    }

    // Error ratio for 4th-order: e(dt) / e(dt/2) ≈ 16
    let ratio1 = errors[0] / errors[1];
    let ratio2 = errors[1] / errors[2];

    assert!(
        ratio1 > 14.0 && ratio1 < 18.0,
        "dt convergence ratio 1: {ratio1:.2} (expected ~16). errors: {errors:?}",
    );
    assert!(
        ratio2 > 14.0 && ratio2 < 18.0,
        "dt convergence ratio 2: {ratio2:.2} (expected ~16). errors: {errors:?}",
    );
}

// =====================================================================
// Scheduled burn: ΔV matches expected for a timed burn
// =====================================================================

#[test]
fn scheduled_burn_delta_v() {
    let thrust = 10.0;
    let isp = 300.0;
    let m0 = 1000.0;
    let burn_start = 50.0;
    let burn_end = 150.0;
    let burn_duration = burn_end - burn_start;

    let mass_rate = thrust / (isp * G0);
    let mf = m0 - mass_rate * burn_duration;
    let dv_analytical = isp * G0 * (m0 / mf).ln() / 1000.0;

    let thruster = Thruster::new(thrust, isp, Vector3::x()).with_profile(Box::new(ScheduledBurn {
        windows: vec![BurnWindow::full(burn_start, burn_end)],
    }));
    let dynamics = free_space_dynamics(symmetric_inertia(10.0), thruster);

    // Use small dt to minimize error from the step straddling burn boundaries.
    let dt = 0.01;
    let state0 = identity_spacecraft(m0);
    // Integrate past the burn end to verify coast
    let result = Rk4.integrate(&dynamics, state0, 0.0, 200.0, dt, |_, _| {});

    let dv = result.orbit.velocity.magnitude();
    let rel_err = (dv - dv_analytical).abs() / dv_analytical;

    assert!(
        rel_err < 5e-4,
        "Scheduled burn ΔV: numerical={dv:.6e}, analytical={dv_analytical:.6e}, rel_err={rel_err:.3e}"
    );

    // Mass should not change after burn end (boundary effect may cause small error)
    let mass_after_burn = m0 - mass_rate * burn_duration;
    let mass_rel_err = (result.mass - mass_after_burn).abs() / mass_after_burn;
    assert!(
        mass_rel_err < 1e-6,
        "Mass after coast: {:.10} vs expected {mass_after_burn:.10}, rel_err={mass_rel_err:.3e}",
        result.mass
    );
}
