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

use super::sync_bindings::orts::plugin::host_env;
use super::sync_bindings::orts::plugin::msg_io;
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
    /// import. Phase P1 defaults to `TiltedDipole::earth()`; Phase
    /// D-5 will replace this with an IGRF spherical-harmonic model.
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
    /// Frozen msg-io inbox for the current tick. Set on each
    /// `wait_tick` from the incoming [`TickPacket`]; drained by
    /// `recv-batch`. Leftover messages are dropped at the next tick
    /// boundary (the outer side decides carry-over policy).
    inbox: VecDeque<wit::Message>,
    /// Messages the guest emitted via `send-message` during the current
    /// tick (append). Forwarded to the outer thread on the next
    /// `wait_tick` and then cleared.
    outbox: Vec<wit::Outbound>,
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

// ─── host-env interface ─────────────────────────────────────────

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

// ─── tick-io interface ──────────────────────────────────────────

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
            // If the outer side has dropped the receiver (Controller
            // was dropped), this send fails — that's fine, we'll
            // return None below and the guest will exit cleanly.
            let _ = self
                .output_tx
                .send(GuestResponse::Tick { command, outgoing });
        } else {
            self.is_first_wait = false;
        }

        // `recv()` returns Err only when the sender half (input_tx in
        // the outer WasmController) has been dropped. We translate this
        // into `None` so the guest can exit its main loop without the
        // host function panicking.
        match self.input_rx.recv() {
            Ok(packet) => {
                // Freeze this tick's inbox. Any messages left undrained
                // from the previous tick are dropped here.
                self.inbox = packet.inbox.into();
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
    /// `update()` on the next `wait_tick`; `src` / `host-seq` /
    /// `deliver-tick` are stamped by the host there.
    fn send_message(&mut self, msg: wit::Outbound) {
        self.outbox.push(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::host_env::Host as _;
    use super::msg_io::Host as _;
    use super::*;

    fn make_state() -> HostState {
        let (_, input_rx) = mpsc::channel();
        let (output_tx, _) = mpsc::sync_channel(1);
        let current_mode = Arc::new(Mutex::new(None));
        HostState::new("test", input_rx, output_tx, current_mode)
    }

    fn test_message(host_seq: u64) -> wit::Message {
        wit::Message {
            src: wit::NodeId::Ground,
            dst: wit::NodeId::Satellite(0),
            kind: "test.cmd.v1".to_string(),
            host_seq,
            deliver_tick: 0,
            payload: wit::Payload::KeyValue(vec![]),
        }
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
        assert_eq!(b1[0].host_seq, 0);
        assert_eq!(b1[1].host_seq, 1);
        // max larger than remaining → take the rest.
        let b2 = state.recv_batch(10);
        assert_eq!(b2.len(), 3);
        assert_eq!(b2[0].host_seq, 2);
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
