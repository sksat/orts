//! E2E test: dynamic `AddSatellite` for controlled (plugin-backed) mode.
//!
//! Spawns `orts serve` with a TOML config that starts a controlled
//! simulation (pd-rw-control guest), connects via WebSocket, sends
//! an `add_satellite` message with a controller config, and verifies
//! the server responds with `satellite_added` and then streams
//! `state` messages for the new satellite.
//!
//! Requires:
//! - `plugin-wasm-async` feature enabled
//! - `plugins/pd-rw-control/target/wasm32-wasip1/release/...wasm`
//!   built (soft-skips cleanly otherwise)
//! - An `orts` binary to run; picks it up from `ORTS_BIN` if set
//!   (CI `cli-plugin-backend-e2e` job), otherwise
//!   `CARGO_BIN_EXE_orts` (local `cargo test`).

#![cfg(feature = "plugin-wasm-async")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

/// Resolve the absolute path to the pd-rw-control guest WASM, or
/// `None` if it has not been built.
fn pd_rw_guest_wasm() -> Option<std::path::PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm"
    ));
    if wasm_path.exists() {
        Some(wasm_path)
    } else {
        eprintln!(
            "WASM not found: {}\n\
             Build: cd plugins/pd-rw-control && cargo +1.91.0 component build --release\n\
             Skipping serve dynamic-add e2e test.",
            wasm_path.display()
        );
        None
    }
}

/// Pick a port unlikely to collide with other processes / other tests.
fn test_port() -> u16 {
    // Distinct from ws_e2e's test_port (19000..).
    let pid = std::process::id();
    20000 + (pid % 1000) as u16
}

fn write_controlled_config(wasm_path: &std::path::Path) -> tempfile::NamedTempFile {
    let toml = format!(
        r#"body = "earth"
dt = 0.1
output_interval = 1.0
duration = 60.0
epoch = "2024-01-01T00:00:00Z"
stream_interval = 1.0

[[satellites]]
id = "initial-sat"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500
initial_quaternion = [0.966, 0, 0.259, 0]
initial_angular_velocity = [0.0, 0.0, 0.0]

[satellites.controller]
type = "wasm"
path = "{wasm_path}"

[satellites.controller.config]
kp = 1.0
kd = 2.0
sample_period = 0.1

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_momentum = 1.0
max_torque = 0.5
"#,
        wasm_path = wasm_path.display()
    );

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("tempfile");
    file.write_all(toml.as_bytes()).expect("write toml");
    file
}

/// A running server with its child process and stderr drain thread.
struct Server {
    child: std::process::Child,
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    fn spawn_with_config(port: u16, config_path: &str) -> Self {
        let binary = orts_binary();
        let mut child = Command::new(&binary)
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--config",
                config_path,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {binary}: {e}"));

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let (tx, rx) = mpsc::channel::<()>();

        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut notified = false;
            for line in reader.lines() {
                let Ok(line) = line else { break };
                eprintln!("[server stderr] {line}");
                if !notified && line.contains("Server listening on") {
                    let _ = tx.send(());
                    notified = true;
                }
            }
            if !notified {
                let _ = tx.send(());
            }
        });

        rx.recv_timeout(Duration::from_secs(15))
            .expect("server did not print 'listening' message within 15 seconds");

        Server {
            child,
            _stderr_thread: stderr_thread,
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn next_json(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> serde_json::Value {
    let msg = read
        .next()
        .await
        .expect("expected message, got end of stream")
        .expect("error reading message");
    let text = msg.into_text().expect("message is not text");
    serde_json::from_str(&text).expect("message is not valid JSON")
}

async fn read_until_type(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    msg_type: &str,
    max_messages: usize,
) -> serde_json::Value {
    for _ in 0..max_messages {
        let msg = next_json(read).await;
        if msg["type"] == msg_type {
            return msg;
        }
    }
    panic!("did not receive message type '{msg_type}' within {max_messages} messages");
}

#[tokio::test]
async fn serve_dynamic_controlled_add_succeeds() {
    let Some(wasm_path) = pd_rw_guest_wasm() else {
        return;
    };
    let cfg_file = write_controlled_config(&wasm_path);
    let cfg_path = cfg_file.path().to_string_lossy().to_string();

    let port = test_port();
    let mut server = Server::spawn_with_config(port, &cfg_path);

    // Give the server a moment to bring up the initial fleet.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let url = format!("ws://localhost:{port}/ws");
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        // info + history come first.
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let _history = next_json(&mut read).await;

        // Wait for the initial satellite to start streaming state.
        let _initial_state = read_until_type(&mut read, "state", 200).await;

        // Send add_satellite with a controlled config. The new
        // satellite must have attitude + controller (otherwise the
        // server returns an error).
        let add_sat = serde_json::json!({
            "type": "add_satellite",
            "id": "dynamic-sat",
            "name": "Dynamically Added Controlled",
            "orbit": { "type": "circular", "altitude": 500.0 },
            "attitude": {
                "inertia_diag": [10.0, 10.0, 10.0],
                "mass": 500.0,
                "initial_quaternion": [1.0, 0.0, 0.0, 0.0],
                "initial_angular_velocity": [0.0, 0.0, 0.0],
            },
            "controller": {
                "type": "wasm",
                "path": wasm_path.display().to_string(),
                "config": {
                    "kp": 1.0,
                    "kd": 2.0,
                    "sample_period": 0.1,
                },
            },
            "sensors": ["gyroscope", "star_tracker"],
            "reaction_wheels": {
                "type": "three_axis",
                "inertia": 0.01,
                "max_momentum": 1.0,
                "max_torque": 0.5,
            },
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                add_sat.to_string().into(),
            ))
            .await
            .expect("failed to send add_satellite");

        // Expect a satellite_added response referencing the new sat.
        let added = read_until_type(&mut read, "satellite_added", 400).await;
        assert_eq!(
            added["satellite"]["id"], "/world/sat/dynamic-sat",
            "added satellite id mismatch"
        );
        assert!(
            added["t"].as_f64().is_some(),
            "added satellite must report a time"
        );

        // The new satellite should start producing state messages.
        let mut saw_dynamic_state = false;
        for _ in 0..400 {
            let msg = next_json(&mut read).await;
            if msg["type"] == "state" && msg["entity_path"] == "/world/sat/dynamic-sat" {
                saw_dynamic_state = true;
                break;
            }
        }
        assert!(
            saw_dynamic_state,
            "should receive state messages for the dynamically added controlled satellite"
        );
    })
    .await;

    server.kill();
    result.expect("test timed out");
}
