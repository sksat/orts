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
    // The first two tick times the schedule will generate, formed the way
    // `next_tick_t` forms them. Both have to be distinct: requiring only
    // `start_t + period > start_t` accepts a period that separates the first
    // tick from the start but not the ticks from each other — measured, at
    // `start_t = 5.0` and `period = 5e-16` both of these round to
    // 5.000000000000001, so the controller would tick twice at one instant.
    let first = start_t + sample_period;
    let second = start_t + 2.0 * sample_period;
    if first > start_t && second > first {
        Ok(())
    } else {
        Err(format!(
            "controller sample period {sample_period} is below the sim clock's \
             resolution at t={start_t}, so its schedule could never advance"
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

    let third_bodies = default_third_bodies(&params.body);

    // Dynamics を構築。
    let mut dynamics = spacecraft_dynamics_for(spec, att, params, &third_bodies);

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
            let mtq = MtqAssembly::three_axis(*max_moment, Igrf::earth());
            dynamics = dynamics.with_model(mtq);
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
    let sensors = build_sensor_bundle(spec.sensor_choices.as_deref());

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
        let mut mtq = MtqAssembly::three_axis(sat.mtq_max_moment, Igrf::earth());
        if cmd_len != mtq.core().num_mtqs() {
            return Err(format!(
                "mtq command length ({}) != MTQ count ({})",
                cmd_len,
                mtq.core().num_mtqs()
            ));
        }
        mtq.command = mtq_cmd.clone();
        sat.dynamics.replace_model("mtq_assembly", Box::new(mtq));
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
    sat.ticks_done += 1;
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

fn build_sensor_bundle(choices: Option<&[SensorChoice]>) -> SensorBundle {
    let choices = match choices {
        Some(c) => c,
        None => return SensorBundle::new(),
    };

    let field_model: Arc<dyn tobari::magnetic::MagneticFieldModel> = Arc::new(Igrf::earth());

    SensorBundle {
        magnetometers: if choices.contains(&SensorChoice::Magnetometer) {
            vec![Magnetometer::new(Arc::clone(&field_model))]
        } else {
            vec![]
        },
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
            vec![orts::sensor::SunSensor::new()]
        } else {
            vec![]
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        );

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
}
