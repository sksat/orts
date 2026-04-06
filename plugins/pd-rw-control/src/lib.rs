//! PD attitude controller with reaction wheel output — WASM Component guest.
//!
//! Reads star tracker (attitude) and gyroscope (angular velocity) from
//! sensor readings, computes PD torque command, and returns
//! `Command::RwTorque`.
//!
//! Control law (left-invariant quaternion error):
//!
//! ```text
//! q_err = q_target^{-1} * q_current
//! θ_error ≈ 2 * q_err.vector_part  (hemisphere-selected)
//! τ = -Kp * θ_error - Kd * ω
//! ```

#[allow(warnings)]
mod bindings;

use bindings::exports::orts::plugin::controller::Guest;
use bindings::orts::plugin::types::*;

use core::cell::RefCell;

struct PdRwState {
    kp: f64,
    kd: f64,
    target_q: [f64; 4], // [w, x, y, z]
    sample_period: f64,
}

thread_local! {
    static STATE: RefCell<PdRwState> = RefCell::new(PdRwState {
        kp: 1.0,
        kd: 2.0,
        target_q: [1.0, 0.0, 0.0, 0.0], // identity
        sample_period: 0.1,
    });
}

struct Component;

impl Guest for Component {
    fn sample_period_s() -> f64 {
        STATE.with(|s| s.borrow().sample_period)
    }

    fn init(config: String) -> Result<(), String> {
        if config.is_empty() {
            return Ok(());
        }
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
        let cfg: Config =
            serde_json::from_str(&config).map_err(|e| format!("config parse error: {e}"))?;
        STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.kp = cfg.kp;
            s.kd = cfg.kd;
            s.target_q = cfg.target_q;
            s.sample_period = cfg.sample_period;
        });
        Ok(())
    }

    fn initial_command() -> Command {
        Command::RwTorque(Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })
    }

    fn update(input: TickInput) -> Result<Command, String> {
        STATE.with(|state| {
            let s = state.borrow();

            // Read sensors.
            let att = input
                .sensors
                .star_tracker
                .ok_or("star tracker sensor not available")?;
            let omega = input
                .sensors
                .gyroscope
                .ok_or("gyroscope sensor not available")?;

            // Target quaternion.
            let tq = &s.target_q;

            // Left-invariant quaternion error: q_err = q_target^{-1} * q_current
            // q_target^{-1} = conjugate (unit quaternion)
            let q_err = quat_mul_conj_left(tq, &att);

            // Hemisphere selection (shortest path).
            let (ew, ex, ey, ez) = if q_err.0 < 0.0 {
                (-q_err.0, -q_err.1, -q_err.2, -q_err.3)
            } else {
                q_err
            };

            // Body-frame angular error: θ ≈ 2 * q_err.vector_part
            let theta_x = 2.0 * ex;
            let theta_y = 2.0 * ey;
            let theta_z = 2.0 * ez;
            let _ = ew; // scalar part unused

            // PD torque: τ = -Kp * θ_error - Kd * ω
            let tx = -s.kp * theta_x - s.kd * omega.x;
            let ty = -s.kp * theta_y - s.kd * omega.y;
            let tz = -s.kp * theta_z - s.kd * omega.z;

            Ok(Command::RwTorque(Vec3 {
                x: tx,
                y: ty,
                z: tz,
            }))
        })
    }

    fn current_mode() -> Option<String> {
        None
    }
}

bindings::export!(Component with_types_in bindings);

// ─── quaternion helpers ──────────────────────────────────────────

/// Compute q_a^{-1} * q_b where q_a is a unit quaternion (inverse = conjugate).
/// Returns (w, x, y, z).
fn quat_mul_conj_left(
    qa: &[f64; 4],
    qb: &AttitudeBodyToInertial,
) -> (f64, f64, f64, f64) {
    // qa_inv = conjugate = (w, -x, -y, -z)
    let (aw, ax, ay, az) = (qa[0], -qa[1], -qa[2], -qa[3]);
    let (bw, bx, by, bz) = (qb.w, qb.x, qb.y, qb.z);

    // Hamilton product
    let w = aw * bw - ax * bx - ay * by - az * bz;
    let x = aw * bx + ax * bw + ay * bz - az * by;
    let y = aw * by - ax * bz + ay * bw + az * bx;
    let z = aw * bz + ax * by - ay * bx + az * bw;
    (w, x, y, z)
}
