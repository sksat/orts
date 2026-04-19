//! 円軌道 → 円軌道の軌道上昇 (Hohmann) + TCM (Trajectory Correction
//! Maneuver) 最小例。**姿勢追従と推力指令を 1 plugin で同居** させる
//! composite controller の example でもある。
//!
//! # 制御ループの内訳
//!
//! 毎 tick で以下を出力：
//!
//! 1. **RW torque**: body-Y を velocity 方向に向ける PD 制御。
//!    body-Z = 軌道法線、body-X = 半径方向（radial-outward 派生）を
//!    target frame として、誤差クォータニオンに対する `-Kp·θ - Kd·ω`
//!    をホイールトルクとして RW へ発行。
//! 2. **Thruster throttle**: FirstBurn / Coast / SecondBurn / Trim の
//!    state machine で決定。`TickInput.spacecraft.orbit` から r / SMA
//!    を計算し、phase 遷移する。
//!
//! # State Machine
//!
//! ```text
//!                 sma >= (r1+r2)/2        coast_elapsed >= T_transfer/2
//!      FirstBurn ────────────────▶ Coast ───────────────────────────▶ SecondBurn
//!                                                                           │
//!                                                                   sma >= r_target
//!                                                                           │
//!                                                                           ▼
//!                                                                          Trim
//!                                                                           │
//!                                                          sma < r_target - deadband
//!                                                          で再度 throttle = 1 (TCM)
//! ```
//!
//! - **Coast → SecondBurn**: apogee 到達を時間ベース（transfer ellipse の
//!   半周期経過）で判定する。finite-burn 損失で `r` が `target` にぴったり
//!   届かない場合でもロバストに遷移する。実機の maneuver 計画でも
//!   半周期タイマーで apogee を打つのが一般的。
//! - **Trim (TCM)**: drag や初期 burn 誤差で SMA が目標を下回ったときのみ
//!   補正 burn。instantaneous の `r` で判定すると楕円軌道の perigee で
//!   毎周期発火してしまうので、軌道サイズ (SMA) ベースで判定する。
//!
//! # 姿勢追従
//!
//! 目標 body frame を以下で定める（LVLH の tangential-normal-radial 相当）：
//!
//! - y_body_target = v̂ (prograde)
//! - z_body_target = (r × v) / |r × v| (orbit normal)
//! - x_body_target = y_target × z_target (radial-outward 近傍)
//!
//! これにより body-Y の推力ベクトルが常に prograde を向くため、
//! 低推力・長時間 burn や apogee での SecondBurn でも姿勢整合が取れる。
//!
//! # 制約
//!
//! - `num_thrusters` / `num_rws` は WIT v0 に actuator inventory API が
//!   ないため plugin config に書く必要がある。host の
//!   `[satellites.thruster.thrusters]` / `[satellites.reaction_wheels]`
//!   と一致させるのは開発者責任（不一致は CLI 側 length check で reject）。
//! - 目標高度が初期高度より低い場合、prograde のみの推進器では軌道降下
//!   できず plugin は FirstBurn 開始時に Err を返す。

use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, orts_plugin};

const EARTH_RADIUS_KM: f64 = 6378.137;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    FirstBurn,
    Coast,
    SecondBurn,
    Trim,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::FirstBurn => "first_burn",
            Phase::Coast => "coast",
            Phase::SecondBurn => "second_burn",
            Phase::Trim => "trim",
        }
    }
}

struct TransferBurnWithTcm {
    // thruster config
    target_r_km: f64,
    mu_km3_s2: f64,
    deadband_km: f64,
    num_thrusters: usize,
    // attitude config
    num_rws: usize,
    kp: f64,
    kd: f64,
    //
    sample_period: f64,
    // derived (lazy from first update)
    transfer_sma_km: Option<f64>,
    /// transfer ellipse 半周期 [s] — Coast → SecondBurn の切り替えに使う。
    /// finite burn 損失があるため、apogee を `r` ベースで検出するよりも
    /// 時間ベースの方がロバスト（実機の maneuver 計画でも一般的）。
    transfer_half_period_s: Option<f64>,
    /// Coast フェーズに入った時刻 [s]。
    coast_start_t: Option<f64>,
    // state
    phase: Phase,
}

impl Plugin<TickInput, Command> for TransferBurnWithTcm {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(config: &str) -> Result<Self, String> {
        let cfg: Config = if config.is_empty() {
            Config::default()
        } else {
            serde_json::from_str(config).map_err(|e| format!("config parse error: {e}"))?
        };
        if !cfg.target_altitude_km.is_finite() || cfg.target_altitude_km <= 0.0 {
            return Err("target_altitude_km must be positive and finite".into());
        }
        if !cfg.mu_km3_s2.is_finite() || cfg.mu_km3_s2 <= 0.0 {
            return Err("mu_km3_s2 must be positive and finite".into());
        }
        if !cfg.deadband_km.is_finite() || cfg.deadband_km <= 0.0 {
            return Err("deadband_km must be positive and finite".into());
        }
        if cfg.num_thrusters == 0 {
            return Err("num_thrusters must be >= 1".into());
        }
        if cfg.num_rws == 0 {
            return Err("num_rws must be >= 1".into());
        }
        if !cfg.sample_period.is_finite() || cfg.sample_period <= 0.0 {
            return Err("sample_period must be positive and finite".into());
        }
        Ok(Self {
            target_r_km: EARTH_RADIUS_KM + cfg.target_altitude_km,
            mu_km3_s2: cfg.mu_km3_s2,
            deadband_km: cfg.deadband_km,
            num_thrusters: cfg.num_thrusters,
            num_rws: cfg.num_rws,
            kp: cfg.kp,
            kd: cfg.kd,
            sample_period: cfg.sample_period,
            transfer_sma_km: None,
            transfer_half_period_s: None,
            coast_start_t: None,
            phase: Phase::FirstBurn,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        // --- 1. orbit state から phase と throttle を決定 ---------------------
        let p = &input.spacecraft.orbit.position;
        let v = &input.spacecraft.orbit.velocity;
        let r_vec = Vector3::new(p.x, p.y, p.z);
        let v_vec = Vector3::new(v.x, v.y, v.z);
        let r = r_vec.norm();
        let v_sq = v_vec.norm_squared();

        let epsilon = 0.5 * v_sq - self.mu_km3_s2 / r;
        let sma = -self.mu_km3_s2 / (2.0 * epsilon);
        let transfer_sma = *self
            .transfer_sma_km
            .get_or_insert((r + self.target_r_km) / 2.0);

        if r > self.target_r_km + self.deadband_km && self.phase == Phase::FirstBurn {
            return Err(format!(
                "target altitude ({:.1} km) is lower than initial altitude ({:.1} km); \
                 prograde-only thruster cannot deorbit. Aborting.",
                self.target_r_km - EARTH_RADIUS_KM,
                r - EARTH_RADIUS_KM,
            ));
        }

        // transfer 半周期を一度だけ計算してキャッシュ。
        let half_period = *self
            .transfer_half_period_s
            .get_or_insert(std::f64::consts::PI * (transfer_sma.powi(3) / self.mu_km3_s2).sqrt());

        let throttle = match self.phase {
            Phase::FirstBurn => {
                if sma >= transfer_sma {
                    self.phase = Phase::Coast;
                    self.coast_start_t = Some(input.t);
                    0.0
                } else {
                    1.0
                }
            }
            Phase::Coast => {
                // 時間ベースで apogee 到達を判定（finite-burn 損失で r が
                // target に届かない場合でも確実に SecondBurn へ遷移する）。
                let coast_elapsed = input.t - self.coast_start_t.unwrap_or(input.t);
                if coast_elapsed >= half_period {
                    self.phase = Phase::SecondBurn;
                    1.0
                } else {
                    0.0
                }
            }
            Phase::SecondBurn => {
                if sma >= self.target_r_km {
                    self.phase = Phase::Trim;
                    0.0
                } else {
                    1.0
                }
            }
            Phase::Trim => {
                // TCM: 軌道が drag 等で decay して SMA が deadband 分だけ
                // 下がったら補正 burn。instantaneous の r では楕円軌道の
                // perigee で毎周期 trigger してしまうので、SMA で判定。
                if sma < self.target_r_km - self.deadband_km {
                    1.0
                } else {
                    0.0
                }
            }
        };

        // --- 2. 姿勢 target: body-Y を velocity 方向に、body-Z を orbit 法線に ---
        let y_target = v_vec.normalize();
        let z_target = r_vec.cross(&v_vec).normalize();
        let x_target = y_target.cross(&z_target);
        // columns = [x, y, z]_target in inertial coords → body→inertial rotation
        let rot = Matrix3::from_columns(&[x_target, y_target, z_target]);
        let q_target = UnitQuaternion::from_matrix(&rot);

        // --- 3. PD on attitude error → RW torque ------------------------------
        let att = &input.spacecraft.attitude.orientation;
        let q_current = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
            att.w, att.x, att.y, att.z,
        ));
        let q_err = q_target.inverse() * q_current;
        // 半球選択（最短経路）。
        let q_err = if q_err.w < 0.0 {
            UnitQuaternion::from_quaternion(-q_err.into_inner())
        } else {
            q_err
        };
        let theta = 2.0 * q_err.vector();
        let omega_body = Vector3::new(
            input.spacecraft.attitude.angular_velocity.x,
            input.spacecraft.attitude.angular_velocity.y,
            input.spacecraft.attitude.angular_velocity.z,
        );
        let tau = -self.kp * theta - self.kd * omega_body;

        // pd-rw-control と同じく、ホイールが吸収するトルクは身体への希望トルク
        // の反作用（orthogonal 3-axis 前提、各軸 1 wheel 対応）。
        let rw_torques = if self.num_rws == 3 {
            vec![-tau.x, -tau.y, -tau.z]
        } else {
            // 任意本数：X/Y/Z を最初の 3 本に割当、残りは 0（single-wheel
            // の場合は num_rws=1 で最初の 1 本のみが Z 軸相当）。
            let mut v = vec![0.0; self.num_rws];
            if !v.is_empty() {
                v[0] = -tau.x;
            }
            if v.len() > 1 {
                v[1] = -tau.y;
            }
            if v.len() > 2 {
                v[2] = -tau.z;
            }
            v
        };

        Ok(Some(Command {
            rw: Some(RwCommand::Torques(rw_torques)),
            mtq: None,
            thruster: Some(ThrusterCommand::Throttles(vec![throttle; self.num_thrusters])),
        }))
    }

    fn current_mode(&self) -> Option<&str> {
        Some(self.phase.as_str())
    }
}

orts_plugin!(TransferBurnWithTcm, mode);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    target_altitude_km: f64,
    mu_km3_s2: f64,
    deadband_km: f64,
    num_thrusters: usize,
    num_rws: usize,
    kp: f64,
    kd: f64,
    sample_period: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_altitude_km: 600.0,
            mu_km3_s2: 398_600.441_8,
            deadband_km: 1.0,
            num_thrusters: 1,
            num_rws: 3,
            kp: 10.0,
            kd: 20.0,
            sample_period: 1.0,
        }
    }
}
