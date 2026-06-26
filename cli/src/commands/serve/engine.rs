//! Pure simulation orchestration engine for `orts serve`.
//!
//! [`ServeEngine`] owns the simulation state machine — group construction,
//! propagation, orbit-boundary reset, snapshotting, the history buffer,
//! dynamic satellite add, and the terminated-event ring — with **no tokio,
//! no channels, and no wall-clock pacing**. The serve layer
//! ([`super::manager`]) wraps it in a tokio task and owns everything the
//! engine deliberately does not: the broadcast/oneshot wiring, the
//! idle/running/paused state machine, wall-clock pacing, and the stream-io
//! bridge lifecycle.
//!
//! Outbound traffic leaves the engine as return values (pre-serialized
//! broadcast strings on [`EngineInit`] / [`StepOutput`] / [`AddOutput`], and
//! the paced [`HistoryState`] list on [`StepOutput::states`]) rather than via
//! a channel, and stream-io bytes are pumped through an injected [`StreamIo`]
//! sink/source. That is what lets the engine — including its failure modes
//! (integration error, controller fault, stream overflow / stuck peer,
//! boundary reset) — be unit-tested in-process without a runtime.

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

use crate::cli::IntegratorChoice;
use crate::config::SatelliteConfig;
use crate::satellite::{SatelliteInfo, SatelliteSpec};
use crate::sim::controlled::ControlledSatellite;
use crate::sim::core::{
    AttitudePayload, AttitudeSource, HistoryState, accel_breakdown, make_history_state, sat_params,
    spacecraft_accel_breakdown,
};
use crate::sim::params::SimParams;
use orts::setup::{build_orbital_system, build_spacecraft_dynamics, default_third_bodies};

use super::compute::state_message;
use super::history::HistoryBuffer;
use super::protocol::WsMessage;
use super::stream_bridge::{OutboundPush, StreamKey};

/// Sink/source for stream-io bytes, injected into [`ServeEngine::step_chunk`].
///
/// The engine knows only each satellite's declared stream *names* (indexed by
/// satellite position in the fleet); resolving those to concrete transports
/// (the tokio-backed [`super::stream_bridge::StreamEndpoint`]s) is the serve
/// layer's job. Keeping the resolution behind this trait is what lets tests
/// drive the engine's stream failure modes (overflow halt, stuck-peer halt)
/// with an in-memory fake instead of a live WebSocket bridge.
pub(super) trait StreamIo {
    /// Drain bytes staged for satellite index `sat_idx`, stream `name`, since
    /// the last tick. The bool is `true` when the staging buffer overflowed —
    /// bytes were lost to the bound, so the engine must halt the sim.
    fn take_inbound(&mut self, sat_idx: usize, name: &str) -> (Vec<u8>, bool);

    /// Forward FSW-produced bytes towards the active peer for satellite index
    /// `sat_idx`, stream `name`. [`OutboundPush::Stuck`] halts the sim.
    fn push_outbound(&mut self, sat_idx: usize, name: &str, bytes: Vec<u8>) -> OutboundPush;
}

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
    ///
    /// Controller errors are fatal: a `PluginError` from `update()` means the
    /// controller output cannot be trusted (bad command, guest trap, stream-io
    /// overrun fault, ...), so the error is propagated and the caller halts
    /// the simulation instead of integrating bad state forward.
    fn step_controlled_to(
        &mut self,
        current_t: f64,
        target_t: f64,
        params: &SimParams,
    ) -> Result<(), String> {
        let SimGroup::Controlled(sats) = self else {
            return Ok(());
        };
        for sat in sats.iter_mut() {
            let dt_ctrl = sat.controller.sample_period();
            let dt_ode = params.dt.min(dt_ctrl);
            let mut t = current_t;
            while t < target_t - 1e-12 {
                let dt = dt_ctrl.min(target_t - t);
                crate::sim::controlled::step_controlled(sat, t, dt, dt_ode, params.epoch.as_ref())
                    .map_err(|e| format!("controlled simulation error at t={t:.3}: {e}"))?;
                t += dt;
            }
        }
        Ok(())
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
        // (step_chunk) uses metas[i] for the real ID.
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
            // step_chunk uses target_t directly for these.
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

/// Per-satellite metadata for serve mode.
struct SatMeta {
    spec: SatelliteSpec,
    orbit_end_t: f64,
    next_save_t: f64,
}

/// Snapshot of engine state for a (re)connecting client, assembled by the
/// serve layer into a [`super::manager::SimStatusResponse`]. The engine is
/// deliberately unaware of the running/paused distinction (that lives in the
/// serve loop), so this carries only the data, not the state label.
pub(super) struct StatusData {
    pub info_json: String,
    pub terminated_events: Vec<String>,
    pub history_states: Vec<HistoryState>,
}

/// What [`ServeEngine::build`] hands back to the serve layer: the engine plus
/// the side effects the serve layer must perform (broadcast the initial
/// Info/State messages; register the stream-io bridge endpoints).
pub(super) struct EngineInit {
    pub engine: ServeEngine,
    /// Pre-serialized Info + initial-state messages to broadcast immediately.
    pub initial_broadcasts: Vec<String>,
    /// Per-satellite `(sat_id, declared stream names)`, indexed like the
    /// fleet. The serve layer flattens this into the bridge's endpoint keys
    /// and resolves each into a [`super::stream_bridge::StreamEndpoint`].
    pub stream_layout: Vec<(String, Vec<String>)>,
}

/// Result of [`ServeEngine::step_chunk`]: the paced state samples plus any
/// immediate broadcasts (currently `simulation_terminated` events) produced
/// during the chunk.
pub(super) struct StepOutput {
    /// Per-output-interval state samples, sorted by time. The serve layer
    /// paces these to the wall clock.
    pub states: Vec<HistoryState>,
    /// Pre-serialized messages to broadcast immediately (not paced), in order.
    pub broadcasts: Vec<String>,
}

/// Result of [`ServeEngine::add_satellite`].
pub(super) struct AddOutput {
    pub info: SatelliteInfo,
    pub t: f64,
    /// Pre-serialized State + `satellite_added` messages to broadcast.
    pub broadcasts: Vec<String>,
}

/// Pure simulation orchestration engine. See the module docs for the
/// engine/serve responsibility split.
pub(super) struct ServeEngine {
    params: Arc<SimParams>,
    group: SimGroup,
    metas: Vec<SatMeta>,
    history: HistoryBuffer,
    info_json: String,
    /// Ring-buffered queue of `simulation_terminated` payloads, replayed to
    /// late-connecting clients. Bounded by [`TERMINATED_EVENTS_CAP`] to avoid
    /// unbounded growth in long-running sims with many deorbiting satellites.
    terminated_events: VecDeque<String>,
    current_t: f64,
    has_perturbations: bool,
    /// Each satellite's declared stream names, indexed like `metas`. The
    /// engine iterates these to pump the injected [`StreamIo`]; resolving a
    /// name to a transport is the serve layer's concern.
    sat_streams: Vec<Vec<String>>,
    /// Sim-time step per output interval. Equal to `params.stream_interval`
    /// normally; lowered to the controller tick when stream-io streams are
    /// wired so the bridge pumps (and the serve wall clock syncs) at tick
    /// granularity.
    stream_step: f64,
    /// Whether any satellite declared stream-io streams. When `true` the
    /// serve loop must pace in realtime (1 sim s = 1 wall s).
    realtime: bool,
    /// Shared WASM plugin cache, kept alive for the whole engine lifetime so
    /// dynamic `add_satellite` calls can reuse the compiled guest components
    /// and (for the async backend) the shared runtime thread. `None` outside
    /// controlled mode or when `plugin-wasm` is disabled.
    #[cfg(feature = "plugin-wasm")]
    wasm_cache: Option<orts::plugin::wasm::WasmPluginCache>,
    /// Resolved plugin backend (sync / async) for this engine. Locked in at
    /// construction so dynamic additions stay on the same backend as the
    /// initial fleet.
    #[cfg(feature = "plugin-wasm")]
    plugin_backend: Option<crate::sim::params::ResolvedPluginBackend>,
}

impl ServeEngine {
    /// Build the engine from resolved [`SimParams`], emitting the initial
    /// Info/State broadcasts and the stream layout the serve layer needs.
    ///
    /// Keeping the constructor's only "input" type `Arc<SimParams>` is
    /// deliberate: when config assembly is unified (issue #98) this is the
    /// single line that changes to take a `SimulationPlan` — nothing leaks
    /// into the serve loop or the connection layer.
    pub(super) fn build(
        params: Arc<SimParams>,
        mut history: HistoryBuffer,
    ) -> Result<EngineInit, String> {
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
        // `add_satellite` calls received later. For non-controlled
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
                    seed_epoch: params.epoch,
                    wasm_cache: wasm_cache
                        .as_mut()
                        .expect("wasm_cache must be Some when has_controller"),
                    plugin_backend: plugin_backend
                        .expect("plugin_backend must be Some when has_controller"),
                };
                #[cfg(not(feature = "plugin-wasm"))]
                let mut ctx = crate::sim::controlled::ControlledBuildContext {
                    params: &params,
                    seed_epoch: params.epoch,
                };
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

                let orbit = spec.initial_state(params.mu, params.epoch)?;
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
                let initial = spec.initial_state(params.mu, params.epoch)?;
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

        // Build the Info message (broadcast by the serve layer; also retained
        // for status replay to late-connecting clients).
        let info_msg = build_info_message(&params);
        let info_json = serde_json::to_string(&info_msg).expect("failed to serialize info");
        let mut initial_broadcasts = vec![info_json.clone()];

        // Emit initial states.
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
            initial_broadcasts.push(msg);
        }

        // Validate every declared stream-io stream of this fleet. Registration
        // of the bridge endpoints themselves is the serve layer's job (it owns
        // the tokio-backed bridge); the engine only validates and reports the
        // layout.
        let stream_keys: Vec<StreamKey> = metas
            .iter()
            .flat_map(|m| {
                let id = m.spec.id.clone();
                m.spec
                    .streams
                    .iter()
                    .map(move |s| (id.clone(), s.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        // Streams are pumped through the controller; in orbit-only /
        // spacecraft mode the endpoints would be black holes (accepted but
        // never drained). Reject loudly instead.
        if !stream_keys.is_empty() && !matches!(group, SimGroup::Controlled(_)) {
            return Err(
                "stream-io streams are declared but no satellite has a controller; \
                 streams require a plugin-controlled simulation"
                    .to_string(),
            );
        }
        // A duplicate (sat, stream) pair would pump the same controller
        // stream twice per tick and alias one endpoint; reject it. Both
        // parts also become URL path segments (`/stream/{sat}/{stream}`),
        // so an empty or '/'-containing name would mint an unreachable
        // endpoint — reject those too.
        {
            let mut seen = std::collections::HashSet::new();
            for key in &stream_keys {
                let (sat, stream) = key;
                if invalid_path_segment(sat) || invalid_path_segment(stream) {
                    return Err(format!(
                        "invalid stream declaration '{stream}' on satellite '{sat}': \
                         satellite ids and stream names must be non-empty and must \
                         not contain '/' (they form the endpoint URL path)"
                    ));
                }
                if !seen.insert(key) {
                    return Err(format!(
                        "duplicate stream declaration '{stream}' on satellite '{sat}'"
                    ));
                }
            }
        }

        // Per-satellite declared stream names, indexed like `metas`. The
        // engine pumps these; the serve layer resolves them to endpoints.
        let sat_streams: Vec<Vec<String>> = metas.iter().map(|m| m.spec.streams.clone()).collect();
        let stream_layout: Vec<(String, Vec<String>)> = metas
            .iter()
            .map(|m| (m.spec.id.clone(), m.spec.streams.clone()))
            .collect();

        // With stream-io streams wired, the loop runs in realtime and steps
        // one controller tick at a time. `step_controlled_to` steps *every*
        // controller over each interval, so a global tick shorter than a
        // controller's own period would over-step it; require a uniform
        // sample period (resolving it here surfaces a mixed-rate fleet as a
        // construction error, handled uniformly with other startup errors).
        let realtime = metas.iter().any(|m| !m.spec.streams.is_empty());
        let stream_step = if realtime {
            let SimGroup::Controlled(sats) = &group else {
                // Unreachable: the stream-keys check above already rejected
                // declared streams outside controlled mode.
                return Err("stream-io bridge requires a controlled simulation".to_string());
            };
            let periods: Vec<f64> = sats.iter().map(|s| s.controller.sample_period()).collect();
            uniform_tick(&periods)?
        } else {
            params.stream_interval
        };

        let engine = ServeEngine {
            params,
            group,
            metas,
            history,
            info_json,
            terminated_events: VecDeque::new(),
            current_t: 0.0,
            has_perturbations,
            sat_streams,
            stream_step,
            realtime,
            #[cfg(feature = "plugin-wasm")]
            wasm_cache,
            #[cfg(feature = "plugin-wasm")]
            plugin_backend,
        };

        Ok(EngineInit {
            engine,
            initial_broadcasts,
            stream_layout,
        })
    }

    /// Whether the fleet declared stream-io streams — the serve loop must then
    /// pace in realtime (1 sim s = 1 wall s) at the tick from [`Self::step`].
    pub(super) fn is_realtime(&self) -> bool {
        self.realtime
    }

    /// Sim-time advanced per output interval (the realtime tick when streams
    /// are wired, otherwise `params.stream_interval`). The serve layer uses
    /// this to size each chunk's wall-clock budget. Distinct from
    /// [`step_chunk`](Self::step_chunk), which actually advances the sim.
    pub(super) fn effective_step(&self) -> f64 {
        self.stream_step
    }

    /// Current simulation time.
    pub(super) fn current_t(&self) -> f64 {
        self.current_t
    }

    /// Pump staged inbound bytes from the injected sink into each controller
    /// (frozen into the FSW's next tick). A staging overflow halts the sim
    /// (bytes were lost to the bound — the no-drop contract is broken).
    fn pump_streams_inbound(&mut self, streams: &mut dyn StreamIo) -> Result<(), String> {
        let SimGroup::Controlled(sats) = &mut self.group else {
            return Ok(());
        };
        for (i, sat) in sats.iter_mut().enumerate() {
            let Some(names) = self.sat_streams.get(i) else {
                continue; // defensive: sat_streams is kept metas-aligned
            };
            for name in names {
                let (bytes, overflowed) = streams.take_inbound(i, name);
                if overflowed {
                    return Err(format!(
                        "stream-io: inbound staging overflow on {}/{name} (sim not draining fast enough)",
                        self.metas[i].spec.id
                    ));
                }
                if !bytes.is_empty() {
                    sat.controller.stream_deliver(name, bytes);
                }
            }
        }
        Ok(())
    }

    /// Pump FSW-written bytes out through the injected source. A connected but
    /// stuck peer halts the sim (no-drop contract); no peer discards
    /// (transient-disconnect policy).
    fn pump_streams_outbound(&mut self, streams: &mut dyn StreamIo) -> Result<(), String> {
        let SimGroup::Controlled(sats) = &mut self.group else {
            return Ok(());
        };
        for (i, sat) in sats.iter_mut().enumerate() {
            let Some(names) = self.sat_streams.get(i) else {
                continue; // defensive: sat_streams is kept metas-aligned
            };
            for name in names {
                let bytes = sat.controller.stream_take(name);
                if bytes.is_empty() {
                    continue;
                }
                match streams.push_outbound(i, name, bytes) {
                    OutboundPush::Sent | OutboundPush::NoPeer => {}
                    OutboundPush::Stuck => {
                        return Err(format!(
                            "stream-io: peer on {}/{name} is connected but not draining",
                            self.metas[i].spec.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Propagate `outputs_per_chunk` output intervals, collecting outputs.
    ///
    /// Any fatal error — a controller fault (bad command / guest trap /
    /// stream-io overrun) or an integration error — aborts the chunk with
    /// `Err`; the serve layer halts (pauses) the simulation rather than
    /// integrating untrusted state forward.
    pub(super) fn step_chunk(
        &mut self,
        outputs_per_chunk: usize,
        streams: &mut dyn StreamIo,
    ) -> Result<StepOutput, String> {
        let mut all_outputs = Vec::new();
        let mut broadcasts = Vec::new();
        let body_radius = self.params.body.properties().radius;

        for _ in 0..outputs_per_chunk {
            let target_t = self.current_t + self.stream_step;

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
                                self.metas[i]
                                    .spec
                                    .initial_state(self.params.mu, self.params.epoch)
                                    .unwrap_or_else(|e| panic!("{e}")),
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

            // stream-io bridge: freeze this interval's inbound bytes into the
            // controllers before stepping; flush FSW output to the peers
            // after. (With streams wired, `stream_step` equals the controller
            // tick, so this is the tick-boundary pump.)
            self.pump_streams_inbound(streams)?;

            // Controlled satellites: step in dt_ctrl increments up to target_t.
            self.group
                .step_controlled_to(self.current_t, target_t, &self.params)?;

            self.pump_streams_outbound(streams)?;

            // Integration errors are no longer fatal panics: surface them as
            // an engine error so the serve layer halts gracefully (and so the
            // failure mode is unit-testable without a runtime).
            let outcome = self
                .group
                .propagate_to(target_t)
                .map_err(|e| format!("integration error at t={target_t:.3}: {e}"))?;

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
                push_terminated_capped(&mut self.terminated_events, msg.clone());
                broadcasts.push(msg);
            }

            self.current_t = target_t;
        }

        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        Ok(StepOutput {
            states: all_outputs,
            broadcasts,
        })
    }

    /// Assemble the status snapshot for a (re)connecting client.
    ///
    /// The history is always a bounded, downsampled overview maintained in
    /// O(1) regardless of sim duration — see [`HistoryBuffer::overview`] — so
    /// reconnects to long-running sims never ship an unbounded payload. The
    /// server is deliberately time-range-agnostic: any display window the
    /// client cares about is served from its own buffers plus follow-up
    /// [`query_range`](Self::query_range) requests.
    pub(super) fn status_data(&self) -> StatusData {
        StatusData {
            info_json: self.info_json.clone(),
            terminated_events: self.terminated_events.iter().cloned().collect(),
            history_states: self.history.overview(),
        }
    }

    /// Query a time range from history (filtered before downsampling so the
    /// `max_points` budget applies to the target entity only).
    pub(super) fn query_range(
        &self,
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        entity_path: Option<&orts::record::entity_path::EntityPath>,
    ) -> Vec<HistoryState> {
        self.history
            .query_range(t_min, t_max, max_points, entity_path)
    }

    /// Add a satellite to the running simulation, dispatching on the current
    /// mode. Returns the new satellite's info plus the State + `satellite_added`
    /// messages for the serve layer to broadcast.
    pub(super) fn add_satellite(
        &mut self,
        satellite: SatelliteConfig,
    ) -> Result<AddOutput, String> {
        // stream-io streams require an endpoint registered before the loop's
        // realtime/pacing mode was decided; dynamic wiring is not supported.
        if !satellite.streams.is_empty() {
            return Err(
                "Dynamically added satellites cannot declare stream-io streams; \
                 declare them in the initial config."
                    .to_string(),
            );
        }

        // Branch on the running simulation mode. Controlled and spacecraft
        // paths are handled inline; the orbit-only path continues below.
        match &self.group {
            SimGroup::Controlled(_) => return self.add_controlled_satellite(satellite),
            SimGroup::Spacecraft(_) => {
                return Err("Cannot add satellite to spacecraft simulation".to_string());
            }
            SimGroup::OrbitOnly(_) => {}
        }

        // Reject attitude-enabled satellites in orbit-only mode
        if satellite.attitude.is_some() {
            return Err(
                "Cannot add attitude-enabled satellite to orbit-only simulation. \
                 Start with attitude config for all satellites to use spacecraft mode."
                    .to_string(),
            );
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
        let seed_epoch = self.params.epoch.map(|e| e.add_si_seconds(self.current_t));
        let initial = spec.initial_state(self.params.mu, seed_epoch)?;
        self.group
            .push_orbit_satellite(spec.id.as_str(), initial.clone(), self.current_t, system);

        let sat_info = SatelliteInfo {
            id: spec.entity_path().to_string(),
            name: spec.name.clone(),
            altitude: spec.altitude(&self.params.body),
            period: spec.period,
            perturbations: vec![],
            shape: spec.shape,
        };
        let t = self.current_t;
        let sat_entity_path = spec.entity_path();

        self.metas.push(SatMeta {
            spec,
            orbit_end_t: self.current_t + self.metas.last().map_or(5554.0, |m| m.spec.period),
            next_save_t: self.current_t + self.params.output_interval,
        });
        // Keep `sat_streams` index-aligned with `metas` (dynamic adds cannot
        // declare streams, so the entry is empty).
        self.sat_streams.push(Vec::new());

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
        let state_msg = state_message(
            sat_entity_path,
            self.current_t,
            &initial,
            self.params.mu,
            body_radius,
            std::collections::HashMap::new(),
            None,
        );

        let added_msg = serde_json::to_string(&WsMessage::SatelliteAdded {
            satellite: sat_info.clone(),
            t,
        })
        .expect("failed to serialize satellite_added");

        Ok(AddOutput {
            info: sat_info,
            t,
            broadcasts: vec![state_msg, added_msg],
        })
    }

    /// Build and install a new controlled satellite at runtime.
    ///
    /// Only available when the engine was started in controlled mode (the
    /// initial fleet had all satellites with `controller` config). Re-uses the
    /// shared `WasmPluginCache` held on the engine so dynamic adds do not pay
    /// the Cranelift compile cost again.
    #[cfg(feature = "plugin-wasm")]
    fn add_controlled_satellite(
        &mut self,
        satellite: SatelliteConfig,
    ) -> Result<AddOutput, String> {
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
                seed_epoch: self.params.epoch.map(|e| e.add_si_seconds(self.current_t)),
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
            shape: spec.shape,
        };
        let t = self.current_t;
        let sat_entity_path = spec.entity_path();

        self.metas.push(SatMeta {
            spec,
            orbit_end_t: self.current_t + sat_info.period,
            next_save_t: self.current_t + self.params.output_interval,
        });
        // Keep `sat_streams` index-aligned with `metas` (dynamic adds cannot
        // declare streams, so the entry is empty).
        self.sat_streams.push(Vec::new());

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
        let state_msg = state_message(
            sat_entity_path,
            self.current_t,
            &initial,
            self.params.mu,
            body_radius,
            std::collections::HashMap::new(),
            Some(attitude_payload),
        );

        let added_msg = serde_json::to_string(&WsMessage::SatelliteAdded {
            satellite: sat_info.clone(),
            t,
        })
        .expect("failed to serialize satellite_added");

        Ok(AddOutput {
            info: sat_info,
            t,
            broadcasts: vec![state_msg, added_msg],
        })
    }

    /// Non-plugin-wasm stub: dynamic add into controlled mode is impossible
    /// without the WASM backend compiled in.
    #[cfg(not(feature = "plugin-wasm"))]
    fn add_controlled_satellite(
        &mut self,
        _satellite: SatelliteConfig,
    ) -> Result<AddOutput, String> {
        Err("Controlled simulation requires the `plugin-wasm` feature; \
             cannot add controlled satellite"
            .to_string())
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
                shape: s.shape,
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

/// Validate a single satellite's attitude configuration so that
/// `build_spacecraft_dynamics` cannot panic on a singular inertia
/// tensor or a non-positive mass. Used from both the startup config
/// validator and the runtime `add_satellite` path.
pub(super) fn validate_satellite_spec(spec: &SatelliteSpec) -> Result<(), String> {
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

/// Whether `s` cannot be used as one path segment of a stream endpoint URL
/// (`/stream/{sat}/{stream}`): empty or containing a path separator.
fn invalid_path_segment(s: &str) -> bool {
    s.is_empty() || s.contains('/')
}

/// The single sample period shared by all controllers, or an error when the
/// fleet mixes periods (the stream-io bridge steps every controller on the
/// same global tick, so mixed rates would over-step the slower ones).
fn uniform_tick(periods: &[f64]) -> Result<f64, String> {
    let Some(&first) = periods.first() else {
        return Err("stream-io bridge requires at least one controller".to_string());
    };
    if periods.iter().any(|p| (p - first).abs() > 1e-9) {
        return Err(format!(
            "stream-io bridge requires a uniform controller sample period, got {periods:?}; \
             mixed-rate fleets are not supported with streams yet"
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A [`StreamIo`] that has nothing to pump. Orbit-only / spacecraft runs
    /// never touch it (their pumps early-return), so it just satisfies
    /// `step_chunk`'s parameter for the non-streamed engine tests below.
    struct NullStreamIo;

    impl StreamIo for NullStreamIo {
        fn take_inbound(&mut self, _sat_idx: usize, _name: &str) -> (Vec<u8>, bool) {
            (Vec::new(), false)
        }
        fn push_outbound(&mut self, _sat_idx: usize, _name: &str, _bytes: Vec<u8>) -> OutboundPush {
            OutboundPush::NoPeer
        }
    }

    /// Build an engine from a TOML config snippet. The whole point of the
    /// engine/serve split (issue #99) is that this needs no tokio runtime,
    /// no broadcast channel, and no stream bridge.
    fn engine_from_toml(toml: &str) -> Result<EngineInit, String> {
        let config: crate::config::SimConfig = toml::from_str(toml).expect("valid test toml");
        let params = Arc::new(SimParams::from_config(&config, true));
        let data_dir = std::env::temp_dir().join(format!(
            "orts-engine-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let body_radius = params.body.properties().radius;
        let history = HistoryBuffer::new(5000, data_dir, params.mu, body_radius);
        ServeEngine::build(params, history)
    }

    /// A single, stable orbit-only satellite — the cheapest engine to build.
    const ORBIT_ONLY: &str = r#"
[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }
"#;

    #[test]
    fn new_orbit_only_emits_info_and_initial_state() {
        let init = engine_from_toml(ORBIT_ONLY).expect("engine builds");
        // First broadcast is Info, then one initial State for the sole sat.
        assert!(init.initial_broadcasts[0].contains("\"type\":\"info\""));
        assert_eq!(init.initial_broadcasts.len(), 2);
        assert!(init.initial_broadcasts[1].contains("\"type\":\"state\""));
        assert!(!init.engine.is_realtime());
        assert_eq!(init.engine.current_t(), 0.0);
        // No declared streams → empty layout entry for the one satellite.
        assert_eq!(init.stream_layout, vec![("sat-a".to_string(), vec![])]);
    }

    #[test]
    fn step_chunk_advances_time_and_produces_states() {
        let mut init = engine_from_toml(ORBIT_ONLY).expect("engine builds");
        let step = init.engine.effective_step();
        let out = init
            .engine
            .step_chunk(3, &mut NullStreamIo)
            .expect("stable orbit propagates");
        // current_t advances by exactly n * step.
        assert!((init.engine.current_t() - 3.0 * step).abs() < 1e-9);
        assert!(!out.states.is_empty(), "a live satellite emits states");
        // A stable 500 km orbit neither deorbits nor collides.
        assert!(
            out.broadcasts.is_empty(),
            "no terminations for a stable orbit"
        );
    }

    #[test]
    fn low_orbit_terminates_and_fills_the_replay_ring() {
        // Below the Kármán line (Earth atmosphere_altitude = 100 km): the
        // event checker breaks on the first interval → an atmospheric-entry
        // termination, broadcast immediately and retained for replay.
        let mut init = engine_from_toml(
            r#"
[[satellites]]
id = "doomed"
orbit = { type = "circular", altitude = 50 }
"#,
        )
        .expect("engine builds");
        let out = init
            .engine
            .step_chunk(1, &mut NullStreamIo)
            .expect("a termination is not an error");
        assert_eq!(out.broadcasts.len(), 1, "exactly one termination event");
        assert!(out.broadcasts[0].contains("simulation_terminated"));
        // The event is retained for late-connecting clients (status replay).
        assert_eq!(init.engine.status_data().terminated_events.len(), 1);
    }

    #[test]
    fn add_orbit_satellite_succeeds_and_broadcasts() {
        let mut init = engine_from_toml(ORBIT_ONLY).expect("engine builds");
        let cfg: SatelliteConfig = serde_json::from_str(
            r#"{ "id": "sat-b", "orbit": { "type": "circular", "altitude": 700 } }"#,
        )
        .expect("valid satellite config");
        let added = init.engine.add_satellite(cfg).expect("orbit-only add ok");
        assert_eq!(added.t, 0.0);
        assert!(added.info.id.contains("sat-b"));
        // A State message plus a satellite_added announcement.
        assert_eq!(added.broadcasts.len(), 2);
        assert!(
            added
                .broadcasts
                .iter()
                .any(|m| m.contains("satellite_added"))
        );
    }

    #[test]
    fn add_attitude_satellite_to_orbit_only_is_rejected() {
        let mut init = engine_from_toml(ORBIT_ONLY).expect("engine builds");
        let cfg: SatelliteConfig = serde_json::from_str(
            r#"{
                "id": "att",
                "orbit": { "type": "circular", "altitude": 700 },
                "attitude": { "inertia_diag": [10, 10, 10], "mass": 50 }
            }"#,
        )
        .expect("valid satellite config");
        let err = init.engine.add_satellite(cfg).err().unwrap();
        assert!(err.contains("orbit-only simulation"), "got: {err}");
    }

    #[test]
    fn add_satellite_with_streams_is_rejected() {
        let mut init = engine_from_toml(ORBIT_ONLY).expect("engine builds");
        let cfg: SatelliteConfig = serde_json::from_str(
            r#"{
                "id": "streamed",
                "orbit": { "type": "circular", "altitude": 700 },
                "streams": ["comlink"]
            }"#,
        )
        .expect("valid satellite config");
        let err = init.engine.add_satellite(cfg).err().unwrap();
        assert!(err.contains("stream-io streams"), "got: {err}");
    }

    #[test]
    fn mixed_attitude_config_is_rejected_at_construction() {
        let err = engine_from_toml(
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
"#,
        )
        .err()
        .unwrap();
        assert!(err.contains("Mixed attitude config"), "got: {err}");
    }

    #[test]
    fn streams_without_controller_are_rejected_at_construction() {
        let err = engine_from_toml(
            r#"
[[satellites]]
id = "sat-a"
orbit = { type = "circular", altitude = 500 }
streams = ["comlink"]
"#,
        )
        .err()
        .unwrap();
        assert!(
            err.contains("streams require a plugin-controlled simulation"),
            "got: {err}"
        );
    }

    #[test]
    fn invalid_path_segment_rejects_empty_and_slash() {
        assert!(invalid_path_segment(""));
        assert!(invalid_path_segment("a/b"));
        assert!(!invalid_path_segment("comlink"));
        assert!(!invalid_path_segment("uart-0"));
    }

    #[test]
    fn uniform_tick_accepts_a_single_shared_period() {
        assert_eq!(uniform_tick(&[1.0]), Ok(1.0));
        assert_eq!(uniform_tick(&[0.5, 0.5, 0.5]), Ok(0.5));
    }

    #[test]
    fn uniform_tick_rejects_mixed_periods() {
        // A 1.0 s controller stepped on a 0.1 s global tick would update
        // 10x too often — must be rejected, not silently mis-simulated.
        assert!(uniform_tick(&[0.1, 1.0]).is_err());
    }

    #[test]
    fn uniform_tick_rejects_empty_fleet() {
        assert!(uniform_tick(&[]).is_err());
    }

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
}
