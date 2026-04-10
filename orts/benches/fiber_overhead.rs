//! Benchmark: per-tick cost of WASM plugin on Pulley.
//!
//! Measures the full round-trip of `WasmController::update()`:
//! outer API → worker-thread channel → guest `wait_tick` resume →
//! guest control logic → guest `send_command` → channel → return.
//!
//! Prerequisites (build the guest first):
//!   cd plugins/pd-rw-control && cargo +1.91.0 component build --release

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use nalgebra::{Vector3, Vector4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::plugin::tick_input::{ActuatorState, Sensors};
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
    use kaname::frame::{Body, Vec3};
    use orts::plugin::tick_input::MagneticFieldBody;
    Sensors {
        magnetometer: Some(MagneticFieldBody::new(Vec3::<Body>::new(2e-5, -1e-5, 3e-5))),
        gyroscope: None,
        star_tracker: None,
    }
}

fn try_read_wasm(plugin: &str, binary: &str) -> Option<Vec<u8>> {
    let path = format!(
        "{}/../plugins/{plugin}/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build the guest first:\n  \
                 cd plugins/{plugin} && cargo +1.91.0 component build --release"
            );
            None
        }
    }
}

fn bench_update(c: &mut Criterion) {
    let Some(wasm_bytes) = try_read_wasm("pd-rw-control", "orts_example_plugin_pd_rw_control")
    else {
        return;
    };

    let engine = Arc::new(WasmEngine::new().expect("WasmEngine"));
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare");

    let config = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;
    let mut ctrl = WasmController::new(&pre, "bench", config).expect("new");

    let spacecraft = dummy_spacecraft();
    let sensors = dummy_sensors();
    let actuators = ActuatorState::default();
    let mut t = 0.0;

    c.bench_function("plugin_update", |b| {
        b.iter(|| {
            t += 0.1;
            let input = TickInput {
                t,
                epoch: None,
                spacecraft: &spacecraft,
                sensors: &sensors,
                actuators: &actuators,
            };
            let _ = ctrl.update(&input);
        })
    });
}

criterion_group!(benches, bench_update);
criterion_main!(benches);
