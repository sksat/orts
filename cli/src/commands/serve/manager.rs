//! Tokio serve layer wrapping the pure [`ServeEngine`].
//!
//! This module owns everything the engine deliberately does not: the
//! idle/running/paused state machine, the mpsc command / broadcast / oneshot
//! wiring, wall-clock pacing, and the stream-io bridge lifecycle. The actual
//! simulation orchestration — state transitions, snapshotting, history,
//! dynamic add, boundary reset — lives in [`super::engine`].

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};

use super::engine::{EngineInit, ServeEngine, StreamIo};
use super::history::HistoryBuffer;
use super::protocol::WsMessage;
use super::stream_bridge::{OutboundPush, StreamBridge, StreamEndpoint, StreamKey};
use crate::cli::{PluginAsyncModeChoice, PluginBackendChoice, SimArgs};
use crate::config::{SatelliteConfig, SimConfig};
use crate::satellite::SatelliteInfo;
use crate::sim::params::SimParams;
use orts::setup::default_third_bodies;

/// CLI-time backend overrides that must apply to every `SimParams`
/// built inside the serve manager — including ones derived from
/// configs received from the client at runtime.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PluginBackendOverrides {
    pub choice: Option<PluginBackendChoice>,
    pub threshold: Option<usize>,
    pub async_mode: Option<PluginAsyncModeChoice>,
}

impl PluginBackendOverrides {
    /// The async mode stays `None` when the flag was left out, so a
    /// simulation built from a config keeps the `Deterministic` that
    /// `SimParams::from_config` gave it. The backend choice is carried either
    /// way — its `Auto` default reads the same as no flag — and the threshold
    /// is already absent when unset.
    pub fn from_sim_args(sim: &SimArgs) -> Self {
        Self {
            choice: Some(sim.plugin_backend),
            threshold: sim.plugin_backend_threshold,
            async_mode: sim.plugin_backend_async_mode,
        }
    }

    pub fn apply(&self, params: &mut SimParams) {
        if let Some(c) = self.choice {
            params.plugin_backend_choice = c;
        }
        if self.threshold.is_some() {
            params.plugin_backend_threshold = self.threshold;
        }
        if let Some(mode) = self.async_mode {
            params.plugin_backend_async_mode = mode;
        }
    }
}

/// Command sent from connection handlers to the simulation manager.
pub(super) enum SimCommand {
    /// Start a simulation from idle state.
    ///
    /// `respond` is answered once the simulation is built, with `Ok`, or with
    /// the reason it could not be: the config was refused, its parameters did
    /// not build, or the engine did not.
    Start {
        config: Box<SimConfig>,
        respond: oneshot::Sender<Result<(), String>>,
    },
    /// Add a satellite to a running simulation.
    AddSatellite {
        satellite: Box<SatelliteConfig>,
        respond: oneshot::Sender<Result<(SatelliteInfo, f64), String>>,
    },
    /// Query the current simulation status.
    ///
    /// The returned history is always a bounded, downsampled overview of the
    /// full simulation, so re-connects to long-running sims never ship an
    /// unbounded payload. Clients that need higher-resolution data for a
    /// specific time window issue a follow-up [`SimCommand::QueryRange`]
    /// request — the connection handshake itself is time-range-agnostic.
    GetStatus {
        respond: oneshot::Sender<SimStatusResponse>,
    },
    /// Query a time range from history.
    QueryRange {
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        entity_path: Option<orts::record::entity_path::EntityPath>,
        respond: oneshot::Sender<Vec<crate::sim::core::HistoryState>>,
    },
    /// Pause the simulation.
    Pause {
        respond: oneshot::Sender<Result<(), String>>,
    },
    /// Resume a paused simulation.
    Resume {
        respond: oneshot::Sender<Result<(), String>>,
    },
    /// Terminate the simulation and return to idle.
    Terminate {
        respond: oneshot::Sender<Result<(), String>>,
    },
}

pub(super) enum SimStatusResponse {
    Idle,
    Running {
        info_json: String,
        terminated_events: Vec<String>,
        history_states: Vec<crate::sim::core::HistoryState>,
    },
    Paused {
        info_json: String,
        terminated_events: Vec<String>,
        history_states: Vec<crate::sim::core::HistoryState>,
    },
}

/// A `start_simulation` the idle loop accepted and has not answered yet.
///
/// `params` are built once, from the request's config; `respond` is the
/// requesting connection's reply channel, answered once the engine is built
/// (see [`run_simulation_loop`]).
struct PendingStart {
    params: SimParams,
    respond: oneshot::Sender<Result<(), String>>,
}

/// Why the simulation loop exited.
enum LoopExit {
    /// Terminated by client request; server should return to idle.
    Terminated,
    /// Command channel disconnected (all clients gone).
    Disconnected,
}

/// [`StreamIo`] adapter backed by the live [`StreamBridge`] endpoints.
///
/// Holds each satellite's resolved `(stream name, endpoint)` handles, indexed
/// like the engine's fleet. Resolving once at construction keeps the per-tick
/// pumps off a (key-allocating) registry lookup on the hot path. A satellite
/// index with no entry (e.g. a dynamically-added, streamless satellite)
/// resolves to "no bytes / no peer", matching the engine's expectations.
struct BridgeStreamIo {
    sat_streams: Vec<Vec<(String, Arc<StreamEndpoint>)>>,
}

impl BridgeStreamIo {
    fn endpoint(&self, sat_idx: usize, name: &str) -> Option<&Arc<StreamEndpoint>> {
        self.sat_streams
            .get(sat_idx)
            .and_then(|streams| streams.iter().find(|(n, _)| n == name).map(|(_, ep)| ep))
    }
}

impl StreamIo for BridgeStreamIo {
    fn take_inbound(&mut self, sat_idx: usize, name: &str) -> (Vec<u8>, bool) {
        match self.endpoint(sat_idx, name) {
            Some(ep) => ep.take_staged(),
            None => (Vec::new(), false),
        }
    }

    fn push_outbound(&mut self, sat_idx: usize, name: &str, bytes: Vec<u8>) -> OutboundPush {
        match self.endpoint(sat_idx, name) {
            Some(ep) => ep.push_outbound(bytes),
            None => OutboundPush::NoPeer,
        }
    }
}

/// Collect all body names in the simulation system (central + third bodies).
fn system_body_names(params: &SimParams) -> Vec<String> {
    body_names_for(&params.body)
}

/// Return the central body name plus all third-body names for the given central body.
///
/// The names are what the texture downloader fetches, so a central body with no
/// Sun ephemeris contributes its own name and no third bodies. That body is
/// rejected where the force models are built; a texture list has nothing to
/// report.
fn body_names_for(body: &arika::body::KnownBody) -> Vec<String> {
    let mut names = vec![body.properties().name.to_lowercase()];
    for tb in default_third_bodies(body).unwrap_or_default().iter() {
        // tb.name is like "third_body_sun" → extract the body name after the prefix
        if let Some(name) = tb.name.strip_prefix("third_body_") {
            names.push(name.to_string());
        }
    }
    names
}

/// Simulation manager that starts with a pre-built SimParams (legacy CLI args path).
pub(super) async fn simulation_manager_with_params(
    params: Arc<SimParams>,
    cli_plugin_overrides: PluginBackendOverrides,
    cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
    bridge: Arc<StreamBridge>,
) {
    // Request texture downloads for all bodies in the system.
    let _ = texture_tx.send(system_body_names(&params)).await;

    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let body_radius = params.body.properties().radius;
    let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
    // No client asked for this simulation, so there is nobody to answer.
    match run_simulation_loop(
        params,
        None,
        cmd_rx,
        tx.clone(),
        history,
        Arc::clone(&bridge),
    )
    .await
    {
        (LoopExit::Terminated, mut returned_rx) => {
            // Legacy path: after terminate, go idle and allow restart.
            eprintln!("Simulation manager: idle, waiting for start_simulation...");
            let next = idle_loop(&mut returned_rx).await;
            // Run the clients' simulations the way the standard manager does.
            run_client_starts(
                next,
                cli_plugin_overrides,
                returned_rx,
                tx,
                texture_tx,
                bridge,
            )
            .await;
        }
        (LoopExit::Disconnected, _) => {}
    }
}

/// Validate a SimConfig before starting. Returns Err with a user-facing message
/// if the config is invalid (e.g., mixed attitude settings).
///
/// The config alone decides: nothing here opens a file or fetches. The
/// satellites are built afterwards, once, by `SimParams::from_config` — a
/// NORAD orbit's TLE is fetched there. Building them here as well fetched
/// every NORAD TLE twice (#555). What a built spec used to be checked for is
/// refused by `SimConfig::validate` from the config already (a TLE or NORAD
/// orbit about another body, attitude or a controller on only part of the
/// fleet, an attitude that cannot be propagated), and `ServeEngine::build`
/// checks the built specs again.
fn validate_sim_config(config: &SimConfig) -> Result<(), String> {
    // Field-level validation (non-zero direction, finite values, …) mirrors
    // what `SimConfig::load` runs for file-based configs, so WebSocket
    // `StartSimulation` cannot smuggle in thruster config that panics later
    // in `ThrusterSpec::new()`.
    config.validate()?;
    // The serve loop does not drain a `[[command]]` timeline, so a config that
    // `orts serve --config` rejects must not slip in through a WebSocket
    // `start_simulation` and have its uplinks dropped instead. The same gate
    // refuses `frame = "gcrs"` (serve is `SimpleEci`-locked).
    config.ensure_serve_supported()?;
    // `[gravity_field]` names a file on the server's filesystem. A WebSocket
    // client must not be able to make the server open arbitrary paths (or
    // panic on a missing one), so the field is CLI / config-file only.
    if config.gravity_field.is_some() {
        return Err(
            "gravity_field is not accepted over WebSocket: the field comes with the \
                    simulation `orts serve` starts from its own command line — a `--config` \
                    file carrying `[gravity_field]`, or an orbit with `--gravity-field <PATH>`"
                .to_string(),
        );
    }
    // `space_weather` is a file on the server unless it is `"auto"`, the
    // CelesTrak fetch, so a client may ask for that alone. A FIFO path held
    // the manager in `read_to_string`, and the server stopped completing new
    // WebSocket handshakes (#556).
    if let Some(path) = config.space_weather.as_deref()
        && path != "auto"
    {
        return Err(format!(
            "space_weather = \"{path}\" is not accepted over WebSocket: a client may ask for \
             \"auto\" (fetched from CelesTrak); a file comes with the simulation `orts serve` \
             starts from its own command line — a `--config` file carrying `space_weather`, or \
             an orbit with `--space-weather <PATH>`"
        ));
    }
    Ok(())
}

/// Drain the cmd_rx, handling only GetStatus (as idle) and rejecting others,
/// until a Start command whose parameters build arrives or the channel
/// disconnects.
///
/// A Start is answered here only when it fails: `validate_sim_config` refuses
/// the config, or `SimParams::from_config` cannot build it (a NORAD TLE or a
/// `space_weather = "auto"` that cannot be fetched). A Start whose parameters
/// build comes back unanswered, as a [`PendingStart`].
async fn idle_loop(cmd_rx: &mut mpsc::Receiver<SimCommand>) -> Option<PendingStart> {
    loop {
        let Some(cmd) = cmd_rx.recv().await else {
            return None; // All senders dropped
        };
        match cmd {
            SimCommand::GetStatus { respond, .. } => {
                let _ = respond.send(SimStatusResponse::Idle);
            }
            SimCommand::Start { config, respond } => {
                if let Err(e) = validate_sim_config(&config) {
                    let _ = respond.send(Err(e));
                    continue;
                }
                // The manager task must survive a config it cannot build, so
                // the failure goes back to the client and the loop keeps
                // waiting for the next `start_simulation`.
                match SimParams::from_config(&config) {
                    Ok(params) => return Some(PendingStart { params, respond }),
                    Err(e) => {
                        eprintln!("Simulation manager: cannot start simulation: {e}");
                        let _ = respond.send(Err(e));
                    }
                }
            }
            SimCommand::AddSatellite { respond, .. } => {
                let _ = respond.send(Err("Simulation is not running".to_string()));
            }
            SimCommand::QueryRange { respond, .. } => {
                let _ = respond.send(vec![]);
            }
            SimCommand::Pause { respond } => {
                let _ = respond.send(Err("Simulation is not running".to_string()));
            }
            SimCommand::Resume { respond } => {
                let _ = respond.send(Err("Simulation is not running".to_string()));
            }
            SimCommand::Terminate { respond } => {
                let _ = respond.send(Err("Simulation is not running".to_string()));
            }
        }
    }
}

/// Simulation manager: handles idle/running state and commands.
/// Loops between idle and running states; after terminate it returns to idle.
pub(super) async fn simulation_manager(
    cli_plugin_overrides: PluginBackendOverrides,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
    bridge: Arc<StreamBridge>,
) {
    eprintln!("Simulation manager: idle, waiting for start_simulation...");
    let first = idle_loop(&mut cmd_rx).await;
    run_client_starts(first, cli_plugin_overrides, cmd_rx, tx, texture_tx, bridge).await;
}

/// Run each simulation a client's `start_simulation` asked for, from `next`
/// on: run until terminated, return to idle, and take the next one, until the
/// command channel disconnects.
async fn run_client_starts(
    mut next: Option<PendingStart>,
    cli_plugin_overrides: PluginBackendOverrides,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
    bridge: Arc<StreamBridge>,
) {
    while let Some(PendingStart {
        mut params,
        respond,
    }) = next
    {
        cli_plugin_overrides.apply(&mut params);
        let params = Arc::new(params);

        // Request texture downloads for all bodies in the system.
        let _ = texture_tx.send(system_body_names(&params)).await;

        let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
        let body_radius = params.body.properties().radius;
        let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
        eprintln!("Simulation manager: starting simulation...");
        match run_simulation_loop(
            params,
            Some(respond),
            cmd_rx,
            tx.clone(),
            history,
            Arc::clone(&bridge),
        )
        .await
        {
            (LoopExit::Terminated, returned_rx) => {
                cmd_rx = returned_rx;
                eprintln!("Simulation manager: idle, waiting for start_simulation...");
                next = idle_loop(&mut cmd_rx).await;
            }
            (LoopExit::Disconnected, _) => return,
        }
    }
}

/// Serialize one history state as a `WsMessage::State` JSON string.
fn state_json(out: &crate::sim::core::HistoryState) -> String {
    serde_json::to_string(&WsMessage::State {
        entity_path: out.entity_path.clone(),
        t: out.t,
        position: out.position,
        velocity: out.velocity,
        semi_major_axis: out.semi_major_axis,
        eccentricity: out.eccentricity,
        inclination: out.inclination,
        raan: out.raan,
        argument_of_periapsis: out.argument_of_periapsis,
        true_anomaly: out.true_anomaly,
        altitude: out.altitude,
        specific_energy: out.specific_energy,
        angular_momentum: out.angular_momentum,
        velocity_mag: out.velocity_mag,
        accelerations: out.accelerations.clone(),
        torques: out.torques.clone(),
        attitude: out.attitude.clone(),
    })
    .expect("failed to serialize state")
}

/// Handle a single command from the connection handler against the running
/// engine. Returns `ControlFlow::Break(())` if the simulation should terminate.
///
/// `paused` is **serve-layer** state: pausing does not change physics, it just
/// stops the loop from calling [`ServeEngine::step_chunk`]. The engine itself
/// is unaware of it (which is why it has no `paused` field).
fn handle_command(
    engine: &mut ServeEngine,
    paused: &mut bool,
    tx: &broadcast::Sender<String>,
    cmd: SimCommand,
) -> ControlFlow<()> {
    match cmd {
        SimCommand::GetStatus { respond } => {
            let data = engine.status_data();
            let response = if *paused {
                SimStatusResponse::Paused {
                    info_json: data.info_json,
                    terminated_events: data.terminated_events,
                    history_states: data.history_states,
                }
            } else {
                SimStatusResponse::Running {
                    info_json: data.info_json,
                    terminated_events: data.terminated_events,
                    history_states: data.history_states,
                }
            };
            let _ = respond.send(response);
        }
        SimCommand::Start { respond, .. } => {
            let _ = respond.send(Err("Simulation is already running".to_string()));
        }
        SimCommand::Pause { respond } => {
            if *paused {
                let _ = respond.send(Err("Simulation is already paused".to_string()));
            } else {
                *paused = true;
                eprintln!("Simulation paused at t={:.2}s", engine.current_t());
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "paused".to_string(),
                })
                .expect("failed to serialize status");
                let _ = tx.send(status);
                let _ = respond.send(Ok(()));
            }
        }
        SimCommand::Resume { respond } => {
            if !*paused {
                let _ = respond.send(Err("Simulation is not paused".to_string()));
            } else {
                *paused = false;
                eprintln!("Simulation resumed at t={:.2}s", engine.current_t());
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "running".to_string(),
                })
                .expect("failed to serialize status");
                let _ = tx.send(status);
                let _ = respond.send(Ok(()));
            }
        }
        SimCommand::Terminate { respond } => {
            eprintln!("Simulation terminated at t={:.2}s", engine.current_t());
            let status = serde_json::to_string(&WsMessage::Status {
                state: "idle".to_string(),
            })
            .expect("failed to serialize status");
            let _ = tx.send(status);
            let _ = respond.send(Ok(()));
            return ControlFlow::Break(());
        }
        SimCommand::AddSatellite { satellite, respond } => match engine.add_satellite(*satellite) {
            Ok(out) => {
                for msg in &out.broadcasts {
                    let _ = tx.send(msg.clone());
                }
                let _ = respond.send(Ok((out.info, out.t)));
            }
            Err(e) => {
                let _ = respond.send(Err(e));
            }
        },
        SimCommand::QueryRange {
            t_min,
            t_max,
            max_points,
            entity_path,
            respond,
        } => {
            let states = engine.query_range(t_min, t_max, max_points, entity_path.as_ref());
            let _ = respond.send(states);
        }
    }
    ControlFlow::Continue(())
}

/// Core simulation loop: builds the engine, drives propagation, dispatches
/// commands, and paces output to the wall clock. Returns the exit reason and
/// gives back the command receiver for reuse.
///
/// `requester` is the reply channel of the `start_simulation` that asked for
/// this simulation. It is answered once the engine is built: `Err` if the
/// build fails, and no other client hears of that failure; `Ok` if it
/// succeeds, before the first `info` is broadcast. `None` is the simulation
/// `orts serve` starts from its own command line, which no client asked for;
/// its build failure is broadcast to whoever is connected.
async fn run_simulation_loop(
    params: Arc<SimParams>,
    requester: Option<oneshot::Sender<Result<(), String>>>,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    history: HistoryBuffer,
    bridge: Arc<StreamBridge>,
) -> (LoopExit, mpsc::Receiver<SimCommand>) {
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;
    let wall_per_sim_sec = ((params.dt / 100.0).max(0.01)) / params.stream_interval;
    let default_chunk_wall_time = Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    let EngineInit {
        mut engine,
        initial_broadcasts,
        stream_layout,
    } = match ServeEngine::build(params, history) {
        Ok(init) => init,
        Err(e) => {
            eprintln!("Simulation startup error: {e}");
            match requester {
                Some(respond) => {
                    let _ = respond.send(Err(e));
                }
                None => {
                    let err_msg = serde_json::to_string(&WsMessage::Error { message: e })
                        .expect("failed to serialize error");
                    let _ = tx.send(err_msg);
                }
            }
            return (LoopExit::Terminated, cmd_rx);
        }
    };
    if let Some(respond) = requester {
        let _ = respond.send(Ok(()));
    }

    // Broadcast the engine's initial Info + state messages.
    for msg in initial_broadcasts {
        let _ = tx.send(msg);
    }

    // Register the stream-io bridge endpoints for this run (replacing any from
    // a previous config — their lingering WS connections see `defunct` and
    // close), then resolve each into a `StreamIo` adapter handle.
    let stream_keys: Vec<StreamKey> = stream_layout
        .iter()
        .flat_map(|(id, names)| names.iter().map(|n| (id.clone(), n.clone())))
        .collect();
    for (sat, stream) in &stream_keys {
        eprintln!("stream-io endpoint: /stream/{sat}/{stream}");
    }
    bridge.reset(stream_keys);
    let mut streams = BridgeStreamIo {
        sat_streams: stream_layout
            .iter()
            .map(|(id, names)| {
                names
                    .iter()
                    .filter_map(|name| bridge.lookup(id, name).map(|ep| (name.clone(), ep)))
                    .collect()
            })
            .collect(),
    };

    // With stream-io streams wired, the loop runs in **realtime**: interactive
    // byte protocols on the other side of kble assume wall-clock time, so the
    // default compute-a-chunk-ahead-then-sleep pacing (which also runs much
    // faster than 1:1) would break them. Step one controller tick at a time,
    // pumping the bridge at each boundary, syncing each tick to the wall clock.
    let realtime = engine.is_realtime();
    let (outputs_per_chunk, chunk_wall_time) = if realtime {
        let tick = engine.effective_step();
        eprintln!("stream-io bridge active: realtime pacing (1 sim s = 1 wall s), tick = {tick} s");
        (1, Duration::from_secs_f64(tick))
    } else {
        (OUTPUTS_PER_CHUNK, default_chunk_wall_time)
    };

    let mut paused = false;

    loop {
        let chunk_start = tokio::time::Instant::now();

        // Process any pending commands between chunks
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if handle_command(&mut engine, &mut paused, &tx, cmd).is_break() {
                        // Tear down the bridge endpoints with the loop — while
                        // the manager is idle there is nothing to drain them
                        // (lingering peers see `defunct`).
                        bridge.reset(Vec::new());
                        return (LoopExit::Terminated, cmd_rx);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    bridge.reset(Vec::new());
                    return (LoopExit::Disconnected, cmd_rx);
                }
            }
        }

        // Skip propagation while paused
        if paused {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        // Offload the blocking propagation work to a dedicated blocking thread
        // so the tokio worker is free to handle WebSocket I/O and command
        // dispatch while the physics/controller step runs. This also keeps
        // `Handle::block_on` inside WASM async backends from starving the
        // serve runtime. The engine + stream adapter are moved in and handed
        // back so the loop retains ownership.
        let (chunk_result, engine_back, streams_back) = tokio::task::spawn_blocking(move || {
            let outputs = engine.step_chunk(outputs_per_chunk, &mut streams);
            (outputs, engine, streams)
        })
        .await
        .expect("simulation blocking task panicked");
        engine = engine_back;
        streams = streams_back;

        let step_output = match chunk_result {
            Ok(output) => output,
            Err(e) => {
                // Controller fault (bad command / guest trap / stream-io
                // overrun) or integration error. The sim state can no longer
                // be trusted; halt (pause) instead of integrating forward, and
                // tell clients.
                log::error!("simulation halted: {e}");
                let msg = serde_json::to_string(&WsMessage::Error {
                    message: format!("simulation halted: {e}"),
                })
                .expect("failed to serialize error");
                let _ = tx.send(msg);
                paused = true;
                // Clients drive their server-state UI off `status` messages;
                // without this they'd show a stale "running" after the halt.
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "paused".to_string(),
                })
                .expect("failed to serialize status");
                let _ = tx.send(status);
                continue;
            }
        };

        // Immediate (non-paced) broadcasts: `simulation_terminated` events.
        for msg in &step_output.broadcasts {
            let _ = tx.send(msg.clone());
        }
        let all_outputs = step_output.states;

        if realtime {
            // Realtime: ship states immediately, then sync this tick to the
            // wall clock (anchored to chunk_start so compute time is not
            // added on top — the controller never runs more than one tick
            // ahead of the peers).
            for out in &all_outputs {
                let _ = tx.send(state_json(out));
            }
            tokio::time::sleep_until(chunk_start + chunk_wall_time).await;
        } else if !all_outputs.is_empty() {
            let send_interval = chunk_wall_time / all_outputs.len() as u32;
            for out in &all_outputs {
                let send_start = tokio::time::Instant::now();
                let _ = tx.send(state_json(out));

                let send_elapsed = send_start.elapsed();
                if send_elapsed < send_interval {
                    tokio::time::sleep(send_interval - send_elapsed).await;
                }
            }
        } else {
            let elapsed = chunk_start.elapsed();
            if elapsed < chunk_wall_time {
                tokio::time::sleep(chunk_wall_time - elapsed).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::body::KnownBody;

    fn sim_args(extra: &[&str]) -> SimArgs {
        use clap::Parser;
        let mut argv = vec!["orts"];
        argv.extend_from_slice(extra);
        SimArgs::try_parse_from(argv).expect("valid sim args")
    }

    /// The async mode rides the overrides only when its flag was written.
    ///
    /// A `serve` nobody asked runs `Deterministic`, while `run` runs
    /// `throughput`, so the flag carries no clap default and a left-out flag
    /// arrives as `None`. Carrying a value either way would move every
    /// existing server off `Deterministic`. The backend choice is carried
    /// whether or not it was written, as before.
    #[test]
    fn the_async_mode_is_carried_only_when_its_flag_was_written() {
        let asked = PluginBackendOverrides::from_sim_args(&sim_args(&[
            "--plugin-backend-async-mode",
            "throughput",
        ]));
        assert_eq!(
            asked.async_mode,
            Some(PluginAsyncModeChoice::Throughput),
            "a written flag is what the overrides carry"
        );

        let unasked = PluginBackendOverrides::from_sim_args(&sim_args(&[]));
        assert_eq!(
            unasked.async_mode, None,
            "an absent flag leaves the mode to whoever built the params"
        );
        assert!(
            unasked.choice.is_some(),
            "the backend choice is carried whether or not it was written"
        );
    }

    /// Params keep the mode they were built with when the overrides carry none.
    #[test]
    fn params_keep_their_mode_when_the_overrides_carry_none() {
        let config: SimConfig = toml::from_str(
            r#"
body = "earth"

[[satellites]]
id = "a"

[satellites.orbit]
type = "circular"
altitude = 400.0
"#,
        )
        .expect("valid test toml");
        let mut params = SimParams::from_config(&config).expect("valid test config");
        assert_eq!(
            params.plugin_backend_async_mode,
            PluginAsyncModeChoice::Deterministic,
            "a config-built simulation starts deterministic"
        );

        PluginBackendOverrides::from_sim_args(&sim_args(&[])).apply(&mut params);

        assert_eq!(
            params.plugin_backend_async_mode,
            PluginAsyncModeChoice::Deterministic,
            "and keeps it when nothing on the command line asked otherwise"
        );
    }

    /// A WebSocket `start_simulation` goes through the same config gate as
    /// `orts serve --config`: the serve loop never drains a `[[command]]`
    /// timeline, so accepting one here would drop every scheduled uplink.
    #[test]
    fn ws_start_rejects_a_command_timeline() {
        let config: SimConfig = toml::from_str(
            r#"
body = "earth"
dt = 1.0

[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }

[[command]]
t = 10.0
sat = "sat-a"
kind = "orts.cmd.set-mode.v1"
"#,
        )
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("`[[command]]`"), "got: {err}");
    }

    /// A fleet where only some satellites have a controller cannot be honored
    /// by any mode, and is rejected here rather than at engine build.
    #[test]
    fn ws_start_rejects_mixed_controller_config() {
        let config: SimConfig = toml::from_str(
            r#"
body = "earth"
dt = 1.0

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
controller = { type = "wasm", path = "ctrl.wasm" }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
"#,
        )
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("Mixed controller config"), "got: {err}");
    }

    #[test]
    fn body_names_for_earth_includes_sun_and_moon() {
        let names = body_names_for(&KnownBody::Earth);
        assert_eq!(names[0], "earth");
        assert!(names.contains(&"sun".to_string()));
        assert!(names.contains(&"moon".to_string()));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn body_names_for_mars_includes_sun_only() {
        let names = body_names_for(&KnownBody::Mars);
        assert_eq!(names[0], "mars");
        assert!(names.contains(&"sun".to_string()));
        assert!(!names.contains(&"moon".to_string()));
        assert_eq!(names.len(), 2);
    }

    /// Moon-centred propagation has Earth as a third body, so its texture is
    /// wanted too.
    #[test]
    fn body_names_for_moon_includes_the_sun_and_earth() {
        let names = body_names_for(&KnownBody::Moon);
        assert_eq!(names[0], "moon");
        assert!(names.contains(&"sun".to_string()));
        assert!(names.contains(&"earth".to_string()));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn validate_sim_config_rejects_non_earth_tle() {
        // A WebSocket `StartSimulation` with a non-Earth body + TLE must be
        // rejected here (graceful Err), not reach the panic in `from_config`.
        let mars = r#"{
            "body": "mars",
            "satellites": [{
                "id": "iss",
                "orbit": {
                    "type": "tle",
                    "line1": "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996",
                    "line2": "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008"
                }
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(mars).unwrap();
        let err = validate_sim_config(&config)
            .expect_err("a non-Earth TLE config must be rejected, not panic");
        assert!(err.contains("Earth-centered"), "unexpected error: {err}");

        // The same satellite on Earth must not trip the body guard.
        let earth: SimConfig =
            serde_json::from_str(&mars.replace("\"mars\"", "\"earth\"")).unwrap();
        if let Err(e) = validate_sim_config(&earth) {
            assert!(
                !e.contains("Earth-centered"),
                "Earth config tripped the body guard: {e}"
            );
        }
    }

    /// `[gravity_field]` names a server-side file, so a WebSocket client must
    /// not be able to set it.
    #[test]
    fn ws_start_rejects_gravity_field() {
        let config: SimConfig = toml::from_str(
            r#"
[gravity_field]
path = "/etc/passwd"

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
"#,
        )
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("not accepted over WebSocket"), "got: {err}");
    }

    /// A `space_weather` path names a server-side file too, so a WebSocket
    /// client may ask only for the fetch (#556).
    ///
    /// Measured before this check: a FIFO path held the manager in
    /// `read_to_string`, and the server stopped completing new WebSocket
    /// handshakes.
    #[test]
    fn ws_start_accepts_only_the_space_weather_fetch() {
        let config_with = |space_weather: &str| -> SimConfig {
            toml::from_str(&format!(
                "{space_weather}\n[[satellites]]\nid = \"a\"\n\
                 orbit = {{ type = \"circular\", altitude = 500 }}\n"
            ))
            .expect("valid test toml")
        };
        let err =
            validate_sim_config(&config_with("space_weather = \"/tmp/sw.fifo\"")).unwrap_err();
        assert!(err.contains("not accepted over WebSocket"), "got: {err}");
        assert!(err.contains("/tmp/sw.fifo"), "the path is named: {err}");

        validate_sim_config(&config_with("space_weather = \"auto\""))
            .expect("the CelesTrak fetch opens no server file");
        validate_sim_config(&config_with("")).expect("no space weather at all");
    }

    /// A WebSocket `start_simulation` asking for `gcrs` is told, not served
    /// the `SimpleEci` propagation the engine actually does.
    #[test]
    fn ws_start_rejects_the_gcrs_frame() {
        let config: SimConfig = toml::from_str(
            r#"
frame = "gcrs"
eop = "zero"

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
"#,
        )
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("not supported by `orts serve`"), "got: {err}");
    }
}
