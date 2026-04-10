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

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tobari::magnetic::{MagneticFieldModel, TiltedDipole};

use super::bindings::orts::plugin::host_env;
use super::bindings::orts::plugin::tick_io;
use super::bindings::orts::plugin::types as wit;

// The `types` interface has no host functions, but the bindgen-generated
// `add_to_linker` requires a blanket `types::Host` impl for the host state.
impl wit::Host for HostState {}

/// Guest response delivered to the outer `update()` via `output_tx`.
///
/// Sent by the worker thread at the start of each `wait_tick` call
/// (except the very first one, which primes the guest with an initial
/// input without producing a response).
pub(super) enum GuestResponse {
    /// A command from the previous tick (possibly `None` if the guest
    /// didn't call `send_command` during that tick).
    Command(Option<wit::Command>),
    /// The guest's `run()` function returned or errored. No more
    /// commands will be produced.
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

    /// Receiver for tick inputs from the outer `update()` call.
    input_rx: mpsc::Receiver<wit::TickInput>,
    /// Sender for guest responses (commands / done signal).
    output_tx: mpsc::SyncSender<GuestResponse>,
    /// Command captured from the most recent `send_command` call,
    /// forwarded to the outer thread on the next `wait_tick`.
    pending_cmd: Option<wit::Command>,
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
        input_rx: mpsc::Receiver<wit::TickInput>,
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
        let pos = kaname::Eci::new(position_eci_km.x, position_eci_km.y, position_eci_km.z);
        let epoch = kaname::epoch::Epoch::from_jd(epoch.julian_date);
        let b = self.field.field_eci(&pos, &epoch);
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
            let cmd = self.pending_cmd.take();
            // If the outer side has dropped the receiver (Controller
            // was dropped), this send fails — that's fine, we'll
            // return None below and the guest will exit cleanly.
            let _ = self.output_tx.send(GuestResponse::Command(cmd));
        } else {
            self.is_first_wait = false;
        }

        // `recv()` returns Err only when the sender half (input_tx in
        // the outer WasmController) has been dropped. We translate this
        // into `None` so the guest can exit its main loop without the
        // host function panicking.
        self.input_rx.recv().ok()
    }

    fn send_command(&mut self, cmd: wit::Command) {
        // Last-write-wins semantics: if the guest calls send_command
        // multiple times in one tick, the last value is kept.
        self.pending_cmd = Some(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::host_env::Host as _;
    use super::*;

    fn make_state() -> HostState {
        let (_, input_rx) = mpsc::channel();
        let (output_tx, _) = mpsc::sync_channel(1);
        let current_mode = Arc::new(Mutex::new(None));
        HostState::new("test", input_rx, output_tx, current_mode)
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
