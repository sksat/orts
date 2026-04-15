//! Oracle: native PD+RW vs WASM PD+RW guest.
//!
//! Both paths:
//! 1. Same PD control law (left-invariant quaternion error)
//! 2. Same RW assembly (3-axis, same parameters)
//! 3. Same gravity gradient disturbance
//! 4. Same initial condition (10 deg error about Z)
//!
//! The native path evaluates the PD law inline; the WASM path reads
//! star tracker + gyroscope sensors and returns `Command::rw_torque`.
//!
//! **Prerequisites**: build the guest first:
//!
//! ```sh
//! cd plugins/pd-rw-control
//! cargo +1.91.0 component build --release
//! ```

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use arika::earth::{MU as MU_EARTH, R as R_EARTH};
use arika::epoch::Epoch;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use utsuroi::{Integrator, Rk4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{AttitudeState, AugmentedAttitudeSystem, GravityGradientTorque};
use orts::effector::AugmentedState;
use orts::plugin::wasm::{WasmController, WasmEngine, WasmPluginCache};
use orts::plugin::{ActuatorBundle, ActuatorState, PluginController, TickInput};
use orts::sensor::{Gyroscope, SensorBundle, StarTracker};
use orts::spacecraft::ReactionWheelAssembly;

const MASS: f64 = 500.0;
const ALT_KM: f64 = 400.0;
const KP: f64 = 1.0;
const KD: f64 = 2.0;
const DT_CTRL: f64 = 0.1;
const DT_ODE: f64 = 0.01;
const T_END: f64 = 60.0;
const RW_INERTIA: f64 = 0.01;
const RW_MAX_MOMENTUM: f64 = 1.0;
const RW_MAX_TORQUE: f64 = 0.5;

fn inertia() -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0))
}

fn initial_attitude() -> AttitudeState {
    let angle0 = 10.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::z());
    let initial_q = UnitQuaternion::from_axis_angle(&axis, angle0);
    AttitudeState::new(initial_q, Vector3::zeros())
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

/// Native PD+RW path (same as oracle_adcs_integration.rs).
fn run_native(initial: AttitudeState) -> AugmentedState<AttitudeState> {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;
    let target_q = UnitQuaternion::identity();
    let rw = ReactionWheelAssembly::three_axis(RW_INERTIA, RW_MAX_MOMENTUM, RW_MAX_TORQUE);

    let mut state = AugmentedState {
        plant: initial,
        aux: vec![0.0, 0.0, 0.0],
        aux_bounds: vec![],
    };
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + DT_CTRL).min(T_END);

        // PD control law (inline).
        let mut q_err = target_q.inverse() * state.plant.orientation();
        if q_err.w < 0.0 {
            q_err = UnitQuaternion::new_unchecked(-q_err.into_inner());
        }
        let q_vec = q_err.as_ref().vector();
        let theta_error = 2.0 * Vector3::new(q_vec[0], q_vec[1], q_vec[2]);
        let tau_cmd = -KP * theta_error - KD * state.plant.angular_velocity;

        let mut rw_seg = rw.clone();
        rw_seg.command = orts::spacecraft::RwCommand::Torques(rw_seg.core().allocate(&tau_cmd));
        let gg = GravityGradientTorque::circular_orbit(mu, radius, inertia());
        let system = AugmentedAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(gg)
            .with_effector(rw_seg);
        state = Rk4.integrate(&system, state, t, t_next, DT_ODE, |_, _| {});
        t = t_next;
    }

    state
}

/// Try to load the guest WASM. Returns `None` when the component
/// is not built yet so the tests can soft-skip (matches the
/// `wasm_async_backend` / `plugin_backend_e2e` convention). The
/// `rust-test-plugin-wasm` CI job builds this guest explicitly.
fn try_guest_wasm_bytes() -> Option<Vec<u8>> {
    let path = format!(
        "{}/../plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build: cd plugins/pd-rw-control && cargo +1.91.0 component build --release\n\
                 Skipping this test."
            );
            None
        }
    }
}

fn pd_rw_config() -> String {
    format!(
        r#"{{"kp":{},"kd":{},"target_q":[1,0,0,0],"sample_period":{}}}"#,
        KP, KD, DT_CTRL
    )
}

/// WASM PD+RW path via the sync `WasmController`.
///
/// Returns `None` when the guest component is not built.
fn run_wasm(initial: AttitudeState) -> Option<AugmentedState<AttitudeState>> {
    let wasm_bytes = try_guest_wasm_bytes()?;
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component must compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare must succeed");
    let ctrl =
        WasmController::new(&pre, "oracle-pd-rw", &pd_rw_config()).expect("new must succeed");
    Some(drive_wasm(Box::new(ctrl), initial))
}

/// WASM PD+RW path via the async `AsyncWasmController`.
#[cfg(feature = "plugin-wasm-async")]
fn run_wasm_async(initial: AttitudeState) -> Option<AugmentedState<AttitudeState>> {
    let path = std::path::PathBuf::from(format!(
        "{}/../plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm",
        env!("CARGO_MANIFEST_DIR")
    ));
    if !path.exists() {
        eprintln!("WASM not found: {}\nSkipping this test.", path.display());
        return None;
    }
    let mut cache = WasmPluginCache::new().expect("cache init");
    let ctrl = cache
        .build_async_controller(&path, "oracle-pd-rw-async", &pd_rw_config())
        .expect("build_async_controller");
    Some(drive_wasm(Box::new(ctrl), initial))
}

fn drive_wasm(
    mut ctrl: Box<dyn PluginController>,
    initial: AttitudeState,
) -> AugmentedState<AttitudeState> {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;
    let epoch = Epoch::j2000();

    let rw = ReactionWheelAssembly::three_axis(RW_INERTIA, RW_MAX_MOMENTUM, RW_MAX_TORQUE);

    let mut bundle = ActuatorBundle::new();

    let mut sensor_bundle = SensorBundle {
        magnetometers: vec![],
        gyroscopes: vec![Gyroscope::new()],
        star_trackers: vec![StarTracker::new()],
    };

    let mut state = AugmentedState {
        plant: initial,
        aux: vec![0.0, 0.0, 0.0],
        aux_bounds: vec![],
    };
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + DT_CTRL).min(T_END);

        // Set RW command from plugin's last output.
        let mut rw_seg = rw.clone();
        if let Some(rw_cmd) = bundle.rw_command() {
            rw_seg.command = rw_cmd.clone();
        }
        let gg = GravityGradientTorque::circular_orbit(mu, radius, inertia());
        let system = AugmentedAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(gg)
            .with_effector(rw_seg);
        state = Rk4.integrate(&system, state, t, t_next, DT_ODE, |_, _| {});
        t = t_next;

        // Build observation from current state.
        let current_epoch = epoch.add_seconds(t);
        let orbit = circular_orbit_at(t, mu, radius);
        let snapshot = SpacecraftState {
            orbit,
            attitude: state.plant.clone(),
            mass: MASS,
        };
        let sensors = sensor_bundle.evaluate(&snapshot, &current_epoch);
        let actuator_state = ActuatorState {
            rw_momentum: Some(state.aux.clone()),
            ..Default::default()
        };
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

/// Sync and async backends must produce bit-exact identical
/// trajectories for the same initial condition.
#[cfg(feature = "plugin-wasm-async")]
#[test]
fn wasm_pd_rw_sync_matches_async_bit_exact() {
    let initial = initial_attitude();

    let Some(sync_state) = run_wasm(initial.clone()) else {
        return;
    };
    let Some(async_state) = run_wasm_async(initial) else {
        return;
    };

    assert_eq!(
        sync_state.plant.quaternion.as_slice(),
        async_state.plant.quaternion.as_slice(),
        "quaternion: sync vs async must be bit-exact"
    );
    assert_eq!(
        sync_state.plant.angular_velocity.as_slice(),
        async_state.plant.angular_velocity.as_slice(),
        "angular_velocity: sync vs async must be bit-exact"
    );
    assert_eq!(
        sync_state.aux, async_state.aux,
        "RW momentum: sync vs async must be bit-exact"
    );
}

#[test]
fn wasm_pd_rw_matches_native() {
    let initial = initial_attitude();

    let Some(wasm_state) = run_wasm(initial.clone()) else {
        return;
    };
    let native_state = run_native(initial);

    // The WASM guest implements the same PD math but with hand-rolled
    // quaternion multiplication (different float op order from nalgebra).
    // Over 60 s of closed-loop control, small per-tick differences
    // accumulate. Tolerance is 1e-4 (tighter than the ~1 deg convergence
    // target, but accounts for float divergence).
    let q_diff = (native_state.plant.quaternion - wasm_state.plant.quaternion).norm();
    let w_diff = (native_state.plant.angular_velocity - wasm_state.plant.angular_velocity).norm();
    let h_diff: f64 = native_state
        .aux
        .iter()
        .zip(&wasm_state.aux)
        .map(|(a, b)| (a - b).abs())
        .sum();

    assert!(
        q_diff < 1e-4,
        "quaternion difference too large: {q_diff:.3e}"
    );
    assert!(
        w_diff < 1e-4,
        "angular velocity difference too large: {w_diff:.3e}"
    );
    assert!(
        h_diff < 1e-4,
        "RW momentum difference too large: {h_diff:.3e}"
    );
}

#[test]
fn wasm_pd_rw_converges() {
    let initial = initial_attitude();
    let Some(state) = run_wasm(initial) else {
        return;
    };

    let target_q = UnitQuaternion::identity();
    let q_err = target_q.inverse() * state.plant.orientation();
    let angle_err_deg = q_err.angle().to_degrees();

    assert!(
        angle_err_deg < 1.0,
        "should converge to <1 deg, got {angle_err_deg:.4} deg"
    );

    let omega_mag = state.plant.angular_velocity.magnitude();
    assert!(
        omega_mag < 0.01,
        "angular velocity should be small, got {omega_mag:.6} rad/s"
    );
}
