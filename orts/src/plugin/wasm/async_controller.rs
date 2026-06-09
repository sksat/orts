//! Async WASM plugin controller: fiber-based backend.
//!
//! Parallel to [`super::sync_controller`] but backed by the async
//! bindgen output and a shared [`super::async_runtime::AsyncRuntime`].
//! The guest runs as a long-lived `tokio::spawn`ed task that owns its
//! `Store<AsyncHostState>` and drives `call_run(&mut store, config)`
//! to completion. The outer controller handle communicates with the
//! task through `tokio::sync::mpsc` channels and uses
//! `Handle::block_on` to expose a sync `PluginController` API.
//!
//! ## Why this exists
//!
//! The sync backend spawns one OS thread per satellite. That works
//! for small fleets but does not scale to constellations of hundreds
//! or thousands of satellites (8-16 MB stack per thread × 1000 sats
//! ≈ 8-16 GB of reserved address space). This backend multiplexes N
//! satellites onto the single worker thread inside `AsyncRuntime`,
//! so per-satellite memory is dominated by the Store + tokio task
//! (a few KB each) instead of a full OS thread stack.
//!
//! ## Trade-offs
//!
//! - **Dispatch overhead**: roughly on par with the sync backend
//!   per `update()` call on a realistic guest (measured ~13-14 µs
//!   for `pd-rw-control` in release on Pulley, both backends).
//!   The per-tick cost is dominated by guest computation through
//!   the Pulley interpreter, so the channel round-trip / fiber
//!   suspend-resume overhead is not a practical differentiator.
//! - **Memory per satellite**: dramatic win for async. Sync spawns
//!   one OS thread per controller (~8-16 MB of reserved stack each);
//!   async spawns a tokio task (~few KB). This is what makes 1000+
//!   satellites feasible.
//! - **Isolation**: a misbehaving guest (infinite loop, runaway
//!   computation) can stall the single worker thread and starve all
//!   other satellites on it. Mitigation: future work with
//!   `Engine::increment_epoch()` + epoch deadlines.
//! - **Determinism**: the runtime uses `worker_threads(1)` so task
//!   scheduling is stable across runs, which is required for the
//!   oracle / replay workflow.
//!
//! ## Calling context constraint
//!
//! [`AsyncWasmController::update`] and `Drop` internally call
//! `tokio::runtime::Handle::block_on`, which **panics if called from
//! directly inside another tokio task**. This means:
//!
//! - ✅ Safe: calling from plain sync code (CLI `orts run`).
//! - ✅ Safe: calling from inside `tokio::task::spawn_blocking` even
//!   if the outer thread is driven by a different tokio runtime
//!   (this is the `orts serve` path, which wraps the sim loop in
//!   `spawn_blocking` — see `cli/src/commands/serve/manager.rs`).
//! - ❌ Unsafe: calling directly from within a regular tokio task
//!   of any runtime (will panic with "Cannot start a runtime from
//!   within a runtime").
//!
//! Callers embedding orts inside their own tokio host must ensure
//! `PluginController::update` is reached via a blocking-thread
//! boundary.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use wasmtime::Store;
use wasmtime::component::Component;

use super::async_bindings::Plugin as AsyncPlugin;
use super::async_bindings::PluginPre as AsyncPluginPre;
use super::async_bindings::orts::plugin::types as wit;
use super::async_host_state::{AsyncHostState, GuestResponse, TickPacket};
use super::async_runtime::AsyncRuntime;
use super::convert::r#async as convert;
use super::engine::WasmEngine;
use super::stream_state::{DEFAULT_STREAM_CAPACITY, StreamDelivery, Streams};

use crate::plugin::controller::PluginController;
use crate::plugin::tick_input::TickInput;
use crate::plugin::{Command, Message, NodeId, Outbound, PluginError};

/// Pre-linked async bindings ready to spawn satellite tasks against.
///
/// Like the sync backend's `WasmController::prepare(...)` step: built
/// once per component and shared by every `AsyncWasmController` that
/// uses that component.
pub struct AsyncPluginPreBuilt {
    engine: Arc<WasmEngine>,
    runtime: Arc<AsyncRuntime>,
    pre: AsyncPluginPre<AsyncHostState>,
    component: Component,
}

impl AsyncPluginPreBuilt {
    /// Pre-link a Component against the async host imports.
    pub fn new(
        engine: &Arc<WasmEngine>,
        runtime: &Arc<AsyncRuntime>,
        component: &Component,
    ) -> Result<Self, PluginError> {
        let mut linker = wasmtime::component::Linker::new(engine.inner());
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| PluginError::Init(format!("WASI add_to_linker_async failed: {e}")))?;
        AsyncPlugin::add_to_linker::<AsyncHostState, AsyncHostState>(&mut linker, |state| state)
            .map_err(|e| PluginError::Init(format!("async add_to_linker failed: {e}")))?;
        let instance_pre = linker
            .instantiate_pre(component)
            .map_err(|e| PluginError::Init(format!("async instantiate_pre failed: {e}")))?;
        let pre = AsyncPluginPre::new(instance_pre)
            .map_err(|e| PluginError::Init(format!("AsyncPluginPre::new failed: {e}")))?;
        Ok(Self {
            engine: Arc::clone(engine),
            runtime: Arc::clone(runtime),
            pre,
            component: component.clone(),
        })
    }

    /// Borrow the engine associated with this pre-built plugin.
    pub fn engine(&self) -> &Arc<WasmEngine> {
        &self.engine
    }

    /// Borrow the runtime associated with this pre-built plugin.
    pub fn runtime(&self) -> &Arc<AsyncRuntime> {
        &self.runtime
    }

    /// Borrow the underlying pre-linked instance.
    pub(super) fn pre(&self) -> &AsyncPluginPre<AsyncHostState> {
        &self.pre
    }

    /// Borrow the underlying component.
    pub(super) fn component(&self) -> &Component {
        &self.component
    }
}

/// A `PluginController` backed by an async WASM task on the shared
/// `AsyncRuntime`.
///
/// The outer side (`update`, `Drop`) is sync and uses
/// `handle.block_on` to exchange messages with the task.
pub struct AsyncWasmController {
    runtime: Arc<AsyncRuntime>,
    input_tx: mpsc::Sender<Option<TickPacket>>,
    output_rx: mpsc::Receiver<GuestResponse>,
    sample_period_s: f64,
    name: String,

    // ─── msg-io transport state (mirror of sync backend) ────────
    node_id: NodeId,
    pending_inbound: Vec<Message>,
    outbound_buffer: Vec<Message>,

    // ─── stream-io transport state (mirror of sync backend) ─────
    pending_stream_inbound: Vec<StreamDelivery>,
    stream_outbound_buffer: HashMap<String, Vec<u8>>,
}

impl AsyncWasmController {
    /// Spawn a new satellite task on the runtime and wait for its
    /// `metadata(config)` call to succeed.
    ///
    /// Blocks the calling thread on `runtime.handle().block_on` until
    /// the task reports its startup result. On success, returns a
    /// handle that can be driven synchronously via
    /// `PluginController::update`.
    pub fn new(
        built: &AsyncPluginPreBuilt,
        label: impl Into<String>,
        config: &str,
    ) -> Result<Self, PluginError> {
        Self::new_with_streams(built, label, config, Vec::new())
    }

    /// As [`new`](Self::new) but wired to the given `stream-io` streams
    /// (declared up front).
    pub fn new_with_streams(
        built: &AsyncPluginPreBuilt,
        label: impl Into<String>,
        config: &str,
        stream_names: Vec<String>,
    ) -> Result<Self, PluginError> {
        let label = label.into();
        let config = config.to_string();

        let (input_tx, input_rx) = mpsc::channel::<Option<TickPacket>>(1);
        let (output_tx, output_rx) = mpsc::channel::<GuestResponse>(1);
        let (meta_tx, meta_rx) = oneshot::channel::<Result<f64, String>>();

        let engine = Arc::clone(built.engine());
        let runtime = Arc::clone(built.runtime());
        let component = built.component().clone();
        let pre = built.pre().clone();
        let label_for_task = label.clone();

        // Spawn the satellite task onto the runtime. Ownership of
        // `Store` and the `call_run` future is entirely inside the
        // task, which avoids a self-referential controller struct.
        runtime.handle().spawn(async move {
            let host_state = AsyncHostState {
                label: label_for_task,
                field: tobari::magnetic::TiltedDipole::earth(),
                wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
                table: wasmtime_wasi::ResourceTable::new(),
                input_rx,
                output_tx: output_tx.clone(),
                pending_cmd: None,
                inbox: VecDeque::new(),
                outbox: Vec::new(),
                streams: Streams::new(stream_names, DEFAULT_STREAM_CAPACITY),
                is_first_wait: true,
            };
            let mut store = Store::new(engine.inner(), host_state);

            let plugin = match pre.instantiate_async(&mut store).await {
                Ok(p) => p,
                Err(e) => {
                    let _ = meta_tx.send(Err(format!("instantiate_async: {e}")));
                    return;
                }
            };
            let _ = &component; // keep component alive with the task

            // Query metadata(config) — validates config and returns
            // sample_period. Errors are surfaced to `new()` below.
            let metadata = match plugin.call_metadata(&mut store, &config).await {
                Ok(Ok(md)) => md,
                Ok(Err(guest_err)) => {
                    let _ = meta_tx.send(Err(format!("metadata: {guest_err}")));
                    return;
                }
                Err(trap) => {
                    let _ = meta_tx.send(Err(format!("metadata call: {trap}")));
                    return;
                }
            };
            if !metadata.sample_period_s.is_finite() || metadata.sample_period_s <= 0.0 {
                let _ = meta_tx.send(Err(format!(
                    "guest returned invalid sample_period: {}",
                    metadata.sample_period_s
                )));
                return;
            }
            let _ = meta_tx.send(Ok(metadata.sample_period_s));

            // Drive the guest main loop for the rest of the task's
            // lifetime. When the outer side drops its `input_tx`,
            // `wait_tick` sees `None` and the guest `run()` returns.
            let run_result = plugin.call_run(&mut store, &config).await;
            let done = match run_result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(guest_err)) => Err(guest_err),
                Err(trap) => Err(format!("trap: {trap}")),
            };
            let _ = output_tx.send(GuestResponse::Done(done)).await;
        });

        // Wait for metadata on the caller thread using the runtime's
        // handle. This thread is NOT in the runtime, so block_on is
        // safe (no nested-block_on panic).
        let sample_period_s = runtime.handle().block_on(async move {
            meta_rx
                .await
                .map_err(|_| PluginError::Init("async task dropped before metadata".to_string()))?
                .map_err(|e| PluginError::Init(format!("metadata: {e}")))
        })?;

        Ok(Self {
            runtime,
            input_tx,
            output_rx,
            sample_period_s,
            name: format!("wasm-async:{label}"),
            node_id: NodeId::Satellite(0),
            pending_inbound: Vec::new(),
            outbound_buffer: Vec::new(),
            pending_stream_inbound: Vec::new(),
            stream_outbound_buffer: HashMap::new(),
        })
    }
}

impl PluginController for AsyncWasmController {
    fn name(&self) -> &str {
        &self.name
    }

    fn sample_period(&self) -> f64 {
        self.sample_period_s
    }

    fn deliver(&mut self, msg: Message) {
        self.pending_inbound.push(msg);
    }

    fn take_outbound(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.outbound_buffer)
    }

    fn set_node_id(&mut self, id: NodeId) {
        self.node_id = id;
    }

    fn stream_deliver(&mut self, stream: &str, bytes: Vec<u8>) {
        self.pending_stream_inbound.push(StreamDelivery {
            name: stream.to_string(),
            bytes,
            closed: false,
        });
    }

    fn stream_take(&mut self, stream: &str) -> Vec<u8> {
        self.stream_outbound_buffer
            .remove(stream)
            .unwrap_or_default()
    }

    fn stream_close(&mut self, stream: &str) {
        self.pending_stream_inbound.push(StreamDelivery {
            name: stream.to_string(),
            bytes: Vec::new(),
            closed: true,
        });
    }

    fn update(&mut self, obs: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
        let wit_obs = convert::tick_input_to_wit(obs);
        let inbox: Vec<wit::Message> = std::mem::take(&mut self.pending_inbound)
            .into_iter()
            .map(convert::message_to_wit)
            .collect();
        let stream_inbound = std::mem::take(&mut self.pending_stream_inbound);
        let input_tx = self.input_tx.clone();
        let output_rx = &mut self.output_rx;

        let (command, outgoing, stream_outbound) = self.runtime.handle().block_on(async move {
            input_tx
                .send(Some(TickPacket {
                    input: wit_obs,
                    inbox,
                    stream_inbound,
                }))
                .await
                .map_err(|_| PluginError::Runtime("async task dropped".to_string()))?;
            match output_rx
                .recv()
                .await
                .ok_or_else(|| PluginError::Runtime("async task channel closed".to_string()))?
            {
                GuestResponse::Tick {
                    command,
                    outgoing,
                    stream_outbound,
                    stream_fault,
                } => {
                    // Host-authoritative stream fault → halt the simulation.
                    if let Some(fault) = stream_fault {
                        return Err(PluginError::Runtime(fault));
                    }
                    Ok((command, outgoing, stream_outbound))
                }
                GuestResponse::Done(Ok(())) => Err(PluginError::Runtime(
                    "guest run() returned early".to_string(),
                )),
                GuestResponse::Done(Err(e)) => {
                    Err(PluginError::Runtime(format!("guest error: {e}")))
                }
            }
        })?;

        // Inject the host-controlled `src` onto guest outbound (anti-spoof).
        for ob in outgoing {
            let ob: Outbound = convert::outbound_from_wit(ob);
            self.outbound_buffer.push(Message {
                src: self.node_id,
                dst: ob.dst,
                kind: ob.kind,
                payload: ob.payload,
            });
        }

        // Buffer guest-written stream bytes for `stream_take`. Bound it here
        // too (the worker's queue is drained every tick into this outer
        // buffer) so a consumer that never drains halts the sim via overrun
        // rather than growing without limit.
        for (name, bytes) in stream_outbound {
            let buf = self.stream_outbound_buffer.entry(name.clone()).or_default();
            buf.extend(bytes);
            if buf.len() > DEFAULT_STREAM_CAPACITY {
                return Err(PluginError::Runtime(format!(
                    "stream-io: outbound backlog overrun on stream '{name}' (consumer not draining)"
                )));
            }
        }

        match command {
            Some(wit_cmd) => convert::command_from_wit(wit_cmd).map(Some),
            None => Ok(None),
        }
    }
}

impl Drop for AsyncWasmController {
    fn drop(&mut self) {
        // Best-effort shutdown signal: send None on the input channel
        // so the guest's wait_tick returns None and the main loop
        // exits. If the runtime has already been dropped, the send
        // silently fails and we just return.
        let input_tx = self.input_tx.clone();
        let _ = self
            .runtime
            .handle()
            .block_on(async move { input_tx.send(None).await });
    }
}
