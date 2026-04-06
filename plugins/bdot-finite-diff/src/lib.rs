//! B-dot finite-difference detumbling controller — WASM Component guest.
//!
//! The guest reads `sensors.magnetometer` from the tick input
//! (pre-evaluated by the host's magnetometer sensor) and computes the
//! finite-difference dB/dt approximation.

#[allow(warnings)]
mod bindings;

use bindings::exports::orts::plugin::controller::Guest;
use bindings::orts::plugin::types::*;

/// Internal state for the finite-difference B-dot controller.
struct BdotFiniteDiff {
    gain: f64,
    max_moment: [f64; 3],
    sample_period: f64,
    prev_b_body: Option<[f64; 3]>,
    prev_t: f64,
}

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

    fn update(input: TickInput) -> Result<Command, String> {
        STATE.with(|state| {
            let mut s = state.borrow_mut();

            let b_body = input
                .sensors
                .magnetometer
                .ok_or("magnetometer sensor not available")?;

            let b_mag_sq = b_body.x * b_body.x + b_body.y * b_body.y + b_body.z * b_body.z;
            if b_mag_sq < 1e-60 {
                return Ok(zero_moment());
            }

            let m_cmd = match s.prev_b_body {
                Some(prev_b) => {
                    let dt = input.t - s.prev_t;
                    if dt < 1e-15 {
                        return Ok(zero_moment());
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
            s.prev_t = input.t;

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

fn zero_moment() -> Command {
    Command::MagneticMoment(Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
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
