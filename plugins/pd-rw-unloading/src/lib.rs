//! PD attitude controller with reaction wheel output + magnetorquer
//! desaturation -- WASM Component guest.
//!
//! Extends the PD+RW controller with a momentum unloading law that
//! uses magnetorquers to dump accumulated RW angular momentum.
//!
//! ## Control law
//!
//! ### PD attitude control (same as pd-rw-control)
//!
//! ```text
//! q_err = q_target^{-1} * q_current
//! theta_error ~ 2 * q_err.vector_part  (hemisphere-selected)
//! tau = -Kp * theta_error - Kd * omega
//! ```
//!
//! ### Desaturation via magnetorquer
//!
//! The RW momentum vector in the body frame `h_rw` is read from
//! `actuators.rw_momentum`. The magnetorquer command is computed as:
//!
//! ```text
//! m_desat = k_desat * (h_rw × B_body) / |B_body|²
//! ```
//!
//! This produces a torque `m × B` that opposes the RW momentum
//! buildup, gradually dumping the stored angular momentum.
//!
//! ### Simultaneous command
//!
//! Both `rw_torque` (PD) and `magnetic_moment` (desaturation) are
//! returned in a single `Command`.

#[allow(warnings)]
mod bindings;

use bindings::exports::orts::plugin::controller::Guest;
use bindings::orts::plugin::types::*;

use core::cell::RefCell;

struct PdRwUnloadingState {
    kp: f64,
    kd: f64,
    k_desat: f64,
    target_q: [f64; 4], // [w, x, y, z]
    sample_period: f64,
}

thread_local! {
    static STATE: RefCell<PdRwUnloadingState> = RefCell::new(PdRwUnloadingState {
        kp: 1.0,
        kd: 2.0,
        k_desat: 0.001,
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
        let cfg: Config =
            serde_json::from_str(&config).map_err(|e| format!("config parse error: {e}"))?;
        STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.kp = cfg.kp;
            s.kd = cfg.kd;
            s.k_desat = cfg.k_desat;
            s.target_q = cfg.target_q;
            s.sample_period = cfg.sample_period;
        });
        Ok(())
    }

    fn update(input: TickInput) -> Result<Option<Command>, String> {
        STATE.with(|state| {
            let s = state.borrow();

            // ── PD attitude control ────────────────────────────────
            let att = input
                .sensors
                .star_tracker
                .ok_or("star tracker sensor not available")?;
            let omega = input
                .sensors
                .gyroscope
                .ok_or("gyroscope sensor not available")?;

            let tq = &s.target_q;

            // Left-invariant quaternion error: q_err = q_target^{-1} * q_current
            let q_err = quat_mul_conj_left(tq, &att);

            // Hemisphere selection (shortest path).
            let (ew, ex, ey, ez) = if q_err.0 < 0.0 {
                (-q_err.0, -q_err.1, -q_err.2, -q_err.3)
            } else {
                q_err
            };

            // Body-frame angular error: theta ~ 2 * q_err.vector_part
            let theta_x = 2.0 * ex;
            let theta_y = 2.0 * ey;
            let theta_z = 2.0 * ez;
            let _ = ew; // scalar part unused

            // PD torque: tau = -Kp * theta_error - Kd * omega
            let tx = -s.kp * theta_x - s.kd * omega.x;
            let ty = -s.kp * theta_y - s.kd * omega.y;
            let tz = -s.kp * theta_z - s.kd * omega.z;

            // ── Desaturation via magnetorquer ──────────────────────
            let mag_cmd = compute_desaturation(&input, s.k_desat);

            Ok(Some(Command {
                rw_torque: Some(CommandedRwTorque {
                    x: tx,
                    y: ty,
                    z: tz,
                }),
                magnetic_moment: mag_cmd,
            }))
        })
    }

    fn current_mode() -> Option<String> {
        None
    }
}

bindings::export!(Component with_types_in bindings);

// ── desaturation helper ─────────────────────────────────────────────

/// Compute the magnetorquer desaturation command.
///
/// `m_desat = k_desat * (h_rw × B_body) / |B_body|²`
///
/// Returns `None` if the magnetometer or RW momentum data is
/// unavailable, or if the magnetic field is too weak.
fn compute_desaturation(input: &TickInput, k_desat: f64) -> Option<CommandedMagneticMoment> {
    let b_body = input.sensors.magnetometer.as_ref()?;
    let rw_momentum = input.actuators.rw_momentum.as_ref()?;

    if rw_momentum.len() < 3 {
        return None;
    }

    // For a 3-axis orthogonal RW assembly, the body-frame momentum
    // vector is simply [h0, h1, h2].
    let hx = rw_momentum[0];
    let hy = rw_momentum[1];
    let hz = rw_momentum[2];

    let bx = b_body.x;
    let by = b_body.y;
    let bz = b_body.z;

    let b_mag_sq = bx * bx + by * by + bz * bz;
    if b_mag_sq < 1e-60 {
        return None;
    }

    // Cross product: h_rw × B_body
    let cx = hy * bz - hz * by;
    let cy = hz * bx - hx * bz;
    let cz = hx * by - hy * bx;

    let scale = k_desat / b_mag_sq;

    Some(CommandedMagneticMoment {
        x: scale * cx,
        y: scale * cy,
        z: scale * cz,
    })
}

// ── quaternion helpers ──────────────────────────────────────────────

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
