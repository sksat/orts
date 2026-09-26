//! Test fixture, not an example to copy: a guest that breaks the host's
//! limits in the way its config asks, so the host's tests can check that each
//! one fails with an error instead of hanging or exhausting the process.
//!
//! `{"fault": "<fault>"}` picks the behaviour; without a config, or with
//! `"fault": "none"`, the guest behaves and commands nothing.
//!
//! | `fault` | what the guest does |
//! |---|---|
//! | `none` | returns no command every tick |
//! | `spin-in-init` | loops forever in `init`, which the host reaches through `metadata` |
//! | `wait-tick-in-init` | calls `wait-tick` from `init`, before `run` has started |
//! | `spin-in-update` | loops forever in its first `update` |
//! | `grow-memory-in-update` | allocates 1 MiB after 1 MiB in its first `update` |
//! | `sleep-in-update` | sleeps for an hour in its first `update` (a WASI clock wait) |
//! | `flood-messages-in-update` | sends `count` msg-io messages in every `update` |

use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, msg, orts_plugin};

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Fault {
    None,
    SpinInInit,
    WaitTickInInit,
    SpinInUpdate,
    GrowMemoryInUpdate,
    SleepInUpdate,
    FloodMessagesInUpdate,
}

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    fault: Fault,
    sample_period: f64,
    /// How many messages `flood-messages-in-update` sends per update.
    count: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fault: Fault::None,
            sample_period: 0.1,
            count: 5000,
        }
    }
}

struct MisbehavingGuest {
    config: Config,
}

/// Spin without end, in a way the compiler keeps.
fn spin() -> ! {
    let mut n = 0u64;
    loop {
        n = std::hint::black_box(n.wrapping_add(1));
    }
}

impl Plugin<TickInput, Command> for MisbehavingGuest {
    fn sample_period(&self) -> f64 {
        self.config.sample_period
    }

    fn init(config: &str) -> Result<Self, String> {
        let config: Config = if config.is_empty() {
            Config::default()
        } else {
            serde_json::from_str(config).map_err(|e| format!("config parse error: {e}"))?
        };
        match config.fault {
            Fault::SpinInInit => spin(),
            Fault::WaitTickInInit => {
                let _ = orts_plugin_sdk::bindings::orts::plugin::tick_io::wait_tick();
            }
            _ => {}
        }
        Ok(Self { config })
    }

    fn update(&mut self, _input: &TickInput) -> Result<Option<Command>, String> {
        match self.config.fault {
            Fault::SpinInUpdate => spin(),
            Fault::GrowMemoryInUpdate => {
                // Reserve without writing, so each step is one memory.grow
                // rather than a fill the interpreter walks byte by byte.
                let mut kept: Vec<Vec<u8>> = Vec::new();
                loop {
                    let mut chunk = Vec::<u8>::with_capacity(1 << 20);
                    chunk.push(1);
                    kept.push(std::hint::black_box(chunk));
                }
            }
            Fault::SleepInUpdate => {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
            Fault::FloodMessagesInUpdate => {
                for _ in 0..self.config.count {
                    msg::send_to(
                        NodeId::Ground,
                        "test.flood.v1",
                        Payload::Binary(vec![0; 16]),
                    );
                }
            }
            Fault::None | Fault::SpinInInit | Fault::WaitTickInInit => {}
        }
        Ok(None)
    }
}

orts_plugin!(MisbehavingGuest);
