//! E2E: msg-io C&T チャネルで FSW (WASM guest) にモード切替コマンドを
//! 送り、結果を受け取る。fire-and-forget と request-response の両プロトコルを
//! 同じ `msg-io` transport の上で検証する。
//!
//! transport は host が所有する決定論的なキュー + テレメトリシンク。ここでは
//! `WasmController::deliver` / `take_outbound` の **test-API transport** を使い、
//! 地上局 + router 役をテストが演じる（WebSocket / config 時刻シーケンスは
//! 別の transport adapter として後付けする）。
//!
//! **Prerequisites**: guest を先にビルドすること:
//!
//! ```sh
//! cd plugin-sdk/examples
//! cargo +1.91.0 component build \
//!   -p orts-example-plugin-commandable-mode-ff \
//!   -p orts-example-plugin-commandable-mode-rr --release
//! ```
//!
//! guest 未ビルド時は soft-skip する（wasm ツールチェインのない CI ジョブ向け）。

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use nalgebra::{Vector3, Vector4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{
    ActuatorTelemetry, Message, NodeId, Payload, PluginController, Sensors, TickInput, Value,
};

const KIND_SET_MODE: &str = "orts.cmd.set-mode.v1";
const KIND_MODE_TLM: &str = "orts.tlm.mode.v1";
const KIND_SET_MODE_ACK: &str = "orts.ack.set-mode.v1";

/// Load a guest component and build a `WasmController`, or soft-skip
/// (return `None`) when the guest has not been built.
fn load(binary: &str) -> Option<WasmController> {
    let path = format!(
        "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build: cd plugin-sdk/examples && cargo +1.91.0 component build -p {binary_pkg} --release\n\
                 Skipping this test.",
                binary_pkg = binary.replace('_', "-")
            );
            return None;
        }
    };
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let component = Component::new(engine.inner(), &bytes).expect("Component must compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare must succeed");
    Some(WasmController::new(&pre, "msg-test", "").expect("new must succeed"))
}

fn spacecraft() -> SpacecraftState {
    SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        },
        mass: 50.0,
    }
}

/// Ground → satellite(0) set-mode command. The host re-stamps `src` /
/// `host_seq` / `deliver_tick` on outbound; for inbound the test plays
/// the router and sets them directly.
fn set_mode(payload: Payload) -> Message {
    Message {
        src: NodeId::Ground,
        dst: NodeId::Satellite(0),
        kind: KIND_SET_MODE.to_string(),
        host_seq: 0,
        deliver_tick: 0,
        payload,
    }
}

/// Find the first message of `kind` in a drained outbound batch.
fn find<'a>(out: &'a [Message], kind: &str) -> Option<&'a Message> {
    out.iter().find(|m| m.kind == kind)
}

// ─────────────────────── fire-and-forget ───────────────────────

#[test]
fn fire_and_forget_set_mode_is_confirmed_by_telemetry() {
    let Some(mut ctrl) = load("orts_example_plugin_commandable_mode_ff") else {
        return;
    };
    let sc = spacecraft();
    let sensors = Sensors::empty();
    let act = ActuatorTelemetry::default();
    let obs = TickInput {
        t: 0.0,
        spacecraft: &sc,
        epoch: None,
        sensors: &sensors,
        actuators: &act,
    };

    // tick 0: no uplink → FSW downlinks its default mode.
    ctrl.update(&obs).expect("update tick 0");
    let out0 = ctrl.take_outbound();
    let tlm0 = find(&out0, KIND_MODE_TLM).expect("mode telemetry every tick");
    assert_eq!(
        tlm0.payload.get("mode").and_then(Value::as_text),
        Some("detumble")
    );
    // Host stamped the envelope: src injected, dst preserved.
    assert_eq!(tlm0.src, NodeId::Satellite(0));
    assert_eq!(tlm0.dst, NodeId::Ground);

    // Uplink set-mode nadir (fire-and-forget), then step.
    ctrl.deliver(set_mode(Payload::key_value([(
        "mode",
        Value::Text("nadir".to_string()),
    )])));
    ctrl.update(&obs).expect("update tick 1");
    let out1 = ctrl.take_outbound();

    // Ground "waits for telemetry": the mode tlm now reports nadir.
    let tlm1 = find(&out1, KIND_MODE_TLM).expect("mode telemetry every tick");
    assert_eq!(
        tlm1.payload.get("mode").and_then(Value::as_text),
        Some("nadir")
    );
    // Fire-and-forget: no ack message is produced.
    assert!(find(&out1, KIND_SET_MODE_ACK).is_none());
}

#[test]
fn fire_and_forget_invalid_mode_is_silently_ignored() {
    let Some(mut ctrl) = load("orts_example_plugin_commandable_mode_ff") else {
        return;
    };
    let sc = spacecraft();
    let sensors = Sensors::empty();
    let act = ActuatorTelemetry::default();
    let obs = TickInput {
        t: 0.0,
        spacecraft: &sc,
        epoch: None,
        sensors: &sensors,
        actuators: &act,
    };

    ctrl.deliver(set_mode(Payload::key_value([(
        "mode",
        Value::Text("bogus".to_string()),
    )])));
    ctrl.update(&obs).expect("update");
    let out = ctrl.take_outbound();

    // Invalid mode ignored → telemetry still reports the unchanged default.
    let tlm = find(&out, KIND_MODE_TLM).expect("mode telemetry");
    assert_eq!(
        tlm.payload.get("mode").and_then(Value::as_text),
        Some("detumble")
    );
}

// ─────────────────────── request-response ───────────────────────

#[test]
fn request_response_set_mode_acked_with_correlation() {
    let Some(mut ctrl) = load("orts_example_plugin_commandable_mode_rr") else {
        return;
    };
    let sc = spacecraft();
    let sensors = Sensors::empty();
    let act = ActuatorTelemetry::default();
    let obs = TickInput {
        t: 0.0,
        spacecraft: &sc,
        epoch: None,
        sensors: &sensors,
        actuators: &act,
    };

    // Request set-mode nadir with correlation id 7.
    ctrl.deliver(set_mode(Payload::key_value([
        ("req-id", Value::Integer(7)),
        ("mode", Value::Text("nadir".to_string())),
    ])));
    ctrl.update(&obs).expect("update");
    let out = ctrl.take_outbound();

    let ack = find(&out, KIND_SET_MODE_ACK).expect("ack for the request");
    // correlation id echoed back.
    assert_eq!(
        ack.payload.get("req-id").and_then(Value::as_integer),
        Some(7)
    );
    assert_eq!(
        ack.payload.get("status").and_then(Value::as_text),
        Some("accepted")
    );
    assert_eq!(
        ack.payload.get("mode").and_then(Value::as_text),
        Some("nadir")
    );
    assert_eq!(ack.src, NodeId::Satellite(0));
    assert_eq!(ack.dst, NodeId::Ground);
}

#[test]
fn request_response_invalid_mode_is_rejected() {
    let Some(mut ctrl) = load("orts_example_plugin_commandable_mode_rr") else {
        return;
    };
    let sc = spacecraft();
    let sensors = Sensors::empty();
    let act = ActuatorTelemetry::default();
    let obs = TickInput {
        t: 0.0,
        spacecraft: &sc,
        epoch: None,
        sensors: &sensors,
        actuators: &act,
    };

    ctrl.deliver(set_mode(Payload::key_value([
        ("req-id", Value::Integer(8)),
        ("mode", Value::Text("bogus".to_string())),
    ])));
    ctrl.update(&obs).expect("update");
    let out = ctrl.take_outbound();

    let ack = find(&out, KIND_SET_MODE_ACK).expect("ack for the request");
    assert_eq!(
        ack.payload.get("req-id").and_then(Value::as_integer),
        Some(8)
    );
    assert_eq!(
        ack.payload.get("status").and_then(Value::as_text),
        Some("rejected")
    );
    // Mode unchanged (still the default).
    assert_eq!(
        ack.payload.get("mode").and_then(Value::as_text),
        Some("detumble")
    );
}
