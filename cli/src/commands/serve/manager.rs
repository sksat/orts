use std::ops::ControlFlow;
use std::sync::Arc;

use orts::OrbitalState;
use orts::group::prop_group::SatId;
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital_system::OrbitalSystem;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::cli::IntegratorChoice;
use crate::config::{SimConfig, SatelliteConfig};
use crate::satellite::{SatelliteInfo, SatelliteSpec};
use crate::sim::core::{accel_breakdown, make_history_state, sat_params};
use orts::setup::build_orbital_system;
use crate::sim::params::SimParams;

use super::history::HistoryBuffer;
use super::protocol::WsMessage;
use super::compute::state_message;

/// Command sent from connection handlers to the simulation manager.
pub(super) enum SimCommand {
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

/// Per-satellite metadata for serve mode.
struct SatMeta {
    spec: SatelliteSpec,
    orbit_end_t: f64,
    next_save_t: f64,
}

/// Why the simulation loop exited.
enum LoopExit {
    /// Terminated by client request; server should return to idle.
    Terminated,
    /// Command channel disconnected (all clients gone).
    Disconnected,
}

/// Simulation manager that starts with a pre-built SimParams (legacy CLI args path).
pub(super) async fn simulation_manager_with_params(
    params: Arc<SimParams>,
    cmd_rx: mpsc::Receiver<SimCommand>,
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
pub(super) async fn simulation_manager(
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

/// Mutable state for the running simulation loop.
struct SimLoopContext {
    params: Arc<SimParams>,
    group: IndependentGroup<OrbitalSystem>,
    metas: Vec<SatMeta>,
    history: HistoryBuffer,
    tx: broadcast::Sender<String>,
    info_json: String,
    terminated_events: Vec<String>,
    paused: bool,
    current_t: f64,
    has_perturbations: bool,
}

impl SimLoopContext {
    fn new(
        params: Arc<SimParams>,
        tx: broadcast::Sender<String>,
        mut history: HistoryBuffer,
    ) -> Self {
        let config = match params.integrator {
            IntegratorChoice::Rk4 => IntegratorConfig::Rk4 { dt: params.dt },
            IntegratorChoice::Dp45 => IntegratorConfig::Dp45 {
                dt: params.dt,
                tolerances: params.tolerances.clone(),
            },
        };

        let body_radius = params.body.properties().radius;
        let atmosphere_altitude = params.body.properties().atmosphere_altitude;
        let event_checker = move |_t: f64, state: &OrbitalState| -> ControlFlow<String> {
            let r = state.position().magnitude();
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

        // Emit initial states
        for (i, (entry, dyn_sys)) in group.satellites_with_dynamics().enumerate() {
            let accels = accel_breakdown(dyn_sys, 0.0, &entry.state);
            let hs = make_history_state(
                metas[i].spec.id.as_str(),
                0.0,
                entry.state.position(),
                entry.state.velocity(),
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

        SimLoopContext {
            params,
            group,
            metas,
            history,
            tx,
            info_json,
            terminated_events: Vec::new(),
            paused: false,
            current_t: 0.0,
            has_perturbations,
        }
    }

    /// Handle a single command from the connection handler.
    /// Returns `ControlFlow::Break(())` if the simulation should terminate.
    fn handle_command(&mut self, cmd: SimCommand) -> ControlFlow<()> {
        match cmd {
            SimCommand::GetStatus { respond } => {
                let all_states = self.history.load_all();
                let response = if self.paused {
                    SimStatusResponse::Paused {
                        info_json: self.info_json.clone(),
                        terminated_events: self.terminated_events.clone(),
                        history_states: all_states,
                    }
                } else {
                    SimStatusResponse::Running {
                        info_json: self.info_json.clone(),
                        terminated_events: self.terminated_events.clone(),
                        history_states: all_states,
                    }
                };
                let _ = respond.send(response);
            }
            SimCommand::Start { respond, .. } => {
                let _ = respond.send(Err("Simulation is already running".to_string()));
            }
            SimCommand::Pause { respond } => {
                if self.paused {
                    let _ = respond.send(Err("Simulation is already paused".to_string()));
                } else {
                    self.paused = true;
                    eprintln!("Simulation paused at t={:.2}s", self.current_t);
                    let status = serde_json::to_string(&WsMessage::Status {
                        state: "paused".to_string(),
                    }).expect("failed to serialize status");
                    let _ = self.tx.send(status);
                    let _ = respond.send(Ok(()));
                }
            }
            SimCommand::Resume { respond } => {
                if !self.paused {
                    let _ = respond.send(Err("Simulation is not paused".to_string()));
                } else {
                    self.paused = false;
                    eprintln!("Simulation resumed at t={:.2}s", self.current_t);
                    let status = serde_json::to_string(&WsMessage::Status {
                        state: "running".to_string(),
                    }).expect("failed to serialize status");
                    let _ = self.tx.send(status);
                    let _ = respond.send(Ok(()));
                }
            }
            SimCommand::Terminate { respond } => {
                eprintln!("Simulation terminated at t={:.2}s", self.current_t);
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "idle".to_string(),
                }).expect("failed to serialize status");
                let _ = self.tx.send(status);
                let _ = respond.send(Ok(()));
                return ControlFlow::Break(());
            }
            SimCommand::AddSatellite { satellite, respond } => {
                let sat_index = self.metas.len();
                let spec = satellite.to_satellite_spec(
                    sat_index,
                    self.params.body,
                    self.params.mu,
                );
                let system = build_orbital_system(
                    &self.params.body,
                    self.params.mu,
                    self.params.epoch,
                    &sat_params(&spec),
                    self.params.build_atmosphere_model(),
                );
                let initial = spec.initial_state(self.params.mu);
                self.group.push_satellite_at(
                    spec.id.as_str(),
                    initial.clone(),
                    self.current_t,
                    system,
                );

                let sat_info = SatelliteInfo {
                    id: spec.id.clone(),
                    name: spec.name.clone(),
                    altitude: spec.altitude(&self.params.body),
                    period: spec.period,
                    perturbations: vec![],
                };
                let t = self.current_t;

                self.metas.push(SatMeta {
                    spec,
                    orbit_end_t: self.current_t + self.metas.last().map_or(5554.0, |m| m.spec.period),
                    next_save_t: self.current_t + self.params.output_interval,
                });

                let hs = make_history_state(
                    &sat_info.id,
                    self.current_t,
                    initial.position(),
                    initial.velocity(),
                    self.params.mu,
                    std::collections::HashMap::new(),
                );
                self.history.push(hs);
                let msg = state_message(
                    &sat_info.id,
                    self.current_t,
                    &initial,
                    self.params.mu,
                    std::collections::HashMap::new(),
                );
                let _ = self.tx.send(msg);

                let added_msg = serde_json::to_string(&WsMessage::SatelliteAdded {
                    satellite: sat_info.clone(),
                    t,
                })
                .expect("failed to serialize satellite_added");
                let _ = self.tx.send(added_msg);

                let _ = respond.send(Ok((sat_info, t)));
            }
            SimCommand::QueryRange {
                t_min,
                t_max,
                max_points,
                satellite_id,
                respond,
            } => {
                let mut states = self.history.query_range(t_min, t_max, max_points);
                if let Some(ref sid) = satellite_id {
                    states.retain(|s| s.satellite_id == *sid);
                }
                let _ = respond.send(states);
            }
        }
        ControlFlow::Continue(())
    }

    /// Propagate one chunk of simulation time, collecting outputs.
    fn propagate_chunk(
        &mut self,
        outputs_per_chunk: usize,
    ) -> Vec<crate::sim::core::HistoryState> {
        let mut all_outputs = Vec::new();

        for _ in 0..outputs_per_chunk {
            let target_t = self.current_t + self.params.stream_interval;

            // Orbit boundary reset (only for unperturbed 2-body)
            if !self.has_perturbations {
                let resets: Vec<(SatId, OrbitalState)> = self.group
                    .satellites_with_dynamics()
                    .enumerate()
                    .filter_map(|(i, (entry, _))| {
                        if !entry.terminated && self.current_t >= self.metas[i].orbit_end_t - 1e-9 {
                            Some((entry.id.clone(), self.metas[i].spec.initial_state(self.params.mu)))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (id, new_state) in &resets {
                    self.group.reset_state(id, new_state.clone());
                    if let Some(i) = self.metas.iter().position(|m| {
                        m.spec.id.as_str() == AsRef::<str>::as_ref(id)
                    }) {
                        self.metas[i].orbit_end_t = self.current_t + self.metas[i].spec.period;
                    }
                }
            }

            let outcome = self.group.propagate_to(target_t).unwrap();

            for (i, (entry, dyn_sys)) in self.group.satellites_with_dynamics().enumerate() {
                if entry.terminated || entry.t < target_t - 1e-9 {
                    continue;
                }

                let accels = accel_breakdown(dyn_sys, entry.t, &entry.state);
                let hs = make_history_state(
                    self.metas[i].spec.id.as_str(),
                    entry.t,
                    entry.state.position(),
                    entry.state.velocity(),
                    self.params.mu,
                    accels,
                );

                if hs.t >= self.metas[i].next_save_t - 1e-9 {
                    self.history.push(hs.clone());
                    self.metas[i].next_save_t += self.params.output_interval;
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
                let _ = self.tx.send(msg.clone());
                self.terminated_events.push(msg);
            }

            self.current_t = target_t;
        }

        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        all_outputs
    }
}

/// Core simulation loop: builds group, propagates, handles commands.
/// Returns the exit reason and gives back the command receiver for reuse.
async fn run_simulation_loop(
    params: Arc<SimParams>,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    history: HistoryBuffer,
) -> (LoopExit, mpsc::Receiver<SimCommand>) {
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;
    let wall_per_sim_sec = ((params.dt / 100.0).max(0.01)) / params.stream_interval;
    let chunk_wall_time =
        std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    let mut ctx = SimLoopContext::new(params, tx, history);

    loop {
        let chunk_start = tokio::time::Instant::now();

        // Process any pending commands between chunks
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if ctx.handle_command(cmd).is_break() {
                        return (LoopExit::Terminated, cmd_rx);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => return (LoopExit::Disconnected, cmd_rx),
            }
        }

        // Skip propagation while paused
        if ctx.paused {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }

        let all_outputs = ctx.propagate_chunk(OUTPUTS_PER_CHUNK);

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
                let _ = ctx.tx.send(msg);

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
