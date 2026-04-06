//! 離散制御 + ZOH 積分ループ。
//!
//! Config から宇宙機ダイナミクス + プラグインコントローラ + センサ + RW を
//! 組み立て、制御サンプル周期ごとに積分 → センサ評価 → プラグイン呼び出し →
//! アクチュエータ更新 を繰り返す。`orts run` と `orts serve` の両方から使う。

use std::sync::Arc;

use kaname::epoch::Epoch;
use orts::attitude::CoupledGravityGradient;
use orts::effector::AugmentedState;
use orts::orbital::gravity::GravityField;
use orts::plugin::{ActuatorBundle, PluginController, TickInput};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle, StarTracker};
use orts::setup::{build_spacecraft_dynamics, default_third_bodies};

use crate::sim::core::sat_params;
use orts::spacecraft::{ReactionWheelAssembly, SpacecraftDynamics, SpacecraftState};
use tobari::magnetic::TiltedDipole;
use utsuroi::{Integrator, Rk4};

use crate::config::{ControllerConfig, ReactionWheelConfig, SensorChoice};
use crate::satellite::SatelliteSpec;
use crate::sim::params::SimParams;

/// 制御付き衛星の状態。
pub struct ControlledSatellite {
    pub dynamics: SpacecraftDynamics<Box<dyn GravityField>>,
    pub state: AugmentedState<SpacecraftState>,
    pub controller: Box<dyn PluginController>,
    pub sensors: SensorBundle,
    pub actuators: ActuatorBundle,
    /// RW effector が登録されているかどうか。
    pub has_rw: bool,
}

/// Config からプラグイン制御付き衛星を構築する。
pub fn build_controlled_satellite(
    spec: &SatelliteSpec,
    params: &SimParams,
) -> Result<ControlledSatellite, String> {
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
            } => ReactionWheelAssembly::three_axis(*inertia, *max_momentum, *max_torque),
        };
        dynamics = dynamics.with_effector(rw);
    }

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

    // コントローラを構築。
    let controller = build_controller(ctrl_config)?;

    // センサを構築。
    let sensors = build_sensor_bundle(spec.sensor_choices.as_deref());

    let mut actuators = ActuatorBundle::new();
    actuators
        .apply(&controller.initial_command())
        .map_err(|e| format!("initial command error: {e}"))?;

    Ok(ControlledSatellite {
        dynamics,
        state,
        controller,
        sensors,
        actuators,
        has_rw,
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
        && let Some(rw) = sat.dynamics.effector_mut::<ReactionWheelAssembly>(0)
    {
        rw.commanded_torque = sat.actuators.rw_torque();
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
    let input = TickInput {
        t: t_next,
        epoch: current_epoch.as_ref(),
        sensors: &sensors,
        spacecraft: &sat.state.plant,
    };
    let cmd = sat
        .controller
        .update(&input)
        .map_err(|e| format!("controller error at t={t_next:.3}: {e}"))?;
    sat.actuators
        .apply(&cmd)
        .map_err(|e| format!("actuator error at t={t_next:.3}: {e}"))?;

    Ok(())
}

// ─── builder helpers ─────────────────────────────────────────────

fn build_controller(config: &ControllerConfig) -> Result<Box<dyn PluginController>, String> {
    match config {
        #[cfg(feature = "plugin-wasm")]
        ControllerConfig::Wasm { path, config } => {
            use orts::plugin::wasm::{WasmController, WasmEngine};
            use wasmtime::component::Component;

            let engine =
                Arc::new(WasmEngine::new().map_err(|e| format!("WasmEngine init failed: {e}"))?);
            let wasm_bytes =
                std::fs::read(path).map_err(|e| format!("cannot read WASM at '{path}': {e}"))?;
            let component = Component::new(engine.inner(), &wasm_bytes)
                .map_err(|e| format!("WASM compile failed: {e}"))?;
            let pre = WasmController::prepare(&engine, &component)
                .map_err(|e| format!("WASM prepare failed: {e}"))?;
            let config_str = config.to_string();
            let ctrl = WasmController::new(&pre, "controlled", &config_str)
                .map_err(|e| format!("WasmController init failed: {e}"))?;
            Ok(Box::new(ctrl))
        }
        #[cfg(not(feature = "plugin-wasm"))]
        ControllerConfig::Wasm { .. } => {
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
        Arc::new(TiltedDipole::earth());

    SensorBundle {
        magnetometer: if choices.contains(&SensorChoice::Magnetometer) {
            Some(Magnetometer::new(Arc::clone(&field_model)))
        } else {
            None
        },
        gyroscope: if choices.contains(&SensorChoice::Gyroscope) {
            Some(Gyroscope::new())
        } else {
            None
        },
        star_tracker: if choices.contains(&SensorChoice::StarTracker) {
            Some(StarTracker::new())
        } else {
            None
        },
    }
}
