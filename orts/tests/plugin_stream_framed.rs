//! E2E: stream-io byte-stream channel. A FSW (WASM guest) speaks its own
//! `SYNC|LEN|PAYLOAD|CRC16` framing over a raw byte stream — the host is a
//! dumb conduit. Exercises the realities that distinguish byte-streams from
//! `msg-io` packets: **reassembly of a frame split across ticks** and
//! **resync past a corrupt frame**.
//!
//! Here the test plays the kble peer using the `stream_deliver` /
//! `stream_take` test-API transport (the `ws://` / stdio bridge is a
//! follow-up adapter).
//!
//! **Prerequisites**: build the guest first:
//!
//! ```sh
//! cd plugin-sdk/examples
//! cargo +1.91.0 component build -p orts-example-plugin-stream-framed-commander --release
//! ```
//!
//! Soft-skips when the guest has not been built (CI jobs without the wasm
//! toolchain).

#![cfg(feature = "plugin-wasm")]

use std::sync::Arc;

use arika::frame::{Body, Vec3};
use nalgebra::{Vector3, Vector4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::AttitudeState;
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{ActuatorTelemetry, AngularVelocityBody, PluginController, Sensors, TickInput};

const STREAM: &str = "comlink";
const SYNC: [u8; 2] = [0xEB, 0x90];

/// Settled: below the FSW's 0.05 rad/s nadir gate. Tumbling: above it.
const SETTLED: f64 = 0.0;
const TUMBLING: f64 = 0.2;

fn load() -> Option<WasmController> {
    let binary = "orts_example_plugin_stream_framed_commander";
    let path = format!(
        "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/{binary}.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "WASM not found: {path}\n\
                 Build: cd plugin-sdk/examples && cargo +1.91.0 component build \
                 -p orts-example-plugin-stream-framed-commander --release\n\
                 Skipping this test."
            );
            return None;
        }
    };
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let component = Component::new(engine.inner(), &bytes).expect("Component must compile");
    let pre = WasmController::prepare(&engine, &component).expect("prepare must succeed");
    // The FSW is wired to a single "comlink" stream (declared at construction).
    Some(
        WasmController::new_with_streams(&pre, "stream-test", "", vec![STREAM.to_string()])
            .expect("new must succeed"),
    )
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

fn gyro_sensors(omega_x: f64) -> Sensors {
    Sensors {
        magnetometers: vec![],
        gyroscopes: vec![AngularVelocityBody::new(Vec3::<Body>::new(
            omega_x, 0.0, 0.0,
        ))],
        star_trackers: vec![],
        sun_sensors: vec![],
    }
}

// ─── host-side framing (mirror of the guest's wire format) ──────

fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn build_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut body = Vec::new();
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(payload);
    let crc = crc16_ccitt(&body);
    let mut frame = Vec::new();
    frame.extend_from_slice(&SYNC);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// Parse the first complete frame in `bytes`, returning its payload if the
/// CRC checks out.
fn parse_frame(bytes: &[u8]) -> Option<Vec<u8>> {
    let pos = bytes.windows(2).position(|w| w == SYNC)?;
    let rest = &bytes[pos..];
    if rest.len() < 4 {
        return None;
    }
    let len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
    if rest.len() < 4 + len + 2 {
        return None;
    }
    let crc_calc = crc16_ccitt(&rest[2..4 + len]);
    let crc_rx = u16::from_be_bytes([rest[4 + len], rest[5 + len]]);
    (crc_calc == crc_rx).then(|| rest[4..4 + len].to_vec())
}

fn obs<'a>(
    sc: &'a SpacecraftState,
    sensors: &'a Sensors,
    act: &'a ActuatorTelemetry,
) -> TickInput<'a> {
    TickInput {
        t: 0.0,
        spacecraft: sc,
        epoch: None,
        sensors,
        actuators: act,
    }
}

// ─────────────────────────── tests ─────────────────────────────

#[test]
fn framed_command_switches_mode_and_replies() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    // Uplink a framed "nadir" command over the byte stream.
    ctrl.stream_deliver(STREAM, build_frame(b"nadir"));
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update");

    // The FSW deframed it, applied the mode (settled → allowed), and wrote
    // a framed reply carrying the resulting mode.
    let reply = ctrl.stream_take(STREAM);
    assert!(!reply.is_empty(), "expected a framed reply");
    assert_eq!(parse_frame(&reply).as_deref(), Some(&b"nadir"[..]));
}

#[test]
fn frame_is_reassembled_across_ticks() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    let frame = build_frame(b"nadir");
    let (head, tail) = frame.split_at(frame.len() / 2);

    // Tick 0: only the first half arrives — no complete frame, no reply.
    ctrl.stream_deliver(STREAM, head.to_vec());
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update 0");
    assert!(
        ctrl.stream_take(STREAM).is_empty(),
        "partial frame must not produce a reply"
    );

    // Tick 1: the rest arrives — the frame is reassembled and acted on.
    ctrl.stream_deliver(STREAM, tail.to_vec());
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update 1");
    assert_eq!(
        parse_frame(&ctrl.stream_take(STREAM)).as_deref(),
        Some(&b"nadir"[..])
    );
}

#[test]
fn corrupt_crc_frame_is_dropped() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    // Flip a payload byte after the CRC was computed → CRC mismatch.
    let mut frame = build_frame(b"nadir");
    let payload_start = 4;
    frame[payload_start] ^= 0xFF;

    ctrl.stream_deliver(STREAM, frame);
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update");

    // The FSW drops the corrupt frame: no reply (mode stays detumble — see
    // the unchanged-mode reply asserted in the gated/tumbling test).
    assert!(
        ctrl.stream_take(STREAM).is_empty(),
        "corrupt frame must be dropped"
    );
}

#[test]
fn nadir_blocked_while_tumbling() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(TUMBLING),
        ActuatorTelemetry::default(),
    );

    ctrl.stream_deliver(STREAM, build_frame(b"nadir"));
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update");

    // Gate blocked the switch; the reply reports the unchanged mode.
    assert_eq!(
        parse_frame(&ctrl.stream_take(STREAM)).as_deref(),
        Some(&b"detumble"[..])
    );
}

#[test]
fn duplicate_close_is_idempotent() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    // A bridge tearing down may signal close more than once. A close is a
    // metadata event with no bytes, so it must stay idempotent — it must not
    // latch a host fault (which would fail `update`).
    ctrl.stream_close(STREAM);
    ctrl.stream_close(STREAM);
    ctrl.update(&obs(&sc, &sensors, &act))
        .expect("duplicate close must not fault the simulation");
}

#[test]
fn resyncs_past_a_bogus_length_prefix() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    // A false sync followed by an implausible LEN (0xFFFF) — if the FSW
    // trusted it, it would buffer toward a 64 KiB frame that never arrives
    // and never process anything. It must resync and still handle the valid
    // frame that follows.
    let mut buf = vec![0xEB, 0x90, 0xFF, 0xFF];
    buf.extend(build_frame(b"nadir"));
    ctrl.stream_deliver(STREAM, buf);
    ctrl.update(&obs(&sc, &sensors, &act)).expect("update");

    assert_eq!(
        parse_frame(&ctrl.stream_take(STREAM)).as_deref(),
        Some(&b"nadir"[..]),
        "FSW must resync past the bogus length and process the real frame"
    );
}

#[test]
fn empty_delivery_to_undeclared_stream_still_faults() {
    let Some(mut ctrl) = load() else { return };
    let (sc, sensors, act) = (
        spacecraft(),
        gyro_sensors(SETTLED),
        ActuatorTelemetry::default(),
    );

    // An *empty, non-close* chunk to an undeclared stream is a host wiring
    // bug. It must still be validated (fault) — not silently skipped just
    // because it carries no bytes (only a pure close is exempt).
    ctrl.stream_deliver("not-a-real-stream", Vec::new());
    assert!(
        ctrl.update(&obs(&sc, &sensors, &act)).is_err(),
        "delivery to an undeclared stream must fault the simulation"
    );
}
