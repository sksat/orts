//! Host-side state for the wasmtime `Store` and implementation of
//! the WIT `host-env` and `tick-io` import interfaces.
//!
//! Each satellite's `WasmController` spawns a dedicated worker thread
//! that owns the `Store<HostState>` and runs the guest's `run()` loop.
//! The worker thread and the outer controller communicate via blocking
//! mpsc channels:
//!
//! - **Inputs** (`update()` → guest's `wait_tick`): outer thread sends
//!   `TickInput`, guest's `wait_tick` blocks on `input_rx.recv()`.
//! - **Outputs** (guest's `send_command` → `update()` return): guest
//!   captures `Command` in `pending_cmd`; on the next `wait_tick` the
//!   pending command is forwarded through `output_tx` and the outer
//!   `update()` receives it.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tobari::magnetic::TiltedDipole;

use super::stream_state::{
    DEFAULT_STREAM_CAPACITY, ReadOutcome, StreamDelivery, Streams, WriteOutcome,
};
use super::sync_bindings::orts::plugin::host_env;
use super::sync_bindings::orts::plugin::msg_io;
use super::sync_bindings::orts::plugin::stream_io;
use super::sync_bindings::orts::plugin::tick_io;
use super::sync_bindings::orts::plugin::types as wit;

// The `types` interface has no host functions, but the bindgen-generated
// `add_to_linker` requires a blanket `types::Host` impl for the host state.
impl wit::Host for HostState {}

/// One tick's worth of host → guest input.
///
/// Carries the physical [`wit::TickInput`] snapshot **and** the frozen
/// inbox of msg-io messages the host has decided to deliver this tick.
/// Freezing happens at the tick boundary (when the outer `update()`
/// sends this packet), which keeps `recv-batch` deterministic regardless
/// of when the guest drains it.
pub(super) struct TickPacket {
    pub input: wit::TickInput,
    pub inbox: Vec<wit::Message>,
    /// stream-io inbound byte deliveries (and close signals) frozen into
    /// this tick. Applied before the guest runs, like `inbox`.
    pub stream_inbound: Vec<StreamDelivery>,
}

/// Guest response delivered to the outer `update()` via `output_tx`.
///
/// Sent by the worker thread at the start of each `wait_tick` call
/// (except the very first one, which primes the guest with an initial
/// input without producing a response).
pub(super) enum GuestResponse {
    /// The outcome of one tick: the actuator command (possibly `None`
    /// if the guest didn't call `send_command`) plus every message the
    /// guest emitted via `send-message` during the tick (append
    /// semantics — a `Vec`, possibly empty).
    Tick {
        command: Option<wit::Command>,
        outgoing: Vec<wit::Outbound>,
        /// Bytes the guest wrote to each named stream this tick, flushed
        /// to the outer controller for pickup by the host bridge.
        stream_outbound: Vec<(String, Vec<u8>)>,
        /// Host-authoritative stream fault (overrun / wiring inconsistency)
        /// latched up to this tick. `Some(_)` tells `update()` to halt the
        /// simulation with a `PluginError`, independent of whether the guest
        /// observed the `Err(overrun)` on its own `read`/`write`.
        stream_fault: Option<String>,
    },
    /// The guest's `run()` function returned or errored. No more
    /// responses will be produced.
    Done(Result<(), String>),
}

/// Per-satellite host state stored inside each `wasmtime::Store`.
///
/// Holds the WASI context (required by Rust std-based guests), the
/// geomagnetic field model, and the channels used to communicate with
/// the outer `WasmController`.
pub struct HostState {
    /// Human-readable satellite / controller label for log messages.
    pub label: String,
    /// Geomagnetic field model used by the `magnetic-field-eci` host
    /// import. Currently fixed to `TiltedDipole::earth()`.
    field: TiltedDipole,
    /// WASI context.
    wasi: wasmtime_wasi::WasiCtx,
    /// Resource table for WASI resources.
    table: wasmtime_wasi::ResourceTable,

    /// Receiver for tick packets (physical input + frozen inbox) from
    /// the outer `update()` call.
    input_rx: mpsc::Receiver<TickPacket>,
    /// Sender for guest responses (tick result / done signal).
    output_tx: mpsc::SyncSender<GuestResponse>,
    /// Command captured from the most recent `send_command` call,
    /// forwarded to the outer thread on the next `wait_tick`.
    pending_cmd: Option<wit::Command>,
    /// Frozen msg-io inbox for the current tick. On each `wait_tick`
    /// the incoming [`TickPacket`]'s deliveries are appended; drained by
    /// `recv-batch`. Messages left undrained carry over to the next tick
    /// (appended before the new deliveries) — backpressure per the
    /// msg-io contract.
    inbox: VecDeque<wit::Message>,
    /// Messages the guest emitted via `send-message` during the current
    /// tick (append). Forwarded to the outer thread on the next
    /// `wait_tick` and then cleared.
    outbox: Vec<wit::Outbound>,
    /// stream-io byte streams (declared at construction). Frozen inbound
    /// is applied each `wait_tick`; guest `read`/`write` operate here;
    /// outbound is drained into the next `GuestResponse`.
    streams: Streams,
    /// `true` until the first `wait_tick` call. The very first call
    /// must NOT send a response (there's nothing to report yet), it
    /// just blocks waiting for the first input.
    is_first_wait: bool,

    /// Current mode name reported by the guest's `current_mode` export.
    /// Stored in an `Arc<Mutex>` so the outer `WasmController` can read
    /// it without owning the `Store`. Updated by the worker thread.
    #[allow(dead_code)] // TODO: wire up current-mode polling
    current_mode: Arc<Mutex<Option<String>>>,
}

impl HostState {
    pub(super) fn new(
        label: impl Into<String>,
        input_rx: mpsc::Receiver<TickPacket>,
        output_tx: mpsc::SyncSender<GuestResponse>,
        current_mode: Arc<Mutex<Option<String>>>,
        stream_names: Vec<String>,
    ) -> Self {
        Self {
            label: label.into(),
            field: TiltedDipole::earth(),
            wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
            table: wasmtime_wasi::ResourceTable::new(),
            input_rx,
            output_tx,
            pending_cmd: None,
            inbox: VecDeque::new(),
            outbox: Vec::new(),
            streams: Streams::new(stream_names, DEFAULT_STREAM_CAPACITY),
            is_first_wait: true,
            current_mode,
        }
    }
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

// host-env interface

impl host_env::Host for HostState {
    fn log(&mut self, level: host_env::LogLevel, message: String) {
        match level {
            host_env::LogLevel::Trace => log::trace!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Debug => log::debug!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Info => log::info!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Warn => log::warn!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Error => log::error!("[wasm:{}] {}", self.label, message),
        }
    }

    fn magnetic_field_eci(&mut self, position_eci_km: wit::Vec3, epoch: wit::Epoch) -> wit::Vec3 {
        let pos = arika::frame::Vec3::<arika::frame::SimpleEci>::new(
            position_eci_km.x,
            position_eci_km.y,
            position_eci_km.z,
        );
        let epoch = arika::epoch::Epoch::from_jd(epoch.julian_date);
        let b = crate::magnetic::field_eci(&self.field, &pos, &epoch);
        wit::Vec3 {
            x: b.x(),
            y: b.y(),
            z: b.z(),
        }
    }
}

// tick-io interface

impl tick_io::Host for HostState {
    /// Called by the guest at the start of each control loop iteration.
    ///
    /// Blocks the worker thread on `input_rx.recv()` until the outer
    /// `update()` sends the next `TickInput`. On subsequent calls (not
    /// the first), forwards the pending command from the previous tick
    /// through `output_tx` before blocking.
    ///
    /// Returns `None` if the outer `WasmController` has been dropped,
    /// signaling the guest to exit its main loop cleanly.
    fn wait_tick(&mut self) -> Option<wit::TickInput> {
        if !self.is_first_wait {
            let command = self.pending_cmd.take();
            let outgoing = std::mem::take(&mut self.outbox);
            let stream_outbound = self.streams.drain_outbound();
            let stream_fault = self.streams.fault().map(str::to_string);
            // If the outer side has dropped the receiver (Controller
            // was dropped), this send fails — that's fine, we'll
            // return None below and the guest will exit cleanly.
            let _ = self.output_tx.send(GuestResponse::Tick {
                command,
                outgoing,
                stream_outbound,
                stream_fault,
            });
        } else {
            self.is_first_wait = false;
        }

        // `recv()` returns Err only when the sender half (input_tx in
        // the outer WasmController) has been dropped. We translate this
        // into `None` so the guest can exit its main loop without the
        // host function panicking.
        match self.input_rx.recv() {
            Ok(packet) => {
                // Freeze this tick's inbox: append the newly delivered
                // messages after any left undrained from the previous tick
                // (carry-over / backpressure per the msg-io contract).
                self.inbox.extend(packet.inbox);
                // Freeze this tick's stream-io byte deliveries the same way.
                for d in packet.stream_inbound {
                    // Skip `deliver` only for a *pure close* event (closed +
                    // no bytes) so a duplicate / teardown close stays
                    // idempotent. Any other delivery — including an empty
                    // non-close chunk — still goes through `deliver` to
                    // validate the stream name/state (latching a host fault
                    // on a wiring bug: undeclared / already-closed stream).
                    if !(d.closed && d.bytes.is_empty()) {
                        self.streams.deliver(&d.name, &d.bytes);
                    }
                    if d.closed {
                        self.streams.close(&d.name);
                    }
                }
                Some(packet.input)
            }
            Err(_) => None,
        }
    }

    fn send_command(&mut self, cmd: wit::Command) {
        // Last-write-wins semantics: if the guest calls send_command
        // multiple times in one tick, the last value is kept.
        self.pending_cmd = Some(cmd);
    }
}

// ─── msg-io interface ───────────────────────────────────────────

impl msg_io::Host for HostState {
    /// Drain up to `max` messages from this tick's frozen inbox, in
    /// host-assigned order. Returns an empty list once the inbox is
    /// exhausted for the tick.
    fn recv_batch(&mut self, max: u32) -> Vec<wit::Message> {
        let n = (max as usize).min(self.inbox.len());
        self.inbox.drain(..n).collect()
    }

    /// Capture an outbound message (append). Forwarded to the outer
    /// `update()` on the next `wait_tick`; `src` is stamped by the host there.
    fn send_message(&mut self, msg: wit::Outbound) {
        self.outbox.push(msg);
    }
}

// ─── stream-io interface ────────────────────────────────────────

impl stream_io::Host for HostState {
    /// Drain up to `max` bytes from a named stream's frozen inbound
    /// buffer. `Err(overrun)` here is the guest-visible signal; the host
    /// also halts the simulation via the latched fault.
    fn read(&mut self, name: String, max: u32) -> Result<wit::StreamRead, wit::StreamError> {
        match self.streams.read(&name, max as usize) {
            ReadOutcome::Data(bytes) => Ok(wit::StreamRead::Data(bytes)),
            ReadOutcome::NoData => Ok(wit::StreamRead::NoData),
            ReadOutcome::Closed => Ok(wit::StreamRead::Closed),
            ReadOutcome::Overrun => Err(wit::StreamError::Overrun),
            ReadOutcome::Unknown => Err(wit::StreamError::UnknownStream),
        }
    }

    /// Append bytes to a named stream's outbound buffer (flushed at the
    /// next tick boundary).
    fn write(&mut self, name: String, bytes: Vec<u8>) -> Result<(), wit::StreamError> {
        match self.streams.write(&name, &bytes) {
            WriteOutcome::Ok => Ok(()),
            WriteOutcome::Overrun => Err(wit::StreamError::Overrun),
            WriteOutcome::Closed => Err(wit::StreamError::Closed),
            WriteOutcome::Unknown => Err(wit::StreamError::UnknownStream),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::host_env::Host as _;
    use super::msg_io::Host as _;
    use super::tick_io::Host as _;
    use super::*;

    fn make_state() -> HostState {
        let (_, input_rx) = mpsc::channel();
        let (output_tx, _) = mpsc::sync_channel(1);
        let current_mode = Arc::new(Mutex::new(None));
        HostState::new("test", input_rx, output_tx, current_mode, Vec::new())
    }

    /// `seq` is encoded into `kind` so tests can assert delivery order /
    /// carry-over by message identity.
    fn test_message(seq: u64) -> wit::Message {
        wit::Message {
            src: wit::NodeId::Ground,
            dst: wit::NodeId::Satellite(0),
            kind: format!("test.cmd.{seq}"),
            payload: wit::Payload::KeyValue(vec![]),
        }
    }

    fn dummy_tick_input() -> wit::TickInput {
        wit::TickInput {
            t: 0.0,
            spacecraft: wit::SpacecraftState {
                orbit: wit::OrbitalState {
                    position: wit::PositionEciKm {
                        x: 7000.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    velocity: wit::VelocityEciKms {
                        x: 0.0,
                        y: 7.5,
                        z: 0.0,
                    },
                },
                attitude: wit::AttitudeState {
                    orientation: wit::Quat {
                        w: 1.0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    angular_velocity: wit::Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                mass: 50.0,
            },
            epoch: None,
            sensors: wit::Sensors {
                magnetometers: vec![],
                gyroscopes: vec![],
                star_trackers: vec![],
                sun_sensors: vec![],
            },
            actuators: wit::ActuatorTelemetry { rw: None },
        }
    }

    #[test]
    fn wait_tick_carries_over_undrained_inbox() {
        let (input_tx, input_rx) = mpsc::channel::<TickPacket>();
        let (output_tx, _output_rx) = mpsc::sync_channel::<GuestResponse>(4);
        let current_mode = Arc::new(Mutex::new(None));
        let mut state = HostState::new("test", input_rx, output_tx, current_mode, Vec::new());

        // Tick 0 delivers two messages; the guest drains only one.
        input_tx
            .send(TickPacket {
                input: dummy_tick_input(),
                inbox: vec![test_message(0), test_message(1)],
                stream_inbound: vec![],
            })
            .unwrap();
        state.wait_tick();
        assert_eq!(state.recv_batch(1).len(), 1); // test.cmd.0 drained

        // Tick 1 delivers one more; the undrained test.cmd.1 must carry over,
        // with the newly frozen delivery appended after it.
        input_tx
            .send(TickPacket {
                input: dummy_tick_input(),
                inbox: vec![test_message(2)],
                stream_inbound: vec![],
            })
            .unwrap();
        state.wait_tick();

        let kinds: Vec<String> = state
            .recv_batch(10)
            .iter()
            .map(|m| m.kind.clone())
            .collect();
        assert_eq!(kinds, vec!["test.cmd.1", "test.cmd.2"]); // leftover first, then newly delivered
    }

    #[test]
    fn recv_batch_drains_frozen_inbox_in_order() {
        let mut state = make_state();
        for seq in 0..5u64 {
            state.inbox.push_back(test_message(seq));
        }
        // First batch: 2 of 5, in order.
        let b1 = state.recv_batch(2);
        assert_eq!(b1.len(), 2);
        assert_eq!(b1[0].kind, "test.cmd.0");
        assert_eq!(b1[1].kind, "test.cmd.1");
        // max larger than remaining → take the rest.
        let b2 = state.recv_batch(10);
        assert_eq!(b2.len(), 3);
        assert_eq!(b2[0].kind, "test.cmd.2");
        // Exhausted.
        assert!(state.recv_batch(1).is_empty());
    }

    #[test]
    fn recv_batch_zero_takes_nothing() {
        let mut state = make_state();
        state.inbox.push_back(test_message(0));
        assert!(state.recv_batch(0).is_empty());
        assert_eq!(state.inbox.len(), 1);
    }

    #[test]
    fn send_message_appends_to_outbox() {
        let mut state = make_state();
        assert!(state.outbox.is_empty());
        state.send_message(wit::Outbound {
            dst: wit::NodeId::Ground,
            kind: "test.tlm.v1".to_string(),
            payload: wit::Payload::Json("{}".to_string()),
        });
        state.send_message(wit::Outbound {
            dst: wit::NodeId::Ground,
            kind: "test.tlm.v2".to_string(),
            payload: wit::Payload::Binary(vec![1]),
        });
        // Append (not last-write-wins).
        assert_eq!(state.outbox.len(), 2);
        assert_eq!(state.outbox[0].kind, "test.tlm.v1");
        assert_eq!(state.outbox[1].kind, "test.tlm.v2");
    }

    #[test]
    fn magnetic_field_returns_finite_nonzero_for_leo() {
        let mut state = make_state();
        let pos = wit::Vec3 {
            x: 7000.0,
            y: 0.0,
            z: 0.0,
        };
        let epoch = wit::Epoch {
            julian_date: 2451545.0,
        };
        let b = state.magnetic_field_eci(pos, epoch);
        assert!(b.x.is_finite());
        assert!(b.y.is_finite());
        assert!(b.z.is_finite());
        let magnitude = (b.x * b.x + b.y * b.y + b.z * b.z).sqrt();
        assert!(
            magnitude > 1e-5 && magnitude < 1e-4,
            "expected LEO-range magnetic field (~20-60 µT), got {magnitude:.3e} T"
        );
    }
}
