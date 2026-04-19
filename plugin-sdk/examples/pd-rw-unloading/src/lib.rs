//! PD attitude controller + RW momentum unloading — コールバック型。
//!
//! PD 姿勢制御 (RW) と磁気トルカによるアンローディングを同時指令する。
//!
//! ## 制御則
//!
//! ### PD 姿勢制御
//!
//! ```text
//! q_err = q_target^{-1} * q_current
//! theta_error ~ 2 * q_err.vector_part  (hemisphere-selected)
//! tau = -Kp * theta_error - Kd * omega
//! ```
//!
//! ### アンローディング
//!
//! ```text
//! m_desat = k_desat * (h_rw × B_body) / |B_body|²
//! ```

use orts_plugin_sdk::bindings::orts::plugin::types::*;
use nalgebra::{UnitQuaternion, Vector3};
use orts_plugin_sdk::{Plugin, orts_plugin};

struct PdRwUnloading {
    kp: f64,
    kd: f64,
    k_desat: f64,
    target_q: UnitQuaternion<f64>,
    sample_period: f64,
}

impl Plugin<TickInput, Command> for PdRwUnloading {
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
            k_desat: cfg.k_desat,
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
        // ── PD attitude control ─────────────────────────────────
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

        let q_current =
            UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(att.w, att.x, att.y, att.z));

        let q_err = self.target_q.inverse() * q_current;
        let q_err = if q_err.w < 0.0 {
            UnitQuaternion::from_quaternion(-q_err.into_inner())
        } else {
            q_err
        };

        let theta = 2.0 * q_err.vector();
        let omega_body = Vector3::new(omega.x, omega.y, omega.z);
        let tau = -self.kp * theta - self.kd * omega_body;

        // ── Desaturation via magnetorquer ───────────────────────
        let mag_cmd = compute_desaturation(input, self.k_desat).map(MtqCommand::Moments);

        // Per-wheel motor torque (Newton's 3rd law for orthogonal 3-axis)
        Ok(Some(Command {
            rw: Some(RwCommand::Torques(vec![-tau.x, -tau.y, -tau.z])),
            mtq: mag_cmd,
            thruster: None,
        }))
    }
}

orts_plugin!(PdRwUnloading);

/// Desaturation: m = k_desat * (h_rw × B_body) / |B_body|²
fn compute_desaturation(input: &TickInput, k_desat: f64) -> Option<Vec<f64>> {
    let b = input.sensors.magnetometers.first()?;
    let rw_tlm = input.actuators.rw.as_ref()?;

    if rw_tlm.momentum.len() < 3 {
        return None;
    }

    let h = Vector3::new(rw_tlm.momentum[0], rw_tlm.momentum[1], rw_tlm.momentum[2]);
    let b_body = Vector3::new(b.x, b.y, b.z);

    let b_mag_sq = b_body.norm_squared();
    if b_mag_sq < 1e-60 {
        return None;
    }

    let m = (k_desat / b_mag_sq) * h.cross(&b_body);

    Some(vec![m.x, m.y, m.z])
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    kp: f64,
    kd: f64,
    k_desat: f64,
    target_q: [f64; 4],
    sample_period: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kp: 1.0,
            kd: 2.0,
            k_desat: 0.001,
            target_q: [1.0, 0.0, 0.0, 0.0],
            sample_period: 0.1,
        }
    }
}
