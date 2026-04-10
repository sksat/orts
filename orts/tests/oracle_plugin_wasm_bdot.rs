//! Phase P1-c oracle: WASM Component guest B-dot vs native BdotFiniteDiff.
//!
//! This test loads the `orts-example-plugin-bdot-finite-diff` WASM
//! Component and drives it through the full host pipeline
//! (`WasmEngine` -> `Component` -> `WasmController` -> `PluginController`
//! -> `ActuatorBundle` -> `CommandedMagnetorquer` -> ODE integration).
//! The resulting trajectory is compared against the pre-existing
//! native `BdotFiniteDiff` controller.
//!
//! **Prerequisites**: the guest must be built before running this test:
//!
//! ```sh
//! cd plugins/bdot-finite-diff
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
use orts::plugin::wasm::{WasmController, WasmEngine, WasmPluginCache};
use orts::plugin::{ActuatorBundle, ActuatorState, PluginController, TickInput};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle};

const MASS: f64 = 50.0;
const ALT_KM: f64 = 500.0;
const SAMPLE_PERIOD: f64 = 1.0;
const ODE_DT: f64 = 0.1;
const T_END: f64 = 20.0;
const GAIN: f64 = 1e4;
const MAX_MOMENT: f64 = 10.0;

/// Try to load the guest WASM. Returns `None` when the component
/// has not been built yet, so the tests can soft-skip (matches the
/// `wasm_async_backend` / `plugin_backend_e2e` convention). The
/// `rust-test-plugin-wasm` CI job builds this guest explicitly and
/// relies on the tests actually running; callers must check the
/// return value and skip if it is `None`.
fn try_guest_wasm_bytes() -> Option<Vec<u8>> {
    // `cargo component build` places the output under `wasm32-wasip1/`
    // even though the Component Model uses WASI preview 2 internally.
    // This is because `cargo-component` applies a wasip1->component
    // adapter as a post-build step. If a future `cargo-component`
    // version switches the target directory to `wasm32-wasip2/`, this
    // path will need updating.
    let path = format!(
        "{}/../plugins/bdot-finite-diff/target/wasm32-wasip1/release/orts_example_plugin_bdot_finite_diff.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build: cd plugins/bdot-finite-diff && cargo +1.91.0 component build --release\n\
                 Skipping this test."
            );
            None
        }
    }
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

/// Run the WASM Component guest path via WasmController (sync backend).
///
/// Returns `None` when the guest component is not built (CI jobs
/// without a WASM toolchain soft-skip in that case).
fn run_wasm(initial: AttitudeState, epoch: Epoch) -> Option<AttitudeState> {
    let wasm_bytes = try_guest_wasm_bytes()?;
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component must compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare must succeed");
    let ctrl = WasmController::new(&pre, "oracle-bdot", "").expect("new must succeed");
    Some(drive_wasm(Box::new(ctrl), initial, epoch))
}

/// Run the WASM Component guest path via the async backend.
#[cfg(feature = "plugin-wasm-async")]
fn run_wasm_async(initial: AttitudeState, epoch: Epoch) -> Option<AttitudeState> {
    let path = std::path::PathBuf::from(format!(
        "{}/../plugins/bdot-finite-diff/target/wasm32-wasip1/release/orts_example_plugin_bdot_finite_diff.wasm",
        env!("CARGO_MANIFEST_DIR")
    ));
    if !path.exists() {
        eprintln!("WASM not found: {}\nSkipping this test.", path.display());
        return None;
    }
    let mut cache = WasmPluginCache::new().expect("cache init");
    let ctrl = cache
        .build_async_controller(&path, "oracle-bdot-async", "")
        .expect("build_async_controller");
    Some(drive_wasm(Box::new(ctrl), initial, epoch))
}

fn drive_wasm(
    mut ctrl: Box<dyn PluginController>,
    initial: AttitudeState,
    epoch: Epoch,
) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let field_model: Arc<dyn tobari::magnetic::MagneticFieldModel> =
        Arc::new(TiltedDipole::earth());

    let mut bundle = ActuatorBundle::new();

    let mut sensor_bundle = SensorBundle {
        magnetometer: Some(Magnetometer::new(Arc::clone(&field_model))),
        gyroscope: Some(Gyroscope::new()),
        star_tracker: None,
    };
    let mut state = initial;
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(
            bundle.magnetic_moment().into_inner(),
            TiltedDipole::earth(),
        );
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
        let sensors = sensor_bundle.evaluate(&snapshot, &current_epoch);
        let actuator_state = ActuatorState::default();
        let input = TickInput {
            t,
            epoch: Some(&current_epoch),
            sensors: &sensors,
            actuators: &actuator_state,
            spacecraft: &snapshot,
        };
        if let Some(cmd) = ctrl
            .update(&input)
            .expect("WASM controller must return a valid command")
        {
            bundle.apply(&cmd).expect("WASM command must be finite");
        }
    }
    state
}

#[test]
fn wasm_bdot_matches_native() {
    let epoch = Epoch::j2000();
    let initial = initial_attitude();

    let Some(wasm_state) = run_wasm(initial.clone(), epoch) else {
        return;
    };
    let native_state = run_native(initial, epoch);

    // The WASM guest reimplements the same finite-diff B-dot math but
    // reads B_body from sensors.magnetometer (pre-evaluated by
    // the host's Magnetometer sensor) rather than computing it inline.
    // Float operation order may differ, so we allow a small tolerance
    // rather than demanding bit-exact match.
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

/// The sync and async backends must produce the **bit-exact** same
/// trajectory for identical inputs. Both backends run the same guest
/// WASM through Pulley on `worker_threads(1)` with identical host
/// imports, so the only freedom is our own conversion/dispatch code.
#[cfg(feature = "plugin-wasm-async")]
#[test]
fn wasm_bdot_sync_matches_async_bit_exact() {
    let epoch = Epoch::j2000();
    let initial = initial_attitude();

    let Some(sync_state) = run_wasm(initial.clone(), epoch) else {
        return;
    };
    let Some(async_state) = run_wasm_async(initial, epoch) else {
        return;
    };

    assert_eq!(
        sync_state.quaternion.as_slice(),
        async_state.quaternion.as_slice(),
        "quaternion: sync vs async must be bit-exact"
    );
    assert_eq!(
        sync_state.angular_velocity.as_slice(),
        async_state.angular_velocity.as_slice(),
        "angular_velocity: sync vs async must be bit-exact"
    );
}
