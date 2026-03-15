pub mod protocol;
pub mod compute;

use std::ops::ControlFlow;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use orts_integrator::State;
use orts::group::prop_group::SatId;
use orts::group::{IndependentGroup, IntegratorConfig};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::cli::{IntegratorChoice, SimArgs};
use crate::config::{SimConfig, SatelliteConfig};
use crate::satellite::{SatelliteInfo, SatelliteSpec};
use crate::sim::core::{accel_breakdown, make_history_state, sat_params};
use orts::setup::build_orbital_system;
use crate::sim::params::SimParams;

use protocol::{ClientMessage, HistoryBuffer, WsMessage};
use compute::state_message;

/// Command sent from connection handlers to the simulation manager.
enum SimCommand {
    /// Start a simulation from idle state.
    Start {
        config: SimConfig,
        respond: oneshot::Sender<Result<(), String>>,
    },
    /// Add a satellite to a running simulation.
    AddSatellite {
        satellite: SatelliteConfig,
        respond: oneshot::Sender<Result<(SatelliteInfo, f64), String>>,
    },
    /// Query the current simulation status.
    GetStatus {
        respond: oneshot::Sender<SimStatusResponse>,
    },
    /// Query a time range from history.
    QueryRange {
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        satellite_id: Option<String>,
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

enum SimStatusResponse {
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

pub fn run_server(sim: &SimArgs, port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(sim, port));
}

/// Detect whether CLI args specify an explicit simulation configuration.
fn has_explicit_sim_args(sim: &SimArgs) -> bool {
    sim.config.is_some()
        || !sim.sats.is_empty()
        || sim.tle.is_some()
        || sim.tle_line1.is_some()
        || sim.norad_id.is_some()
        // --altitude with non-default value
        || (sim.altitude - 400.0).abs() > 1e-9
}

async fn async_server(sim: &SimArgs, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    let actual_port = listener.local_addr().unwrap().port();
    eprintln!("WebSocket server listening on ws://localhost:{actual_port}");

    let (tx, _rx) = broadcast::channel::<String>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<SimCommand>(16);

    // Determine initial config: if CLI args specify simulation, auto-start.
    let initial_config = if has_explicit_sim_args(sim) {
        sim.config.as_ref().map(|config_path| {
            SimConfig::load(std::path::Path::new(config_path))
                .unwrap_or_else(|e| panic!("Error: {e}"))
        })
    } else {
        None
    };

    // Spawn simulation manager
    let mgr_tx = tx.clone();
    if has_explicit_sim_args(sim) && initial_config.is_none() {
        // Legacy path: build SimParams from CLI args directly
        let params = Arc::new(SimParams::from_sim_args(sim, true));
        tokio::spawn(simulation_manager_with_params(params, cmd_rx, mgr_tx));
    } else {
        tokio::spawn(simulation_manager(initial_config, cmd_rx, mgr_tx));
    }

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        eprintln!("New connection from {peer}");
        let rx = tx.subscribe();
        let client_cmd_tx = cmd_tx.clone();

        tokio::spawn(async move {
            handle_connection(stream, rx, client_cmd_tx).await;
        });
    }
}

/// Per-satellite metadata for serve mode.
struct SatMeta {
    spec: SatelliteSpec,
    orbit_end_t: f64,
    next_save_t: f64,
}

/// Simulation manager that starts with a pre-built SimParams (legacy CLI args path).
async fn simulation_manager_with_params(
    params: Arc<SimParams>,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
) {
    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let history = HistoryBuffer::new(5000, data_dir, params.mu);
    match run_simulation_loop(params, cmd_rx, tx.clone(), history).await {
        (LoopExit::Terminated, mut returned_rx) => {
            // Legacy path: after terminate, go idle and allow restart.
            eprintln!("Simulation manager: idle, waiting for start_simulation...");
            if let Some(config) = idle_loop(&mut returned_rx).await {
                // Delegate to the standard manager for subsequent runs.
                simulation_manager(Some(config), returned_rx, tx).await;
            }
        }
        (LoopExit::Disconnected, _) => {}
    }
}

/// Drain the cmd_rx, handling only GetStatus (as idle) and rejecting others,
/// until a Start command arrives or the channel disconnects.
async fn idle_loop(
    cmd_rx: &mut mpsc::Receiver<SimCommand>,
) -> Option<SimConfig> {
    loop {
        let Some(cmd) = cmd_rx.recv().await else {
            return None; // All senders dropped
        };
        match cmd {
            SimCommand::GetStatus { respond } => {
                let _ = respond.send(SimStatusResponse::Idle);
            }
            SimCommand::Start { config, respond } => {
                let _ = respond.send(Ok(()));
                return Some(config);
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
async fn simulation_manager(
    initial_config: Option<SimConfig>,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
) {
    // Determine the first config to start with.
    let mut next_config = if let Some(config) = initial_config {
        Some(config)
    } else {
        eprintln!("Simulation manager: idle, waiting for start_simulation...");
        idle_loop(&mut cmd_rx).await
    };

    // Main manager loop: start simulation, run until terminated, return to idle.
    while let Some(config) = next_config {
        let params = Arc::new(SimParams::from_config(&config));
        let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
        let history = HistoryBuffer::new(5000, data_dir, params.mu);
        eprintln!("Simulation manager: starting simulation...");
        match run_simulation_loop(params, cmd_rx, tx.clone(), history).await {
            (LoopExit::Terminated, returned_rx) => {
                cmd_rx = returned_rx;
                eprintln!("Simulation manager: idle, waiting for start_simulation...");
                next_config = idle_loop(&mut cmd_rx).await;
            }
            (LoopExit::Disconnected, _) => return,
        }
    }
}

/// Build the Info WsMessage from SimParams.
fn build_info_message(params: &SimParams) -> WsMessage {
    let satellites_info: Vec<SatelliteInfo> = params
        .satellites
        .iter()
        .map(|s| {
            let system = build_orbital_system(
                &params.body,
                params.mu,
                params.epoch,
                &sat_params(s),
                params.build_atmosphere_model(),
            );
            SatelliteInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                altitude: s.altitude(&params.body),
                period: s.period,
                perturbations: system
                    .perturbation_names()
                    .into_iter()
                    .map(String::from)
                    .collect(),
            }
        })
        .collect();
    WsMessage::Info {
        mu: params.mu,
        dt: params.dt,
        output_interval: params.output_interval,
        stream_interval: params.stream_interval,
        central_body: serde_json::to_value(params.body)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string(),
        central_body_radius: params.body.properties().radius,
        epoch_jd: params.epoch.map(|e| e.jd()),
        satellites: satellites_info,
    }
}

/// Why the simulation loop exited.
enum LoopExit {
    /// Terminated by client request; server should return to idle.
    Terminated,
    /// Command channel disconnected (all clients gone).
    Disconnected,
}

/// Core simulation loop: builds group, propagates, handles commands.
/// Returns the exit reason and gives back the command receiver for reuse.
async fn run_simulation_loop(
    params: Arc<SimParams>,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    mut history: HistoryBuffer,
) -> (LoopExit, mpsc::Receiver<SimCommand>) {
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;

    let wall_per_sim_sec = ((params.dt / 100.0).max(0.01)) / params.stream_interval;
    let chunk_wall_time =
        std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    let config = match params.integrator {
        IntegratorChoice::Rk4 => IntegratorConfig::Rk4 { dt: params.dt },
        IntegratorChoice::Dp45 => IntegratorConfig::Dp45 {
            dt: params.dt,
            tolerances: params.tolerances.clone(),
        },
    };

    let body_radius = params.body.properties().radius;
    let atmosphere_altitude = params.body.properties().atmosphere_altitude;
    let event_checker = move |_t: f64, state: &State| -> ControlFlow<String> {
        let r = state.position.magnitude();
        if r < body_radius {
            ControlFlow::Break(format!("collision at {:.1} km altitude", r - body_radius))
        } else if let Some(atm_alt) = atmosphere_altitude {
            if r < body_radius + atm_alt {
                ControlFlow::Break(format!(
                    "atmospheric entry at {:.1} km altitude",
                    r - body_radius
                ))
            } else {
                ControlFlow::Continue(())
            }
        } else {
            ControlFlow::Continue(())
        }
    };

    let mut group = IndependentGroup::new(config).with_event_checker(event_checker);

    let mut metas: Vec<SatMeta> = Vec::new();
    for spec in &params.satellites {
        let system = build_orbital_system(
            &params.body,
            params.mu,
            params.epoch,
            &sat_params(spec),
            params.build_atmosphere_model(),
        );
        let initial = spec.initial_state(params.mu);
        group = group.add_satellite(spec.id.as_str(), initial, system);
        metas.push(SatMeta {
            spec: spec.clone(),
            orbit_end_t: spec.period,
            next_save_t: params.output_interval,
        });
    }

    let has_perturbations = params.body.properties().j2.is_some();

    // Build and broadcast Info message
    let info_msg = build_info_message(&params);
    let info_json = serde_json::to_string(&info_msg).expect("failed to serialize info");
    let _ = tx.send(info_json.clone());

    // Track terminated events for late-connecting clients
    let mut terminated_events: Vec<String> = Vec::new();

    // Emit initial states
    for (i, (entry, dyn_sys)) in group.satellites_with_dynamics().enumerate() {
        let accels = accel_breakdown(dyn_sys, 0.0, &entry.state);
        let hs = make_history_state(
            metas[i].spec.id.as_str(),
            0.0,
            &entry.state.position,
            &entry.state.velocity,
            params.mu,
            accels.clone(),
        );
        history.push(hs);
        let msg = state_message(
            metas[i].spec.id.as_str(),
            0.0,
            &entry.state,
            params.mu,
            accels,
        );
        let _ = tx.send(msg);
    }

    let mut current_t = 0.0_f64;
    let mut paused = false;

    loop {
        let chunk_start = tokio::time::Instant::now();
        let mut all_outputs: Vec<crate::sim::core::HistoryState> = Vec::new();

        // Process any pending commands between chunks
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => match cmd {
                    SimCommand::GetStatus { respond } => {
                        let all_states = history.load_all();
                        let response = if paused {
                            SimStatusResponse::Paused {
                                info_json: info_json.clone(),
                                terminated_events: terminated_events.clone(),
                                history_states: all_states,
                            }
                        } else {
                            SimStatusResponse::Running {
                                info_json: info_json.clone(),
                                terminated_events: terminated_events.clone(),
                                history_states: all_states,
                            }
                        };
                        let _ = respond.send(response);
                    }
                    SimCommand::Start { respond, .. } => {
                        let _ = respond.send(Err("Simulation is already running".to_string()));
                    }
                    SimCommand::Pause { respond } => {
                        if paused {
                            let _ = respond.send(Err("Simulation is already paused".to_string()));
                        } else {
                            paused = true;
                            eprintln!("Simulation paused at t={current_t:.2}s");
                            let status = serde_json::to_string(&WsMessage::Status {
                                state: "paused".to_string(),
                            }).expect("failed to serialize status");
                            let _ = tx.send(status);
                            let _ = respond.send(Ok(()));
                        }
                    }
                    SimCommand::Resume { respond } => {
                        if !paused {
                            let _ = respond.send(Err("Simulation is not paused".to_string()));
                        } else {
                            paused = false;
                            eprintln!("Simulation resumed at t={current_t:.2}s");
                            let status = serde_json::to_string(&WsMessage::Status {
                                state: "running".to_string(),
                            }).expect("failed to serialize status");
                            let _ = tx.send(status);
                            let _ = respond.send(Ok(()));
                        }
                    }
                    SimCommand::Terminate { respond } => {
                        eprintln!("Simulation terminated at t={current_t:.2}s");
                        let status = serde_json::to_string(&WsMessage::Status {
                            state: "idle".to_string(),
                        }).expect("failed to serialize status");
                        let _ = tx.send(status);
                        let _ = respond.send(Ok(()));
                        return (LoopExit::Terminated, cmd_rx);
                    }
                    SimCommand::AddSatellite {
                        satellite,
                        respond,
                    } => {
                        let sat_index = metas.len();
                        let spec = satellite.to_satellite_spec(
                            sat_index,
                            params.body,
                            params.mu,
                        );
                        let system = build_orbital_system(
                            &params.body,
                            params.mu,
                            params.epoch,
                            &sat_params(&spec),
                            params.build_atmosphere_model(),
                        );
                        let initial = spec.initial_state(params.mu);
                        group.push_satellite_at(
                            spec.id.as_str(),
                            initial.clone(),
                            current_t,
                            system,
                        );

                        let sat_info = SatelliteInfo {
                            id: spec.id.clone(),
                            name: spec.name.clone(),
                            altitude: spec.altitude(&params.body),
                            period: spec.period,
                            perturbations: vec![], // simplified
                        };
                        let t = current_t;

                        metas.push(SatMeta {
                            spec,
                            orbit_end_t: current_t + metas.last().map_or(5554.0, |m| m.spec.period),
                            next_save_t: current_t + params.output_interval,
                        });

                        // Emit initial state for new satellite
                        let hs = make_history_state(
                            &sat_info.id,
                            current_t,
                            &initial.position,
                            &initial.velocity,
                            params.mu,
                            std::collections::HashMap::new(),
                        );
                        history.push(hs);
                        let msg = state_message(
                            &sat_info.id,
                            current_t,
                            &initial,
                            params.mu,
                            std::collections::HashMap::new(),
                        );
                        let _ = tx.send(msg);

                        // Broadcast satellite_added to all clients
                        let added_msg = serde_json::to_string(&WsMessage::SatelliteAdded {
                            satellite: sat_info.clone(),
                            t,
                        })
                        .expect("failed to serialize satellite_added");
                        let _ = tx.send(added_msg);

                        let _ = respond.send(Ok((sat_info, t)));
                    }
                    SimCommand::QueryRange {
                        t_min,
                        t_max,
                        max_points,
                        satellite_id,
                        respond,
                    } => {
                        let mut states = history.query_range(t_min, t_max, max_points);
                        if let Some(ref sid) = satellite_id {
                            states.retain(|s| s.satellite_id == *sid);
                        }
                        let _ = respond.send(states);
                    }
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => return (LoopExit::Disconnected, cmd_rx),
            }
        }

        // Skip propagation while paused
        if paused {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }

        for _ in 0..OUTPUTS_PER_CHUNK {
            let target_t = current_t + params.stream_interval;

            // Orbit boundary reset (only for unperturbed 2-body)
            if !has_perturbations {
                let resets: Vec<(SatId, State)> = group
                    .satellites_with_dynamics()
                    .enumerate()
                    .filter_map(|(i, (entry, _))| {
                        if !entry.terminated && current_t >= metas[i].orbit_end_t - 1e-9 {
                            Some((entry.id.clone(), metas[i].spec.initial_state(params.mu)))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (id, new_state) in &resets {
                    group.reset_state(id, new_state.clone());
                    if let Some(i) = metas.iter().position(|m| {
                        m.spec.id.as_str() == AsRef::<str>::as_ref(id)
                    }) {
                        metas[i].orbit_end_t = current_t + metas[i].spec.period;
                    }
                }
            }

            let outcome = group.propagate_to(target_t).unwrap();

            for (i, (entry, dyn_sys)) in group.satellites_with_dynamics().enumerate() {
                if entry.terminated || entry.t < target_t - 1e-9 {
                    continue;
                }

                let accels = accel_breakdown(dyn_sys, entry.t, &entry.state);
                let hs = make_history_state(
                    metas[i].spec.id.as_str(),
                    entry.t,
                    &entry.state.position,
                    &entry.state.velocity,
                    params.mu,
                    accels,
                );

                if hs.t >= metas[i].next_save_t - 1e-9 {
                    history.push(hs.clone());
                    metas[i].next_save_t += params.output_interval;
                }

                all_outputs.push(hs);
            }

            for term in &outcome.terminations {
                eprintln!(
                    "Simulation terminated for {} at t={:.2}s: {}",
                    term.satellite_id, term.t, term.reason
                );
                let sid_str: &str = term.satellite_id.as_ref();
                let msg = serde_json::to_string(&WsMessage::SimulationTerminated {
                    satellite_id: sid_str.to_string(),
                    t: term.t,
                    reason: term.reason.clone(),
                })
                .expect("failed to serialize termination message");
                let _ = tx.send(msg.clone());
                terminated_events.push(msg);
            }

            current_t = target_t;
        }

        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        if !all_outputs.is_empty() {
            let send_interval = chunk_wall_time / all_outputs.len() as u32;
            for out in &all_outputs {
                let send_start = tokio::time::Instant::now();
                let msg = serde_json::to_string(&WsMessage::State {
                    satellite_id: out.satellite_id.clone(),
                    t: out.t,
                    position: out.position,
                    velocity: out.velocity,
                    semi_major_axis: out.semi_major_axis,
                    eccentricity: out.eccentricity,
                    inclination: out.inclination,
                    raan: out.raan,
                    argument_of_periapsis: out.argument_of_periapsis,
                    true_anomaly: out.true_anomaly,
                    accelerations: out.accelerations.clone(),
                })
                .expect("failed to serialize state");
                let _ = tx.send(msg);

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

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // 1. Query current status from the manager
    let (status_tx, status_rx) = oneshot::channel();
    if cmd_tx
        .send(SimCommand::GetStatus {
            respond: status_tx,
        })
        .await
        .is_err()
    {
        return;
    }
    let status = match status_rx.await {
        Ok(s) => s,
        Err(_) => return,
    };

    let is_paused = matches!(status, SimStatusResponse::Paused { .. });

    match status {
        SimStatusResponse::Idle => {
            let idle_msg = serde_json::to_string(&WsMessage::Status {
                state: "idle".to_string(),
            })
            .expect("failed to serialize status");
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(idle_msg.into()))
                .await
                .is_err()
            {
                return;
            }
        }
        SimStatusResponse::Running {
            info_json,
            terminated_events,
            history_states,
        }
        | SimStatusResponse::Paused {
            info_json,
            terminated_events,
            history_states,
        } => {
            // Send info
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    info_json.into(),
                ))
                .await
                .is_err()
            {
                return;
            }

            // If paused, send status so the client knows immediately
            if is_paused {
                let paused_msg = serde_json::to_string(&WsMessage::Status {
                    state: "paused".to_string(),
                })
                .expect("failed to serialize status");
                if ws_sender
                    .send(tokio_tungstenite::tungstenite::Message::Text(paused_msg.into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // Replay terminated events
            for event_json in &terminated_events {
                if ws_sender
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        event_json.clone().into(),
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // Send overview history
            let overview = HistoryBuffer::downsample(&history_states, 1000);
            let history_msg = WsMessage::History { states: overview };
            let history_json =
                serde_json::to_string(&history_msg).expect("failed to serialize history");
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    history_json.into(),
                ))
                .await
                .is_err()
            {
                return;
            }

            // Send full detail in background
            let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<String>(16);
            tokio::spawn(async move {
                let chunk_size = 1000;
                for chunk in history_states.chunks(chunk_size) {
                    let msg = WsMessage::HistoryDetail {
                        states: chunk.to_vec(),
                    };
                    let json =
                        serde_json::to_string(&msg).expect("failed to serialize detail chunk");
                    if detail_tx.send(json).await.is_err() {
                        return;
                    }
                }
                let complete = serde_json::to_string(&WsMessage::HistoryDetailComplete)
                    .expect("failed to serialize detail complete");
                let _ = detail_tx.send(complete).await;
            });

            // Drain detail messages before entering main loop
            // (they'll be interleaved with broadcast in the select below)
            // Actually, let's handle them in the main loop via detail_rx.
            // We need to pass detail_rx into the main loop.
            main_loop(&mut ws_sender, &mut ws_receiver, &mut rx, &cmd_tx, Some(&mut detail_rx))
                .await;
            eprintln!("Client disconnected");
            return;
        }
    }

    // Idle client: main loop (waiting for start_simulation or other messages)
    main_loop(&mut ws_sender, &mut ws_receiver, &mut rx, &cmd_tx, None).await;
    eprintln!("Client disconnected");
}

async fn main_loop(
    ws_sender: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        tokio_tungstenite::tungstenite::Message,
    >,
    ws_receiver: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    >,
    rx: &mut broadcast::Receiver<String>,
    cmd_tx: &mpsc::Sender<SimCommand>,
    mut detail_rx: Option<&mut tokio::sync::mpsc::Receiver<String>>,
) {
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_sender
                            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Client lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            detail = async {
                if let Some(ref mut drx) = detail_rx {
                    drx.recv().await
                } else {
                    std::future::pending::<Option<String>>().await
                }
            } => {
                if let Some(json) = detail {
                    if ws_sender
                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                } else {
                    // Detail sender finished
                    detail_rx = None;
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, satellite_id } => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::QueryRange {
                                        t_min, t_max, max_points, satellite_id, respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    if let Ok(states) = resp_rx.await {
                                        let resp = WsMessage::QueryRangeResponse { t_min, t_max, states };
                                        let json = serde_json::to_string(&resp)
                                            .expect("failed to serialize query_range_response");
                                        if ws_sender
                                            .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                                ClientMessage::StartSimulation { config } => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::Start {
                                        config, respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    match resp_rx.await {
                                        Ok(Ok(())) => {
                                            // Manager will broadcast Info via the broadcast channel.
                                            // Nothing to send here — the client will receive it via rx.
                                        }
                                        Ok(Err(e)) => {
                                            let err_msg = serde_json::to_string(&WsMessage::Error {
                                                message: e,
                                            }).expect("failed to serialize error");
                                            if ws_sender
                                                .send(tokio_tungstenite::tungstenite::Message::Text(err_msg.into()))
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Err(_) => break,
                                    }
                                }
                                ClientMessage::PauseSimulation => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::Pause {
                                        respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    match resp_rx.await {
                                        Ok(Err(e)) => {
                                            let err_msg = serde_json::to_string(&WsMessage::Error {
                                                message: e,
                                            }).expect("failed to serialize error");
                                            if ws_sender
                                                .send(tokio_tungstenite::tungstenite::Message::Text(err_msg.into()))
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Ok(Ok(())) => {
                                            // Status broadcast via tx
                                        }
                                        Err(_) => break,
                                    }
                                }
                                ClientMessage::ResumeSimulation => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::Resume {
                                        respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    match resp_rx.await {
                                        Ok(Err(e)) => {
                                            let err_msg = serde_json::to_string(&WsMessage::Error {
                                                message: e,
                                            }).expect("failed to serialize error");
                                            if ws_sender
                                                .send(tokio_tungstenite::tungstenite::Message::Text(err_msg.into()))
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Ok(Ok(())) => {
                                            // Status broadcast via tx
                                        }
                                        Err(_) => break,
                                    }
                                }
                                ClientMessage::TerminateSimulation => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::Terminate {
                                        respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    match resp_rx.await {
                                        Ok(Err(e)) => {
                                            let err_msg = serde_json::to_string(&WsMessage::Error {
                                                message: e,
                                            }).expect("failed to serialize error");
                                            if ws_sender
                                                .send(tokio_tungstenite::tungstenite::Message::Text(err_msg.into()))
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Ok(Ok(())) => {
                                            // Status broadcast via tx
                                        }
                                        Err(_) => break,
                                    }
                                }
                                ClientMessage::AddSatellite { satellite } => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::AddSatellite {
                                        satellite, respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    match resp_rx.await {
                                        Ok(Err(e)) => {
                                            let err_msg = serde_json::to_string(&WsMessage::Error {
                                                message: e,
                                            }).expect("failed to serialize error");
                                            if ws_sender
                                                .send(tokio_tungstenite::tungstenite::Message::Text(err_msg.into()))
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Ok(Ok(_)) => {
                                            // SatelliteAdded already broadcast via tx
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        break;
                    }
                }
            }
        }
    }
}
