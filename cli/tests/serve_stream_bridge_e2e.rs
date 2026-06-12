//! E2E: the stream-io kble bridge. Spawns `orts serve` with a satellite that
//! declares a `comlink` byte stream and runs the framed-commander FSW, then
//! plays the kble peer over the binary WebSocket endpoint
//! `ws://…/stream/{sat}/{stream}`: uplinks a `SYNC|LEN|payload|CRC16` framed
//! command and expects the FSW's framed reply — raw bytes end-to-end through
//! serve's realtime loop.
//!
//! Requires the framed-commander guest WASM (soft-skips otherwise):
//!
//! ```sh
//! cd plugin-sdk/examples
//! cargo +1.91.0 component build -p orts-example-plugin-stream-framed-commander --release
//! ```

#![cfg(feature = "plugin-wasm-async")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[path = "framed_protocol/mod.rs"]
mod framed_protocol;
use framed_protocol::{build_frame, parse_frame};

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

fn framed_commander_wasm() -> Option<std::path::PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_stream_framed_commander.wasm"
    ));
    if wasm_path.exists() {
        Some(wasm_path)
    } else {
        eprintln!(
            "WASM not found: {}\n\
             Build: cd plugin-sdk/examples && cargo +1.91.0 component build \
             -p orts-example-plugin-stream-framed-commander --release\n\
             Skipping stream bridge e2e test.",
            wasm_path.display()
        );
        None
    }
}

/// Distinct port range from the other serve E2Es (19000.. / 20000..).
fn test_port() -> u16 {
    let pid = std::process::id();
    21000 + (pid % 1000) as u16
}

fn write_config(wasm_path: &std::path::Path) -> tempfile::NamedTempFile {
    let toml = format!(
        r#"body = "earth"
dt = 0.1
output_interval = 1.0
stream_interval = 1.0
epoch = "2024-01-01T00:00:00Z"

[[satellites]]
id = "sat0"
sensors = ["gyroscope"]
streams = ["comlink"]

[satellites.orbit]
type = "circular"
altitude = 500

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 100
initial_quaternion = [1, 0, 0, 0]
initial_angular_velocity = [0, 0, 0]

[satellites.controller]
type = "wasm"
path = "{wasm}"
"#,
        wasm = wasm_path.display()
    );

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("tempfile");
    file.write_all(toml.as_bytes()).expect("write toml");
    file
}

struct Server {
    child: std::process::Child,
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    fn spawn_with_config(port: u16, config_path: &str) -> Self {
        let binary = orts_binary();
        let mut child = Command::new(&binary)
            .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
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
                // Ready once the bridge endpoint is registered (printed by
                // the sim loop context after controller construction).
                if !notified && line.contains("stream-io endpoint:") {
                    let _ = tx.send(());
                    notified = true;
                }
            }
            if !notified {
                let _ = tx.send(());
            }
        });

        rx.recv_timeout(Duration::from_secs(30))
            .expect("server did not register the stream endpoint within 30 seconds");

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

// Kill the server even when a test panics / returns early, so a leaked
// child can't hold the port and destabilise later tests.
impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

// ─────────────────────────── tests ─────────────────────────────

#[tokio::test]
async fn framed_command_round_trips_over_the_ws_bridge() {
    let Some(wasm) = framed_commander_wasm() else {
        return;
    };
    let config = write_config(&wasm);
    let port = test_port();
    let mut server = Server::spawn_with_config(port, config.path().to_str().unwrap());

    let url = format!("ws://127.0.0.1:{port}/stream/sat0/comlink");
    let (ws, _resp) = connect_async(&url)
        .await
        .unwrap_or_else(|e| panic!("connect to {url} failed: {e}"));
    let (mut write, mut read) = ws.split();

    // Uplink a framed "nadir" command as the kble peer would: raw bytes in a
    // binary WS message.
    write
        .send(Message::Binary(build_frame(b"nadir").into()))
        .await
        .expect("send framed command");

    // The FSW deframes it on its next tick (1 s realtime) and writes a
    // framed reply carrying the resulting mode. Reassemble across WS
    // messages in case the reply straddles pump boundaries.
    let mut rxbuf: Vec<u8> = Vec::new();
    let reply = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    rxbuf.extend_from_slice(&bytes);
                    if let Some(payload) = parse_frame(&rxbuf) {
                        return payload;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("bridge socket ended unexpectedly: {other:?}"),
            }
        }
    })
    .await
    .expect("no framed reply within 15 s");

    assert_eq!(
        reply,
        b"nadir".to_vec(),
        "FSW must apply the framed command and reply with the new mode"
    );

    server.kill();
}

#[tokio::test]
async fn undeclared_stream_endpoint_is_rejected() {
    let Some(wasm) = framed_commander_wasm() else {
        return;
    };
    let config = write_config(&wasm);
    let port = test_port() + 1;
    let mut server = Server::spawn_with_config(port, config.path().to_str().unwrap());

    // `uart0` is not declared in the config → the HTTP upgrade must be
    // rejected with 404 specifically (any other failure — e.g. server not
    // reachable — must not pass this test).
    let url = format!("ws://127.0.0.1:{port}/stream/sat0/uart0");
    let err = connect_async(&url)
        .await
        .expect_err("connecting to an undeclared stream endpoint must fail");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(
                response.status(),
                tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND,
                "undeclared stream must be rejected with 404"
            );
        }
        other => panic!("expected an HTTP 404 rejection, got: {other:?}"),
    }

    server.kill();
}
