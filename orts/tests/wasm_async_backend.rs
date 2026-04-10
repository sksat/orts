//! Integration tests for the async WASM backend.
//!
//! Exercises [`orts::plugin::wasm::WasmPluginCache::build_async_controller`]
//! end-to-end by loading a real guest component, spawning N satellites
//! on the shared async runtime, and driving them through the sync
//! `PluginController::update` facade.
//!
//! The `_scale_1000_sats` test is `#[ignore]`d by default because
//! compiling 1000 stores still takes ~1 second in debug builds; run
//! with `--ignored` to include it.

#![cfg(all(feature = "plugin-wasm", feature = "plugin-wasm-async"))]

use std::time::Instant;

use nalgebra::{Vector3, Vector4};

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::plugin::tick_input::{ActuatorState, Sensors};
use orts::plugin::wasm::WasmPluginCache;
use orts::plugin::{PluginController, TickInput};

fn load_wasm(plugin: &str, binary: &str) -> Option<Vec<u8>> {
    let path = format!(
        "{}/../plugins/{plugin}/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build: cd plugins/{plugin} && cargo +1.91.0 component build --release"
            );
            None
        }
    }
}

fn dummy_spacecraft() -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.1, 0.05, -0.03),
        },
        mass: 50.0,
    }
}

fn dummy_sensors() -> Sensors {
    use kaname::frame::{Body, Vec3};
    use nalgebra::Vector4;
    use orts::plugin::tick_input::{
        AngularVelocityBody, AttitudeBodyToInertial, MagneticFieldBody,
    };
    Sensors {
        magnetometer: Some(MagneticFieldBody::new(Vec3::<Body>::new(2e-5, -1e-5, 3e-5))),
        gyroscope: Some(AngularVelocityBody::new(Vec3::<Body>::new(
            0.1, 0.05, -0.03,
        ))),
        star_tracker: Some(AttitudeBodyToInertial::new(Vector4::new(
            1.0, 0.0, 0.0, 0.0,
        ))),
    }
}

fn make_cache_with_path(
    plugin: &str,
    binary: &str,
) -> Option<(WasmPluginCache, std::path::PathBuf)> {
    // Check that the wasm file exists before constructing the cache
    // so tests skip cleanly on clean checkouts.
    let _ = load_wasm(plugin, binary)?;
    let cache = WasmPluginCache::new().expect("WasmPluginCache::new");
    let path = std::path::PathBuf::from(format!(
        "{}/../plugins/{plugin}/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    ));
    Some((cache, path))
}

/// Smoke test: spawn one satellite, drive it for 10 ticks, verify a
/// command is returned each tick.
#[test]
fn async_backend_single_satellite() {
    let Some((mut cache, path)) =
        make_cache_with_path("pd-rw-control", "orts_example_plugin_pd_rw_control")
    else {
        return;
    };

    let config = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;
    let mut ctrl = cache
        .build_async_controller(&path, "sat0", config)
        .expect("build_async_controller must succeed");

    assert!(
        (ctrl.sample_period() - 0.1).abs() < 1e-12,
        "sample_period_s = {} (expected 0.1)",
        ctrl.sample_period()
    );

    let spacecraft = dummy_spacecraft();
    let sensors = dummy_sensors();
    let actuators = ActuatorState::default();

    for i in 0..10 {
        let input = TickInput {
            t: i as f64 * 0.1,
            epoch: None,
            spacecraft: &spacecraft,
            sensors: &sensors,
            actuators: &actuators,
        };
        let cmd = ctrl.update(&input).expect("update must succeed");
        assert!(cmd.is_some(), "tick {i} must return a command");
    }
}

/// Concurrency test: spawn N satellites, verify they all make progress
/// on a single runtime thread.
#[test]
fn async_backend_100_satellites() {
    run_multi_satellite(100, 10);
}

#[test]
#[ignore = "slow; run with --ignored"]
fn async_backend_1000_satellites() {
    run_multi_satellite(1000, 5);
}

fn run_multi_satellite(n_sats: usize, n_ticks: usize) {
    let Some((mut cache, path)) =
        make_cache_with_path("pd-rw-control", "orts_example_plugin_pd_rw_control")
    else {
        return;
    };

    let spawn_start = Instant::now();
    let mut controllers: Vec<_> = (0..n_sats)
        .map(|i| {
            let config = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;
            cache
                .build_async_controller(&path, &format!("sat{i}"), config)
                .unwrap_or_else(|e| panic!("spawn sat{i}: {e}"))
        })
        .collect();
    let spawn_elapsed = spawn_start.elapsed();
    eprintln!(
        "[{n_sats} sats] spawn: {spawn_elapsed:?} ({:.2} ms/sat)",
        spawn_elapsed.as_secs_f64() * 1e3 / n_sats as f64
    );

    let spacecraft = dummy_spacecraft();
    let sensors = dummy_sensors();
    let actuators = ActuatorState::default();

    let drive_start = Instant::now();
    for tick in 0..n_ticks {
        for (i, ctrl) in controllers.iter_mut().enumerate() {
            let input = TickInput {
                t: tick as f64 * 0.1,
                epoch: None,
                spacecraft: &spacecraft,
                sensors: &sensors,
                actuators: &actuators,
            };
            let cmd = ctrl
                .update(&input)
                .unwrap_or_else(|e| panic!("sat{i} tick {tick}: {e}"));
            assert!(cmd.is_some(), "sat{i} tick {tick} must return a command");
        }
    }
    let drive_elapsed = drive_start.elapsed();
    let total_ticks = (n_sats * n_ticks) as f64;
    eprintln!(
        "[{n_sats} sats] drive {n_ticks} ticks each = {total_ticks:.0} ticks in {drive_elapsed:?} \
         ({:.2} µs/tick avg)",
        drive_elapsed.as_secs_f64() * 1e6 / total_ticks
    );
}

/// Drop test: build a controller and drop it immediately. The runtime
/// thread should clean up the spawned task without hanging.
#[test]
fn async_backend_drop_during_wait() {
    let Some((mut cache, path)) =
        make_cache_with_path("pd-rw-control", "orts_example_plugin_pd_rw_control")
    else {
        return;
    };

    let config = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;
    let ctrl = cache
        .build_async_controller(&path, "droptest", config)
        .expect("build_async_controller");
    drop(ctrl);
    // If the drop signal did not propagate, the runtime thread would
    // still be hosting the guest task. We can't easily assert that
    // here without leaking state, but the test finishing cleanly is
    // the happy-path signal.
}
