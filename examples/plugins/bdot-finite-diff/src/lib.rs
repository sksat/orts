//! B-dot finite-difference detumbling controller — WASM Component guest.
//!
//! This is the **third independent implementation** of the same control
//! law: after `orts::attitude::BdotFiniteDiff` (native) and the plugin-
//! layer test-only reference in `orts/tests/plugin_bdot_finitediff.rs`.
//! Bit-exact agreement between these three implementations is the Phase
//! P1 determinism oracle.
//!
//! The guest uses the WIT `host-env.magnetic-field-eci` import to
//! evaluate the geomagnetic field at each sample tick, transforms it
//! to the body frame, and computes the finite-difference dB/dt
//! approximation.

#[allow(warnings)]
mod bindings;

use bindings::exports::orts::plugin::controller::Guest;
use bindings::orts::plugin::host_env;
use bindings::orts::plugin::types::*;

/// Internal state for the finite-difference B-dot controller.
struct BdotFiniteDiff {
    gain: f64,
    max_moment: [f64; 3],
    sample_period: f64,
    prev_b_body: Option<[f64; 3]>,
    prev_t: f64,
}

/// Global singleton. Component Model guests are single-instance so
/// we use a `RefCell` for interior mutability (WASM is single-threaded).
use core::cell::RefCell;

thread_local! {
    static STATE: RefCell<BdotFiniteDiff> = RefCell::new(BdotFiniteDiff {
        gain: 1e4,
        max_moment: [10.0, 10.0, 10.0],
        sample_period: 1.0,
        prev_b_body: None,
        prev_t: 0.0,
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
        // JSON config: {"gain": 1e4, "max_moment": 10.0, "sample_period": 1.0}
        // All fields optional (defaults apply if omitted).
        #[derive(serde::Deserialize)]
        #[serde(default)]
        struct Config {
            gain: f64,
            max_moment: f64,
            sample_period: f64,
        }
        impl Default for Config {
            fn default() -> Self {
                Self {
                    gain: 1e4,
                    max_moment: 10.0,
                    sample_period: 1.0,
                }
            }
        }
        let cfg: Config =
            serde_json::from_str(&config).map_err(|e| format!("config parse error: {e}"))?;
        STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.gain = cfg.gain;
            s.max_moment = [cfg.max_moment, cfg.max_moment, cfg.max_moment];
            s.sample_period = cfg.sample_period;
        });
        Ok(())
    }

    fn initial_command() -> Command {
        Command::MagneticMoment(Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })
    }

    fn update(obs: Observation) -> Result<Command, String> {
        STATE.with(|state| {
            let mut s = state.borrow_mut();

            let epoch = match obs.epoch {
                Some(e) => e,
                None => {
                    return Ok(Command::MagneticMoment(Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    }));
                }
            };

            // Query the host's geomagnetic field model via host-env import.
            let b_eci = host_env::magnetic_field_eci(obs.spacecraft.orbit.position, epoch);

            let b_mag_sq = b_eci.x * b_eci.x + b_eci.y * b_eci.y + b_eci.z * b_eci.z;
            // Threshold 1e-60 (native uses 1e-30 on magnitude, equivalent
            // to 1e-60 on squared magnitude). Both are far below any
            // realistic geomagnetic field so the difference is cosmetic.
            if b_mag_sq < 1e-60 {
                return Ok(Command::MagneticMoment(Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                }));
            }

            // Transform B from ECI to body frame using the attitude quaternion.
            let q = &obs.spacecraft.attitude.orientation;
            let b_body = quat_rotate_inverse(q, &b_eci);

            let m_cmd = match s.prev_b_body {
                Some(prev_b) => {
                    let dt = obs.t - s.prev_t;
                    if dt < 1e-15 {
                        return Ok(Command::MagneticMoment(Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        }));
                    }
                    let db_x = (b_body.x - prev_b[0]) / dt;
                    let db_y = (b_body.y - prev_b[1]) / dt;
                    let db_z = (b_body.z - prev_b[2]) / dt;
                    let mx = clamp(-s.gain * db_x, -s.max_moment[0], s.max_moment[0]);
                    let my = clamp(-s.gain * db_y, -s.max_moment[1], s.max_moment[1]);
                    let mz = clamp(-s.gain * db_z, -s.max_moment[2], s.max_moment[2]);
                    [mx, my, mz]
                }
                None => [0.0, 0.0, 0.0],
            };

            s.prev_b_body = Some([b_body.x, b_body.y, b_body.z]);
            s.prev_t = obs.t;

            Ok(Command::MagneticMoment(Vec3 {
                x: m_cmd[0],
                y: m_cmd[1],
                z: m_cmd[2],
            }))
        })
    }

    fn current_mode() -> Option<String> {
        None
    }
}

bindings::export!(Component with_types_in bindings);

// ─── helpers ────────────────────────────────────────────────────

/// Rotate a vector from the inertial frame to the body frame using
/// the conjugate of the given quaternion (body→inertial).
///
/// q is Hamilton scalar-first (w, x, y, z).
/// v_body = q* · v_eci · q  (passive rotation).
fn quat_rotate_inverse(q: &Quat, v: &Vec3) -> Vec3 {
    // Conjugate: negate the vector part.
    let qw = q.w;
    let qx = -q.x;
    let qy = -q.y;
    let qz = -q.z;

    // t = 2 * (q_vec × v)
    let tx = 2.0 * (qy * v.z - qz * v.y);
    let ty = 2.0 * (qz * v.x - qx * v.z);
    let tz = 2.0 * (qx * v.y - qy * v.x);

    Vec3 {
        x: v.x + qw * tx + (qy * tz - qz * ty),
        y: v.y + qw * ty + (qz * tx - qx * tz),
        z: v.z + qw * tz + (qx * ty - qy * tx),
    }
}

fn clamp(val: f64, lo: f64, hi: f64) -> f64 {
    if val < lo {
        lo
    } else if val > hi {
        hi
    } else {
        val
    }
}
