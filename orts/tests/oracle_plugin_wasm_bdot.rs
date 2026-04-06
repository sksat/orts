//! Phase P1-c oracle: WASM Component guest B-dot vs native BdotFiniteDiff.
//!
//! This test loads the `orts-example-plugin-bdot-finite-diff` WASM
//! Component and drives it through the full host pipeline
//! (`WasmEngine` → `Component` → `WasmController` → `PluginController`
//! → `ActuatorBundle` → `CommandedMagnetorquer` → ODE integration).
//! The resulting trajectory is compared against the pre-existing
//! native `BdotFiniteDiff` controller.
//!
//! **Prerequisites**: the guest must be built before running this test:
//!
//! ```sh
//! cd examples/plugins/bdot-finite-diff
//! cargo +1.91.0 component build --release
//! ```
//!
//! The test is feature-gated behind `plugin-wasm` so it only runs
//! when wasmtime is linked.

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use kaname::constants::{MU_EARTH, R_EARTH};
use kaname::epoch::Epoch;
use nalgebra::{Matrix3, Vector3, Vector4};
use tobari::magnetic::TiltedDipole;
use utsuroi::{Integrator, Rk4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{
    AttitudeState, BdotFiniteDiff as NativeBdot, CommandedMagnetorquer, DecoupledAttitudeSystem,
};
use orts::control::DiscreteController;
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{ActuatorBundle, PluginController, Sensors, TickInput};

const MASS: f64 = 50.0;
const ALT_KM: f64 = 500.0;
const SAMPLE_PERIOD: f64 = 1.0;
const ODE_DT: f64 = 0.1;
const T_END: f64 = 20.0;
const GAIN: f64 = 1e4;
const MAX_MOMENT: f64 = 10.0;

fn guest_wasm_bytes() -> Vec<u8> {
    // `cargo component build` places the output under `wasm32-wasip1/`
    // even though the Component Model uses WASI preview 2 internally.
    // This is because `cargo-component` applies a wasip1→component
    // adapter as a post-build step. If a future `cargo-component`
    // version switches the target directory to `wasm32-wasip2/`, this
    // path will need updating.
    let path = format!(
        "{}/../examples/plugins/bdot-finite-diff/target/wasm32-wasip1/release/orts_example_plugin_bdot_finite_diff.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "Could not read guest WASM at {path}: {e}\n\
             Build it first: cd examples/plugins/bdot-finite-diff && \
             cargo +1.91.0 component build --release"
        )
    })
}

fn inertia() -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(1.0, 1.0, 1.0))
}

fn initial_attitude() -> AttitudeState {
    AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: Vector3::new(0.1, 0.05, -0.03),
    }
}

fn circular_orbit_at(t: f64, mu: f64, radius: f64) -> OrbitalState {
    let n = (mu / radius.powi(3)).sqrt();
    let v = (mu / radius).sqrt();
    let theta = n * t;
    OrbitalState::new(
        Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
        Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
    )
}

/// Run the native BdotFiniteDiff path (same as oracle_discrete_bdot).
fn run_native(initial: AttitudeState, epoch: Epoch) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;
    let mut ctrl = NativeBdot::new(
        GAIN,
        Vector3::new(MAX_MOMENT, MAX_MOMENT, MAX_MOMENT),
        TiltedDipole::earth(),
        SAMPLE_PERIOD,
    );
    let mut state = initial;
    let mut cmd = ctrl.initial_command();
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(cmd, TiltedDipole::earth());
        let system = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(actuator)
            .with_epoch(epoch);
        state = Rk4.integrate(&system, state, t, t_next, ODE_DT, |_, _| {});
        t = t_next;
        let orbit_at_t = circular_orbit_at(t, mu, radius);
        let current_epoch = epoch.add_seconds(t);
        cmd = ctrl.update(t, &state, &orbit_at_t, Some(&current_epoch));
    }
    state
}

/// Run the WASM Component guest path via WasmController.
fn run_wasm(initial: AttitudeState, epoch: Epoch) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let wasm_bytes = guest_wasm_bytes();
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component must compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare must succeed");
    let mut ctrl = WasmController::new(&pre, "oracle-bdot", "").expect("new must succeed");

    let mut bundle = ActuatorBundle::new();
    bundle
        .apply(&ctrl.initial_command())
        .expect("initial command must be finite");

    let sensors = Sensors::empty();
    let mut state = initial;
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(bundle.magnetic_moment(), TiltedDipole::earth());
        let system = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(actuator)
            .with_epoch(epoch);
        state = Rk4.integrate(&system, state, t, t_next, ODE_DT, |_, _| {});
        t = t_next;
        let orbit_at_t = circular_orbit_at(t, mu, radius);
        let current_epoch = epoch.add_seconds(t);
        let snapshot = SpacecraftState {
            orbit: orbit_at_t,
            attitude: state.clone(),
            mass: MASS,
        };
        let obs = TickInput {
            t,
            spacecraft: &snapshot,
            epoch: Some(&current_epoch),
            sensors: &sensors,
        };
        let cmd = ctrl
            .update(&obs)
            .expect("WASM controller must return a valid command");
        bundle.apply(&cmd).expect("WASM command must be finite");
    }
    state
}

#[test]
fn wasm_bdot_matches_native() {
    let epoch = Epoch::j2000();
    let initial = initial_attitude();

    let native_state = run_native(initial.clone(), epoch);
    let wasm_state = run_wasm(initial, epoch);

    // The WASM guest reimplements the same finite-diff B-dot math but
    // uses its own quaternion rotation (hand-rolled) and gets magnetic
    // field from the host import rather than directly calling
    // TiltedDipole::field_eci. Float operation order may differ, so
    // we allow a small tolerance rather than demanding bit-exact match.
    let q_diff = (native_state.quaternion - wasm_state.quaternion).norm();
    let w_diff = (native_state.angular_velocity - wasm_state.angular_velocity).norm();

    assert!(
        q_diff < 1e-12,
        "quaternion difference too large: {q_diff:.3e}"
    );
    assert!(
        w_diff < 1e-12,
        "angular velocity difference too large: {w_diff:.3e}"
    );
}
