//! Detumble → nadir pointing モード遷移デモ — コールバック型。
//!
//! enum でモードを表現し、収束条件で遷移する。

#[allow(warnings)]
mod bindings;

use bindings::orts::plugin::types::*;
use nalgebra::{UnitQuaternion, Vector3};
use orts_plugin_sdk::{Plugin, orts_plugin};

enum Mode {
    Detumble {
        gain: f64,
        max_moment: f64,
        omega_threshold: f64,
    },
    Nadir {
        kp: f64,
        kd: f64,
    },
}

impl From<&Mode> for &'static str {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Detumble { .. } => "detumble",
            Mode::Nadir { .. } => "nadir",
        }
    }
}

struct Controller {
    mode: Mode,
    sample_period: f64,
    // nadir パラメータ（遷移時に使う）
    nadir_kp: f64,
    nadir_kd: f64,
}

impl Plugin<TickInput, Command> for Controller {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(config: &str) -> Result<Self, String> {
        let cfg: Config = if config.is_empty() {
            Config::default()
        } else {
            serde_json::from_str(config).map_err(|e| format!("config parse error: {e}"))?
        };
        Ok(Self {
            mode: Mode::Detumble {
                gain: cfg.detumble_gain,
                max_moment: cfg.max_moment,
                omega_threshold: cfg.omega_threshold,
            },
            sample_period: cfg.sample_period,
            nadir_kp: cfg.nadir_kp,
            nadir_kd: cfg.nadir_kd,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        match &self.mode {
            Mode::Detumble {
                gain,
                max_moment,
                omega_threshold,
            } => {
                let omega = match &input.sensors.gyroscope {
                    Some(g) => Vector3::new(g.x, g.y, g.z),
                    None => return Ok(None),
                };

                if omega.norm() < *omega_threshold {
                    self.mode = Mode::Nadir {
                        kp: self.nadir_kp,
                        kd: self.nadir_kd,
                    };
                    return Ok(None);
                }

                let b = match &input.sensors.magnetometer {
                    Some(m) => Vector3::new(m.x, m.y, m.z),
                    None => return Ok(None),
                };
                if b.norm_squared() < 1e-60 {
                    return Ok(None);
                }

                let m = -gain * omega.cross(&b);
                let max = *max_moment;

                Ok(Some(Command {
                    magnetic_moment: Some(CommandedMagneticMoment {
                        x: m.x.clamp(-max, max),
                        y: m.y.clamp(-max, max),
                        z: m.z.clamp(-max, max),
                    }),
                    rw_torque: None,
                }))
            }

            Mode::Nadir { kp, kd } => {
                let att = match &input.sensors.star_tracker {
                    Some(a) => a,
                    None => return Ok(None),
                };
                let omega = match &input.sensors.gyroscope {
                    Some(g) => Vector3::new(g.x, g.y, g.z),
                    None => return Ok(None),
                };

                let q = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                    att.w, att.x, att.y, att.z,
                ));
                let q = if q.w < 0.0 {
                    UnitQuaternion::from_quaternion(-q.into_inner())
                } else {
                    q
                };

                let theta = 2.0 * q.vector();
                let tau = -*kp * theta - *kd * omega;

                Ok(Some(Command {
                    rw_torque: Some(CommandedRwTorque {
                        x: tau.x,
                        y: tau.y,
                        z: tau.z,
                    }),
                    magnetic_moment: None,
                }))
            }
        }
    }

    fn current_mode(&self) -> Option<&str> {
        Some((&self.mode).into())
    }
}

orts_plugin!(Controller, mode);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    sample_period: f64,
    detumble_gain: f64,
    max_moment: f64,
    omega_threshold: f64,
    nadir_kp: f64,
    nadir_kd: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_period: 1.0,
            detumble_gain: 1e4,
            max_moment: 10.0,
            omega_threshold: 0.01,
            nadir_kp: 1.0,
            nadir_kd: 2.0,
        }
    }
}
