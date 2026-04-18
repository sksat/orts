//! 離散制御 + ZOH 積分ループ。
//!
//! Config から宇宙機ダイナミクス + プラグインコントローラ + センサ + RW を
//! 組み立て、制御サンプル周期ごとに積分 -> センサ評価 -> プラグイン呼び出し ->
//! アクチュエータ更新 を繰り返す。`orts run` と `orts serve` の両方から使う。

use std::sync::Arc;

use arika::epoch::Epoch;
use orts::attitude::CoupledGravityGradient;
use orts::effector::AugmentedState;
use orts::orbital::gravity::GravityField;
use orts::plugin::{
    ActuatorBundle, ActuatorTelemetry, MtqCommand, PluginController, RwTelemetry, TickInput,
};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle, StarTracker};
use orts::setup::{build_spacecraft_dynamics, default_third_bodies};

use crate::sim::core::sat_params;
use orts::spacecraft::{MtqAssembly, ReactionWheelAssembly, SpacecraftDynamics, SpacecraftState};
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
}

/// Config からプラグイン制御付き衛星を構築する。
///
/// 複数衛星をループで構築する場合は、[`ControlledBuildContext`] 内の
/// `wasm_cache` を使い回すことで WASM コンポーネントのコンパイルが
/// 1 ファイルにつき 1 回だけで済む。
pub fn build_controlled_satellite(
    spec: &SatelliteSpec,
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

    let inertia = att.inertia_matrix();
    let third_bodies = default_third_bodies(&params.body);

    // Dynamics を構築。
    let mut dynamics = build_spacecraft_dynamics(
        &params.body,
        params.mu,
        params.epoch,
        &sat_params(spec),
        &third_bodies,
        inertia,
        params.build_atmosphere_model(),
    );
    dynamics = dynamics.with_model(CoupledGravityGradient::new(params.mu, inertia));

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

    // 初期状態。
    let orbit = spec.initial_state(params.mu);
    let plant = SpacecraftState {
        orbit,
        attitude: orts::attitude::AttitudeState {
            quaternion: nalgebra::Vector4::from_row_slice(&att.initial_quaternion),
            angular_velocity: nalgebra::Vector3::from_row_slice(&att.initial_angular_velocity),
        },
        mass: att.mass,
    };
    let state = dynamics.initial_augmented_state(plant);

    // コントローラを構築（cache 経由）。
    let controller = build_controller(ctrl_config, &spec.id, ctx)?;

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

    // 結合伝播（軌道 + 姿勢 + RW）。
    sat.state = Rk4.integrate(
        &sat.dynamics,
        sat.state.clone(),
        t,
        t_next,
        dt_ode,
        |_, _| {},
    );

    // センサ評価 + プラグイン呼び出し。
    let current_epoch = epoch.map(|e| e.add_seconds(t_next));
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

// --- builder helpers ---------------------------------------------------------

fn build_controller(
    config: &ControllerConfig,
    label: &str,
    ctx: &mut ControlledBuildContext<'_>,
) -> Result<Box<dyn PluginController>, String> {
    match config {
        #[cfg(feature = "plugin-wasm")]
        ControllerConfig::Wasm { path, config } => {
            let config_str = config.to_string();
            let wasm_path = std::path::Path::new(path);
            match ctx.plugin_backend {
                ResolvedPluginBackend::Sync => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_sync_controller(wasm_path, label, &config_str)
                        .map_err(|e| format!("WasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
                #[cfg(feature = "plugin-wasm-async")]
                ResolvedPluginBackend::Async => {
                    let ctrl = ctx
                        .wasm_cache
                        .build_async_controller(wasm_path, label, &config_str)
                        .map_err(|e| format!("AsyncWasmController build failed: {e}"))?;
                    Ok(Box::new(ctrl))
                }
            }
        }
        #[cfg(not(feature = "plugin-wasm"))]
        ControllerConfig::Wasm { .. } => {
            let _ = ctx;
            let _ = label;
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

    let field_model: Arc<dyn tobari::magnetic::MagneticFieldModel> =
        Arc::new(Igrf::earth());

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
