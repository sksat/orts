//! `WasmController`: a `PluginController` backed by a WebAssembly
//! Component guest.
//!
//! The guest targets the `plugin` world and uses a main-loop style:
//! it exports `run(config)` and imports `tick-io::{wait-tick, send-command}`.
//! Callback-style guests (via the `Plugin` trait in `orts-plugin-sdk`)
//! are automatically wrapped into this same shape by the SDK macro.
//!
//! ## Architecture
//!
//! Each `WasmController` spawns a **dedicated worker thread** that owns
//! the `Store<HostState>` and the guest instance. The worker thread
//! enters `call_run(&mut store, config)` which blocks for the entire
//! lifetime of the guest. Inside, the guest calls `wait_tick` → the
//! worker blocks on `input_rx.recv()`. The outer thread calls `update()`
//! which sends a `TickInput` through `input_tx` and receives the captured
//! command through `output_rx`.
//!
//! Sync wasmtime is sufficient: the guest runs until it blocks on
//! `wait_tick`, which blocks the worker thread on the channel. No
//! fiber / JSPI is needed on the host side (though the same guest
//! binary can run in a browser via JSPI).
//!
//! ## Lifecycle
//!
//! ```text
//! WasmEngine::new()           -> Engine (shared, Arc)
//! Component::new(&engine, ..) -> Component (shared, Arc)
//! Linker + Plugin::add_to_linker -> PluginPre (shared, Arc)
//! WasmController::new(pre, label, config):
//!   - spawn worker thread
//!   - worker: Store::new, instantiate, call metadata() + call run()
//!   - outer: receive metadata, return
//! WasmController::update(input):
//!   - send input via channel
//!   - receive command from channel
//! Drop WasmController:
//!   - drop input_tx → guest's wait_tick fails → worker thread exits
//! ```

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use wasmtime::Store;
use wasmtime::component::Component;

use super::convert::sync as convert;
use super::engine::WasmEngine;
use super::sync_bindings::orts::plugin::types as wit;
use super::sync_bindings::{Plugin, PluginPre};
use super::sync_host_state::{GuestResponse, HostState};

use crate::plugin::controller::PluginController;
use crate::plugin::tick_input::TickInput;
use crate::plugin::{Command, PluginError};

/// A `PluginController` backed by a WebAssembly Component guest.
pub struct WasmController {
    /// Worker thread handle. Joined on Drop.
    worker: Option<thread::JoinHandle<()>>,
    /// Channel for sending tick inputs to the worker.
    input_tx: Option<mpsc::SyncSender<wit::TickInput>>,
    /// Channel for receiving guest responses from the worker.
    output_rx: mpsc::Receiver<GuestResponse>,
    /// Cached sample period from the guest's `metadata()` export,
    /// queried once at startup.
    sample_period: f64,
    /// Cached controller name.
    name: String,
    /// Current mission mode name, refreshed by the worker thread.
    /// Not yet implemented — always `None` in the current design.
    _current_mode: Arc<Mutex<Option<String>>>,
}

impl WasmController {
    /// Instantiate a WASM guest controller for one satellite.
    ///
    /// Spawns a dedicated worker thread that owns the `Store` and
    /// drives the guest's `run()` loop. Returns after the guest's
    /// `metadata()` has been called (so `sample_period` is known).
    pub fn new(
        pre: &PluginPre<HostState>,
        label: impl Into<String>,
        config: &str,
    ) -> Result<Self, PluginError> {
        let label = label.into();
        let config = config.to_string();
        let pre = pre.clone();

        // Channels for outer ↔ worker communication.
        let (input_tx, input_rx) = mpsc::sync_channel::<wit::TickInput>(1);
        let (output_tx, output_rx) = mpsc::sync_channel::<GuestResponse>(1);
        // Separate metadata channel used only during startup so the
        // outer thread can synchronously wait for `metadata()` without
        // consuming from the regular output queue.
        let (metadata_tx, metadata_rx) = mpsc::sync_channel::<Result<f64, String>>(1);

        let current_mode = Arc::new(Mutex::new(None));
        let worker_current_mode = Arc::clone(&current_mode);
        let worker_label = label.clone();

        let worker = thread::Builder::new()
            .name(format!("wasm-plugin-{label}"))
            .spawn(move || {
                worker_main(
                    pre,
                    worker_label,
                    config,
                    input_rx,
                    output_tx,
                    metadata_tx,
                    worker_current_mode,
                );
            })
            .map_err(|e| PluginError::Init(format!("failed to spawn worker thread: {e}")))?;

        // Wait for the worker to send metadata (or fail).
        let sample_period = metadata_rx
            .recv()
            .map_err(|_| PluginError::Init("worker thread exited before metadata".to_string()))?
            .map_err(|e| PluginError::Init(format!("metadata failed: {e}")))?;

        Ok(Self {
            worker: Some(worker),
            input_tx: Some(input_tx),
            output_rx,
            sample_period,
            name: format!("wasm:{label}"),
            _current_mode: current_mode,
        })
    }

    /// Pre-link a Component against the host imports.
    pub fn prepare(
        engine: &Arc<WasmEngine>,
        component: &Component,
    ) -> Result<PluginPre<HostState>, PluginError> {
        let mut linker = wasmtime::component::Linker::new(engine.inner());
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| PluginError::Init(format!("WASI add_to_linker failed: {e}")))?;
        Plugin::add_to_linker::<HostState, HostState>(&mut linker, |state| state)
            .map_err(|e| PluginError::Init(format!("add_to_linker failed: {e}")))?;
        let instance_pre = linker
            .instantiate_pre(component)
            .map_err(|e| PluginError::Init(format!("instantiate_pre failed: {e}")))?;
        PluginPre::new(instance_pre)
            .map_err(|e| PluginError::Init(format!("PluginPre::new failed: {e}")))
    }
}

impl Drop for WasmController {
    fn drop(&mut self) {
        // Drop the input sender so the worker's `wait_tick` unblocks
        // with an error and the guest's run() returns.
        self.input_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl PluginController for WasmController {
    fn name(&self) -> &str {
        &self.name
    }

    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn update(&mut self, obs: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
        let wit_obs = convert::tick_input_to_wit(obs);

        let input_tx = self
            .input_tx
            .as_ref()
            .ok_or_else(|| PluginError::Runtime("controller is shut down".to_string()))?;

        input_tx
            .send(wit_obs)
            .map_err(|_| PluginError::Runtime("worker thread exited".to_string()))?;

        match self
            .output_rx
            .recv()
            .map_err(|_| PluginError::Runtime("worker thread exited".to_string()))?
        {
            GuestResponse::Command(Some(wit_cmd)) => convert::command_from_wit(wit_cmd).map(Some),
            GuestResponse::Command(None) => Ok(None),
            GuestResponse::Done(Ok(())) => Err(PluginError::Runtime(
                "guest run() returned early".to_string(),
            )),
            GuestResponse::Done(Err(e)) => Err(PluginError::Runtime(format!("guest error: {e}"))),
        }
    }
}

/// Worker thread entry point.
///
/// Owns the `Store` and drives `call_run()` for the guest's lifetime.
/// Communication with the outer `WasmController` happens through the
/// mpsc channels stored inside `HostState`.
fn worker_main(
    pre: PluginPre<HostState>,
    label: String,
    config: String,
    input_rx: mpsc::Receiver<wit::TickInput>,
    output_tx: mpsc::SyncSender<GuestResponse>,
    metadata_tx: mpsc::SyncSender<Result<f64, String>>,
    current_mode: Arc<Mutex<Option<String>>>,
) {
    let engine = pre.engine();
    let host_state = HostState::new(&label, input_rx, output_tx.clone(), current_mode);
    let mut store = Store::new(engine, host_state);

    let plugin = match pre.instantiate(&mut store) {
        Ok(p) => p,
        Err(e) => {
            let _ = metadata_tx.send(Err(format!("instantiate: {e}")));
            return;
        }
    };

    // Query metadata first (before run() takes over the thread).
    // `metadata(config)` also validates config — a bad config fails
    // here instead of on the first `update()` call.
    let metadata = match plugin.call_metadata(&mut store, &config) {
        Ok(Ok(md)) => md,
        Ok(Err(guest_err)) => {
            let _ = metadata_tx.send(Err(format!("metadata: {guest_err}")));
            return;
        }
        Err(e) => {
            let _ = metadata_tx.send(Err(format!("metadata call: {e}")));
            return;
        }
    };
    // Validate sample_period — guest is supposed to return positive
    // finite values, but host-side scheduler code expects this too.
    let sample_period_s = metadata.sample_period_s;
    if !sample_period_s.is_finite() || sample_period_s <= 0.0 {
        let _ = metadata_tx.send(Err(format!(
            "guest returned invalid sample_period: {sample_period_s}"
        )));
        return;
    }
    let _ = metadata_tx.send(Ok(sample_period_s));

    // Drive the guest's run() loop. This blocks for the entire
    // lifetime of the guest. On normal termination or error, send
    // a Done signal through the output channel.
    let result = plugin.call_run(&mut store, &config);
    let done = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(guest_err)) => Err(guest_err),
        Err(trap) => Err(format!("trap: {trap}")),
    };
    let _ = output_tx.send(GuestResponse::Done(done));
}
