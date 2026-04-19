//! In-plane phasing demo。parking 軌道で `raise_delay_s`
//! だけ待機してから Hohmann 遷移で operational 軌道へ上昇することで、
//! parking と operational の平均運動差
//! `Δn = √(μ/r_park³) − √(μ/r_op³)` を使って phase offset を作り出す。
//!
//! 同じ .wasm を複数衛星に assign して per-sat config で `raise_delay_s`
//! だけ変える構成を想定している。
//!
//! # State Machine
//!
//! ```text
//! ┌────────┐  t >= raise_delay_s    ┌───────────┐          ┌───────┐          ┌────────────┐          ┌──────┐
//! │ Parked │───────────────────────▶│ FirstBurn │─────────▶│ Coast │─────────▶│ SecondBurn │─────────▶│ Trim │
//! └────────┘                         └───────────┘          └───────┘          └────────────┘          └──────┘
//! ```
//!
//! 姿勢追従 (body-Y を prograde に向ける PD + RW) と thruster throttle の
//! composite controller は transfer-burn-with-tcm と同じロジックを使う。

use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, orts_plugin};

const EARTH_RADIUS_KM: f64 = 6378.137;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Parked,
    FirstBurn,
    Coast,
    SecondBurn,
    Trim,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Parked => "parked",
            Phase::FirstBurn => "first_burn",
            Phase::Coast => "coast",
            Phase::SecondBurn => "second_burn",
            Phase::Trim => "trim",
        }
    }
}

struct ConstellationPhasing {
    raise_delay_s: f64,
    target_r_km: f64,
    mu_km3_s2: f64,
    deadband_km: f64,
    num_thrusters: usize,
    num_rws: usize,
    kp: f64,
    kd: f64,
    sample_period: f64,
    transfer_sma_km: Option<f64>,
    transfer_half_period_s: Option<f64>,
    coast_start_t: Option<f64>,
    phase: Phase,
    /// Last SMA [km] seen by `update()`. Used by SecondBurn/Trim to predict
    /// the next step's SMA change and throttle back to avoid overshoot.
    prev_sma_km: Option<f64>,
}

impl Plugin<TickInput, Command> for ConstellationPhasing {
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
        if !cfg.raise_delay_s.is_finite() || cfg.raise_delay_s < 0.0 {
            return Err("raise_delay_s must be non-negative and finite".into());
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
        let initial_phase = if cfg.raise_delay_s > 0.0 {
            Phase::Parked
        } else {
            Phase::FirstBurn
        };
        Ok(Self {
            raise_delay_s: cfg.raise_delay_s,
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
            phase: initial_phase,
            prev_sma_km: None,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        let p = &input.spacecraft.orbit.position;
        let v = &input.spacecraft.orbit.velocity;
        let r_vec = Vector3::new(p.x, p.y, p.z);
        let v_vec = Vector3::new(v.x, v.y, v.z);
        let r = r_vec.norm();
        let v_sq = v_vec.norm_squared();

        let epsilon = 0.5 * v_sq - self.mu_km3_s2 / r;
        let sma = -self.mu_km3_s2 / (2.0 * epsilon);

        // target 高度が parking 高度より低い場合（prograde-only では deorbit 不可）。
        // Parked 中はまだ burn していないので check しない。FirstBurn 遷移時に検証する。
        if self.phase == Phase::FirstBurn && r > self.target_r_km + self.deadband_km {
            return Err(format!(
                "target altitude ({:.1} km) is lower than parking altitude ({:.1} km); \
                 prograde-only thruster cannot deorbit. Aborting.",
                self.target_r_km - EARTH_RADIUS_KM,
                r - EARTH_RADIUS_KM,
            ));
        }

        let throttle = match self.phase {
            Phase::Parked => {
                if input.t >= self.raise_delay_s {
                    // Check reachability BEFORE transitioning so we don't
                    // accidentally command one prograde step on an impossible
                    // (target below parking) configuration.
                    if r > self.target_r_km + self.deadband_km {
                        return Err(format!(
                            "target altitude ({:.1} km) is lower than parking altitude ({:.1} km); \
                             prograde-only thruster cannot deorbit. Aborting.",
                            self.target_r_km - EARTH_RADIUS_KM,
                            r - EARTH_RADIUS_KM,
                        ));
                    }
                    self.phase = Phase::FirstBurn;
                    // FirstBurn に入ったら transfer orbit のパラメータをキャッシュする
                    // （parking 軌道の現 r を起点に計算）。
                    let transfer_sma = (r + self.target_r_km) / 2.0;
                    self.transfer_sma_km = Some(transfer_sma);
                    self.transfer_half_period_s =
                        Some(std::f64::consts::PI * (transfer_sma.powi(3) / self.mu_km3_s2).sqrt());
                    1.0
                } else {
                    0.0
                }
            }
            Phase::FirstBurn => {
                let transfer_sma = *self
                    .transfer_sma_km
                    .get_or_insert((r + self.target_r_km) / 2.0);
                let _ = self.transfer_half_period_s.get_or_insert(
                    std::f64::consts::PI * (transfer_sma.powi(3) / self.mu_km3_s2).sqrt(),
                );
                if sma >= transfer_sma {
                    self.phase = Phase::Coast;
                    self.coast_start_t = Some(input.t);
                    0.0
                } else {
                    1.0
                }
            }
            Phase::Coast => {
                let half_period = self.transfer_half_period_s.unwrap_or(0.0);
                let coast_elapsed = input.t - self.coast_start_t.unwrap_or(input.t);
                if coast_elapsed >= half_period {
                    self.phase = Phase::SecondBurn;
                    1.0
                } else {
                    0.0
                }
            }
            Phase::SecondBurn => {
                // Predict next step's SMA from the previous step's delta and
                // taper the throttle in the final step so we land near the
                // target instead of overshooting by a whole step's worth of
                // burn. Without this, sat-2 ended up at SMA=6957 km (target
                // 6928) which left a visible eccentricity and made Δφ
                // oscillate at the orbital period.
                if sma >= self.target_r_km {
                    self.phase = Phase::Trim;
                    0.0
                } else {
                    let dsma = self
                        .prev_sma_km
                        .map(|prev| (sma - prev).max(0.0))
                        .unwrap_or(0.0);
                    if dsma > 0.0 && sma + dsma >= self.target_r_km {
                        // fractional throttle to land exactly at target
                        ((self.target_r_km - sma) / dsma).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                }
            }
            Phase::Trim => {
                // Same predictive tapering for TCM burns.
                if sma < self.target_r_km - self.deadband_km {
                    let dsma = self
                        .prev_sma_km
                        .map(|prev| (sma - prev).max(0.0))
                        .unwrap_or(0.0);
                    if dsma > 0.0 && sma + dsma >= self.target_r_km {
                        ((self.target_r_km - sma) / dsma).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                } else {
                    0.0
                }
            }
        };
        self.prev_sma_km = Some(sma);

        // 姿勢 target: body-Y を velocity 方向、body-Z を orbit normal に向ける。
        // Parked 中でも attitude tracking は有効にしておく（姿勢が安定していないと
        // FirstBurn 開始時に誤差が大きい）。
        let y_target = v_vec.normalize();
        let z_target = r_vec.cross(&v_vec).normalize();
        let x_target = y_target.cross(&z_target);
        let rot = Matrix3::from_columns(&[x_target, y_target, z_target]);
        let q_target = UnitQuaternion::from_matrix(&rot);

        let att = &input.spacecraft.attitude.orientation;
        let q_current =
            UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(att.w, att.x, att.y, att.z));
        let q_err = q_target.inverse() * q_current;
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

        let rw_torques = if self.num_rws == 3 {
            vec![-tau.x, -tau.y, -tau.z]
        } else {
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
            thruster: Some(ThrusterCommand::Throttles(vec![
                throttle;
                self.num_thrusters
            ])),
        }))
    }

    fn current_mode(&self) -> Option<&str> {
        Some(self.phase.as_str())
    }
}

orts_plugin!(ConstellationPhasing, mode);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    target_altitude_km: f64,
    raise_delay_s: f64,
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
            target_altitude_km: 550.0,
            raise_delay_s: 0.0,
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
