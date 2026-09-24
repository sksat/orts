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
//! WasmController::new(pre, label, config, body):
//!   - spawn worker thread
//!   - worker: Store::new, instantiate, call metadata() + call run()
//!   - outer: receive metadata, return
//! WasmController::update(input):
//!   - send input via channel
//!   - receive command from channel
//! Drop WasmController:
//!   - drop input_tx → guest's wait_tick fails → worker thread exits
//!   - wait for the worker for at most one turn deadline, then leave it
//! ```
//!
//! ## Limits
//!
//! The store runs under [`GuestLimits`]: a turn deadline enforced through the
//! engine's epoch, and memory / table / instance caps through a
//! `ResourceLimiter`. The outer thread never waits on the worker for longer
//! than the deadline plus a grace, so a guest blocked in a host call where
//! the epoch cannot reach it fails the call instead of holding its caller.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use wasmtime::Store;
use wasmtime::component::Component;

use super::convert::sync as convert;
use super::engine::WasmEngine;
use super::limits::{
    GuestLimits, OutboundBacklog, clock_waits_return_at_once, guest_error, stuck_in_host_message,
};
use super::stream_state::{DEFAULT_STREAM_CAPACITY, StreamDelivery};
use super::sync_bindings::orts::plugin::types as wit;
use super::sync_bindings::{Plugin, PluginPre};
use super::sync_host_state::{GuestResponse, HostState, TickPacket};

use crate::plugin::controller::PluginController;
use crate::plugin::tick_input::TickInput;
use crate::plugin::{Command, Message, NodeId, Outbound, PluginError};

/// A `PluginController` backed by a WebAssembly Component guest.
pub struct WasmController {
    /// Worker thread handle. Joined on Drop.
    worker: Option<thread::JoinHandle<()>>,
    /// Channel for sending tick packets (input + frozen inbox) to the worker.
    input_tx: Option<mpsc::SyncSender<TickPacket>>,
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

    // ─── msg-io transport state ─────────────────────────────────
    /// This controller's node identity, stamped as `src` on outbound
    /// messages. Defaults to `NodeId::Satellite(0)`.
    node_id: NodeId,
    /// Inbound messages queued via [`Self::deliver`], frozen into the
    /// next tick's inbox on `update()`.
    pending_inbound: Vec<Message>,
    /// Outbound messages emitted by the guest, awaiting pickup via
    /// [`Self::take_outbound`]; bounded, so a caller that never takes them
    /// halts the simulation instead of filling memory.
    outbound: OutboundBacklog,
    /// The limits the guest runs under; also how long the outer thread
    /// waits for the worker.
    limits: GuestLimits,
    /// Set when the worker did not answer in time. It is then left to finish
    /// on its own: `Drop` does not wait for it again.
    abandoned: bool,

    // ─── stream-io transport state ──────────────────────────────
    /// Inbound byte deliveries / close signals queued via
    /// [`Self::stream_deliver`] / [`Self::stream_close`], frozen into the
    /// next tick on `update()`.
    pending_stream_inbound: Vec<StreamDelivery>,
    /// Bytes the guest wrote to each stream, awaiting pickup via
    /// [`Self::stream_take`] (keyed by stream name).
    stream_outbound_buffer: HashMap<String, Vec<u8>>,
}

impl WasmController {
    /// Instantiate a WASM guest controller for one satellite (no `stream-io`
    /// streams). Use [`new_with_streams`](Self::new_with_streams) to wire
    /// named byte streams.
    pub fn new(
        pre: &PluginPre<HostState>,
        label: impl Into<String>,
        config: &str,
        body: arika::body::KnownBody,
    ) -> Result<Self, PluginError> {
        Self::new_with_streams(pre, label, config, Vec::new(), body)
    }

    /// Instantiate a WASM guest controller wired to the given `stream-io`
    /// streams (declared up front; the host maps each local name to an
    /// external endpoint), under the default [`GuestLimits`].
    ///
    /// Spawns a dedicated worker thread that owns the `Store` and
    /// drives the guest's `run()` loop. Returns after the guest's
    /// `metadata()` has been called (so `sample_period` is known).
    pub fn new_with_streams(
        pre: &PluginPre<HostState>,
        label: impl Into<String>,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
    ) -> Result<Self, PluginError> {
        Self::new_with_limits(
            pre,
            label,
            config,
            stream_names,
            body,
            GuestLimits::default(),
        )
    }

    /// As [`new_with_streams`](Self::new_with_streams), under `limits`.
    pub fn new_with_limits(
        pre: &PluginPre<HostState>,
        label: impl Into<String>,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
        limits: GuestLimits,
    ) -> Result<Self, PluginError> {
        let label = label.into();
        let config = config.to_string();
        let pre = pre.clone();
        let worker_streams = stream_names.clone();

        // Channels for outer ↔ worker communication.
        let (input_tx, input_rx) = mpsc::sync_channel::<TickPacket>(1);
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
                    worker_streams,
                    body,
                    limits,
                );
            })
            .map_err(|e| PluginError::Init(format!("failed to spawn worker thread: {e}")))?;

        // Wait for the worker to send metadata (or fail). Instantiation and
        // `metadata` are a turn each; past both deadlines the guest is blocked
        // in a host call, and the worker is left to it.
        let waited = limits.host_wait(2);
        let sample_period = match metadata_rx.recv_timeout(waited) {
            Ok(result) => result.map_err(|e| PluginError::Init(format!("metadata failed: {e}")))?,
            // Returning drops the join handle, which leaves the worker to
            // finish on its own.
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(PluginError::Init(stuck_in_host_message(waited)));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(PluginError::Init(
                    "worker thread exited before metadata".to_string(),
                ));
            }
        };

        Ok(Self {
            worker: Some(worker),
            input_tx: Some(input_tx),
            output_rx,
            sample_period,
            name: format!("wasm:{label}"),
            _current_mode: current_mode,
            node_id: NodeId::Satellite(0),
            pending_inbound: Vec::new(),
            outbound: OutboundBacklog::default(),
            limits,
            abandoned: false,
            pending_stream_inbound: Vec::new(),
            stream_outbound_buffer: HashMap::new(),
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
        clock_waits_return_at_once(&mut linker)?;
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
        let Some(worker) = self.worker.take() else {
            return;
        };
        if self.abandoned {
            // Already found stuck; waiting again would only repeat that.
            return;
        }
        // Read responses until the worker says it is done, so a send of its
        // never blocks on a full channel, and give up after one turn: a guest
        // still running wasm has trapped by then, and one blocked in a host
        // call is left to finish on its own.
        let wait = self.limits.host_wait(1);
        let started = Instant::now();
        loop {
            let left = wait.saturating_sub(started.elapsed());
            match self.output_rx.recv_timeout(left) {
                Ok(GuestResponse::Done(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                    return;
                }
                Ok(GuestResponse::Tick { .. }) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => return,
            }
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

    /// Queue an inbound message for delivery on the **next** `update()`
    /// tick. The host freezes the queued set into that tick's frozen
    /// inbox (`recv-batch`).
    fn deliver(&mut self, msg: Message) {
        self.pending_inbound.push(msg);
    }

    /// Drain every message the guest has emitted so far. Each message
    /// carries the host-stamped `src`.
    fn take_outbound(&mut self) -> Vec<Message> {
        self.outbound.take()
    }

    /// Set this controller's node identity (stamped as `src` on
    /// outbound messages). Defaults to `NodeId::Satellite(0)`.
    fn set_node_id(&mut self, id: NodeId) {
        self.node_id = id;
    }

    /// Queue inbound bytes for a named stream, frozen into the next
    /// `update()` tick.
    fn stream_deliver(&mut self, stream: &str, bytes: Vec<u8>) {
        self.pending_stream_inbound.push(StreamDelivery {
            name: stream.to_string(),
            bytes,
            closed: false,
        });
    }

    /// Drain the bytes the guest has written to `stream` since the last call.
    fn stream_take(&mut self, stream: &str) -> Vec<u8> {
        self.stream_outbound_buffer
            .remove(stream)
            .unwrap_or_default()
    }

    /// Signal that a named stream's peer has closed.
    fn stream_close(&mut self, stream: &str) {
        self.pending_stream_inbound.push(StreamDelivery {
            name: stream.to_string(),
            bytes: Vec::new(),
            closed: true,
        });
    }

    fn update(&mut self, obs: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
        let wit_obs = convert::tick_input_to_wit(obs);

        // Freeze this tick's inbox: drain whatever was queued via
        // `deliver()` and lower it into WIT messages.
        let inbox: Vec<wit::Message> = std::mem::take(&mut self.pending_inbound)
            .into_iter()
            .map(convert::message_to_wit)
            .collect();

        let input_tx = self
            .input_tx
            .as_ref()
            .ok_or_else(|| PluginError::Runtime("controller is shut down".to_string()))?;

        let stream_inbound = std::mem::take(&mut self.pending_stream_inbound);

        if input_tx
            .send(TickPacket {
                input: wit_obs,
                inbox,
                stream_inbound,
            })
            .is_err()
        {
            // The worker is gone. If it said why on the way out, that is the
            // error worth reporting, not the closed channel.
            return Err(match self.output_rx.try_recv() {
                Ok(GuestResponse::Done(Err(e))) => {
                    PluginError::Runtime(format!("guest error: {e}"))
                }
                _ => PluginError::Runtime("worker thread exited".to_string()),
            });
        }

        let waited = self.limits.host_wait(1);
        let response = match self.output_rx.recv_timeout(waited) {
            Ok(response) => response,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // The guest is neither running wasm (the epoch would have
                // stopped it) nor waiting for a tick. Nothing more is sent to
                // it, and `Drop` leaves it be.
                self.abandoned = true;
                self.input_tx = None;
                return Err(PluginError::Runtime(stuck_in_host_message(waited)));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(PluginError::Runtime("worker thread exited".to_string()));
            }
        };

        match response {
            GuestResponse::Tick {
                command,
                outgoing,
                stream_outbound,
                host_fault,
            } => {
                // A latched host fault (stream overrun / wiring inconsistency,
                // msg-io flood) is host-authoritative: halt the simulation
                // regardless of whether the guest observed an error.
                if let Some(fault) = host_fault {
                    return Err(PluginError::Runtime(fault));
                }

                // Inject the host-controlled `src` onto each guest outbound
                // (prevents the guest from spoofing its origin) and buffer it.
                for ob in outgoing {
                    let ob: Outbound = convert::outbound_from_wit(ob);
                    self.outbound
                        .push(Message {
                            src: self.node_id,
                            dst: ob.dst,
                            kind: ob.kind,
                            payload: ob.payload,
                        })
                        .map_err(PluginError::Runtime)?;
                }

                // Buffer guest-written stream bytes for `stream_take`. The
                // worker's queue is bounded but drained every tick into this
                // outer buffer; bound it here too so a consumer (bridge) that
                // never calls `stream_take` halts the sim (overrun) instead of
                // growing without limit.
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
// Internal worker entry point: the parameters are the things the worker
// needs to own (channels, config, declared streams), not a public API.
#[allow(clippy::too_many_arguments)]
fn worker_main(
    pre: PluginPre<HostState>,
    label: String,
    config: String,
    input_rx: mpsc::Receiver<TickPacket>,
    output_tx: mpsc::SyncSender<GuestResponse>,
    metadata_tx: mpsc::SyncSender<Result<f64, String>>,
    current_mode: Arc<Mutex<Option<String>>>,
    stream_names: Vec<String>,
    body: arika::body::KnownBody,
    limits: GuestLimits,
) {
    let engine = pre.engine();
    let host_state = HostState::new(
        &label,
        input_rx,
        output_tx.clone(),
        current_mode,
        stream_names,
        body,
        limits,
    );
    let mut store = Store::new(engine, host_state);
    // Limits go on before any guest code runs: instantiation may run start
    // functions, and allocates the guest's initial memory.
    store.limiter(|state| &mut state.limiter);
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(|ctx| ctx.data().turn.on_epoch(false));

    store.data_mut().turn.start();
    let plugin = match pre.instantiate(&mut store) {
        Ok(p) => p,
        Err(e) => {
            let _ = metadata_tx.send(Err(format!("instantiate: {}", guest_error(&e))));
            return;
        }
    };
    // Instantiation is a turn of its own (start functions run in it).
    if let Some(fault) = store.data_mut().take_turn_fault() {
        let _ = metadata_tx.send(Err(format!("instantiate: {fault}")));
        return;
    }

    // Query metadata first (before run() takes over the thread).
    // `metadata(config)` also validates config — a bad config fails
    // here instead of on the first `update()` call.
    store.data_mut().turn.start();
    let metadata = match plugin.call_metadata(&mut store, &config) {
        Ok(Ok(md)) => md,
        Ok(Err(guest_err)) => {
            let _ = metadata_tx.send(Err(format!("metadata: {guest_err}")));
            return;
        }
        Err(e) => {
            let _ = metadata_tx.send(Err(format!("metadata call: {}", guest_error(&e))));
            return;
        }
    };
    if let Some(fault) = store.data_mut().take_turn_fault() {
        let _ = metadata_tx.send(Err(format!("metadata: {fault}")));
        return;
    }
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
    store.data_mut().begin_run();
    store.data_mut().turn.start();
    let result = plugin.call_run(&mut store, &config);
    let done = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(guest_err)) => Err(guest_err),
        Err(trap) => Err(format!("trap: {}", guest_error(&trap))),
    };
    let _ = output_tx.send(GuestResponse::Done(done));
}
