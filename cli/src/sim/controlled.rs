//! 離散制御 + ZOH 積分ループ。
//!
//! Config から宇宙機ダイナミクス + プラグインコントローラ + センサ + RW を
//! 組み立て、制御サンプル周期ごとに積分 -> センサ評価 -> プラグイン呼び出し ->
//! アクチュエータ更新 を繰り返す。`orts run` と `orts serve` の両方から使う。

use std::sync::Arc;

use arika::epoch::Epoch;
use orts::effector::AugmentedState;
use orts::orbital::gravity::GravityField;
use orts::plugin::{
    ActuatorBundle, ActuatorTelemetry, MtqCommand, PluginController, RwTelemetry, TickInput,
};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle, StarTracker};
use orts::setup::default_third_bodies;

use crate::sim::core::spacecraft_dynamics_for;
use nalgebra::Vector3;
use orts::spacecraft::{
    MtqAssembly, ReactionWheelAssembly, SpacecraftDynamics, SpacecraftState, ThrusterAssembly,
    ThrusterAssemblyCore, ThrusterSpec,
};
use tobari::magnetic::igrf::Igrf;
use utsuroi::{Integrator, Rk4};

use crate::config::{ControllerConfig, MtqConfig, ReactionWheelConfig, SensorChoice};
use crate::satellite::SatelliteSpec;
#[cfg(feature = "plugin-wasm")]
use crate::sim::params::ResolvedPluginBackend;
use crate::sim::params::SimParams;

#[cfg(feature = "plugin-wasm")]
use orts::plugin::wasm::WasmPluginCache;

/// Shared build context for constructing multiple controlled satellites.
///
/// Holds resources that should be shared across all satellites in a
/// simulation (e.g. the WASM engine + compiled component cache), so
/// that 1000 satellites don't each pay the full WASM compilation cost.
pub struct ControlledBuildContext<'a> {
    pub params: &'a SimParams,
    #[cfg(feature = "plugin-wasm")]
    pub wasm_cache: &'a mut WasmPluginCache,
    /// Which WASM backend to build controllers with. Resolved once by
    /// the caller (based on `--plugin-backend` and fleet size).
    #[cfg(feature = "plugin-wasm")]
    pub plugin_backend: ResolvedPluginBackend,
}

/// 制御付き衛星の状態。
pub struct ControlledSatellite {
    pub dynamics: SpacecraftDynamics<Box<dyn GravityField>>,
    pub state: AugmentedState<SpacecraftState>,
    pub controller: Box<dyn PluginController>,
    pub sensors: SensorBundle,
    pub actuators: ActuatorBundle,
    /// RW effector が登録されているかどうか。
    pub has_rw: bool,
    /// MTQ model が登録されているかどうか。
    pub has_mtq: bool,
    /// MTQ per-axis max moment [A·m²] (for rebuilding the model).
    pub mtq_max_moment: f64,
    /// Central body this satellite orbits.
    ///
    /// A commanded MTQ is rebuilt on every tick that carries a command, and the
    /// rebuild has to pick the same field model the first build did — so it
    /// needs the body, not just the moment.
    pub body: arika::body::KnownBody,
    /// Thruster specs (空なら thruster なし)。ZOH 境界で ThrusterAssembly を
    /// 作り直すために保持する。
    pub thruster_specs: Vec<ThrusterSpec>,
    /// Thruster assembly-level propellant floor [kg]。
    pub thruster_dry_mass: f64,
    /// Sim time this satellite's controller schedule is anchored at [s]: where
    /// the satellite entered the simulation.
    tick_base_t: f64,
    /// Ticks this controller has already run since `tick_base_t`.
    ///
    /// The schedule belongs to the satellite because `sample_period` is fixed
    /// per controller and a fleet may mix rates. Output samples, stream flushes
    /// and the end of a run all cut the timeline at their own boundaries, and
    /// keeping the count here is what stops those cuts from moving the
    /// controller.
    ///
    /// A count rather than a running `next_tick_t += sample_period`: the sum
    /// drifts by a few ULPs per tick, which then has to be absorbed by a
    /// tolerance on every boundary comparison — and a tolerance wide enough to
    /// cover the drift also fires ticks that lie just past the boundary.
    ticks_done: u64,
}

impl ControlledSatellite {
    /// Sim time of this satellite's next controller tick [s].
    pub fn next_tick_t(&self) -> f64 {
        self.tick_base_t + (self.ticks_done + 1) as f64 * self.controller.sample_period()
    }

    /// Whether the next tick lands at or before `t`.
    pub fn tick_due_at(&self, t: f64) -> bool {
        self.next_tick_t() <= t
    }
}

/// Reject a sample period too small to move the clock from `start_t`.
///
/// [`crate::config::validate_sample_period`] only asks for a positive finite
/// period, which `1e-16` satisfies — and `5.0 + 1e-16 == 5.0`, so the schedule
/// would never leave `start_t` and every loop waiting on it would spin.
///
/// What this guarantees is that the schedule leaves the anchor. A tick time is
/// `base + n · period`, and an ULP grows with the magnitude of the value, so a
/// period that clears one ULP at the anchor does not clear one at every later
/// tick: far enough out, `base + n · period` and `base + (n+1) · period` round
/// together. That is not a run anyone reaches — a 0.1 s period needs t of about
/// 1e15 s, some 32 million years, before an ULP catches up with it — and this
/// check does not rule it out. What it does rule out is a period that cannot
/// separate the first two ticks from each other at the start.
fn validate_tick_advances(start_t: f64, sample_period: f64) -> Result<(), String> {
    // At least one ULP, so the ticks this schedule generates near the start
    // land on different f64 values. Later ticks are the runtime guard's
    // business: the ULP grows with the time, and no check here can bound
    // that (see the doc above).
    //
    // Sampling the first few ticks instead is not enough. `start_t + period >
    // start_t` accepts a period that separates the first tick from the start
    // but not the ticks from each other (measured: at `start_t = 5.0` and
    // `period = 5e-16`, ticks 1 and 2 both round to 5.000000000000001).
    // Checking two ticks is not enough either: `3 · f64::EPSILON` is 0.75 ULP
    // there, so ticks 1 and 2 do advance while tick 3 rounds back onto tick 2
    // (5.000000000000002 twice), and the controller would run twice at one
    // instant.
    //
    // The ULP grows with the magnitude of the time, so a period accepted here
    // can still collide far enough into a run. That is the case the doc above
    // describes and does not rule out: a 0.1 s period needs t of about 1e15 s.
    // The resolution at the anchor, and at the first tick: crossing a binade
    // doubles the ULP, so a period equal to the anchor's can be half of one
    // just above it. Measured: at `start_t = 8.0f64.next_down()` with
    // `period = 8.0 - start_t`, ticks 1 and 2 both land on 8.0.
    let ulp = start_t.next_up() - start_t;
    let first = start_t + sample_period;
    let ulp_at_first = first.next_up() - first;
    if sample_period >= ulp && sample_period >= ulp_at_first {
        Ok(())
    } else {
        // Name the resolution that the period actually fell short of: at a
        // binade boundary the anchor's is met and the first tick's is not.
        let needed = ulp.max(ulp_at_first);
        Err(format!(
            "controller sample period {sample_period} is below the sim clock's \
             resolution around t={start_t} ({needed}), so consecutive ticks \
             would land on the same instant"
        ))
    }
}

/// Config からプラグイン制御付き衛星を構築する。
///
/// `start_t` is the sim time this satellite starts at [s]: 0 for a fleet built
/// before the run, the current sim time for one added to a running `serve`. It
/// sets the phase of the satellite's controller schedule.
///
/// 複数衛星をループで構築する場合は、[`ControlledBuildContext`] 内の
/// `wasm_cache` を使い回すことで WASM コンポーネントのコンパイルが
/// 1 ファイルにつき 1 回だけで済む。
/// `initial_epoch` is the wall-clock instant at which the orbital initial
/// state is evaluated: the simulation epoch for a satellite present from the
/// start, or the simulation epoch advanced by the current sim time for a
/// dynamic add (so a TLE/OMM is propagated to the moment it enters). The
/// dynamics themselves use `params.epoch` as the `t = 0` reference, so
/// time-dependent force models stay aligned regardless of when the satellite
/// is added.
pub fn build_controlled_satellite(
    spec: &SatelliteSpec,
    initial_epoch: Option<Epoch>,
    start_t: f64,
    ctx: &mut ControlledBuildContext<'_>,
) -> Result<ControlledSatellite, String> {
    let params = ctx.params;

    let att = spec
        .attitude_config
        .as_ref()
        .ok_or("controller requires attitude config")?;
    let ctrl_config = spec
        .controller_config
        .as_ref()
        .ok_or("controlled satellite requires controller config")?;

    let third_bodies = default_third_bodies(&params.body)
        .map_err(|e| format!("central body {}: {e}", params.body.properties().name))?;

    // Dynamics を構築。
    let mut dynamics = spacecraft_dynamics_for(spec, att, params, &third_bodies)?;

    // RW を追加。
    let has_rw = spec.rw_config.is_some();
    if let Some(rw_config) = &spec.rw_config {
        let rw = match rw_config {
            ReactionWheelConfig::ThreeAxis {
                inertia,
                max_momentum,
                max_torque,
                speed_control_gain,
            } => {
                let mut rw =
                    ReactionWheelAssembly::three_axis(*inertia, *max_momentum, *max_torque);
                if let Some(gain) = speed_control_gain {
                    rw.speed_control_gain = *gain;
                }
                rw
            }
        };
        dynamics = dynamics.with_effector(rw);
    }

    // MTQ を追加。
    let has_mtq = spec.mtq_config.is_some();
    let mtq_max_moment = match &spec.mtq_config {
        Some(MtqConfig::ThreeAxis { max_moment }) => {
            dynamics = dynamics.with_model(mtq_for_body(params.body, *max_moment, None, &spec.id));
            *max_moment
        }
        None => 0.0,
    };

    // Thruster を追加。
    let (thruster_specs, thruster_dry_mass) = if let Some(cfg) = &spec.thruster_config {
        let specs: Vec<ThrusterSpec> = cfg
            .thrusters
            .iter()
            .map(|t| {
                let mut s = ThrusterSpec::new(
                    t.thrust_n,
                    t.isp_s,
                    Vector3::from_row_slice(&t.direction_body),
                );
                if let Some(off) = t.offset_body {
                    s = s.with_offset(Vector3::from_row_slice(&off));
                }
                s
            })
            .collect();
        let core = ThrusterAssemblyCore::new(specs.clone(), cfg.dry_mass);
        dynamics = dynamics.with_model(ThrusterAssembly::new(core));
        (specs, cfg.dry_mass)
    } else {
        (Vec::new(), 0.0)
    };

    // 初期状態。`initial_epoch` で評価（動的追加なら epoch + current_t）。
    let orbit = spec.initial_state(params.mu, initial_epoch)?;
    let plant = SpacecraftState {
        orbit,
        attitude: orts::attitude::AttitudeState {
            quaternion: att.normalized_initial_quaternion(),
            angular_velocity: nalgebra::Vector3::from_row_slice(&att.initial_angular_velocity),
        },
        mass: att.mass,
    };
    let state = dynamics.initial_augmented_state(plant);

    // コントローラを構築（cache 経由）。宣言された stream-io stream も
    // ここで配線される（serve が WS endpoint として公開する）。
    let controller = build_controller(ctrl_config, &spec.id, &spec.streams, ctx)?;

    // センサを構築。
    let sensors = build_sensor_bundle(spec.sensor_choices.as_deref(), params.body, &spec.id)?;

    let actuators = ActuatorBundle::new();
    let sample_period = controller.sample_period();
    crate::config::validate_sample_period(sample_period)?;
    validate_tick_advances(start_t, sample_period)?;

    Ok(ControlledSatellite {
        dynamics,
        state,
        controller,
        sensors,
        actuators,
        has_rw,
        has_mtq,
        mtq_max_moment,
        body: params.body,
        thruster_specs,
        thruster_dry_mass,
        tick_base_t: start_t,
        ticks_done: 0,
    })
}

/// Push the commands the actuator bundle currently holds into the dynamics.
///
/// Called right after a controller tick, so any span propagated afterwards is
/// pure integration under a held command. A command the controller did not name
/// keeps its previous value — the zero-order hold the plugin contract promises —
/// so this is also what carries a command across a span with no tick in it.
fn apply_held_commands(sat: &mut ControlledSatellite) -> Result<(), String> {
    // 前 tick のコマンドで RW を設定。
    if sat.has_rw
        && sat.actuators.has_rw_command()
        && let Some(rw) = sat
            .dynamics
            .effector_by_name_mut::<ReactionWheelAssembly>("reaction_wheels")
    {
        use orts::plugin::RwCommand;
        if let Some(rw_cmd) = sat.actuators.rw_command() {
            let cmd_len = match rw_cmd {
                RwCommand::Torques(v) | RwCommand::Speeds(v) => v.len(),
            };
            if cmd_len != rw.wheels().len() {
                return Err(format!(
                    "rw command length ({}) != wheel count ({})",
                    cmd_len,
                    rw.wheels().len()
                ));
            }
            rw.command = rw_cmd.clone();
        }
    }

    // 前 tick のコマンドで MTQ を設定（モデルを差し替え）。
    if sat.has_mtq
        && sat.actuators.has_mtq_command()
        && let Some(mtq_cmd) = sat.actuators.mtq_command()
    {
        let cmd_len = match mtq_cmd {
            MtqCommand::Moments(v) | MtqCommand::NormalizedMoments(v) => v.len(),
        };
        let num_mtqs = orts::spacecraft::MtqAssemblyCore::three_axis(sat.mtq_max_moment).num_mtqs();
        if cmd_len != num_mtqs {
            return Err(format!(
                "mtq command length ({cmd_len}) != MTQ count ({num_mtqs})"
            ));
        }
        // Same factory as the initial build, so the field model stays the one
        // this body has.
        let rebuilt = mtq_for_body(
            sat.body,
            sat.mtq_max_moment,
            Some(&mtq_cmd.clone()),
            "mtq rebuild",
        );
        sat.dynamics.replace_model("mtq_assembly", rebuilt);
    }

    // 前 tick のコマンドで Thruster を設定（モデルを差し替え）。
    // TODO: specs.clone() のコストが気になったら、
    // dynamics.model_by_name_mut::<ThrusterAssembly>() を追加して
    // in-place で command だけ書き換える方式に移行する（MTQ も同様）。
    if !sat.thruster_specs.is_empty()
        && sat.actuators.has_thruster_command()
        && let Some(thruster_cmd) = sat.actuators.thruster_command()
    {
        use orts::plugin::ThrusterCommand;
        let ThrusterCommand::Throttles(v) = thruster_cmd;
        if v.len() != sat.thruster_specs.len() {
            return Err(format!(
                "thruster command length ({}) != thruster count ({})",
                v.len(),
                sat.thruster_specs.len()
            ));
        }
        let core = ThrusterAssemblyCore::new(sat.thruster_specs.clone(), sat.thruster_dry_mass);
        let mut assembly = ThrusterAssembly::new(core);
        assembly.command = thruster_cmd.clone();
        if sat
            .dynamics
            .replace_model("thruster_assembly", Box::new(assembly))
            .is_none()
        {
            return Err("thruster_assembly model not registered in dynamics".into());
        }
    }
    Ok(())
}

/// The next moment anything happens in a fleet: the earliest controller tick
/// due, or `horizon` if none falls before it.
///
/// An empty fleet has no tick, so the horizon is the answer. Taking the
/// shortest `sample_period` in the fleet instead — one tick rate for everyone
/// — ran every slower controller at that rate.
pub fn next_fleet_event_t(sats: &[ControlledSatellite], horizon: f64) -> f64 {
    sats.iter()
        .map(|sat| sat.next_tick_t())
        .fold(f64::INFINITY, f64::min)
        .min(horizon)
}

/// Advance one satellite across `[from, to]`.
///
/// Propagates to each tick due inside the span, ticks the controller there,
/// then propagates the rest of the span under the command that tick left. The
/// span is the caller's — `stream_interval` in `serve`, the gap between fleet
/// events in `run` — and has no reason to be a multiple of the controller's
/// period, so a span shorter than the period integrates without a tick.
pub fn advance_controlled(
    sat: &mut ControlledSatellite,
    from: f64,
    to: f64,
    params_dt: f64,
    epoch: Option<&Epoch>,
) -> Result<(), String> {
    let mut t = from;
    while sat.tick_due_at(to) {
        let tick_t = sat.next_tick_t();
        propagate_controlled(sat, t, tick_t, params_dt)?;
        tick_controller(sat, tick_t, epoch)?;
        t = tick_t;
    }
    propagate_controlled(sat, t, to, params_dt)
}

/// Integrate `[t0, t1]` under the command the actuators already hold.
///
/// No controller call: `t1` is wherever the caller needs the state next — an
/// output sample, a stream flush, the end of the run — and those boundaries do
/// not have to be controller ticks. `PluginController::sample_period` is a
/// *fixed* period, so a controller runs on its own schedule via
/// [`tick_controller`] and nothing else may move it.
///
/// `params_dt` is the requested ODE step; it is capped by the span so a step
/// cannot reach past `t1`.
pub fn propagate_controlled(
    sat: &mut ControlledSatellite,
    t0: f64,
    t1: f64,
    params_dt: f64,
) -> Result<(), String> {
    if t1 <= t0 {
        return Ok(());
    }
    let dt_ode = params_dt.min(t1 - t0);
    // `try_integrate` rather than `integrate`: the latter panics on a bad step
    // or a stalled clock, and this returns `Result` so serve can send the
    // client an Error down its graceful-halt path.
    sat.state = Rk4
        .try_integrate(&sat.dynamics, sat.state.clone(), t0, t1, dt_ode, |_, _| {})
        .map_err(|e| format!("integration failed on [{t0:.3}, {t1:.3}]: {e}"))?;
    Ok(())
}

/// Run one controller tick at `t_next`: read the sensors, call the plugin, and
/// hand the command it returns to the dynamics.
///
/// The state must already have been propagated to `t_next` — the sensors are
/// read from it.
pub fn tick_controller(
    sat: &mut ControlledSatellite,
    t_next: f64,
    epoch: Option<&Epoch>,
) -> Result<(), String> {
    // センサ評価 + プラグイン呼び出し。
    let current_epoch = epoch.map(|e| e.add_si_seconds(t_next));
    let sensors = sat
        .sensors
        .evaluate(&sat.state.plant, &current_epoch.unwrap_or(Epoch::j2000()));
    let actuator_telemetry = ActuatorTelemetry {
        rw: if sat.has_rw {
            sat.dynamics
                .effector_by_name::<ReactionWheelAssembly>("reaction_wheels")
                .map(|rw| {
                    let core = rw.core();
                    let momentum = core.momentum_slice(&sat.state.aux);
                    RwTelemetry {
                        momentum: momentum.to_vec(),
                        speeds: momentum
                            .iter()
                            .zip(rw.wheels())
                            .map(|(h, w)| w.speed_from_momentum(*h))
                            .collect(),
                        realized_torques: core
                            .realized_torque_slice(&sat.state.aux)
                            .map(|s| s.to_vec()),
                    }
                })
        } else {
            None
        },
    };
    let input = TickInput {
        t: t_next,
        epoch: current_epoch.as_ref(),
        sensors: &sensors,
        actuators: &actuator_telemetry,
        spacecraft: &sat.state.plant,
    };
    if let Some(cmd) = sat
        .controller
        .update(&input)
        .map_err(|e| format!("controller error at t={t_next:.3}: {e}"))?
    {
        sat.actuators
            .apply(&cmd)
            .map_err(|e| format!("actuator error at t={t_next:.3}: {e}"))?;
    }
    // Only once the whole tick has landed: `apply_held_commands` can reject a
    // command whose length does not match the actuator, and a schedule advanced
    // past a tick that failed would resume on the wrong phase.
    apply_held_commands(sat)?;
    // The schedule has to advance, or the caller's `while sat.tick_due_at(to)`
    // would tick again at this instant and never finish. Construction refuses
    // a period below the clock's resolution there, but that resolution doubles
    // at each binade boundary, so a period wide enough at the anchor can be
    // too narrow further along. The invariant is checked where it has to hold
    // rather than only where the satellite was built.
    let taken = sat.next_tick_t();
    sat.ticks_done += 1;
    let following = sat.next_tick_t();
    // `partial_cmp` rather than `!(following > taken)`: a NaN either side is
    // incomparable, and that has to count as "did not advance" too.
    if following.partial_cmp(&taken) != Some(core::cmp::Ordering::Greater) {
        return Err(format!(
            "controller sample period {} cannot advance the schedule past \
             t={taken}: the next tick lands on the same instant",
            sat.controller.sample_period()
        ));
    }
    Ok(())
}

// builder helpers

fn build_controller(
    config: &ControllerConfig,
    label: &str,
    streams: &[String],
    ctx: &mut ControlledBuildContext<'_>,
) -> Result<Box<dyn PluginController>, String> {
    match config {
        #[cfg(feature = "plugin-wasm")]
        ControllerConfig::Wasm { path, config } => {
            // An omitted `[satellites.controller.config]` deserializes to
            // `Value::Null`, whose `to_string()` is `"null"` — not something a
            // guest can parse as its config struct. `Plugin::init` takes the
            // empty string to mean "use the defaults", which is what an absent
            // config block asks for.
            let config_str = if config.is_null() {
                String::new()
            } else {
                config.to_string()
            };
            let wasm_path = std::path::Path::new(path);
            match ctx.plugin_backend {
                ResolvedPluginBackend::Sync => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_sync_controller_with_streams(
                            wasm_path,
                            label,
                            &config_str,
                            streams.to_vec(),
                        )
                        .map_err(|e| format!("WasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
                #[cfg(feature = "plugin-wasm-async")]
                ResolvedPluginBackend::Async => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_async_controller_with_streams(
                            wasm_path,
                            label,
                            &config_str,
                            streams.to_vec(),
                        )
                        .map_err(|e| format!("AsyncWasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
            }
        }
        #[cfg(not(feature = "plugin-wasm"))]
        ControllerConfig::Wasm { .. } => {
            let _ = ctx;
            let _ = label;
            let _ = streams;
            Err("WASM controller requires the 'plugin-wasm' feature. \
             Rebuild with: cargo build --features plugin-wasm"
                .to_string())
        }
    }
}

/// The magnetorquer assembly for `body`, with `command` already applied.
///
/// The assembly holds its field model as a type parameter, so the two cases are
/// two types; boxing them here keeps the choice in one place. Both the initial
/// build and the per-command rebuild go through this, so a magnetorquer cannot
/// end up on Earth's field after the first command.
fn mtq_for_body(
    body: arika::body::KnownBody,
    max_moment: f64,
    command: Option<&MtqCommand>,
    sat_id: &str,
) -> Box<dyn orts::model::Model<orts::spacecraft::SpacecraftState>> {
    if body_field_is_modelled(body, "magnetorquer", "its torque is zero", sat_id) {
        let mut mtq = MtqAssembly::three_axis(max_moment, Igrf::earth());
        if let Some(cmd) = command {
            mtq.command = cmd.clone();
        }
        Box::new(mtq)
    } else {
        let mut mtq = MtqAssembly::three_axis(max_moment, tobari::magnetic::NoField);
        if let Some(cmd) = command {
            mtq.command = cmd.clone();
        }
        Box::new(mtq)
    }
}

/// Whether `body`'s magnetic field is modelled, warning once per device if not.
///
/// `Igrf` and `TiltedDipole` are Earth's, and they are the only field models
/// there are. Around another body there is no model to evaluate, and the
/// stand-in is zero: the device is built either way — the same spacecraft
/// definition can be pointed at any body — and is inert there rather than
/// driven by a field measured somewhere else. `effect` says what that means for
/// this device, since a controller that steers on the field goes quiet without
/// failing.
fn body_field_is_modelled(
    body: arika::body::KnownBody,
    device: &str,
    effect: &str,
    sat_id: &str,
) -> bool {
    if body == arika::body::KnownBody::Earth {
        return true;
    }
    log::warn!(
        "{sat_id}: {device} has no magnetic field model for {} (only Earth's is modelled), \
         so {effect}. Control that steers on the field, such as B-dot, has nothing to act on.",
        body.properties().name
    );
    false
}

/// Build the declared sensors for a satellite about `body`.
///
/// The sun sensor's reading is a direction to the Sun, so it depends on the
/// central body the same way the solar force models do.
fn build_sensor_bundle(
    choices: Option<&[SensorChoice]>,
    body: arika::body::KnownBody,
    sat_id: &str,
) -> Result<SensorBundle, String> {
    let choices = match choices {
        Some(c) => c,
        None => return Ok(SensorBundle::new()),
    };

    let magnetometers = if choices.contains(&SensorChoice::Magnetometer) {
        let field: Arc<dyn tobari::magnetic::MagneticFieldModel> =
            if body_field_is_modelled(body, "magnetometer", "its reading is zero", sat_id) {
                Arc::new(Igrf::earth())
            } else {
                Arc::new(tobari::magnetic::NoField)
            };
        vec![Magnetometer::new(field)]
    } else {
        vec![]
    };

    Ok(SensorBundle {
        magnetometers,
        gyroscopes: if choices.contains(&SensorChoice::Gyroscope) {
            vec![Gyroscope::new()]
        } else {
            vec![]
        },
        star_trackers: if choices.contains(&SensorChoice::StarTracker) {
            vec![StarTracker::new()]
        } else {
            vec![]
        },
        sun_sensors: if choices.contains(&SensorChoice::SunSensor) {
            vec![orts::sensor::SunSensor::for_body(body).map_err(|e| format!("sun sensor: {e}"))?]
        } else {
            vec![]
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::body::KnownBody;
    use orts::plugin::{Command, PluginError, TickInput};

    /// A controller that records the `t` of every tick it is given.
    ///
    /// The point of the tests below is *when* the controller runs, so it
    /// commands nothing and only keeps the schedule it saw.
    struct TickRecorder {
        period: f64,
        ticks: Arc<std::sync::Mutex<Vec<f64>>>,
    }

    impl PluginController for TickRecorder {
        fn name(&self) -> &str {
            "tick-recorder"
        }
        fn sample_period(&self) -> f64 {
            self.period
        }
        fn update(&mut self, input: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
            self.ticks
                .lock()
                .expect("no panics in these tests")
                .push(input.t);
            Ok(None)
        }
    }

    /// Build a controlled satellite at 400 km with the given controller.
    ///
    /// Mirrors the dynamics `build_controlled_satellite` assembles, minus the
    /// actuators and the WASM plugin the real builder needs: what is under test
    /// is the tick schedule, and no command is ever issued.
    fn satellite_with(
        period: f64,
        start_t: f64,
    ) -> (ControlledSatellite, Arc<std::sync::Mutex<Vec<f64>>>) {
        use orts::orbital::OrbitalState;
        use orts::spacecraft::SpacecraftState;

        let ticks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let controller = TickRecorder {
            period,
            ticks: Arc::clone(&ticks),
        };

        let body = arika::body::KnownBody::Earth;
        let mu = body.properties().mu;
        let inertia = nalgebra::Matrix3::identity() * 10.0;
        let dynamics = orts::setup::build_spacecraft_dynamics(
            &body,
            mu,
            None,
            &orts::setup::SatelliteParams {
                has_drag: false,
                ballistic_coeff: None,
                srp_area_to_mass: None,
                srp_cr: None,
                // This test watches the controller's tick cadence, so the plant
                // carries no disturbance torque and no panels to make one.
                disturbances: orts::setup::DisturbanceTorques::default(),
                shape: None,
            },
            &[],
            inertia,
            None,
        )
        .expect("Earth has a Sun ephemeris");

        // Circular orbit 400 km up, in the equatorial plane.
        let r = body.properties().radius + 400.0;
        let v = (mu / r).sqrt();
        let plant = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0)),
            attitude: orts::attitude::AttitudeState {
                quaternion: nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        };
        let state = dynamics.initial_augmented_state(plant);

        let sat = ControlledSatellite {
            dynamics,
            state,
            controller: Box::new(controller),
            sensors: SensorBundle::default(),
            actuators: ActuatorBundle::new(),
            has_rw: false,
            has_mtq: false,
            mtq_max_moment: 0.0,
            body: arika::body::KnownBody::Earth,
            thruster_specs: Vec::new(),
            thruster_dry_mass: 0.0,
            tick_base_t: start_t,
            ticks_done: 0,
        };
        (sat, ticks)
    }

    /// Step one satellite the way a caller does — through the same function
    /// `serve` and `run` call, so an error in that loop fails these tests.
    fn advance(sat: &mut ControlledSatellite, from: f64, to: f64, params_dt: f64) {
        advance_controlled(sat, from, to, params_dt, None).expect("integrates and ticks");
    }

    /// A span shorter than the sample period does not tick the controller.
    ///
    /// This is the serve case: `stream_interval` cuts the timeline every
    /// 0.01 s while the controller asks for 0.1 s. The old loop called the
    /// controller once per cut with `dt = 0.01`, so a 10 Hz controller ran at
    /// 100 Hz and held each command for a tenth of the time it asked for.
    #[test]
    fn spans_shorter_than_the_period_do_not_tick_the_controller() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        // 1 s of sim time, cut into 0.01 s spans.
        let mut t = 0.0;
        for _ in 0..100 {
            advance(&mut sat, t, t + 0.01, 0.01);
            t += 0.01;
        }

        let seen = ticks.lock().unwrap().clone();
        assert_eq!(
            seen.len(),
            10,
            "10 Hz over 1 s is 10 ticks, got {} at {seen:?}",
            seen.len()
        );
        for (i, tick_t) in seen.iter().enumerate() {
            let expected = 0.1 * (i + 1) as f64;
            assert!(
                (tick_t - expected).abs() < 1e-9,
                "tick {i} at {tick_t}, expected {expected}"
            );
        }
    }

    /// Ticks stay on the controller's own phase across spans that do not
    /// divide it.
    ///
    /// 0.03 s spans never land on a 0.1 s tick, so every tick falls inside a
    /// span. Truncating the remainder instead of carrying it would drift the
    /// schedule.
    #[test]
    fn a_span_that_does_not_divide_the_period_keeps_the_phase() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        let mut t = 0.0;
        for _ in 0..10 {
            advance(&mut sat, t, t + 0.03, 0.01);
            t += 0.03;
        }
        // 0.3 s of sim time: ticks at 0.1, 0.2, 0.3.
        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 3, "expected 3 ticks in 0.3 s, got {seen:?}");
        assert!((seen[2] - 0.3).abs() < 1e-9, "third tick at {}", seen[2]);
    }

    /// A satellite added mid-run takes its first tick one period after it
    /// enters, not one period after `t = 0`.
    ///
    /// Pins the phase rule, not the wiring: the fixture below anchors the
    /// schedule the same way `build_controlled_satellite` does, so a caller
    /// passing the wrong `start_t` would still pass here. Reaching the real
    /// builder needs a WASM plugin to construct the controller from.
    #[test]
    fn a_satellite_starting_late_phases_its_ticks_from_its_start() {
        let (sat, _) = satellite_with(0.1, 5.0);
        assert!(
            (sat.next_tick_t() - 5.1).abs() < 1e-9,
            "first tick at {}, expected 5.1",
            sat.next_tick_t()
        );
    }

    /// A span that stops just short of the next tick does not run it.
    ///
    /// The end of a run, or a stream flush that lands a hair before a tick,
    /// must leave that tick for the next span. Comparing with a tolerance —
    /// which a drifting `next_tick_t += period` needs — would fire it here and
    /// integrate past the boundary the caller asked for.
    #[test]
    fn a_span_ending_just_before_a_tick_does_not_run_it() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);

        advance(&mut sat, 0.0, 0.1 - 1e-10, 0.01);
        assert!(
            ticks.lock().unwrap().is_empty(),
            "a tick 1e-10 s past the span's end was run: {:?}",
            ticks.lock().unwrap()
        );

        // And it is still there for the span that does contain it.
        advance(&mut sat, 0.1 - 1e-10, 0.15, 0.01);
        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "the deferred tick should run next: {seen:?}");
        assert!((seen[0] - 0.1).abs() < 1e-12, "at {}", seen[0]);
    }

    /// Repeated ticks do not drift off the period.
    ///
    /// `tick_base_t + n · period` is one multiply; summing `period` a thousand
    /// times is not, and the accumulated error is what a boundary tolerance
    /// would have had to cover.
    #[test]
    fn the_thousandth_tick_is_still_on_the_period() {
        let (mut sat, ticks) = satellite_with(0.1, 0.0);
        advance(&mut sat, 0.0, 100.0, 1.0);

        let seen = ticks.lock().unwrap().clone();
        assert_eq!(seen.len(), 1000, "100 s at 10 Hz is 1000 ticks");
        assert_eq!(
            seen[999], 100.0,
            "the last tick should be exactly 100.0, got {}",
            seen[999]
        );
    }

    /// A period the sim clock cannot resolve is refused, not spun on.
    ///
    /// `validate_sample_period` accepts any positive finite period, and
    /// `5.0 + 1e-16 == 5.0`: a schedule anchored there would never advance and
    /// every loop waiting on the next tick would run forever.
    #[test]
    fn a_period_below_the_clock_resolution_is_rejected() {
        validate_tick_advances(5.0, 1e-16)
            .expect_err("a period that cannot move the clock must be refused");
        // Separates the first tick from the start, but not the ticks from each
        // other: `5.0 + 1*5e-16` and `5.0 + 2*5e-16` both round to
        // 5.000000000000001, so accepting it would tick twice at one instant.
        assert_ne!(5.0 + 5e-16, 5.0, "precondition: the first tick does move");
        assert_eq!(
            5.0 + 5e-16,
            5.0 + 2.0 * 5e-16,
            "precondition: the first two ticks land on the same f64"
        );
        validate_tick_advances(5.0, 5e-16)
            .expect_err("a period that cannot separate two ticks must be refused");
        // 0.75 ULP at 5.0: the first two ticks advance, and the third rounds
        // back onto the second. Sampling a fixed number of ticks would accept
        // this; requiring one ULP refuses it.
        let three_eps = 3.0 * f64::EPSILON;
        assert_ne!(5.0 + three_eps, 5.0, "precondition: tick 1 moves");
        assert_ne!(
            5.0 + 2.0 * three_eps,
            5.0 + three_eps,
            "precondition: tick 2 moves"
        );
        assert_eq!(
            5.0 + 3.0 * three_eps,
            5.0 + 2.0 * three_eps,
            "precondition: tick 3 lands back on tick 2"
        );
        validate_tick_advances(5.0, three_eps).expect_err("a period below one ULP must be refused");
        // One ULP exactly is the smallest period that keeps the ticks apart.
        validate_tick_advances(5.0, 5.0f64.next_up() - 5.0)
            .expect("one ULP separates every consecutive pair");
        // Just below a binade boundary the ULP doubles on the way up, so a
        // period equal to the anchor's own ULP is half of one above it.
        let below_eight = 8.0f64.next_down();
        let one_ulp_there = 8.0 - below_eight;
        assert_eq!(
            below_eight + one_ulp_there,
            below_eight + 2.0 * one_ulp_there,
            "precondition: ticks 1 and 2 both land on 8.0"
        );
        let err = validate_tick_advances(below_eight, one_ulp_there)
            .expect_err("a period that collides across the binade must be refused");
        // The message has to name the bound that failed: here the anchor's own
        // resolution is met and the first tick's is not.
        assert!(
            err.contains(&format!("{}", 8.0f64.next_up() - 8.0)),
            "the message should report the first tick's resolution: {err}"
        );
        // The same period is fine from zero, where it is representable.
        validate_tick_advances(0.0, 1e-16).expect("representable at t=0");
        validate_tick_advances(5.0, 0.1).expect("an ordinary period is fine");
    }

    /// Two controllers at different rates each run at their own.
    ///
    /// `orts run` used to drive the fleet on the shortest period, so the 1.0 s
    /// controller here was called every 0.1 s — the very case the streams path
    /// rejects outright rather than mis-simulate.
    #[test]
    fn a_mixed_rate_fleet_ticks_each_controller_at_its_own_period() {
        let (fast, fast_ticks) = satellite_with(0.1, 0.0);
        let (slow, slow_ticks) = satellite_with(1.0, 0.0);

        // The event times come from the same function the run loop calls, so
        // an error in that choice fails this test.
        let mut fleet = vec![fast, slow];
        let mut t = 0.0;
        while t < 1.0 - 1e-12 {
            let next_t = next_fleet_event_t(&fleet, 1.0);
            for sat in &mut fleet {
                propagate_controlled(sat, t, next_t, 0.01).expect("integrates");
                if sat.tick_due_at(next_t) {
                    tick_controller(sat, next_t, None).expect("ticks");
                }
            }
            t = next_t;
        }

        assert_eq!(
            fast_ticks.lock().unwrap().len(),
            10,
            "the 0.1 s controller should tick 10 times in 1 s"
        );
        let slow_seen = slow_ticks.lock().unwrap().clone();
        assert_eq!(
            slow_seen.len(),
            1,
            "the 1.0 s controller should tick once in 1 s, got {slow_seen:?}"
        );
        assert!((slow_seen[0] - 1.0).abs() < 1e-9, "at {}", slow_seen[0]);
    }

    /// A satellite at `position`, its body axes aligned with the inertial ones.
    fn state_at(position: nalgebra::Vector3<f64>) -> orts::spacecraft::SpacecraftState {
        SpacecraftState {
            orbit: orts::orbital::OrbitalState::new(
                position,
                nalgebra::Vector3::new(0.0, 3.0, 0.0),
            ),
            attitude: orts::attitude::AttitudeState {
                quaternion: nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: nalgebra::Vector3::zeros(),
            },
            mass: 100.0,
        }
    }

    fn sun_direction_read_by(
        bundle: &mut SensorBundle,
        state: &orts::spacecraft::SpacecraftState,
        epoch: &Epoch,
    ) -> Option<nalgebra::Vector3<f64>> {
        match bundle.sun_sensors[0].measure(state, epoch) {
            orts::plugin::SunSensorOutput::Fine { direction, .. } => {
                direction.map(|d| d.into_inner().into_inner())
            }
            other => panic!("the CLI builds a fine sensor, got {other:?}"),
        }
    }

    fn angle_deg(a: &nalgebra::Vector3<f64>, b: &nalgebra::Vector3<f64>) -> f64 {
        a.normalize()
            .dot(&b.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    /// The CLI's sun sensor reads the Sun from the central body.
    ///
    /// `SunSensor::for_body`'s own tests pass whichever way this line is
    /// wired, so a regression to `SunSensor::new()` — Earth's Sun, no shadow —
    /// would go unnoticed. Both halves are measured on 2026-03-20, when Mars'
    /// Sun direction is 152.8° from Earth's.
    #[test]
    fn the_cli_sun_sensor_reads_the_sun_from_the_central_body() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars_sun = arika::sun::sun_position_from_body(KnownBody::Mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner();
        let earth_sun = arika::sun::sun_position_eci(&epoch.to_tdb()).into_inner();

        // 20 000 km sunward of Mars, so nothing eclipses the satellite and the
        // direction to Mars' Sun is parallel to `mars_sun`.
        let sunward_of_mars = state_at(mars_sun.normalize() * 20_000.0);
        let mut on_mars = build_sensor_bundle(
            Some(&[SensorChoice::SunSensor]),
            KnownBody::Mars,
            "sat-test",
        )
        .expect("Mars has a Sun ephemeris");
        let read = sun_direction_read_by(&mut on_mars, &sunward_of_mars, &epoch)
            .expect("sunward of Mars, so lit");
        assert!(
            angle_deg(&read, &mars_sun) < 1.0e-4,
            "the reading follows Mars' Sun: {:.6}° away",
            angle_deg(&read, &mars_sun)
        );
        // What the geocentric wiring would have read instead.
        assert!(
            angle_deg(&read, &earth_sun) > 150.0,
            "Earth's Sun is nowhere near it: {:.3}°",
            angle_deg(&read, &earth_sun)
        );

        // On Earth the same wiring carries Earth's conical shadow, so a
        // satellite directly behind the Earth reads no direction at all.
        let behind_earth = state_at(-earth_sun.normalize() * 7000.0);
        let mut on_earth = build_sensor_bundle(
            Some(&[SensorChoice::SunSensor]),
            KnownBody::Earth,
            "sat-test",
        )
        .expect("Earth has a Sun ephemeris");
        assert!(
            sun_direction_read_by(&mut on_earth, &behind_earth, &epoch).is_none(),
            "eclipsed, so the sensor reports no Sun direction"
        );
    }

    /// A magnetometer on a body with no field model reads zero.
    ///
    /// `Igrf` and `TiltedDipole` are Earth's, and they are the only field
    /// models there are. The device is still built — the same spacecraft
    /// definition can be pointed at any body — and reads zero there instead of
    /// reporting a field the body does not have.
    #[test]
    fn a_magnetometer_reads_zero_where_no_field_is_modelled() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let state = state_at(nalgebra::Vector3::new(7000.0, 0.0, 0.0));

        for body in [KnownBody::Mars, KnownBody::Moon, KnownBody::Sun] {
            let mut bundle =
                build_sensor_bundle(Some(&[SensorChoice::Magnetometer]), body, "sat-test")
                    .unwrap_or_else(|e| panic!("{body:?} builds a magnetometer: {e}"));
            let reading = bundle.magnetometers[0]
                .measure(&state, &epoch)
                .into_inner()
                .into_inner();
            assert_eq!(
                reading,
                nalgebra::Vector3::zeros(),
                "{body:?} has no field model, so the reading is zero"
            );
        }

        let mut on_earth = build_sensor_bundle(
            Some(&[SensorChoice::Magnetometer]),
            KnownBody::Earth,
            "sat-test",
        )
        .expect("Earth's field is modelled");
        let earth_reading = on_earth.magnetometers[0]
            .measure(&state, &epoch)
            .into_inner()
            .into_inner();
        assert!(
            earth_reading.norm() > 0.0,
            "Earth's field is modelled, so the reading is not zero: {earth_reading:?}"
        );

        // A sensor that needs no field is unaffected whatever the body is.
        assert!(
            build_sensor_bundle(
                Some(&[SensorChoice::Gyroscope]),
                KnownBody::Mars,
                "sat-test"
            )
            .is_ok(),
            "a gyroscope needs no field"
        );
    }

    /// The field a magnetorquer gets is decided by its body, on both paths.
    ///
    /// `mtq_for_body` is what the initial build and the per-command rebuild
    /// both call, so this ties the body to the installed model rather than to a
    /// flag the test set itself. Measured through the torque the assembly
    /// reports for the same command: zero on Mars, non-zero on Earth.
    #[test]
    fn a_magnetorquer_takes_the_field_of_its_body_on_both_paths() {
        let command = MtqCommand::NormalizedMoments(vec![1.0, 0.0, 0.0]);

        // The state and dynamics are the same for both bodies; only the field
        // model differs, so the torque is what the choice decides.
        let torque_with = |model: Box<dyn orts::model::Model<SpacecraftState>>| -> f64 {
            let (mut sat, _ticks) = satellite_with(1.0, 0.0);
            sat.dynamics = sat
                .dynamics
                .with_epoch(Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0))
                .with_model(model);
            sat.dynamics
                .model_breakdown(0.0, &sat.state.plant)
                .into_iter()
                .find(|(name, _)| *name == "mtq_assembly")
                .expect("the assembly is installed")
                .1
                .torque_body
                .inner()
                .norm()
        };

        assert_eq!(
            torque_with(mtq_for_body(
                KnownBody::Mars,
                10.0,
                Some(&command),
                "sat-test"
            )),
            0.0,
            "Mars has no field model, so a commanded magnetorquer makes no torque"
        );
        assert!(
            torque_with(mtq_for_body(
                KnownBody::Earth,
                10.0,
                Some(&command),
                "sat-test"
            )) > 0.0,
            "Earth's field is modelled, so the same command makes torque"
        );
    }

    /// The rebuild after a command goes through the same factory.
    ///
    /// Measured: a satellite whose body has no field model keeps zero torque
    /// after `apply_held_commands` installs the command. Pointing the rebuild
    /// at Earth's field fails this.
    #[test]
    fn a_commanded_magnetorquer_keeps_the_field_of_its_body() {
        let (mut sat, _ticks) = satellite_with(1.0, 0.0);
        sat.body = KnownBody::Mars;
        sat.has_mtq = true;
        sat.mtq_max_moment = 10.0;
        sat.dynamics = sat
            .dynamics
            .with_epoch(Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0))
            .with_model(mtq_for_body(sat.body, 10.0, None, "sat-test"));

        sat.actuators
            .apply(&Command::mtq_normalized(vec![1.0, 0.0, 0.0]))
            .expect("three moments for three MTQs");
        apply_held_commands(&mut sat).expect("the command length matches the MTQ count");

        let torque = sat
            .dynamics
            .model_breakdown(0.0, &sat.state.plant)
            .into_iter()
            .find(|(name, _)| *name == "mtq_assembly")
            .expect("the assembly is installed")
            .1
            .torque_body
            .inner()
            .norm();
        assert_eq!(
            torque, 0.0,
            "Mars has no field model, so the rebuilt assembly makes no torque"
        );
    }

    /// A magnetorquer on a body with no field model does not block the build.
    ///
    /// The assembly is built with `NoField` there, so its torque is `m × 0`.
    /// Built from a config, the way a user reaches it: the controller points at
    /// a path that does not exist and actuators are built first, so reaching
    /// the plugin error is what says the magnetorquer let the build through.
    #[test]
    fn a_magnetorquer_is_inert_where_no_field_is_modelled() {
        let config_for = |body: &str| -> crate::config::SimConfig {
            toml::from_str(&format!(
                r#"
dt = 1.0
body = "{body}"

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 400.0

[satellites.attitude]
inertia_diag = [10.0, 10.0, 10.0]
mass = 100.0

[satellites.magnetorquers]
type = "three_axis"
max_moment = 0.2

[satellites.controller]
type = "wasm"
path = "does-not-exist.wasm"
"#
            ))
            .expect("the config parses")
        };

        let build = |body: &str| {
            let config = config_for(body);
            let params = SimParams::from_config(&config);
            let spec = params.satellites[0].clone();
            #[cfg(feature = "plugin-wasm")]
            let mut cache =
                orts::plugin::wasm::WasmPluginCache::new().expect("a cache needs no plugin file");
            let mut ctx = ControlledBuildContext {
                params: &params,
                #[cfg(feature = "plugin-wasm")]
                wasm_cache: &mut cache,
                #[cfg(feature = "plugin-wasm")]
                plugin_backend: params.resolve_plugin_backend(),
            };
            build_controlled_satellite(&spec, None, 0.0, &mut ctx).map(|_| ())
        };

        // Every body reaches the plugin path, so no body is stopped by its
        // magnetorquer.
        for body in ["mars", "moon", "earth"] {
            let err = build(body).expect_err("the plugin path does not exist");
            assert!(
                !err.contains("magnetorquer"),
                "{body}: the magnetorquer should not stop the build: {err}"
            );
        }
    }
}
