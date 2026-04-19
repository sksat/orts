//! PD attitude controller with reaction wheel output — コールバック型。
//!
//! 制御則 (left-invariant quaternion error):
//!
//! ```text
//! q_err = q_target^{-1} * q_current
//! theta_error ~ 2 * q_err.vector_part  (hemisphere-selected)
//! tau = -Kp * theta_error - Kd * omega
//! ```

use orts_plugin_sdk::bindings::orts::plugin::types::*;
use nalgebra::{UnitQuaternion, Vector3};
use orts_plugin_sdk::{Plugin, orts_plugin};

struct PdRwControl {
    kp: f64,
    kd: f64,
    target_q: UnitQuaternion<f64>,
    sample_period: f64,
}

impl Plugin<TickInput, Command> for PdRwControl {
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
            kp: cfg.kp,
            kd: cfg.kd,
            target_q: UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                cfg.target_q[0],
                cfg.target_q[1],
                cfg.target_q[2],
                cfg.target_q[3],
            )),
            sample_period: cfg.sample_period,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        let att = input
            .sensors
            .star_trackers
            .first()
            .ok_or("star tracker sensor not available")?;
        let omega = input
            .sensors
            .gyroscopes
            .first()
            .ok_or("gyroscope sensor not available")?;

        // Current attitude as UnitQuaternion (Hamilton scalar-first).
        let q_current = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
            att.w, att.x, att.y, att.z,
        ));

        // Left-invariant quaternion error: q_err = q_target^{-1} * q_current
        let q_err = self.target_q.inverse() * q_current;

        // Hemisphere selection (shortest path).
        let q_err = if q_err.w < 0.0 {
            UnitQuaternion::from_quaternion(-q_err.into_inner())
        } else {
            q_err
        };

        // Body-frame angular error: theta ~ 2 * q_err.vector_part
        let theta = 2.0 * q_err.vector();
        let omega_body = Vector3::new(omega.x, omega.y, omega.z);

        // PD torque (desired body torque): tau = -Kp * theta - Kd * omega
        let tau = -self.kp * theta - self.kd * omega_body;

        // Per-wheel motor torque (Newton's 3rd law for orthogonal 3-axis):
        // wheel absorbs the negative of desired body torque projected onto its axis.
        Ok(Some(Command {
            rw: Some(RwCommand::Torques(vec![-tau.x, -tau.y, -tau.z])),
            mtq: None,
            thruster: None,
        }))
    }
}

orts_plugin!(PdRwControl);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    kp: f64,
    kd: f64,
    target_q: [f64; 4],
    sample_period: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kp: 1.0,
            kd: 2.0,
            target_q: [1.0, 0.0, 0.0, 0.0],
            sample_period: 0.1,
        }
    }
}
