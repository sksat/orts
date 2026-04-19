//! B-dot finite-difference detumbling controller — メインループ型。
//!
//! `wait_tick()` でホストから tick 入力を受け取り、有限差分法で dB/dt を
//! 近似して B-dot 則でコマンドを返す。

use orts_plugin_sdk::bindings;

use bindings::orts::plugin::tick_io::{send_command, wait_tick};
use bindings::orts::plugin::types::*;

struct Component;

impl bindings::Guest for Component {
    fn metadata(config: String) -> Result<PluginMetadata, String> {
        let cfg = Config::from_json(&config)?;
        Ok(PluginMetadata {
            sample_period_s: cfg.sample_period,
        })
    }

    fn current_mode() -> Option<String> {
        None
    }

    fn run(config: String) -> Result<(), String> {
        let cfg = Config::from_json(&config)?;

        let gain = cfg.gain;
        let max_moment = cfg.max_moment;
        let mut prev_b: Option<[f64; 3]> = None;
        let mut prev_t: f64 = 0.0;

        loop {
            let input = match wait_tick() {
                Some(input) => input,
                // ホストが shutdown を要求している。
                None => return Ok(()),
            };
            let b = input
                .sensors
                .magnetometers
                .first()
                .ok_or("magnetometer not available")?;

            let b_mag_sq = b.x * b.x + b.y * b.y + b.z * b.z;
            if b_mag_sq < 1e-60 {
                send_command(&zero_moment());
                // prev_b/prev_t を更新しない — near-zero サンプルは無視
                continue;
            }

            let cmd = match prev_b {
                Some(prev) => {
                    let dt = input.t - prev_t;
                    if dt < 1e-15 {
                        // dt ≈ 0: prev_b/prev_t を更新せず zero command のみ送る
                        send_command(&zero_moment());
                        continue;
                    }
                    let db_x = (b.x - prev[0]) / dt;
                    let db_y = (b.y - prev[1]) / dt;
                    let db_z = (b.z - prev[2]) / dt;
                    Command {
                        mtq: Some(MtqCommand::Moments(vec![
                            clamp(-gain * db_x, -max_moment, max_moment),
                            clamp(-gain * db_y, -max_moment, max_moment),
                            clamp(-gain * db_z, -max_moment, max_moment),
                        ])),
                        rw: None,
                        thruster: None,
                    }
                }
                None => zero_moment(),
            };

            send_command(&cmd);
            prev_b = Some([b.x, b.y, b.z]);
            prev_t = input.t;
        }
    }
}

bindings::export!(Component with_types_in bindings);

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

impl Config {
    fn from_json(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            Ok(Self::default())
        } else {
            serde_json::from_str(s).map_err(|e| format!("config parse error: {e}"))
        }
    }
}

fn zero_moment() -> Command {
    Command {
        mtq: Some(MtqCommand::Moments(vec![0.0, 0.0, 0.0])),
        rw: None,
        thruster: None,
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
