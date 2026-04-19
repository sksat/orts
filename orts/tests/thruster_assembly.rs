//! Integration tests for ThrusterAssembly (plugin-commanded thruster group).
//!
//! Validates that ThrusterAssembly produces the same physical results as
//! the host-scheduled Thruster when used with SpacecraftDynamics + RK4.

use nalgebra::{Matrix3, Vector3};
use orts::OrbitalState;
use orts::attitude::AttitudeState;
use orts::orbital::gravity::PointMass;
use orts::plugin::ThrusterCommand;
use orts::spacecraft::{
    G0, SpacecraftDynamics, SpacecraftState, ThrusterAssembly, ThrusterAssemblyCore, ThrusterSpec,
};
use utsuroi::{Integrator, Rk4};

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

fn identity_spacecraft(mass: f64) -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(
            Vector3::new(1e12, 0.0, 0.0), // far from origin → negligible gravity
            Vector3::zeros(),
        ),
        attitude: AttitudeState::identity(),
        mass,
    }
}

/// Tsiolkovsky rocket equation using ThrusterAssembly instead of Thruster.
#[test]
fn assembly_tsiolkovsky() {
    let thrust = 10.0; // N
    let isp = 300.0; // s
    let m0 = 1000.0; // kg
    let burn_time = 1000.0; // s

    let mass_rate = thrust / (isp * G0);
    let mf = m0 - mass_rate * burn_time;
    let dv_analytical = isp * G0 * (m0 / mf).ln() / 1000.0; // km/s

    let spec = ThrusterSpec::new(thrust, isp, Vector3::x());
    let core = ThrusterAssemblyCore::new(vec![spec], 0.0);
    let mut asm = ThrusterAssembly::new(core);
    asm.command = ThrusterCommand::Throttles(vec![1.0]);

    let dynamics =
        SpacecraftDynamics::new(1e-30, PointMass, symmetric_inertia(10.0)).with_model(asm);

    let state0 = identity_spacecraft(m0);
    let result = Rk4.integrate(&dynamics, state0.into(), 0.0, burn_time, 0.1, |_, _| {});

    let dv_numerical = result.plant.orbit.velocity().magnitude();
    let rel_err = (dv_numerical - dv_analytical).abs() / dv_analytical;
    assert!(
        rel_err < 1e-6,
        "Tsiolkovsky relative error {rel_err:.2e} exceeds 1e-6"
    );
}

/// Two thrusters in opposite directions cancel force but deplete mass at 2×.
#[test]
fn assembly_opposing_thrusters_mass_depletion() {
    let thrust = 10.0;
    let isp = 300.0;
    let m0 = 1000.0;
    let burn_time = 100.0;

    let core = ThrusterAssemblyCore::new(
        vec![
            ThrusterSpec::new(thrust, isp, Vector3::x()),
            ThrusterSpec::new(thrust, isp, -Vector3::x()),
        ],
        0.0,
    );
    let mut asm = ThrusterAssembly::new(core);
    asm.command = ThrusterCommand::Throttles(vec![1.0, 1.0]);

    let dynamics =
        SpacecraftDynamics::new(1e-30, PointMass, symmetric_inertia(10.0)).with_model(asm);

    let state0 = identity_spacecraft(m0);
    let result = Rk4.integrate(&dynamics, state0.into(), 0.0, burn_time, 0.1, |_, _| {});

    // Net velocity should be ~zero (forces cancel)
    let speed = result.plant.orbit.velocity().magnitude();
    assert!(speed < 1e-10, "velocity should be ~0, got {speed:.2e}");

    // Mass should deplete at 2× rate
    let expected_mass = m0 - 2.0 * thrust / (isp * G0) * burn_time;
    let rel_err = (result.plant.mass - expected_mass).abs() / expected_mass;
    assert!(
        rel_err < 1e-6,
        "mass relative error {rel_err:.2e} exceeds 1e-6"
    );
}

/// Multi-direction thrusters produce combined acceleration.
#[test]
fn assembly_multi_direction() {
    let thrust = 10.0;
    let isp = 300.0;
    let m0 = 1000.0;
    let burn_time = 10.0;

    // X and Y thrusters at half throttle each
    let core = ThrusterAssemblyCore::new(
        vec![
            ThrusterSpec::new(thrust, isp, Vector3::x()),
            ThrusterSpec::new(thrust, isp, Vector3::y()),
        ],
        0.0,
    );
    let mut asm = ThrusterAssembly::new(core);
    asm.command = ThrusterCommand::Throttles(vec![0.5, 0.5]);

    let dynamics =
        SpacecraftDynamics::new(1e-30, PointMass, symmetric_inertia(10.0)).with_model(asm);

    let state0 = identity_spacecraft(m0);
    let result = Rk4.integrate(&dynamics, state0.into(), 0.0, burn_time, 0.01, |_, _| {});

    let vel = result.plant.orbit.velocity();
    // Both axes should have similar velocity (symmetric thrust)
    let ratio = vel.x / vel.y;
    assert!(
        (ratio - 1.0).abs() < 0.01,
        "velocity ratio x/y should be ~1, got {ratio:.4}"
    );
}
