//! The state invariants declared through [`utsuroi::OdeState::project`] must
//! hold no matter which integrator runs the simulation.
//!
//! Reaction wheel momentum saturation is enforced *only* by the `aux_bounds`
//! projection of `AugmentedState`: the wheel model zeroes torque in the
//! saturating direction, which bounds the overshoot within a step but does not
//! remove it. The default integrator is `dp45`, so the adaptive steppers have
//! to honour the projection as well as `Rk4` does.

use std::ops::ControlFlow;

use nalgebra::{Matrix3, Vector3};
use utsuroi::{Dop853, DormandPrince, Integrator, Rk4, Tolerances};

use orts::attitude::{AttitudeState, AugmentedAttitudeSystem};
use orts::effector::AugmentedState;
use orts::spacecraft::{ReactionWheelAssembly, RwCommand};

const MAX_MOMENTUM: f64 = 0.5;
const MAX_TORQUE: f64 = 0.1;
/// Time to saturation is `MAX_MOMENTUM / MAX_TORQUE = 5 s`; run well past it.
const T_END: f64 = 20.0;

fn saturating_system() -> AugmentedAttitudeSystem {
    let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0));
    let mut rw = ReactionWheelAssembly::three_axis(0.01, MAX_MOMENTUM, MAX_TORQUE);
    rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
    AugmentedAttitudeSystem::circular_orbit(inertia, 398600.4418, 7000.0, 100.0).with_effector(rw)
}

fn initial_state(system: &AugmentedAttitudeSystem) -> AugmentedState<AttitudeState> {
    AugmentedState {
        plant: AttitudeState::identity(),
        aux: system.initial_aux_state(),
        aux_bounds: system.initial_aux_bounds(),
    }
}

/// Worst violation of `aux_bounds` over a published trajectory, in absolute
/// momentum units. Zero or negative means every published state was inside
/// its bounds.
fn worst_overshoot(state: &AugmentedState<AttitudeState>) -> f64 {
    state
        .aux
        .iter()
        .zip(&state.aux_bounds)
        .map(|(&h, &(lo, hi))| (lo - h).max(h - hi))
        .fold(f64::NEG_INFINITY, f64::max)
}

fn tolerances() -> Vec<Tolerances> {
    vec![
        // cli/src/config.rs defaults.
        Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        },
        // A looser setting, where the adaptive step grows and the overshoot
        // with it.
        Tolerances {
            atol: 1e-3,
            rtol: 1e-3,
        },
    ]
}

#[test]
fn rk4_keeps_wheel_momentum_within_bounds() {
    let system = saturating_system();
    let mut worst = f64::NEG_INFINITY;
    let final_state = Rk4.integrate(
        &system,
        initial_state(&system),
        0.0,
        T_END,
        0.01,
        |_t, s| {
            worst = worst.max(worst_overshoot(s));
        },
    );
    assert!(
        worst <= 0.0,
        "Rk4 published wheel momentum {worst:.3e} beyond aux_bounds"
    );
    assert!((final_state.aux[2] + MAX_MOMENTUM).abs() < 0.01);
}

#[test]
fn dp45_keeps_wheel_momentum_within_bounds() {
    for tol in tolerances() {
        let system = saturating_system();
        let mut stepper = DormandPrince.stepper(&system, initial_state(&system), 0.0, 1.0, tol);
        let mut worst = f64::NEG_INFINITY;
        stepper
            .advance_to::<_, _, ()>(
                T_END,
                |_t, s| worst = worst.max(worst_overshoot(s)),
                |_t, _s| ControlFlow::Continue(()),
            )
            .expect("integration should reach the target");
        assert!(
            worst <= 0.0,
            "dp45 published wheel momentum {worst:.3e} beyond aux_bounds \
             (limit {MAX_MOMENTUM}, reached {})",
            stepper.state().aux[2]
        );
        // The wheel still absorbs the reaction up to its limit — the bound is
        // held by clamping, not by the integration stalling.
        assert!(
            (stepper.state().aux[2] + MAX_MOMENTUM).abs() < 0.01,
            "Z-wheel should saturate at -{MAX_MOMENTUM}, got {}",
            stepper.state().aux[2]
        );
    }
}

#[test]
fn dop853_keeps_wheel_momentum_within_bounds() {
    for tol in tolerances() {
        let system = saturating_system();
        let mut stepper = Dop853.stepper(&system, initial_state(&system), 0.0, 1.0, tol);
        let mut worst = f64::NEG_INFINITY;
        stepper
            .advance_to::<_, _, ()>(
                T_END,
                |_t, s| worst = worst.max(worst_overshoot(s)),
                |_t, _s| ControlFlow::Continue(()),
            )
            .expect("integration should reach the target");
        assert!(
            worst <= 0.0,
            "dop853 published wheel momentum {worst:.3e} beyond aux_bounds \
             (limit {MAX_MOMENTUM}, reached {})",
            stepper.state().aux[2]
        );
        assert!(
            (stepper.state().aux[2] + MAX_MOMENTUM).abs() < 0.01,
            "Z-wheel should saturate at -{MAX_MOMENTUM}, got {}",
            stepper.state().aux[2]
        );
    }
}

/// The fixed-step entry point of the adaptive solvers must project too — it is
/// what `Integrator::integrate` drives.
#[test]
fn fixed_step_adaptive_solvers_keep_wheel_momentum_within_bounds() {
    let system = saturating_system();
    let mut worst_dp45 = f64::NEG_INFINITY;
    DormandPrince.integrate(&system, initial_state(&system), 0.0, T_END, 0.1, |_t, s| {
        worst_dp45 = worst_dp45.max(worst_overshoot(s))
    });
    assert!(
        worst_dp45 <= 0.0,
        "DormandPrince::step published wheel momentum {worst_dp45:.3e} beyond aux_bounds"
    );

    let mut worst_dop853 = f64::NEG_INFINITY;
    Dop853.integrate(&system, initial_state(&system), 0.0, T_END, 0.1, |_t, s| {
        worst_dop853 = worst_dop853.max(worst_overshoot(s))
    });
    assert!(
        worst_dop853 <= 0.0,
        "Dop853::step published wheel momentum {worst_dop853:.3e} beyond aux_bounds"
    );
}
