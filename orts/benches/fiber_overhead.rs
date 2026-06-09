//! Benchmark: per-tick cost of WASM plugin update() on Pulley,
//! comparing the sync worker-thread backend and the async fiber
//! backend.
//!
//! Both benches drive the same `pd-rw-control` guest through the
//! public `PluginController::update` API, so numbers are directly
//! comparable. The sync path measures the round-trip through the
//! worker-thread channel; the async path measures the round-trip
//! through a shared tokio runtime + fiber suspension.
//!
//! Prerequisites:
//!   cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-pd-rw-control --release
//!
//! Run:
//!   cargo bench -p orts --features plugin-wasm-async

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use nalgebra::{Vector3, Vector4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::plugin::tick_input::{ActuatorTelemetry, Sensors};
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{PluginController, TickInput};

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
    use arika::frame::{Body, Vec3};
    use orts::plugin::tick_input::{
        AngularVelocityBody, AttitudeBodyToInertial, MagneticFieldBody,
    };
    Sensors {
        magnetometers: vec![MagneticFieldBody::new(Vec3::<Body>::new(2e-5, -1e-5, 3e-5))],
        gyroscopes: vec![AngularVelocityBody::new(Vec3::<Body>::new(
            0.1, 0.05, -0.03,
        ))],
        star_trackers: vec![AttitudeBodyToInertial::new(Vector4::new(
            1.0, 0.0, 0.0, 0.0,
        ))],
        sun_sensors: vec![],
    }
}

fn try_read_wasm(binary: &str) -> Option<Vec<u8>> {
    let path = format!(
        "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build the guest first:\n  \
                 cd plugin-sdk/examples && cargo +1.91.0 component build --release"
            );
            None
        }
    }
}

const PD_RW_CONFIG: &str = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;

/// Sync backend: `WasmController` + worker thread per satellite.
fn bench_sync_update(c: &mut Criterion) {
    let Some(wasm_bytes) = try_read_wasm("orts_example_plugin_pd_rw_control") else {
        return;
    };

    let engine = Arc::new(WasmEngine::new_sync().expect("WasmEngine::new_sync"));
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare");

    let mut ctrl = WasmController::new(&pre, "bench-sync", PD_RW_CONFIG).expect("new");

    let spacecraft = dummy_spacecraft();
    let sensors = dummy_sensors();
    let actuators = ActuatorTelemetry::default();
    let mut t = 0.0;

    c.bench_function("plugin_update_sync", |b| {
        b.iter(|| {
            t += 0.1;
            let input = TickInput {
                t,
                epoch: None,
                spacecraft: &spacecraft,
                sensors: &sensors,
                actuators: &actuators,
            };
            let cmd = ctrl.update(&input).expect("update must succeed");
            assert!(cmd.is_some());
        })
    });
}

/// Async backend: `AsyncWasmController` on a shared tokio runtime.
#[cfg(feature = "plugin-wasm-async")]
fn bench_async_update(c: &mut Criterion) {
    use orts::plugin::wasm::WasmPluginCache;

    let path = std::path::PathBuf::from(format!(
        "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm",
        env!("CARGO_MANIFEST_DIR")
    ));
    if !path.exists() {
        eprintln!(
            "WASM not found: {}\nBuild: cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-pd-rw-control --release",
            path.display()
        );
        return;
    }

    let mut cache = WasmPluginCache::new().expect("cache");
    let mut ctrl = cache
        .build_async_controller(&path, "bench-async", PD_RW_CONFIG)
        .expect("build_async_controller");

    let spacecraft = dummy_spacecraft();
    let sensors = dummy_sensors();
    let actuators = ActuatorTelemetry::default();
    let mut t = 0.0;

    c.bench_function("plugin_update_async", |b| {
        b.iter(|| {
            t += 0.1;
            let input = TickInput {
                t,
                epoch: None,
                spacecraft: &spacecraft,
                sensors: &sensors,
                actuators: &actuators,
            };
            let cmd = ctrl.update(&input).expect("update must succeed");
            assert!(cmd.is_some());
        })
    });
}

#[cfg(feature = "plugin-wasm-async")]
criterion_group!(benches, bench_sync_update, bench_async_update);

#[cfg(not(feature = "plugin-wasm-async"))]
criterion_group!(benches, bench_sync_update);

criterion_main!(benches);
