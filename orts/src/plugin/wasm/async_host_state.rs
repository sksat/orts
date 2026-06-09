//! Host-side state for the async WASM backend.
//!
//! Mirror of [`super::sync_host_state`] but implements the
//! `async fn` variants of the WIT `host-env` and `tick-io` Host
//! traits required by the async bindgen output in
//! [`super::async_bindings`]. Communication with the outer
//! `AsyncWasmController` handle is done via `tokio::sync::mpsc`
//! channels rather than `std::sync::mpsc`, so that the satellite
//! task can yield to the runtime on every `wait_tick`.

use std::collections::VecDeque;

use tobari::magnetic::TiltedDipole;
use tokio::sync::mpsc;

use super::async_bindings::orts::plugin::host_env;
use super::async_bindings::orts::plugin::msg_io;
use super::async_bindings::orts::plugin::stream_io;
use super::async_bindings::orts::plugin::tick_io;
use super::async_bindings::orts::plugin::types as wit;
use super::stream_state::{ReadOutcome, StreamDelivery, Streams, WriteOutcome};

/// One tick's worth of host → guest input for the async backend.
///
/// Async mirror of [`super::sync_host_state::TickPacket`]: physical
/// snapshot + the frozen msg-io inbox for this tick.
pub(super) struct TickPacket {
    pub input: wit::TickInput,
    pub inbox: Vec<wit::Message>,
    /// stream-io inbound byte deliveries / close signals frozen this tick.
    pub stream_inbound: Vec<StreamDelivery>,
}

/// Response sent back to the outer `AsyncWasmController` via
/// `output_tx`. Same shape as the sync variant — only the channel
/// implementation differs.
#[derive(Debug)]
pub(super) enum GuestResponse {
    /// The outcome of one tick: the actuator command (possibly `None`)
    /// plus every message emitted via `send-message` (append).
    Tick {
        command: Option<wit::Command>,
        outgoing: Vec<wit::Outbound>,
        /// Bytes the guest wrote to each named stream this tick.
        stream_outbound: Vec<(String, Vec<u8>)>,
        /// Host-authoritative stream fault → halt the simulation.
        stream_fault: Option<String>,
    },
    /// The guest's `run()` function returned or errored. No more
    /// responses will be produced.
    Done(Result<(), String>),
}

/// Per-satellite host state for the async backend.
pub(super) struct AsyncHostState {
    pub(super) label: String,
    pub(super) field: TiltedDipole,
    pub(super) wasi: wasmtime_wasi::WasiCtx,
    pub(super) table: wasmtime_wasi::ResourceTable,

    pub(super) input_rx: mpsc::Receiver<Option<TickPacket>>,
    pub(super) output_tx: mpsc::Sender<GuestResponse>,
    pub(super) pending_cmd: Option<wit::Command>,
    /// Frozen msg-io inbox for the current tick (drained by `recv-batch`).
    pub(super) inbox: VecDeque<wit::Message>,
    /// Messages emitted via `send-message` this tick (append).
    pub(super) outbox: Vec<wit::Outbound>,
    /// stream-io byte streams (declared at construction).
    pub(super) streams: Streams,
    pub(super) is_first_wait: bool,
}

impl wasmtime_wasi::WasiView for AsyncHostState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for AsyncHostState {
    type Data<'a> = &'a mut AsyncHostState;
}

// The `types` interface has no host functions, but the bindgen-generated
// `add_to_linker` requires a blanket `types::Host` impl for the host state.
impl wit::Host for AsyncHostState {}

impl host_env::Host for AsyncHostState {
    async fn log(&mut self, level: host_env::LogLevel, message: String) {
        match level {
            host_env::LogLevel::Trace => log::trace!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Debug => log::debug!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Info => log::info!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Warn => log::warn!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Error => log::error!("[wasm:{}] {}", self.label, message),
        }
    }

    async fn magnetic_field_eci(
        &mut self,
        position_eci_km: wit::Vec3,
        epoch: wit::Epoch,
    ) -> wit::Vec3 {
        let pos = arika::frame::Vec3::<arika::frame::SimpleEci>::new(
            position_eci_km.x,
            position_eci_km.y,
            position_eci_km.z,
        );
        let ep = arika::epoch::Epoch::from_jd(epoch.julian_date);
        let b = crate::magnetic::field_eci(&self.field, &pos, &ep);
        wit::Vec3 {
            x: b.x(),
            y: b.y(),
            z: b.z(),
        }
    }
}

impl tick_io::Host for AsyncHostState {
    /// Called by the guest at the start of each control-loop iteration.
    ///
    /// On every call after the first, forwards the pending command
    /// from the previous tick to the outer controller via
    /// `output_tx`. Then awaits the next `TickInput` from `input_rx`.
    /// Returns `None` if the outer controller has been dropped, so
    /// the guest can exit its main loop cleanly.
    async fn wait_tick(&mut self) -> Option<wit::TickInput> {
        if !self.is_first_wait {
            let command = self.pending_cmd.take();
            let outgoing = std::mem::take(&mut self.outbox);
            let stream_outbound = self.streams.drain_outbound();
            let stream_fault = self.streams.fault().map(str::to_string);
            let _ = self
                .output_tx
                .send(GuestResponse::Tick {
                    command,
                    outgoing,
                    stream_outbound,
                    stream_fault,
                })
                .await;
        } else {
            self.is_first_wait = false;
        }
        // `recv` yields `None` when the outer side drops the sender;
        // an inner `None` is the explicit shutdown signal. Either way
        // the guest exits. `Some(packet)` freezes this tick's inbox.
        match self.input_rx.recv().await {
            Some(Some(packet)) => {
                // Carry over undrained messages; append the newly delivered
                // ones (backpressure per the msg-io contract).
                self.inbox.extend(packet.inbox);
                // Freeze this tick's stream-io byte deliveries the same way.
                for d in packet.stream_inbound {
                    // Skip `deliver` for close-only events (empty bytes) so a
                    // duplicate / teardown close stays idempotent (delivering
                    // to a closed/undeclared stream would latch a fault).
                    if !d.bytes.is_empty() {
                        self.streams.deliver(&d.name, &d.bytes);
                    }
                    if d.closed {
                        self.streams.close(&d.name);
                    }
                }
                Some(packet.input)
            }
            Some(None) | None => None,
        }
    }

    async fn send_command(&mut self, cmd: wit::Command) {
        // Last-write-wins: if the guest calls send_command multiple
        // times in one tick, only the last one survives.
        self.pending_cmd = Some(cmd);
    }
}

impl msg_io::Host for AsyncHostState {
    async fn recv_batch(&mut self, max: u32) -> Vec<wit::Message> {
        let n = (max as usize).min(self.inbox.len());
        self.inbox.drain(..n).collect()
    }

    async fn send_message(&mut self, msg: wit::Outbound) {
        self.outbox.push(msg);
    }
}

impl stream_io::Host for AsyncHostState {
    async fn read(&mut self, name: String, max: u32) -> Result<wit::StreamRead, wit::StreamError> {
        match self.streams.read(&name, max as usize) {
            ReadOutcome::Data(bytes) => Ok(wit::StreamRead::Data(bytes)),
            ReadOutcome::NoData => Ok(wit::StreamRead::NoData),
            ReadOutcome::Closed => Ok(wit::StreamRead::Closed),
            ReadOutcome::Overrun => Err(wit::StreamError::Overrun),
            ReadOutcome::Unknown => Err(wit::StreamError::UnknownStream),
        }
    }

    async fn write(&mut self, name: String, bytes: Vec<u8>) -> Result<(), wit::StreamError> {
        match self.streams.write(&name, &bytes) {
            WriteOutcome::Ok => Ok(()),
            WriteOutcome::Overrun => Err(wit::StreamError::Overrun),
            WriteOutcome::Closed => Err(wit::StreamError::Closed),
            WriteOutcome::Unknown => Err(wit::StreamError::UnknownStream),
        }
    }
}
