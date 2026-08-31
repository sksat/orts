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
}

/// Config からプラグイン制御付き衛星を構築する。
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
    })
}

/// 1 制御サイクル分を積分し、コントローラを呼び出す。
pub fn step_controlled(
    sat: &mut ControlledSatellite,
    t: f64,
    dt_ctrl: f64,
    dt_ode: f64,
    epoch: Option<&Epoch>,
) -> Result<(), String> {
    let t_next = t + dt_ctrl;

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

    // 結合伝播（軌道 + 姿勢 + RW）。
    //
    // `try_integrate` を使うのは、`integrate` が不正な刻み幅や停滞した時刻を
    // panic にしてしまうため。この関数は `Result` を返すので、serve は
    // graceful-halt 経路でクライアントへ Error を送れる。
    sat.state = Rk4
        .try_integrate(
            &sat.dynamics,
            sat.state.clone(),
            t,
            t_next,
            dt_ode,
            |_, _| {},
        )
        .map_err(|e| format!("integration failed on [{t:.3}, {t_next:.3}]: {e}"))?;

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
