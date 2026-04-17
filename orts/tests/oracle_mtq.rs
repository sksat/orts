//! Oracle tests for MTQ assembly dynamics.
//!
//! Verifies that `MtqAssembly` registered in a dynamics system produces
//! physical torque effects on the spacecraft attitude.

use nalgebra::{Matrix3, Vector3};
use utsuroi::{Integrator, Rk4};

use arika::epoch::Epoch;
use orts::attitude::AttitudeState;
use orts::orbital::OrbitalState;
use orts::spacecraft::{MtqAssembly, MtqCommand, SpacecraftDynamics, SpacecraftState};
use tobari::magnetic::TiltedDipole;

const MU_EARTH: f64 = 398600.4418;

fn symmetric_inertia(i: f64) -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(i, i, i))
}

fn leo_state(mass: f64) -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState::identity(),
        mass,
    }
}

/// MtqAssembly with nonzero commanded moments produces angular velocity
/// change when integrated with SpacecraftDynamics.
#[test]
fn mtq_assembly_produces_torque_in_dynamics() {
    let inertia = symmetric_inertia(10.0);

    let mut mtq = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
    mtq.command = MtqCommand::Moments(vec![5.0, 0.0, 0.0]);

    let dynamics = SpacecraftDynamics::new(MU_EARTH, orts::orbital::gravity::PointMass, inertia)
        .with_model(mtq)
        .with_epoch(Epoch::j2000());

    let initial = dynamics.initial_augmented_state(leo_state(100.0));
    let final_state = Rk4.integrate(&dynamics, initial, 0.0, 10.0, 0.1, |_, _| {});

    let omega_mag = final_state.plant.attitude.angular_velocity.magnitude();
    assert!(
        omega_mag > 1e-10,
        "MTQ should produce angular velocity change, got |omega|={omega_mag:.3e}"
    );
}

/// MtqAssembly with zero commanded moments produces no torque.
#[test]
fn mtq_assembly_zero_command_no_effect() {
    let inertia = symmetric_inertia(10.0);

    let mtq = MtqAssembly::three_axis(10.0, TiltedDipole::earth());

    let dynamics = SpacecraftDynamics::new(MU_EARTH, orts::orbital::gravity::PointMass, inertia)
        .with_model(mtq)
        .with_epoch(Epoch::j2000());

    let initial = dynamics.initial_augmented_state(leo_state(100.0));
    let final_state = Rk4.integrate(&dynamics, initial, 0.0, 10.0, 0.1, |_, _| {});

    let omega_mag = final_state.plant.attitude.angular_velocity.magnitude();
    assert!(
        omega_mag < 1e-15,
        "Zero MTQ command should produce no angular velocity change, got |omega|={omega_mag:.3e}"
    );
}

/// MtqAssembly torque is bounded by clamped moment × B field magnitude.
#[test]
fn mtq_assembly_clamping_limits_effect() {
    let inertia = symmetric_inertia(10.0);

    // Huge command with small max → clamped
    let mut mtq_clamped = MtqAssembly::three_axis(0.001, TiltedDipole::earth());
    mtq_clamped.command = MtqCommand::Moments(vec![1000.0, 1000.0, 1000.0]);

    let dynamics_clamped =
        SpacecraftDynamics::new(MU_EARTH, orts::orbital::gravity::PointMass, inertia)
            .with_model(mtq_clamped)
            .with_epoch(Epoch::j2000());

    // Reference: commanded = max_moment (no clamp needed)
    let mut mtq_ref = MtqAssembly::three_axis(0.001, TiltedDipole::earth());
    mtq_ref.command = MtqCommand::Moments(vec![0.001, 0.001, 0.001]);

    let dynamics_ref =
        SpacecraftDynamics::new(MU_EARTH, orts::orbital::gravity::PointMass, inertia)
            .with_model(mtq_ref)
            .with_epoch(Epoch::j2000());

    let initial_c = dynamics_clamped.initial_augmented_state(leo_state(100.0));
    let initial_r = dynamics_ref.initial_augmented_state(leo_state(100.0));

    let final_clamped = Rk4.integrate(&dynamics_clamped, initial_c, 0.0, 10.0, 0.1, |_, _| {});
    let final_ref = Rk4.integrate(&dynamics_ref, initial_r, 0.0, 10.0, 0.1, |_, _| {});

    let diff = (final_clamped.plant.attitude.angular_velocity
        - final_ref.plant.attitude.angular_velocity)
        .magnitude();
    assert!(
        diff < 1e-14,
        "Clamped and reference should match, diff={diff:.3e}"
    );
}

/// replace_model swaps the MTQ assembly between integration segments.
#[test]
fn replace_model_updates_mtq_command() {
    let inertia = symmetric_inertia(10.0);

    let mtq = MtqAssembly::three_axis(10.0, TiltedDipole::earth());

    let mut dynamics =
        SpacecraftDynamics::new(MU_EARTH, orts::orbital::gravity::PointMass, inertia)
            .with_model(mtq)
            .with_epoch(Epoch::j2000());

    let initial = dynamics.initial_augmented_state(leo_state(100.0));

    // First segment: zero command → no effect
    let mid = Rk4.integrate(&dynamics, initial, 0.0, 5.0, 0.1, |_, _| {});
    let omega_mid = mid.plant.attitude.angular_velocity.magnitude();
    assert!(
        omega_mid < 1e-15,
        "Zero command segment should have no effect"
    );

    // Replace model with nonzero command
    let mut mtq_active = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
    mtq_active.command = MtqCommand::Moments(vec![5.0, 0.0, 0.0]);
    dynamics.replace_model("mtq_assembly", Box::new(mtq_active));

    // Second segment: nonzero command → produces effect
    let final_state = Rk4.integrate(&dynamics, mid, 5.0, 10.0, 0.1, |_, _| {});
    let omega_final = final_state.plant.attitude.angular_velocity.magnitude();
    assert!(
        omega_final > 1e-10,
        "After replace_model, MTQ should produce effect, got |omega|={omega_final:.3e}"
    );
}
