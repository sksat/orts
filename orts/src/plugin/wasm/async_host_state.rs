//! Host-side state for the async WASM backend.
//!
//! Mirror of [`super::sync_host_state`] but implements the
//! `async fn` variants of the WIT `host-env` and `tick-io` Host
//! traits required by the async bindgen output in
//! [`super::async_bindings`]. Communication with the outer
//! `AsyncWasmController` handle is done via `tokio::sync::mpsc`
//! channels rather than `std::sync::mpsc`, so that the satellite
//! task can yield to the runtime on every `wait_tick`.

use tobari::magnetic::{MagneticFieldModel, TiltedDipole};
use tokio::sync::mpsc;

use super::async_bindings::orts::plugin::host_env;
use super::async_bindings::orts::plugin::tick_io;
use super::async_bindings::orts::plugin::types as wit;

/// Response sent back to the outer `AsyncWasmController` via
/// `output_tx`. Same shape as the sync variant — only the channel
/// implementation differs.
#[derive(Debug)]
pub(super) enum GuestResponse {
    /// A command captured from the previous tick. `None` means the
    /// guest did not call `send_command` during that tick.
    Command(Option<wit::Command>),
    /// The guest's `run()` function returned or errored. No more
    /// commands will be produced.
    Done(Result<(), String>),
}

/// Per-satellite host state for the async backend.
pub(super) struct AsyncHostState {
    pub(super) label: String,
    pub(super) field: TiltedDipole,
    pub(super) wasi: wasmtime_wasi::WasiCtx,
    pub(super) table: wasmtime_wasi::ResourceTable,

    pub(super) input_rx: mpsc::Receiver<Option<wit::TickInput>>,
    pub(super) output_tx: mpsc::Sender<GuestResponse>,
    pub(super) pending_cmd: Option<wit::Command>,
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
        let pos = arika::SimpleEci::new(position_eci_km.x, position_eci_km.y, position_eci_km.z);
        let ep = arika::epoch::Epoch::from_jd(epoch.julian_date);
        let b = self.field.field_eci(&pos, &ep);
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
            let cmd = self.pending_cmd.take();
            let _ = self.output_tx.send(GuestResponse::Command(cmd)).await;
        } else {
            self.is_first_wait = false;
        }
        self.input_rx.recv().await.flatten()
    }

    async fn send_command(&mut self, cmd: wit::Command) {
        // Last-write-wins: if the guest calls send_command multiple
        // times in one tick, only the last one survives.
        self.pending_cmd = Some(cmd);
    }
}
