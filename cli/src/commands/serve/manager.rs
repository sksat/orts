use std::collections::{HashMap, VecDeque};
use std::ops::ControlFlow;
use std::sync::Arc;

use orts::OrbitalState;
use orts::attitude::CoupledGravityGradient;
use orts::group::prop_group::{PropGroupOutcome, SatId};
use orts::group::{IndependentGroup, IntegratorConfig};
use orts::orbital::OrbitalSystem;
use orts::orbital::gravity::GravityField;
use orts::spacecraft::{SpacecraftDynamics, SpacecraftState};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::cli::{IntegratorChoice, PluginBackendChoice, SimArgs};

/// CLI-time backend overrides that must apply to every `SimParams`
/// built inside the serve manager — including ones derived from
/// configs received from the client at runtime.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PluginBackendOverrides {
    pub choice: Option<PluginBackendChoice>,
    pub threshold: Option<usize>,
}

impl PluginBackendOverrides {
    pub fn from_sim_args(sim: &SimArgs) -> Self {
        Self {
            // Only override if the user explicitly asked for a
            // non-default backend on the CLI. If they left it at
            // `Auto` (the clap default) we still apply that, but the
            // threshold override is only applied when set.
            choice: Some(sim.plugin_backend),
            threshold: sim.plugin_backend_threshold,
        }
    }

    pub fn apply(&self, params: &mut SimParams) {
        if let Some(c) = self.choice {
            params.plugin_backend_choice = c;
        }
        if self.threshold.is_some() {
            params.plugin_backend_threshold = self.threshold;
        }
    }
}
use crate::config::{SatelliteConfig, SimConfig};
use crate::satellite::{SatelliteInfo, SatelliteSpec};
use crate::sim::core::{
    AttitudePayload, AttitudeSource, accel_breakdown, make_history_state, sat_params,
    spacecraft_accel_breakdown,
};
use crate::sim::params::SimParams;
use orts::setup::{build_orbital_system, build_spacecraft_dynamics, default_third_bodies};

use super::compute::state_message;
use super::history::HistoryBuffer;
use super::protocol::WsMessage;

use crate::sim::controlled::ControlledSatellite;

/// Simulation group that dynamically switches between orbit-only, spacecraft, and controlled modes.
enum SimGroup {
    OrbitOnly(IndependentGroup<OrbitalSystem>),
    Spacecraft(IndependentGroup<SpacecraftDynamics<Box<dyn GravityField>>>),
    /// Plugin-controlled satellites (direct integration, no IndependentGroup).
    Controlled(Vec<ControlledSatellite>),
}

/// Extracted state from a single satellite for protocol serialization.
struct SatSnapshot {
    orbit: OrbitalState,
    attitude: Option<AttitudePayload>,
    accels: HashMap<String, f64>,
}

impl SimGroup {
    fn propagate_to(&mut self, t: f64) -> Result<PropGroupOutcome, utsuroi::IntegrationError> {
        match self {
            SimGroup::OrbitOnly(g) => g.propagate_to(t),
            SimGroup::Spacecraft(g) => g.propagate_to(t),
            SimGroup::Controlled(_) => {
                // Controlled satellites are stepped via step_controlled_to(),
                // not through IndependentGroup::propagate_to.
                Ok(PropGroupOutcome {
                    terminations: vec![],
                })
            }
        }
    }

    /// Step controlled satellites up to target time `t` in dt_ctrl increments.
    fn step_controlled_to(&mut self, current_t: f64, target_t: f64, params: &SimParams) {
        let SimGroup::Controlled(sats) = self else {
            return;
        };
        for sat in sats.iter_mut() {
            let dt_ctrl = sat.controller.sample_period();
            let dt_ode = params.dt.min(dt_ctrl);
            let mut t = current_t;
            while t < target_t - 1e-12 {
                let dt = dt_ctrl.min(target_t - t);
                crate::sim::controlled::step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref())
                    .unwrap_or_else(|e| {
                        log::error!("controlled simulation error at t={t:.3}: {e}");
                    });
                t += dt;
            }
        }
    }

    /// Number of satellites.
    fn len(&self) -> usize {
        match self {
            SimGroup::OrbitOnly(g) => g.satellites().count(),
            SimGroup::Spacecraft(g) => g.satellites().count(),
            SimGroup::Controlled(sats) => sats.len(),
        }
    }

    /// Get satellite ID at index.
    fn sat_id(&self, idx: usize) -> SatId {
        match self {
            SimGroup::OrbitOnly(g) => g.satellites().nth(idx).unwrap().id.clone(),
            SimGroup::Spacecraft(g) => g.satellites().nth(idx).unwrap().id.clone(),
            SimGroup::Controlled(_) => SatId::from(self.controlled_meta_id(idx)),
        }
    }

    /// Helper: get the satellite ID string for controlled satellites.
    fn controlled_meta_id(&self, _idx: usize) -> &str {
        // Controlled satellites don't have SatId in the group; the ID is in
        // SatMeta which is outside SimGroup. Return a placeholder; the caller
        // (propagate_chunk) uses metas[i] for the real ID.
        "controlled"
    }

    /// Check if satellite at index is terminated.
    fn is_terminated(&self, idx: usize) -> bool {
        match self {
            SimGroup::OrbitOnly(g) => g.satellites().nth(idx).unwrap().terminated,
            SimGroup::Spacecraft(g) => g.satellites().nth(idx).unwrap().terminated,
            SimGroup::Controlled(_) => false, // controlled sats don't terminate via event checker
        }
    }

    /// Get the current time of satellite at index.
    fn sat_t(&self, idx: usize) -> f64 {
        match self {
            SimGroup::OrbitOnly(g) => g.satellites().nth(idx).unwrap().t,
            SimGroup::Spacecraft(g) => g.satellites().nth(idx).unwrap().t,
            // Controlled satellites don't track per-sat time in the group.
            // propagate_chunk uses target_t directly for these.
            SimGroup::Controlled(_) => f64::MAX,
        }
    }

    /// Extract snapshot (orbit + optional attitude + accel breakdown) for satellite at index.
    fn snapshot(&self, idx: usize, t: f64) -> SatSnapshot {
        match self {
            SimGroup::OrbitOnly(g) => {
                let (entry, dyn_sys) = g.satellites_with_dynamics().nth(idx).unwrap();
                SatSnapshot {
                    orbit: entry.state.clone(),
                    attitude: None,
                    accels: accel_breakdown(dyn_sys, t, &entry.state),
                }
            }
            SimGroup::Spacecraft(g) => {
                let (entry, dyn_sys) = g.satellites_with_dynamics().nth(idx).unwrap();
                let sc = &entry.state.plant;
                let q = sc.attitude.quaternion;
                let w = sc.attitude.angular_velocity;
                SatSnapshot {
                    orbit: sc.orbit.clone(),
                    attitude: Some(AttitudePayload {
                        quaternion_wxyz: [q[0], q[1], q[2], q[3]],
                        angular_velocity_body: [w[0], w[1], w[2]],
                        source: AttitudeSource::Propagated,
                        rw_momentum: None,
                    }),
                    accels: spacecraft_accel_breakdown(dyn_sys, t, sc),
                }
            }
            SimGroup::Controlled(sats) => {
                let sat = &sats[idx];
                let sc = &sat.state.plant;
                let q = sc.attitude.quaternion;
                let w = sc.attitude.angular_velocity;
                let rw_mom = if sat.has_rw && !sat.state.aux.is_empty() {
                    Some(sat.state.aux.clone())
                } else {
                    None
                };
                SatSnapshot {
                    orbit: sc.orbit.clone(),
                    attitude: Some(AttitudePayload {
                        quaternion_wxyz: [q[0], q[1], q[2], q[3]],
                        angular_velocity_body: [w[0], w[1], w[2]],
                        source: AttitudeSource::Propagated,
                        rw_momentum: rw_mom,
                    }),
                    accels: HashMap::new(),
                }
            }
        }
    }

    /// Reset state for orbit boundary (unperturbed 2-body only, OrbitOnly mode).
    ///
    /// In Spacecraft mode this is intentionally a no-op: attitude dynamics
    /// cannot be meaningfully reset at orbit boundaries, and the coupled
    /// integrator handles long-duration propagation correctly.
    fn reset_orbit_state(&mut self, id: &SatId, state: OrbitalState) {
        match self {
            SimGroup::OrbitOnly(g) => g.reset_state(id, state),
            SimGroup::Spacecraft(_) | SimGroup::Controlled(_) => {}
        }
    }

    /// Push a new satellite (orbit-only mode).
    fn push_orbit_satellite(
        &mut self,
        id: &str,
        state: OrbitalState,
        t: f64,
        system: OrbitalSystem,
    ) {
        match self {
            SimGroup::OrbitOnly(g) => g.push_satellite_at(id, state, t, system),
            SimGroup::Spacecraft(_) | SimGroup::Controlled(_) => {
                panic!("Cannot add orbit-only satellite to spacecraft/controlled simulation")
            }
        }
    }

    /// Push a new controlled satellite. Requires the group to be in
    /// `Controlled` mode.
    #[cfg(feature = "plugin-wasm")]
    fn push_controlled_satellite(&mut self, sat: ControlledSatellite) {
        match self {
            SimGroup::Controlled(sats) => sats.push(sat),
            SimGroup::OrbitOnly(_) | SimGroup::Spacecraft(_) => {
                panic!("Cannot add controlled satellite to orbit-only/spacecraft simulation")
            }
        }
    }
}

/// Maximum number of replay-able `simulation_terminated` events the server
/// retains for late-connecting clients. Without a cap, long-running sims with
/// many deorbiting satellites would grow this vector unbounded, and every new
/// client would pay the replay cost.
pub(super) const TERMINATED_EVENTS_CAP: usize = 1024;

/// Push a serialized `simulation_terminated` message into a ring-buffered
/// event queue, dropping the oldest entries once the cap is reached.
pub(super) fn push_terminated_capped(events: &mut VecDeque<String>, msg: String) {
    events.push_back(msg);
    while events.len() > TERMINATED_EVENTS_CAP {
        events.pop_front();
    }
}

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

/// Collect all body names in the simulation system (central + third bodies).
fn system_body_names(params: &SimParams) -> Vec<String> {
    body_names_for(&params.body)
}

/// Return the central body name plus all third-body names for the given central body.
fn body_names_for(body: &arika::body::KnownBody) -> Vec<String> {
    let mut names = vec![body.properties().name.to_lowercase()];
    for tb in &default_third_bodies(body) {
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
) {
    // Request texture downloads for all bodies in the system.
    let _ = texture_tx.send(system_body_names(&params)).await;

    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let body_radius = params.body.properties().radius;
    let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
    match run_simulation_loop(params, cmd_rx, tx.clone(), history).await {
        (LoopExit::Terminated, mut returned_rx) => {
            // Legacy path: after terminate, go idle and allow restart.
            eprintln!("Simulation manager: idle, waiting for start_simulation...");
            if let Some(config) = idle_loop(&mut returned_rx).await {
                // Delegate to the standard manager for subsequent runs.
                simulation_manager(
                    Some(config),
                    cli_plugin_overrides,
                    returned_rx,
                    tx,
                    texture_tx,
                )
                .await;
            }
        }
        (LoopExit::Disconnected, _) => {}
    }
}

/// Validate a single satellite's attitude configuration so that
/// `build_spacecraft_dynamics` cannot panic on a singular inertia
/// tensor or a non-positive mass. Used from both the startup config
/// validator and the runtime `AddSatellite` handler.
fn validate_satellite_spec(spec: &SatelliteSpec) -> Result<(), String> {
    let Some(att) = &spec.attitude_config else {
        return Ok(());
    };
    let inertia = att.inertia_matrix();
    if inertia.determinant().abs() < 1e-30 {
        return Err(format!(
            "Satellite '{}' has singular inertia tensor (not invertible)",
            spec.id
        ));
    }
    if att.mass <= 0.0 {
        return Err(format!(
            "Satellite '{}' has non-positive mass: {}",
            spec.id, att.mass
        ));
    }
    Ok(())
}

/// Validate a SimConfig before starting. Returns Err with a user-facing message
/// if the config is invalid (e.g., mixed attitude settings).
fn validate_sim_config(config: &SimConfig) -> Result<(), String> {
    // Field-level validation (non-zero direction, finite values, …) mirrors
    // what `SimConfig::load` runs for file-based configs, so WebSocket
    // `StartSimulation` cannot smuggle in thruster config that panics later
    // in `ThrusterSpec::new()`.
    config.validate()?;

    let body = crate::satellite::parse_body(&config.body);
    let mu = body.properties().mu;
    let specs: Vec<_> = config
        .satellites
        .iter()
        .enumerate()
        .map(|(i, s)| s.to_satellite_spec(i, body, mu))
        .collect();
    let any_att = specs.iter().any(|s| s.attitude_config.is_some());
    let all_att = !specs.is_empty() && specs.iter().all(|s| s.attitude_config.is_some());
    if any_att && !all_att {
        return Err(
            "Mixed attitude config: some satellites have attitude, some don't. \
             Specify attitude for all satellites or remove it from all."
                .to_string(),
        );
    }
    // Validate inertia tensors are invertible
    for spec in &specs {
        validate_satellite_spec(spec)?;
    }
    Ok(())
}

/// Drain the cmd_rx, handling only GetStatus (as idle) and rejecting others,
/// until a Start command arrives or the channel disconnects.
async fn idle_loop(cmd_rx: &mut mpsc::Receiver<SimCommand>) -> Option<SimConfig> {
    loop {
        let Some(cmd) = cmd_rx.recv().await else {
            return None; // All senders dropped
        };
        match cmd {
            SimCommand::GetStatus { respond, .. } => {
                let _ = respond.send(SimStatusResponse::Idle);
            }
            SimCommand::Start { config, respond } => {
                // Validate config before acknowledging
                if let Err(e) = validate_sim_config(&config) {
                    let _ = respond.send(Err(e));
                    continue;
                }
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
    cli_plugin_overrides: PluginBackendOverrides,
    mut cmd_rx: mpsc::Receiver<SimCommand>,
    tx: broadcast::Sender<String>,
    texture_tx: super::textures::TextureRequestSender,
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
        let mut params_inner = SimParams::from_config(&config);
        cli_plugin_overrides.apply(&mut params_inner);
        let params = Arc::new(params_inner);

        // Request texture downloads for all bodies in the system.
        let _ = texture_tx.send(system_body_names(&params)).await;

        let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
        let body_radius = params.body.properties().radius;
        let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
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
            let third_bodies = default_third_bodies(&params.body);
            let system = build_orbital_system(
                &params.body,
                params.mu,
                params.epoch,
                &sat_params(s),
                &third_bodies,
                params.build_atmosphere_model(),
            );
            SatelliteInfo {
                id: s.entity_path().to_string(),
                name: s.name.clone(),
                altitude: s.altitude(&params.body),
                period: s.period,
                perturbations: system.model_names().into_iter().map(String::from).collect(),
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
    group: SimGroup,
    metas: Vec<SatMeta>,
    history: HistoryBuffer,
    tx: broadcast::Sender<String>,
    info_json: String,
    /// Ring-buffered queue of `simulation_terminated` payloads, replayed to
    /// late-connecting clients. Bounded by [`TERMINATED_EVENTS_CAP`] to avoid
    /// unbounded growth in long-running sims with many deorbiting satellites.
    terminated_events: VecDeque<String>,
    paused: bool,
    current_t: f64,
    has_perturbations: bool,
    /// Shared WASM plugin cache, kept alive for the whole sim loop so
    /// dynamic `AddSatellite` commands can reuse the compiled guest
    /// components and (for the async backend) the shared runtime
    /// thread. `None` when the loop is running in non-controlled
    /// mode or when `plugin-wasm` is disabled.
    #[cfg(feature = "plugin-wasm")]
    wasm_cache: Option<orts::plugin::wasm::WasmPluginCache>,
    /// Resolved plugin backend (sync / async) for this loop. Locked
    /// in at `SimLoopContext::new` time so dynamic additions stay on
    /// the same backend as the initial fleet.
    #[cfg(feature = "plugin-wasm")]
    plugin_backend: Option<crate::sim::params::ResolvedPluginBackend>,
}

impl SimLoopContext {
    fn new(
        params: Arc<SimParams>,
        tx: broadcast::Sender<String>,
        mut history: HistoryBuffer,
    ) -> Result<Self, String> {
        let config = match params.integrator {
            IntegratorChoice::Rk4 => IntegratorConfig::Rk4 { dt: params.dt },
            IntegratorChoice::Dp45 => IntegratorConfig::Dp45 {
                dt: params.dt,
                tolerances: params.tolerances.clone(),
            },
            IntegratorChoice::Dop853 => IntegratorConfig::Dop853 {
                dt: params.dt,
                tolerances: params.tolerances.clone(),
            },
        };

        let body_radius = params.body.properties().radius;
        let atmosphere_altitude = params.body.properties().atmosphere_altitude;

        // Determine mode: use SpacecraftDynamics if all satellites have attitude config.
        // Empty satellite list → orbit-only (to support dynamic add_satellite).
        let any_attitude = params
            .satellites
            .iter()
            .any(|s| s.attitude_config.is_some());
        let all_attitude = !params.satellites.is_empty()
            && params
                .satellites
                .iter()
                .all(|s| s.attitude_config.is_some());
        if any_attitude && !all_attitude {
            return Err(
                "Mixed attitude config: some satellites have attitude, some don't. \
                 Specify attitude for all satellites or remove it from all."
                    .to_string(),
            );
        }
        let use_spacecraft = all_attitude;
        let has_controller = !params.satellites.is_empty()
            && params
                .satellites
                .iter()
                .all(|s| s.controller_config.is_some());

        let mut metas: Vec<SatMeta> = Vec::new();
        let third_bodies = default_third_bodies(&params.body);

        // Eagerly build the WASM plugin cache so the same instance
        // can serve both the initial fleet and any dynamic
        // `AddSatellite` commands received later. For non-controlled
        // runs the cache stays `None` and dynamic add will reject
        // satellites with a `controller` config.
        #[cfg(feature = "plugin-wasm")]
        let (mut wasm_cache, plugin_backend) = if has_controller {
            let cache = orts::plugin::wasm::WasmPluginCache::new()
                .map_err(|e| format!("WASM plugin cache init failed: {e}"))?;
            (Some(cache), Some(params.resolve_plugin_backend()))
        } else {
            (None, None)
        };

        let group = if has_controller {
            // Plugin-controlled mode: direct integration with step_controlled.
            let mut controlled_sats = Vec::new();
            {
                #[cfg(feature = "plugin-wasm")]
                let mut ctx = crate::sim::controlled::ControlledBuildContext {
                    params: &params,
                    wasm_cache: wasm_cache
                        .as_mut()
                        .expect("wasm_cache must be Some when has_controller"),
                    plugin_backend: plugin_backend
                        .expect("plugin_backend must be Some when has_controller"),
                };
                #[cfg(not(feature = "plugin-wasm"))]
                let mut ctx = crate::sim::controlled::ControlledBuildContext { params: &params };
                for spec in &params.satellites {
                    let sat = crate::sim::controlled::build_controlled_satellite(spec, &mut ctx)
                        .map_err(|e| format!("controlled satellite '{}': {e}", spec.id))?;
                    controlled_sats.push(sat);
                    metas.push(SatMeta {
                        spec: spec.clone(),
                        orbit_end_t: spec.period,
                        next_save_t: params.output_interval,
                    });
                }
            }

            SimGroup::Controlled(controlled_sats)
        } else if use_spacecraft {
            let sc_event_checker = move |_t: f64,
                                         state: &orts::effector::AugmentedState<
                SpacecraftState,
            >|
                  -> ControlFlow<String> {
                let r = state.plant.orbit.position().magnitude();
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
            let mut sc_group = IndependentGroup::new(config).with_event_checker(sc_event_checker);

            for spec in &params.satellites {
                let att = spec.attitude_config.as_ref().unwrap();
                let inertia = att.inertia_matrix();
                let mut dynamics = build_spacecraft_dynamics(
                    &params.body,
                    params.mu,
                    params.epoch,
                    &sat_params(spec),
                    &third_bodies,
                    inertia,
                    params.build_atmosphere_model(),
                );
                // Default torque: coupled gravity gradient
                dynamics = dynamics.with_model(CoupledGravityGradient::new(params.mu, inertia));

                let orbit = spec.initial_state(params.mu);
                let plant = SpacecraftState {
                    orbit,
                    attitude: orts::attitude::AttitudeState {
                        quaternion: nalgebra::Vector4::from_row_slice(&att.initial_quaternion),
                        angular_velocity: nalgebra::Vector3::from_row_slice(
                            &att.initial_angular_velocity,
                        ),
                    },
                    mass: att.mass,
                };
                let initial = dynamics.initial_augmented_state(plant);
                sc_group = sc_group.add_satellite(spec.id.as_str(), initial, dynamics);
                metas.push(SatMeta {
                    spec: spec.clone(),
                    orbit_end_t: spec.period,
                    next_save_t: params.output_interval,
                });
            }
            SimGroup::Spacecraft(sc_group)
        } else {
            let orbit_event_checker = move |_t: f64, state: &OrbitalState| -> ControlFlow<String> {
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
            let mut orbit_group =
                IndependentGroup::new(config).with_event_checker(orbit_event_checker);

            for spec in &params.satellites {
                let system = build_orbital_system(
                    &params.body,
                    params.mu,
                    params.epoch,
                    &sat_params(spec),
                    &third_bodies,
                    params.build_atmosphere_model(),
                );
                let initial = spec.initial_state(params.mu);
                orbit_group = orbit_group.add_satellite(spec.id.as_str(), initial, system);
                metas.push(SatMeta {
                    spec: spec.clone(),
                    orbit_end_t: spec.period,
                    next_save_t: params.output_interval,
                });
            }
            SimGroup::OrbitOnly(orbit_group)
        };

        let has_perturbations = params.body.properties().j2.is_some();

        // Build and broadcast Info message
        let info_msg = build_info_message(&params);
        let info_json = serde_json::to_string(&info_msg).expect("failed to serialize info");
        let _ = tx.send(info_json.clone());

        // Emit initial states
        let body_radius = params.body.properties().radius;
        #[allow(clippy::needless_range_loop)]
        for i in 0..group.len() {
            let snap = group.snapshot(i, 0.0);
            let hs = make_history_state(
                metas[i].spec.entity_path(),
                0.0,
                snap.orbit.position(),
                snap.orbit.velocity(),
                params.mu,
                body_radius,
                snap.accels.clone(),
                snap.attitude.clone(),
            );
            history.push(hs);
            let msg = state_message(
                metas[i].spec.entity_path(),
                0.0,
                &snap.orbit,
                params.mu,
                body_radius,
                snap.accels,
                snap.attitude,
            );
            let _ = tx.send(msg);
        }

        Ok(SimLoopContext {
            params,
            group,
            metas,
            history,
            tx,
            info_json,
            terminated_events: VecDeque::new(),
            paused: false,
            current_t: 0.0,
            has_perturbations,
            #[cfg(feature = "plugin-wasm")]
            wasm_cache,
            #[cfg(feature = "plugin-wasm")]
            plugin_backend,
        })
    }

    /// Handle a single command from the connection handler.
    /// Returns `ControlFlow::Break(())` if the simulation should terminate.
    fn handle_command(&mut self, cmd: SimCommand) -> ControlFlow<()> {
        match cmd {
            SimCommand::GetStatus { respond } => {
                let history_states = self.history_overview_for_client();
                let terminated_events: Vec<String> =
                    self.terminated_events.iter().cloned().collect();
                let response = if self.paused {
                    SimStatusResponse::Paused {
                        info_json: self.info_json.clone(),
                        terminated_events,
                        history_states,
                    }
                } else {
                    SimStatusResponse::Running {
                        info_json: self.info_json.clone(),
                        terminated_events,
                        history_states,
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
                    })
                    .expect("failed to serialize status");
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
                    })
                    .expect("failed to serialize status");
                    let _ = self.tx.send(status);
                    let _ = respond.send(Ok(()));
                }
            }
            SimCommand::Terminate { respond } => {
                eprintln!("Simulation terminated at t={:.2}s", self.current_t);
                let status = serde_json::to_string(&WsMessage::Status {
                    state: "idle".to_string(),
                })
                .expect("failed to serialize status");
                let _ = self.tx.send(status);
                let _ = respond.send(Ok(()));
                return ControlFlow::Break(());
            }
            SimCommand::AddSatellite { satellite, respond } => {
                // Branch on the running simulation mode. Controlled
                // and spacecraft paths are handled inline; the
                // orbit-only path continues to the main body below.
                match &self.group {
                    SimGroup::Controlled(_) => {
                        let result = self.handle_add_controlled_satellite(satellite);
                        let _ = respond.send(result);
                        return ControlFlow::Continue(());
                    }
                    SimGroup::Spacecraft(_) => {
                        let _ = respond.send(Err(
                            "Cannot add satellite to spacecraft simulation".to_string()
                        ));
                        return ControlFlow::Continue(());
                    }
                    SimGroup::OrbitOnly(_) => {}
                }

                // Reject attitude-enabled satellites in orbit-only mode
                if satellite.attitude.is_some() {
                    let _ = respond.send(Err(
                        "Cannot add attitude-enabled satellite to orbit-only simulation. \
                         Start with attitude config for all satellites to use spacecraft mode."
                            .to_string(),
                    ));
                    return ControlFlow::Continue(());
                }

                let sat_index = self.metas.len();
                let spec = satellite.to_satellite_spec(sat_index, self.params.body, self.params.mu);
                let third_bodies = default_third_bodies(&self.params.body);
                let system = build_orbital_system(
                    &self.params.body,
                    self.params.mu,
                    self.params.epoch,
                    &sat_params(&spec),
                    &third_bodies,
                    self.params.build_atmosphere_model(),
                );
                let initial = spec.initial_state(self.params.mu);
                self.group.push_orbit_satellite(
                    spec.id.as_str(),
                    initial.clone(),
                    self.current_t,
                    system,
                );

                let sat_info = SatelliteInfo {
                    id: spec.entity_path().to_string(),
                    name: spec.name.clone(),
                    altitude: spec.altitude(&self.params.body),
                    period: spec.period,
                    perturbations: vec![],
                };
                let t = self.current_t;
                let sat_entity_path = spec.entity_path();

                self.metas.push(SatMeta {
                    spec,
                    orbit_end_t: self.current_t
                        + self.metas.last().map_or(5554.0, |m| m.spec.period),
                    next_save_t: self.current_t + self.params.output_interval,
                });

                let body_radius = self.params.body.properties().radius;
                let hs = make_history_state(
                    sat_entity_path.clone(),
                    self.current_t,
                    initial.position(),
                    initial.velocity(),
                    self.params.mu,
                    body_radius,
                    std::collections::HashMap::new(),
                    None,
                );
                self.history.push(hs);
                let msg = state_message(
                    sat_entity_path,
                    self.current_t,
                    &initial,
                    self.params.mu,
                    body_radius,
                    std::collections::HashMap::new(),
                    None,
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
                entity_path,
                respond,
            } => {
                // Filter happens inside `query_range` (before downsampling)
                // so the `max_points` budget applies to the target entity
                // only, not to the multi-sat interleaved superset.
                let states =
                    self.history
                        .query_range(t_min, t_max, max_points, entity_path.as_ref());
                let _ = respond.send(states);
            }
        }
        ControlFlow::Continue(())
    }

    /// Build and install a new controlled satellite at runtime.
    ///
    /// Only available when the loop was started in controlled mode
    /// (the initial fleet had all satellites with `controller`
    /// config). Re-uses the shared `WasmPluginCache` held on the
    /// context so dynamic adds do not pay the Cranelift compile
    /// cost again.
    #[cfg(feature = "plugin-wasm")]
    fn handle_add_controlled_satellite(
        &mut self,
        satellite: SatelliteConfig,
    ) -> Result<(SatelliteInfo, f64), String> {
        if satellite.attitude.is_none() {
            return Err("Cannot add orbit-only satellite to controlled simulation. \
                 The dynamically-added satellite must have an attitude config."
                .to_string());
        }
        if satellite.controller.is_none() {
            return Err(
                "Cannot add controller-less satellite to controlled simulation. \
                 The dynamically-added satellite must have a controller config."
                    .to_string(),
            );
        }
        // Field-level validation (thruster direction_body != 0 etc.) so
        // a dynamic add over WebSocket cannot reach ThrusterSpec::new() and
        // panic. Matches SimConfig::load's validation behaviour.
        satellite.validate()?;

        let wasm_cache = self.wasm_cache.as_mut().ok_or_else(|| {
            "WASM plugin cache not initialized; cannot add controlled satellite".to_string()
        })?;
        let plugin_backend = self.plugin_backend.ok_or_else(|| {
            "plugin backend not resolved; cannot add controlled satellite".to_string()
        })?;

        let sat_index = self.metas.len();
        let spec = satellite.to_satellite_spec(sat_index, self.params.body, self.params.mu);
        // Re-use the startup validation so we cannot crash
        // build_controlled_satellite → build_spacecraft_dynamics on
        // a singular inertia tensor or non-positive mass.
        validate_satellite_spec(&spec)?;
        let new_sat = {
            let mut ctx = crate::sim::controlled::ControlledBuildContext {
                params: &self.params,
                wasm_cache,
                plugin_backend,
            };
            crate::sim::controlled::build_controlled_satellite(&spec, &mut ctx)
                .map_err(|e| format!("build controlled satellite: {e}"))?
        };

        let initial = new_sat.state.plant.orbit.clone();
        let attitude_q = new_sat.state.plant.attitude.quaternion;
        let attitude_w = new_sat.state.plant.attitude.angular_velocity;
        let has_rw = new_sat.has_rw;
        let rw_mom = if has_rw && !new_sat.state.aux.is_empty() {
            Some(new_sat.state.aux.clone())
        } else {
            None
        };

        self.group.push_controlled_satellite(new_sat);

        let sat_info = SatelliteInfo {
            id: spec.entity_path().to_string(),
            name: spec.name.clone(),
            altitude: spec.altitude(&self.params.body),
            period: spec.period,
            perturbations: vec![],
        };
        let t = self.current_t;
        let sat_entity_path = spec.entity_path();

        self.metas.push(SatMeta {
            spec,
            orbit_end_t: self.current_t + sat_info.period,
            next_save_t: self.current_t + self.params.output_interval,
        });

        let body_radius = self.params.body.properties().radius;
        let attitude_payload = AttitudePayload {
            quaternion_wxyz: [attitude_q[0], attitude_q[1], attitude_q[2], attitude_q[3]],
            angular_velocity_body: [attitude_w[0], attitude_w[1], attitude_w[2]],
            source: AttitudeSource::Propagated,
            rw_momentum: rw_mom,
        };
        let hs = make_history_state(
            sat_entity_path.clone(),
            self.current_t,
            initial.position(),
            initial.velocity(),
            self.params.mu,
            body_radius,
            std::collections::HashMap::new(),
            Some(attitude_payload.clone()),
        );
        self.history.push(hs);
        let msg = state_message(
            sat_entity_path,
            self.current_t,
            &initial,
            self.params.mu,
            body_radius,
            std::collections::HashMap::new(),
            Some(attitude_payload),
        );
        let _ = self.tx.send(msg);

        let added_msg = serde_json::to_string(&WsMessage::SatelliteAdded {
            satellite: sat_info.clone(),
            t,
        })
        .expect("failed to serialize satellite_added");
        let _ = self.tx.send(added_msg);

        Ok((sat_info, t))
    }

    /// Non-plugin-wasm stub: dynamic add into controlled mode is
    /// impossible without the WASM backend compiled in.
    #[cfg(not(feature = "plugin-wasm"))]
    fn handle_add_controlled_satellite(
        &mut self,
        _satellite: SatelliteConfig,
    ) -> Result<(SatelliteInfo, f64), String> {
        Err("Controlled simulation requires the `plugin-wasm` feature; \
             cannot add controlled satellite"
            .to_string())
    }

    /// Propagate one chunk of simulation time, collecting outputs.
    fn propagate_chunk(&mut self, outputs_per_chunk: usize) -> Vec<crate::sim::core::HistoryState> {
        let mut all_outputs = Vec::new();
        let body_radius = self.params.body.properties().radius;

        for _ in 0..outputs_per_chunk {
            let target_t = self.current_t + self.params.stream_interval;

            // Orbit boundary reset (only for unperturbed 2-body, orbit-only mode)
            if !self.has_perturbations {
                let n = self.group.len();
                let resets: Vec<(SatId, OrbitalState)> = (0..n)
                    .filter_map(|i| {
                        if !self.group.is_terminated(i)
                            && self.current_t >= self.metas[i].orbit_end_t - 1e-9
                        {
                            Some((
                                self.group.sat_id(i),
                                self.metas[i].spec.initial_state(self.params.mu),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (id, new_state) in &resets {
                    self.group.reset_orbit_state(id, new_state.clone());
                    if let Some(i) = self
                        .metas
                        .iter()
                        .position(|m| m.spec.id.as_str() == AsRef::<str>::as_ref(id))
                    {
                        self.metas[i].orbit_end_t = self.current_t + self.metas[i].spec.period;
                    }
                }
            }

            // Controlled satellites: step in dt_ctrl increments up to target_t.
            self.group
                .step_controlled_to(self.current_t, target_t, &self.params);

            let outcome = self.group.propagate_to(target_t).unwrap();

            let n = self.group.len();
            let is_controlled = matches!(self.group, SimGroup::Controlled(_));
            for i in 0..n {
                if self.group.is_terminated(i) {
                    continue;
                }

                let t = if is_controlled {
                    target_t
                } else {
                    self.group.sat_t(i)
                };
                let snap = self.group.snapshot(i, t);
                let hs = make_history_state(
                    self.metas[i].spec.entity_path(),
                    t,
                    snap.orbit.position(),
                    snap.orbit.velocity(),
                    self.params.mu,
                    body_radius,
                    snap.accels,
                    snap.attitude,
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
                let term_entity_path = orts::record::entity_path::EntityPath::parse(&format!(
                    "/world/sat/{}",
                    sid_str
                ));
                let msg = serde_json::to_string(&WsMessage::SimulationTerminated {
                    entity_path: term_entity_path,
                    t: term.t,
                    reason: term.reason.clone(),
                })
                .expect("failed to serialize termination message");
                let _ = self.tx.send(msg.clone());
                push_terminated_capped(&mut self.terminated_events, msg);
            }

            self.current_t = target_t;
        }

        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        all_outputs
    }

    /// Build the bounded history overview that is replayed to a (re)connecting
    /// client.
    ///
    /// The old behaviour was to ship the entire simulation history plus a
    /// full-resolution background detail stream on every connect. On
    /// long-running simulations this dominated reconnect latency and was the
    /// root cause of the "viewer blank after reload" problem.
    ///
    /// Current contract: delegate to [`HistoryBuffer::overview`], which
    /// returns an incrementally-maintained bounded overview in O(1) time
    /// regardless of sim duration — no `load_all()`, no disk I/O, no sort.
    /// The server is deliberately time-range-agnostic: any display window
    /// the client cares about is served from its own local buffers plus
    /// follow-up `QueryRange` requests, not baked into the connect
    /// handshake.
    fn history_overview_for_client(&self) -> Vec<crate::sim::core::HistoryState> {
        self.history.overview()
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
    let chunk_wall_time = std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    let mut ctx = match SimLoopContext::new(params, tx.clone(), history) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Simulation startup error: {e}");
            let err_msg = serde_json::to_string(&WsMessage::Error { message: e })
                .expect("failed to serialize error");
            let _ = tx.send(err_msg);
            return (LoopExit::Terminated, cmd_rx);
        }
    };

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
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return (LoopExit::Disconnected, cmd_rx);
                }
            }
        }

        // Skip propagation while paused
        if ctx.paused {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }

        // Offload the blocking propagation work to a dedicated blocking
        // thread so the tokio worker is free to handle WebSocket I/O
        // and command dispatch while the physics/controller step runs.
        // This also keeps `Handle::block_on` inside WASM async backends
        // from starving the serve runtime.
        let (all_outputs, ctx_back) = tokio::task::spawn_blocking(move || {
            let outputs = ctx.propagate_chunk(OUTPUTS_PER_CHUNK);
            (outputs, ctx)
        })
        .await
        .expect("simulation blocking task panicked");
        ctx = ctx_back;

        if !all_outputs.is_empty() {
            let send_interval = chunk_wall_time / all_outputs.len() as u32;
            for out in &all_outputs {
                let send_start = tokio::time::Instant::now();
                let msg = serde_json::to_string(&WsMessage::State {
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
                    attitude: out.attitude.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use arika::body::KnownBody;

    #[test]
    fn terminated_events_ring_buffer_caps_at_limit() {
        let mut events: VecDeque<String> = VecDeque::new();
        for i in 0..(TERMINATED_EVENTS_CAP + 250) {
            push_terminated_capped(&mut events, format!("event-{i}"));
        }
        assert_eq!(
            events.len(),
            TERMINATED_EVENTS_CAP,
            "ring buffer must stay at cap after overflow"
        );
        // Oldest entries must be dropped first, newest preserved.
        let first = events.front().unwrap();
        let last = events.back().unwrap();
        assert_eq!(first, &format!("event-{}", 250));
        assert_eq!(last, &format!("event-{}", TERMINATED_EVENTS_CAP + 249));
    }

    #[test]
    fn terminated_events_below_cap_keeps_all() {
        let mut events: VecDeque<String> = VecDeque::new();
        for i in 0..10 {
            push_terminated_capped(&mut events, format!("event-{i}"));
        }
        assert_eq!(events.len(), 10);
        assert_eq!(events.front().unwrap(), "event-0");
        assert_eq!(events.back().unwrap(), "event-9");
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

    #[test]
    fn body_names_for_moon_includes_sun_only() {
        let names = body_names_for(&KnownBody::Moon);
        assert_eq!(names[0], "moon");
        assert!(names.contains(&"sun".to_string()));
        assert_eq!(names.len(), 2);
    }
}
