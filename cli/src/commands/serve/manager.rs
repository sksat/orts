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

use super::engine::{ChunkFailure, EngineInit, ServeEngine, StepOutput, StreamIo};
use super::history::HistoryBuffer;
use super::pacing::{LagWarnings, Pacing, RealtimeClock};
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
        /// The pacing asked for; `None` takes the server's.
        pacing: Option<Pacing>,
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
    pacing: Option<Pacing>,
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
    default_pacing: Pacing,
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
        default_pacing,
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
                default_pacing,
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
    // A controller `path` names a file on the server too. The connection
    // refused one before forwarding the message; this is the same rule where
    // the manager takes the command (#556).
    super::controller_upload::refuse_controller_paths(config)?;
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
            SimCommand::Start {
                config,
                pacing,
                respond,
            } => {
                if let Err(e) = validate_sim_config(&config) {
                    let _ = respond.send(Err(e));
                    continue;
                }
                // The manager task must survive a config it cannot build, so
                // the failure goes back to the client and the loop keeps
                // waiting for the next `start_simulation`.
                match SimParams::from_config(&config) {
                    Ok(params) => {
                        return Some(PendingStart {
                            params,
                            pacing,
                            respond,
                        });
                    }
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
    default_pacing: Pacing,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
    bridge: Arc<StreamBridge>,
) {
    eprintln!("Simulation manager: idle, waiting for start_simulation...");
    let first = idle_loop(&mut cmd_rx).await;
    run_client_starts(
        first,
        cli_plugin_overrides,
        default_pacing,
        cmd_rx,
        tx,
        texture_tx,
        bridge,
    )
    .await;
}

/// Run each simulation a client's `start_simulation` asked for, from `next`
/// on: run until terminated, return to idle, and take the next one, until the
/// command channel disconnects.
async fn run_client_starts(
    mut next: Option<PendingStart>,
    cli_plugin_overrides: PluginBackendOverrides,
    default_pacing: Pacing,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
    bridge: Arc<StreamBridge>,
) {
    while let Some(PendingStart {
        mut params,
        pacing: asked_pacing,
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
            asked_pacing.unwrap_or(default_pacing),
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

/// What [`deliver_chunk`] left for the caller to do.
struct Delivery {
    /// State samples still to be sent, paced to the wall clock.
    to_pace: Vec<crate::sim::core::HistoryState>,
    /// A finished chunk's other broadcasts (`simulation_terminated`), still to
    /// be sent: at once when accelerated, with the interval's states when
    /// realtime, so a termination is not announced before its time is due.
    broadcasts: Vec<String>,
    /// Whether the chunk ended on a fault, so the run has to pause.
    halted: bool,
}

/// Send what a failed chunk produced, and say what is left to do.
///
/// A chunk that finished sends nothing from here: its samples and its
/// terminations go back to the caller, which knows when they are due.
///
/// A chunk that failed partway hands back the samples and broadcasts its
/// finished intervals produced, and both reach the clients that are connected
/// here, because the caller pauses the run instead of pacing them out over the
/// next chunks.
///
/// The samples go out before the terminations on that path. The channel holds
/// 256 messages and drops the oldest once a receiver falls behind, so a chunk
/// with more samples than that would push a termination sent first out of a
/// slow client's buffer. A dropped sample that falls on an output interval is
/// recoverable — the history keeps one sample per `output_interval`, and a
/// client can query the range — while a termination reaches a running client
/// only here. The samples between output intervals, which a stream step
/// shorter than `output_interval` produces, are not in the history and are
/// lost with the dropped message.
///
/// The order is what this can do, not a guarantee: a fleet that stops more
/// satellites in one chunk than the channel holds still outruns a client that
/// is not draining. The latest [`super::engine::TERMINATED_EVENTS_CAP`] terminations are in
/// `terminated_events` as well, which a client reads on (re)connect, so what a
/// lagged client loses among those it can still recover — over a reconnect
/// rather than in place. A chunk that stops more satellites than that cap
/// loses its earliest terminations from both paths.
fn deliver_chunk(
    tx: &broadcast::Sender<String>,
    chunk: Result<StepOutput, ChunkFailure>,
) -> Delivery {
    let (partial, error) = match chunk {
        Ok(output) => {
            return Delivery {
                to_pace: output.states,
                broadcasts: output.broadcasts,
                halted: false,
            };
        }
        Err(ChunkFailure { error, partial }) => (partial, error),
    };

    // Controller fault (bad command / guest trap / stream-io overrun) or
    // integration error. The sim state can no longer be trusted, so the caller
    // pauses instead of integrating forward, and the clients are told. The
    // engine keeps the fault, so a resume is refused.
    log::error!("simulation halted: {error}");
    for out in &partial.states {
        let _ = tx.send(state_json(out));
    }
    for msg in &partial.broadcasts {
        let _ = tx.send(msg.clone());
    }
    let msg = serde_json::to_string(&WsMessage::Error {
        message: format!("simulation halted: {error}"),
    })
    .expect("failed to serialize error");
    let _ = tx.send(msg);
    // Clients drive their server-state UI off `status` messages; without this
    // they'd show a stale "running" after the halt.
    let status = serde_json::to_string(&WsMessage::Status {
        state: "paused".to_string(),
        default_pacing: None,
    })
    .expect("failed to serialize status");
    let _ = tx.send(status);

    Delivery {
        to_pace: Vec::new(),
        broadcasts: Vec::new(),
        halted: true,
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
///
/// A run is paused because a client paused it, or because a chunk failed, in
/// which case [`ServeEngine::halted`] holds the fault. Only a run paused by a
/// client resumes.
///
/// `held` is `Some` while a realtime interval waits for its time (see
/// [`HeldBack`]); `None` when everything stepped has gone out.
fn handle_command(
    engine: &mut ServeEngine,
    paused: &mut bool,
    tx: &broadcast::Sender<String>,
    cmd: SimCommand,
    held: Option<HeldBack<'_>>,
) -> ControlFlow<()> {
    match cmd {
        SimCommand::GetStatus { respond } => {
            let data = match &held {
                Some(held) => engine.status_data_until(held.sent_until, held.newest_sent.values()),
                None => engine.status_data(),
            };
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
                    default_pacing: None,
                })
                .expect("failed to serialize status");
                let _ = tx.send(status);
                let _ = respond.send(Ok(()));
            }
        }
        SimCommand::Resume { respond } => {
            if let Some(fault) = engine.halted() {
                // The satellites may be ahead of the engine's clock, which
                // only a new run puts right. The run stays paused.
                let _ = respond.send(Err(format!(
                    "Cannot resume: the simulation halted on a fault ({fault}). \
                     Send terminate_simulation, then start_simulation"
                )));
            } else if !*paused {
                let _ = respond.send(Err("Simulation is not paused".to_string()));
            } else {
                *paused = false;
                eprintln!("Simulation resumed at t={:.2}s", engine.current_t());
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "running".to_string(),
                    default_pacing: None,
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
                default_pacing: None,
            })
            .expect("failed to serialize status");
            let _ = tx.send(status);
            let _ = respond.send(Ok(()));
            return ControlFlow::Break(());
        }
        SimCommand::AddSatellite { satellite, respond } => {
            // Every `add_satellite` comes from a WebSocket client, so its
            // controller never names a path: the connection refused one before
            // forwarding, and the engine is not asked to open one here either.
            let added =
                super::controller_upload::refuse_controller_path(&satellite, "add_satellite")
                    .and_then(|()| engine.add_satellite(*satellite));
            match added {
                Ok(out) => {
                    match held {
                        Some(held) => held.broadcasts.extend(out.broadcasts),
                        None => {
                            for msg in &out.broadcasts {
                                let _ = tx.send(msg.clone());
                            }
                        }
                    }
                    let _ = respond.send(Ok((out.info, out.t)));
                }
                Err(e) => {
                    let _ = respond.send(Err(e));
                }
            }
        }
        SimCommand::QueryRange {
            t_min,
            t_max,
            max_points,
            entity_path,
            respond,
        } => {
            let t_max = held.map_or(t_max, |held| t_max.min(held.sent_until));
            let states = if t_min <= t_max {
                engine.query_range(t_min, t_max, max_points, entity_path.as_ref())
            } else {
                Vec::new()
            };
            let _ = respond.send(states);
        }
    }
    ControlFlow::Continue(())
}

/// The newest state sent, per satellite (keyed by entity path).
type NewestSent = std::collections::HashMap<String, crate::sim::core::HistoryState>;

/// What a realtime interval holds back while it waits for its time.
///
/// The interval was stepped when its start was due, so the engine and the
/// history already stand at its end while its states wait to go out. A status
/// or a query answered meanwhile stops at `sent_until`, so a client does not
/// see what the broadcast is holding back. A satellite added meanwhile joins
/// at the interval's end; it is added at once — the reply would otherwise
/// hold up the client's connection, which reads nothing else until it comes —
/// and its broadcasts go into `broadcasts`, sent with the interval's states.
struct HeldBack<'a> {
    sent_until: f64,
    /// The newest state sent per satellite, which a status ends on (see
    /// [`ServeEngine::status_data_until`]).
    newest_sent: &'a NewestSent,
    broadcasts: &'a mut Vec<String>,
}

/// How [`wait_until_due`] ended.
enum Waited {
    /// The instant waited for came.
    Due,
    /// A command paused the run first.
    Paused,
    /// A command ended the run, or every sender is gone.
    Exit(LoopExit),
}

/// Wait until `due`, handling commands as they come: a realtime interval can
/// be a whole `stream_interval` of wall time, and a pause or a terminate must
/// not wait that out.
///
/// `sent_until`, `newest_sent` and `held_broadcasts` make up the [`HeldBack`]
/// the commands are handled against.
#[allow(clippy::too_many_arguments)]
async fn wait_until_due(
    due: tokio::time::Instant,
    sent_until: f64,
    newest_sent: &NewestSent,
    engine: &mut ServeEngine,
    paused: &mut bool,
    tx: &broadcast::Sender<String>,
    cmd_rx: &mut mpsc::Receiver<SimCommand>,
    held_broadcasts: &mut Vec<String>,
) -> Waited {
    let sleep = tokio::time::sleep_until(due);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return Waited::Due,
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    return Waited::Exit(LoopExit::Disconnected);
                };
                let held = HeldBack {
                    sent_until,
                    newest_sent,
                    broadcasts: held_broadcasts,
                };
                if handle_command(engine, paused, tx, cmd, Some(held)).is_break() {
                    return Waited::Exit(LoopExit::Terminated);
                }
                if *paused {
                    return Waited::Paused;
                }
            }
        }
    }
}

/// Core simulation loop: builds the engine, drives propagation, dispatches
/// commands, and paces output to the wall clock. Returns the exit reason and
/// gives back the command receiver for reuse.
///
/// `pacing` is the one the simulation was asked to run at — its
/// `start_simulation`'s, else the server's `--realtime`; a fleet with stream-io
/// streams runs in realtime whatever it says (see [`Pacing::for_fleet`]).
///
/// `requester` is the reply channel of the `start_simulation` that asked for
/// this simulation. It is answered once the engine is built: `Err` if the
/// build fails, and no other client hears of that failure; `Ok` if it
/// succeeds, before the first `info` is broadcast. `None` is the simulation
/// `orts serve` starts from its own command line, which no client asked for;
/// its build failure is broadcast to whoever is connected.
async fn run_simulation_loop(
    params: Arc<SimParams>,
    pacing: Pacing,
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
    } = match ServeEngine::build(params, history, pacing) {
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

    // In **realtime** the loop steps one interval at a time: with stream-io
    // streams wired that is one controller tick, pumping the bridge at each
    // boundary — interactive byte protocols on the other side of kble assume
    // wall-clock time, so the default compute-a-chunk-ahead pacing (which also
    // runs much faster than 1:1) would break them. `--realtime` asks for the
    // same pacing with one `stream_interval` per step.
    let realtime = engine.pacing() == Pacing::Realtime;
    let (outputs_per_chunk, chunk_wall_time) = if realtime {
        let tick = engine.effective_step();
        if engine.has_streams() {
            eprintln!(
                "stream-io bridge active: realtime pacing (1 sim s = 1 wall s), tick = {tick} s"
            );
        } else {
            eprintln!("realtime pacing (1 sim s = 1 wall s), step = {tick} s");
        }
        (1, Duration::from_secs_f64(tick))
    } else {
        (OUTPUTS_PER_CHUNK, default_chunk_wall_time)
    };

    let mut paused = false;
    // Realtime only: when each sim time is due on the wall clock. Cleared
    // while paused, so a resumed run is anchored where it resumes instead of
    // stepping back to back to make up the pause.
    let mut clock: Option<RealtimeClock> = None;
    let mut lag_warnings = LagWarnings::default();
    let mut newest_sent = NewestSent::new();

    loop {
        let chunk_start = tokio::time::Instant::now();

        // Process any pending commands between chunks
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if handle_command(&mut engine, &mut paused, &tx, cmd, None).is_break() {
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
            clock = None;
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if realtime && clock.is_none() {
            clock = Some(RealtimeClock::anchor(
                tokio::time::Instant::now(),
                engine.current_t(),
            ));
        }

        // Offload the blocking propagation work to a dedicated blocking thread
        // so the tokio worker is free to handle WebSocket I/O and command
        // dispatch while the physics/controller step runs. This also keeps
        // `Handle::block_on` inside WASM async backends from starving the
        // serve runtime. The engine + stream adapter are moved in and handed
        // back so the loop retains ownership.
        // Every state up to here has been sent: a realtime interval's states
        // go out at the end of its wait, before the loop comes back here.
        let sent_until = engine.current_t();
        let (chunk_result, engine_back, streams_back) = tokio::task::spawn_blocking(move || {
            let outputs = engine.step_chunk(outputs_per_chunk, &mut streams);
            (outputs, engine, streams)
        })
        .await
        .expect("simulation blocking task panicked");
        engine = engine_back;
        streams = streams_back;

        let delivery = deliver_chunk(&tx, chunk_result);
        if delivery.halted {
            paused = true;
            continue;
        }
        let all_outputs = delivery.to_pace;
        let mut held_broadcasts = delivery.broadcasts;
        if clock.is_none() {
            // Accelerated: a termination goes out at once, ahead of the paced
            // states.
            for msg in held_broadcasts.drain(..) {
                let _ = tx.send(msg);
            }
        }

        if let Some(clock) = clock.as_mut() {
            // Realtime: the interval was stepped when its start was due, so
            // the controller ran each tick on time and never more than one
            // tick ahead of the peers. Its states are held until the time
            // they belong to, so a client never sees the future.
            let reached = engine.current_t();
            let now = tokio::time::Instant::now();
            if let Some(dropped) = clock.drop_excess_lag(now, reached)
                && let Some(report) = lag_warnings.record(now, dropped)
            {
                log::warn!(
                    "realtime pacing fell behind the wall clock {} time(s), {:.1} s in \
                     total; skipped ahead instead of catching up",
                    report.times,
                    report.dropped.as_secs_f64()
                );
            }
            let waited = wait_until_due(
                clock.due(reached),
                sent_until,
                &newest_sent,
                &mut engine,
                &mut paused,
                &tx,
                &mut cmd_rx,
                &mut held_broadcasts,
            )
            .await;
            match waited {
                // Paused while waiting: the held states are in the history
                // already, so they go out now rather than after the resume.
                Waited::Due | Waited::Paused => {
                    for out in all_outputs {
                        let _ = tx.send(state_json(&out));
                        newest_sent.insert(out.entity_path.to_string(), out);
                    }
                    for msg in held_broadcasts {
                        let _ = tx.send(msg);
                    }
                }
                Waited::Exit(exit) => {
                    bridge.reset(Vec::new());
                    return (exit, cmd_rx);
                }
            }
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

        // A pause during the wait lets go of the anchor here, not at the
        // paused check above: a resume queued behind the pause is handled
        // before that check, which would then never see the run paused.
        if paused {
            clock = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::engine::test_support::{
        Chatty, FailsFirstTick, NullStreamIo, StuckOnce, TM, controlled_engine,
    };
    use super::*;
    use arika::body::KnownBody;

    /// A running serve loop on a paused tokio clock, and what it broadcasts.
    struct LoopUnderTest {
        cmd_tx: mpsc::Sender<SimCommand>,
        rx: broadcast::Receiver<String>,
        /// The instant the loop was spawned at.
        started: tokio::time::Instant,
        _data_dir: tempfile::TempDir,
    }

    impl LoopUnderTest {
        fn spawn(toml: &str, pacing: Pacing) -> Self {
            let config: SimConfig = toml::from_str(toml).expect("valid test toml");
            let params = Arc::new(SimParams::from_config(&config).expect("valid test config"));
            let data_dir = tempfile::tempdir().expect("temp dir");
            let body_radius = params.body.properties().radius;
            let history =
                HistoryBuffer::new(5000, data_dir.path().to_path_buf(), params.mu, body_radius);
            let (tx, rx) = broadcast::channel(256);
            let (cmd_tx, cmd_rx) = mpsc::channel(16);
            let started = tokio::time::Instant::now();
            tokio::spawn(run_simulation_loop(
                params,
                pacing,
                None,
                cmd_rx,
                tx,
                history,
                Arc::new(StreamBridge::new()),
            ));
            Self {
                cmd_tx,
                rx,
                started,
                _data_dir: data_dir,
            }
        }

        /// The next state past t = 0, and the wall time since the spawn at
        /// which it was broadcast.
        async fn next_state(&mut self) -> (f64, Duration) {
            loop {
                let msg = self.rx.recv().await.expect("the loop is broadcasting");
                let v: serde_json::Value = serde_json::from_str(&msg).expect("JSON message");
                if v["type"] == "state" {
                    let t = v["t"].as_f64().expect("a state carries t");
                    if t > 0.0 {
                        return (t, self.started.elapsed());
                    }
                }
            }
        }

        async fn send(
            &self,
            command: fn(oneshot::Sender<Result<(), String>>) -> SimCommand,
        ) -> Result<(), String> {
            let (respond, reply) = oneshot::channel();
            assert!(self.cmd_tx.send(command(respond)).await.is_ok());
            reply.await.expect("the loop answers")
        }
    }

    /// Ten-second output intervals on one stable orbit.
    const TEN_SECOND_INTERVALS: &str = r#"
dt = 1
output_interval = 10

[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }
"#;

    /// Sim time and wall time agree to within this. The clock is paused, so
    /// the only slack is the loop's own polling.
    const WALL_TOLERANCE: Duration = Duration::from_millis(1);

    fn assert_wall_time(at: Duration, expected_secs: f64) {
        let expected = Duration::from_secs_f64(expected_secs);
        assert!(
            at.abs_diff(expected) <= WALL_TOLERANCE,
            "broadcast at {at:?}, expected {expected:?}"
        );
    }

    /// `--realtime`: each state goes out when its sim time is due on the wall
    /// clock, 1 sim s per wall s from the start, not ahead of it.
    #[tokio::test(start_paused = true)]
    async fn realtime_states_go_out_when_their_sim_time_is_due() {
        let mut sim = LoopUnderTest::spawn(TEN_SECOND_INTERVALS, Pacing::Realtime);
        for expected_t in [10.0, 20.0, 30.0] {
            let (t, at) = sim.next_state().await;
            assert_eq!(t, expected_t);
            assert_wall_time(at, t);
        }
    }

    /// Without `--realtime` the same simulation runs far ahead of the wall
    /// clock — the contrast that makes the realtime test above mean something.
    #[tokio::test(start_paused = true)]
    async fn accelerated_states_run_ahead_of_the_wall_clock() {
        let mut sim = LoopUnderTest::spawn(TEN_SECOND_INTERVALS, Pacing::Accelerated);
        let mut last = (0.0, Duration::ZERO);
        while last.0 < 30.0 {
            last = sim.next_state().await;
        }
        assert!(
            last.1 < Duration::from_secs(1),
            "t = {} went out at {:?}",
            last.0,
            last.1
        );
    }

    /// A client connecting or querying in the middle of a realtime interval
    /// sees the history only up to the states already sent. The interval was
    /// stepped when its start was due, so its state is in the history while it
    /// waits for its own time; handing it out early would show a reconnecting
    /// client the future the broadcast is holding back.
    #[tokio::test(start_paused = true)]
    async fn realtime_history_stops_at_what_has_been_sent() {
        const SIXTY_SECOND_INTERVALS: &str = r#"
dt = 10
output_interval = 60

[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }
"#;
        let mut sim = LoopUnderTest::spawn(SIXTY_SECOND_INTERVALS, Pacing::Realtime);
        tokio::time::sleep(Duration::from_secs(5)).await;

        let latest_t = |states: &[crate::sim::core::HistoryState]| {
            states.iter().map(|s| s.t).fold(f64::NEG_INFINITY, f64::max)
        };
        let (respond, reply) = oneshot::channel();
        assert!(
            sim.cmd_tx
                .send(SimCommand::GetStatus { respond })
                .await
                .is_ok()
        );
        let SimStatusResponse::Running { history_states, .. } = reply.await.unwrap() else {
            panic!("the simulation is running");
        };
        assert_eq!(latest_t(&history_states), 0.0, "status at 5 s");

        let (respond, reply) = oneshot::channel();
        let query = SimCommand::QueryRange {
            t_min: 0.0,
            t_max: 1000.0,
            max_points: None,
            entity_path: None,
            respond,
        };
        assert!(sim.cmd_tx.send(query).await.is_ok());
        assert_eq!(latest_t(&reply.await.unwrap()), 0.0, "query at 5 s");

        // Once t = 60 has gone out, it is history like any other.
        let (t, _) = sim.next_state().await;
        assert_eq!(t, 60.0);
        let (respond, reply) = oneshot::channel();
        let query = SimCommand::QueryRange {
            t_min: 0.0,
            t_max: 1000.0,
            max_points: None,
            entity_path: None,
            respond,
        };
        assert!(sim.cmd_tx.send(query).await.is_ok());
        assert_eq!(
            latest_t(&reply.await.unwrap()),
            60.0,
            "query after the send"
        );
    }

    /// A satellite added in the middle of a realtime interval is added at
    /// once — the client's connection reads nothing else until the reply —
    /// and joins at the interval's end, where the engine already stands. Its
    /// broadcasts wait for that time with the fleet's: sent at once, its
    /// first state at t = 60 went out at 5 s.
    #[tokio::test(start_paused = true)]
    async fn realtime_add_satellite_answers_at_once_and_its_states_wait() {
        const SIXTY_SECOND_INTERVALS: &str = r#"
dt = 10
output_interval = 60

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
"#;
        let mut sim = LoopUnderTest::spawn(SIXTY_SECOND_INTERVALS, Pacing::Realtime);
        tokio::time::sleep(Duration::from_secs(5)).await;

        let satellite: SatelliteConfig = serde_json::from_value(serde_json::json!({
            "id": "b",
            "orbit": { "type": "circular", "altitude": 600.0 },
        }))
        .expect("a valid satellite");
        let (respond, reply) = oneshot::channel();
        let add = SimCommand::AddSatellite {
            satellite: Box::new(satellite),
            respond,
        };
        assert!(sim.cmd_tx.send(add).await.is_ok());
        let (_, t) = reply.await.expect("answered").expect("added");
        assert_eq!(t, 60.0, "joins at the interval's end");
        assert_wall_time(sim.started.elapsed(), 5.0);

        // Nothing past t = 0 goes out before 60 s: not the fleet's state, not
        // the new satellite's announcement or its first state.
        let mut seen_added = false;
        let mut seen_b_state = false;
        while !(seen_added && seen_b_state) {
            let msg = sim.rx.recv().await.expect("broadcasting");
            let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
            let after_start = v["type"] == "satellite_added"
                || (v["type"] == "state" && v["t"].as_f64().unwrap() > 0.0);
            if after_start {
                assert_wall_time(sim.started.elapsed(), 60.0);
            }
            seen_added |= v["type"] == "satellite_added";
            seen_b_state |=
                v["type"] == "state" && v["entity_path"].as_str().unwrap().ends_with("/b");
        }
    }

    /// The status a client gets during a realtime wait.
    async fn status_now(sim: &LoopUnderTest) -> (Vec<String>, Vec<crate::sim::core::HistoryState>) {
        let (respond, reply) = oneshot::channel();
        assert!(
            sim.cmd_tx
                .send(SimCommand::GetStatus { respond })
                .await
                .is_ok()
        );
        match reply.await.unwrap() {
            SimStatusResponse::Running {
                terminated_events,
                history_states,
                ..
            } => (terminated_events, history_states),
            _ => panic!("the simulation is running"),
        }
    }

    /// A satellite that comes down inside a realtime interval is announced
    /// with the interval's states, when its end is due, and a status taken
    /// before then does not list it. Sent at once, the viewer marked it
    /// terminated minutes of sim time early.
    #[tokio::test(start_paused = true)]
    async fn realtime_termination_waits_for_its_interval() {
        const DECAYING: &str = r#"
dt = 1
output_interval = 600

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 100.1 }
"#;
        let mut sim = LoopUnderTest::spawn(DECAYING, Pacing::Realtime);
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (terminated, _) = status_now(&sim).await;
        assert!(terminated.is_empty(), "listed early: {terminated:?}");

        loop {
            let msg = sim.rx.recv().await.expect("broadcasting");
            let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
            if v["type"] == "simulation_terminated" {
                assert!(v["t"].as_f64().unwrap() < 600.0, "{v}");
                assert_wall_time(sim.started.elapsed(), 600.0);
                break;
            }
        }
    }

    /// A status taken during a realtime wait ends at the newest state sent,
    /// once the overview has started thinning out. The overview keeps its
    /// newest sample by overwriting its last one, so the not-yet-sent state
    /// had replaced the newest sent one, and cutting it off left the status
    /// ending an interval or more early.
    #[tokio::test(start_paused = true)]
    async fn realtime_status_ends_at_the_newest_state_sent() {
        let mut sim = LoopUnderTest::spawn(TEN_SECOND_INTERVALS, Pacing::Realtime);
        // Past `OVERVIEW_MAX_POINTS_PER_ENTITY` samples, where thinning starts.
        let intervals = 2 * super::super::history::OVERVIEW_MAX_POINTS_PER_ENTITY;
        let mut newest_sent = 0.0;
        for _ in 0..intervals {
            newest_sent = sim.next_state().await.0;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;

        let (_, history) = status_now(&sim).await;
        let newest = history
            .iter()
            .map(|s| s.t)
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(newest, newest_sent);
    }

    /// A pause and a resume that arrive together in the middle of an interval
    /// still re-anchor the clock: the run carries on 1:1 from the resume. The
    /// resume used to be handled before the loop looked for the pause, which
    /// kept the old anchor, so the next state waited out its old due time.
    #[tokio::test(start_paused = true)]
    async fn realtime_pause_and_resume_together_re_anchor() {
        let mut sim = LoopUnderTest::spawn(TEN_SECOND_INTERVALS, Pacing::Realtime);
        tokio::time::sleep(Duration::from_secs(2)).await;

        let (pause, paused) = oneshot::channel();
        let (resume, resumed) = oneshot::channel();
        assert!(
            sim.cmd_tx
                .send(SimCommand::Pause { respond: pause })
                .await
                .is_ok()
        );
        assert!(
            sim.cmd_tx
                .send(SimCommand::Resume { respond: resume })
                .await
                .is_ok()
        );
        paused.await.unwrap().expect("pauses");
        resumed.await.unwrap().expect("resumes");

        let (t, at) = sim.next_state().await;
        assert_eq!(t, 10.0);
        assert_wall_time(at, 2.0);
        let (t, at) = sim.next_state().await;
        assert_eq!(t, 20.0);
        assert_wall_time(at, 12.0);
    }

    /// A pause in the middle of a long realtime interval is answered at once,
    /// and the state that interval produced goes out with it. A resume then
    /// carries on at 1:1 from where it resumed, without stepping back to back
    /// to make up the time spent paused.
    #[tokio::test(start_paused = true)]
    async fn realtime_pause_is_immediate_and_resume_does_not_catch_up() {
        const SIXTY_SECOND_INTERVALS: &str = r#"
dt = 10
output_interval = 60

[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }
"#;
        let mut sim = LoopUnderTest::spawn(SIXTY_SECOND_INTERVALS, Pacing::Realtime);
        tokio::time::sleep(Duration::from_secs(5)).await;

        sim.send(|respond| SimCommand::Pause { respond })
            .await
            .expect("a running simulation pauses");
        assert_wall_time(sim.started.elapsed(), 5.0);
        let (t, at) = sim.next_state().await;
        assert_eq!(t, 60.0, "the interval stepped before the pause");
        assert_wall_time(at, 5.0);

        tokio::time::sleep(Duration::from_secs(100)).await;
        sim.send(|respond| SimCommand::Resume { respond })
            .await
            .expect("a paused simulation resumes");
        // The paused loop looks at its commands every 100 ms.
        let resumed = sim.started.elapsed();
        assert!(resumed.abs_diff(Duration::from_secs(105)) <= Duration::from_millis(100));

        let (t, at) = sim.next_state().await;
        assert_eq!(t, 120.0);
        assert!(
            at.abs_diff(resumed + Duration::from_secs(60)) <= WALL_TOLERANCE,
            "t = 120 went out at {at:?}, 60 s after the resume at {resumed:?}"
        );
    }

    /// What `handle_command` replied to a pause or a resume, and what it
    /// broadcast.
    fn pause_or_resume(
        engine: &mut ServeEngine,
        paused: &mut bool,
        command: fn(oneshot::Sender<Result<(), String>>) -> SimCommand,
    ) -> (Result<(), String>, Vec<String>) {
        let (tx, mut rx) = broadcast::channel(16);
        let (respond, mut reply) = oneshot::channel();
        let flow = handle_command(engine, paused, &tx, command(respond), None);
        assert!(flow.is_continue(), "neither command ends the run");
        let reply = reply.try_recv().expect("the command is answered at once");
        let sent = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        (reply, sent)
    }

    /// A run paused by a fault stays paused when a client asks to resume it
    /// (#538).
    ///
    /// The satellite had been stepped past the engine's clock when the
    /// interval failed, so resuming would integrate the advanced state from
    /// the older time.
    fn assert_resume_is_refused_after(
        mut engine: ServeEngine,
        streams: &mut dyn StreamIo,
        fault: &str,
    ) {
        let error = engine
            .step_chunk(1, streams)
            .err()
            .expect("the fixture's fault halts the run")
            .error;
        assert!(error.contains(fault), "got: {error}");
        // What the serve loop does with a chunk that failed.
        let mut paused = true;

        let (reply, sent) = pause_or_resume(&mut engine, &mut paused, |respond| {
            SimCommand::Resume { respond }
        });
        let refusal = reply.expect_err("resume of a run paused by a fault was accepted");
        assert!(
            refusal.contains(fault),
            "the refusal names the fault: {refusal}"
        );
        assert!(
            refusal.contains("start_simulation"),
            "and says how to go on: {refusal}"
        );
        assert!(paused, "the run stays paused");
        assert!(sent.is_empty(), "no status goes out: {sent:?}");
    }

    #[test]
    fn a_run_a_stuck_peer_paused_is_not_resumed() {
        assert_resume_is_refused_after(
            controlled_engine(Box::new(Chatty), &[TM]),
            &mut StuckOnce::default(),
            "not draining",
        );
    }

    #[test]
    fn a_run_a_failed_controller_paused_is_not_resumed() {
        assert_resume_is_refused_after(
            controlled_engine(Box::new(FailsFirstTick::default()), &[]),
            &mut NullStreamIo,
            "controller error",
        );
    }

    /// A run paused by a client resumes, and steps on from where it stopped.
    #[test]
    fn a_run_the_client_paused_resumes() {
        let mut engine = controlled_engine(Box::new(Chatty), &[TM]);
        let step = engine.effective_step();
        engine
            .step_chunk(1, &mut NullStreamIo)
            .expect("a peer that is not connected discards the bytes");
        let mut paused = false;

        let (reply, sent) = pause_or_resume(&mut engine, &mut paused, |respond| {
            SimCommand::Pause { respond }
        });
        reply.expect("a running run pauses");
        assert!(paused);
        assert!(
            sent.len() == 1 && sent[0].contains("\"paused\""),
            "the clients hear the run paused: {sent:?}"
        );

        let (reply, sent) = pause_or_resume(&mut engine, &mut paused, |respond| {
            SimCommand::Resume { respond }
        });
        reply.expect("a run the client paused resumes");
        assert!(!paused);
        assert!(
            sent.len() == 1 && sent[0].contains("\"running\""),
            "the clients hear the run resumed: {sent:?}"
        );

        engine
            .step_chunk(1, &mut NullStreamIo)
            .expect("the resumed run steps");
        assert_eq!(engine.current_t(), 2.0 * step, "from where it stopped");
    }

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

    /// One state sample, enough to be recognised in the broadcast channel.
    fn a_sample(t: f64) -> crate::sim::core::HistoryState {
        crate::sim::core::make_history_state(
            orts::record::entity_path::EntityPath::parse("/world/sat/healthy"),
            t,
            &nalgebra::Vector3::new(6878.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.6, 0.0),
            KnownBody::Earth.properties().mu,
            KnownBody::Earth.properties().radius,
            crate::sim::core::ModelLoads::default(),
            None,
        )
    }

    /// A chunk that failed partway reaches the connected clients: the samples
    /// its finished intervals produced, then the termination, then the error
    /// and the paused status.
    ///
    /// The samples are sent here rather than handed back, because a fault that
    /// does not clear means no later chunk to pace them with.
    #[test]
    fn a_failed_chunk_is_sent_before_the_run_is_paused() {
        let (tx, mut rx) = broadcast::channel(16);
        let delivery = deliver_chunk(
            &tx,
            Err(ChunkFailure {
                error: "stream-io: inbound staging overflow on doomed/tc".to_string(),
                partial: StepOutput {
                    states: vec![a_sample(1.0)],
                    broadcasts: vec![r#"{"type":"simulation_terminated","t":1.0}"#.to_string()],
                },
            }),
        );

        assert!(delivery.halted, "the caller pauses the run");
        assert!(
            delivery.to_pace.is_empty(),
            "nothing is left for the caller to pace: {:?}",
            delivery.to_pace.len()
        );

        let sent: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            sent.len(),
            4,
            "sample, termination, error, status: {sent:?}"
        );
        assert!(
            sent[0].contains("\"state\""),
            "the sample the finished interval produced goes out first: {}",
            sent[0]
        );
        assert!(
            sent[1].contains("simulation_terminated"),
            "then the termination: {}",
            sent[1]
        );
        assert!(
            sent[2].contains("simulation halted") && sent[2].contains("overflow"),
            "then the error, naming the fault: {}",
            sent[2]
        );
        assert!(
            sent[3].contains("paused"),
            "and the status the client's UI reads: {}",
            sent[3]
        );
    }

    /// More samples than the channel holds must not cost the termination.
    ///
    /// `tokio::sync::broadcast` drops the oldest message once a receiver falls
    /// behind, and `serve` runs a 256-message channel
    /// (`super::super::run_server`). A fleet large enough to fill it in one
    /// chunk would lose whatever was sent first, which is why the samples go
    /// before the termination.
    #[test]
    fn a_termination_survives_more_samples_than_the_channel_holds() {
        const CAPACITY: usize = 256;

        let (tx, mut rx) = broadcast::channel(CAPACITY);
        let states: Vec<crate::sim::core::HistoryState> =
            (0..CAPACITY + 50).map(|i| a_sample(i as f64)).collect();
        let delivery = deliver_chunk(
            &tx,
            Err(ChunkFailure {
                error: "guest trap".to_string(),
                partial: StepOutput {
                    states,
                    broadcasts: vec![r#"{"type":"simulation_terminated","t":9.0}"#.to_string()],
                },
            }),
        );
        assert!(delivery.halted);

        // The receiver never drained, so the oldest messages are gone: what is
        // left has to still carry the termination, the error and the status.
        let mut kept = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(msg) => kept.push(msg),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        assert!(
            kept.iter().any(|m| m.contains("simulation_terminated")),
            "the termination was evicted by the samples ({} messages kept)",
            kept.len()
        );
        assert!(
            kept.iter().any(|m| m.contains("simulation halted")),
            "and so was the error ({} messages kept)",
            kept.len()
        );
        assert!(
            kept.last().is_some_and(|m| m.contains("paused")),
            "the status is the last thing the clients hear"
        );
    }

    /// A chunk that finished leaves its samples and its terminations to the
    /// caller, which paces them against the wall clock: a realtime interval
    /// holds its terminations back with its states.
    #[test]
    fn a_finished_chunk_leaves_its_samples_and_terminations_to_the_caller() {
        let (tx, mut rx) = broadcast::channel(16);
        let delivery = deliver_chunk(
            &tx,
            Ok(StepOutput {
                states: vec![a_sample(1.0), a_sample(2.0)],
                broadcasts: vec![r#"{"type":"simulation_terminated","t":1.0}"#.to_string()],
            }),
        );

        assert!(!delivery.halted, "a termination is not a fault");
        assert_eq!(delivery.to_pace.len(), 2, "the samples are the caller's");
        assert_eq!(delivery.broadcasts.len(), 1, "so is the termination");
        assert!(delivery.broadcasts[0].contains("simulation_terminated"));
        assert!(rx.try_recv().is_err(), "nothing goes out from here");
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
    ///
    /// The controller names an uploaded component by `sha256`, as one from a
    /// WebSocket client does: a `path` would be refused first, for a reason of
    /// its own.
    #[test]
    fn ws_start_rejects_mixed_controller_config() {
        let config: SimConfig = toml::from_str(&format!(
            r#"
body = "earth"
dt = 1.0

[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 500 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 50 }}
controller = {{ type = "wasm", sha256 = "{}" }}

[[satellites]]
id = "b"
orbit = {{ type = "circular", altitude = 600 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 50 }}
"#,
            "0".repeat(64)
        ))
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("Mixed controller config"), "got: {err}");
    }

    /// A controller `path` in a WebSocket `start_simulation` is refused, and
    /// the path is not repeated back (#556).
    ///
    /// Measured before this check: a FIFO path held the manager in the read
    /// of the component, and the server stopped answering new connections.
    #[test]
    fn ws_start_refuses_a_controller_path() {
        let config: SimConfig = toml::from_str(
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
controller = { type = "wasm", path = "/tmp/ctrl.fifo" }
"#,
        )
        .expect("valid test toml");
        let err = validate_sim_config(&config).unwrap_err();
        assert!(err.contains("not accepted over WebSocket"), "got: {err}");
        assert!(err.starts_with("satellites[0].controller"), "got: {err}");
        assert!(!err.contains("ctrl.fifo"), "the path is not echoed: {err}");
    }

    /// An `add_satellite` whose controller names a path is refused before the
    /// engine sees it (#556).
    ///
    /// The engine here runs orbit-only, which refuses any controlled satellite
    /// with a message of its own; getting the path refusal instead shows the
    /// manager checked first, so no mode lets a path through to a build.
    #[test]
    fn ws_add_refuses_a_controller_path_before_the_engine() {
        let config: SimConfig = toml::from_str(
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
"#,
        )
        .expect("valid test toml");
        let params = Arc::new(SimParams::from_config(&config).expect("valid test config"));
        let data_dir = std::env::temp_dir().join(format!(
            "orts-manager-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let body_radius = params.body.properties().radius;
        let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
        let mut engine = ServeEngine::build(params, history, Pacing::Accelerated)
            .expect("an orbit-only engine builds")
            .engine;
        let satellite: SatelliteConfig = serde_json::from_value(serde_json::json!({
            "id": "b",
            "orbit": { "type": "circular", "altitude": 600.0 },
            "attitude": { "inertia_diag": [10.0, 10.0, 10.0], "mass": 50.0 },
            "controller": { "type": "wasm", "path": "/tmp/ctrl.fifo" }
        }))
        .expect("a valid satellite");

        let (tx, _rx) = broadcast::channel(16);
        let (respond, mut answer) = oneshot::channel();
        let mut paused = false;
        let flow = handle_command(
            &mut engine,
            &mut paused,
            &tx,
            SimCommand::AddSatellite {
                satellite: Box::new(satellite),
                respond,
            },
            None,
        );
        assert!(
            flow.is_continue(),
            "a refused add keeps the simulation running"
        );
        let err = answer
            .try_recv()
            .expect("answered at once")
            .expect_err("a path from a client is refused");
        assert!(err.contains("not accepted over WebSocket"), "got: {err}");
        assert!(err.starts_with("add_satellite.controller"), "got: {err}");
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
