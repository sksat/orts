//! E2E tests: controlled satellites a WebSocket client adds to `orts serve`,
//! and the controller components the client sends for them.
//!
//! A client does not name a controller by a path on the server: it sends the
//! component's bytes as a binary message on `/ws`, the server replies with
//! `controller_uploaded` and their SHA-256, and a controller config names the
//! component by that digest. A config file given on the server's command line
//! still names its controller by `path`.
//!
//! Requires:
//! - `plugin-wasm-async` feature enabled
//! - `plugin-sdk/examples/target/wasm32-wasip1/release/...wasm`
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
use tokio_tungstenite::tungstenite::Message;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsRead = futures_util::stream::SplitStream<WsStream>;
type WsWrite = futures_util::stream::SplitSink<WsStream, Message>;

/// How long one message may take before the test calls the server stuck. The
/// replies waited on here come from checks that open no file, so seconds are
/// generous; before the fix, a FIFO path produced no reply at all.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

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
        "{manifest_dir}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm"
    ));
    if wasm_path.exists() {
        Some(wasm_path)
    } else {
        eprintln!(
            "WASM not found: {}\n\
             Build: cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-pd-rw-control --release\n\
             Skipping serve dynamic-add e2e test.",
            wasm_path.display()
        );
        None
    }
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

/// A controlled satellite, as a client sends one, with its controller's
/// component named by `component` (`{"sha256": …}` or `{"path": …}`).
///
/// The initial attitude is off the guest's identity target, so a controller
/// that runs commands the wheels at once.
fn controlled_satellite(id: &str, component: serde_json::Value) -> serde_json::Value {
    let mut controller = serde_json::json!({
        "type": "wasm",
        "config": { "kp": 1.0, "kd": 2.0, "sample_period": 0.1 },
    });
    for (k, v) in component.as_object().expect("component fields") {
        controller[k] = v.clone();
    }
    serde_json::json!({
        "id": id,
        "name": "Dynamically Added Controlled",
        "orbit": { "type": "circular", "altitude": 500.0 },
        "attitude": {
            "inertia_diag": [10.0, 10.0, 10.0],
            "mass": 500.0,
            "initial_quaternion": [0.966, 0.0, 0.259, 0.0],
            "initial_angular_velocity": [0.0, 0.0, 0.0],
        },
        "controller": controller,
        "sensors": ["gyroscope", "star_tracker"],
        "reaction_wheels": {
            "type": "three_axis",
            "inertia": 0.01,
            "max_momentum": 1.0,
            "max_torque": 0.5,
        },
    })
}

/// `satellite` as an `add_satellite` message, which flattens it beside the tag.
fn add_satellite(satellite: serde_json::Value) -> serde_json::Value {
    let mut msg = satellite;
    msg["type"] = "add_satellite".into();
    msg
}

/// A FIFO nobody writes to: opening it for reading blocks until a writer
/// comes, so a server that opened it would stop answering.
#[cfg(unix)]
fn writerless_fifo(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let fifo = dir.path().join("ctrl.fifo");
    let status = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");
    fifo
}

/// A running server with its child process and stderr drain thread. Killed
/// on drop, so a failing assertion does not leave it behind.
struct Server {
    child: std::process::Child,
    port: u16,
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    /// Start `orts serve` on a port the OS picks, with `extra` args, and wait
    /// for it to announce the port.
    fn spawn(extra: &[&str]) -> Self {
        let binary = orts_binary();
        let mut child = Command::new(&binary)
            .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
            .args(["serve", "--port", "0"])
            .args(extra)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {binary}: {e}"));

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let (tx, rx) = mpsc::channel::<Option<u16>>();

        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut notified = false;
            for line in reader.lines() {
                let Ok(line) = line else { break };
                eprintln!("[server stderr] {line}");
                if !notified
                    && let Some(rest) = line.strip_prefix("WebSocket endpoint: ws://localhost:")
                {
                    let _ = tx.send(rest.trim_end_matches("/ws").parse().ok());
                    notified = true;
                }
            }
            if !notified {
                let _ = tx.send(None);
            }
        });

        let port = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("server did not announce its WebSocket endpoint within 15 seconds")
            .expect("server exited before announcing its WebSocket endpoint");

        Server {
            child,
            port,
            _stderr_thread: stderr_thread,
        }
    }

    async fn connect(&self) -> (WsWrite, WsRead) {
        let url = format!("ws://localhost:{}/ws", self.port);
        let (ws, _) = tokio::time::timeout(REPLY_TIMEOUT, connect_async(&url))
            .await
            .expect("the WebSocket handshake did not complete in time")
            .expect("failed to connect");
        ws.split()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn next_json(read: &mut WsRead) -> serde_json::Value {
    let msg = tokio::time::timeout(REPLY_TIMEOUT, read.next())
        .await
        .expect("no message from the server in time")
        .expect("expected message, got end of stream")
        .expect("error reading message");
    let text = msg.into_text().expect("message is not text");
    serde_json::from_str(&text).expect("message is not valid JSON")
}

/// Read until a message matches `pred`, skipping the stream of states and
/// the rest. Panics after `max_messages`.
async fn read_until(
    read: &mut WsRead,
    what: &str,
    max_messages: usize,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..max_messages {
        let msg = next_json(read).await;
        if pred(&msg) {
            return msg;
        }
    }
    panic!("did not receive {what} within {max_messages} messages");
}

async fn read_until_type(read: &mut WsRead, msg_type: &str) -> serde_json::Value {
    read_until(read, msg_type, 400, |m| m["type"] == msg_type).await
}

/// The first `error` or `satellite_added` — what answers an `add_satellite`.
async fn add_reply(read: &mut WsRead) -> serde_json::Value {
    read_until(read, "the reply to the add", 400, |m| {
        m["type"] == "error" || m["type"] == "satellite_added"
    })
    .await
}

async fn send_json(write: &mut WsWrite, msg: &serde_json::Value) {
    write
        .send(Message::Text(msg.to_string().into()))
        .await
        .expect("failed to send");
}

/// A client uploads the guest and adds a controlled satellite naming it, and
/// the satellite's controller runs. The initial satellite comes from a
/// `--config` file naming its controller by path, which still works.
///
/// The component and the `add_satellite` go out back to back: frames on one
/// socket are handled in order, so the client need not wait for the digest.
#[tokio::test]
async fn serve_dynamic_controlled_add_succeeds() {
    let Some(wasm_path) = pd_rw_guest_wasm() else {
        return;
    };
    let cfg_file = write_controlled_config(&wasm_path);
    let cfg_path = cfg_file.path().to_string_lossy().to_string();
    let server = Server::spawn(&["--config", &cfg_path]);

    let wasm = std::fs::read(&wasm_path).expect("read the guest");
    let sha256 = orts::plugin::wasm::ComponentBytes::new(wasm.clone()).sha256_hex();

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let (mut write, mut read) = server.connect().await;

        // info + history come first.
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let _history = next_json(&mut read).await;

        // The initial satellite, whose controller the config names by path,
        // streams state.
        let initial = read_until_type(&mut read, "state").await;
        assert_eq!(initial["entity_path"], "/world/sat/initial-sat");

        write
            .send(Message::Binary(wasm.clone().into()))
            .await
            .expect("failed to send the component");
        let sat = controlled_satellite("dynamic-sat", serde_json::json!({ "sha256": sha256 }));
        send_json(&mut write, &add_satellite(sat)).await;

        let uploaded = read_until(&mut read, "controller_uploaded or error", 400, |m| {
            m["type"] == "controller_uploaded" || m["type"] == "error"
        })
        .await;
        assert_eq!(uploaded["type"], "controller_uploaded", "{uploaded}");
        assert_eq!(
            uploaded["sha256"],
            sha256.as_str(),
            "the digest of the bytes sent"
        );
        assert_eq!(uploaded["size"], wasm.len());

        // Expect a satellite_added response referencing the new sat, keeping the
        // new satellite's own state message on the way: the add broadcasts
        // `[state, added]` in that order, and that state is the one built
        // outside `snapshot` — the sample a regression would empty.
        let mut added: Option<serde_json::Value> = None;
        let mut add_time_state: Option<serde_json::Value> = None;
        for _ in 0..400 {
            let msg = next_json(&mut read).await;
            assert_ne!(msg["type"], "error", "the add is refused: {msg}");
            if msg["type"] == "state" && msg["entity_path"] == "/world/sat/dynamic-sat" {
                add_time_state.get_or_insert(msg);
                continue;
            }
            if msg["type"] == "satellite_added" {
                added = Some(msg);
                break;
            }
        }
        let added = added.expect("did not receive message type 'satellite_added'");
        assert_eq!(
            added["satellite"]["id"], "/world/sat/dynamic-sat",
            "added satellite id mismatch"
        );
        let t_added = added["t"]
            .as_f64()
            .expect("added satellite must report a time");
        // The announcement names the models built for this satellite, which
        // the viewer keys its charts on. The controlled path reads them off
        // `SpacecraftDynamics`, where the orbit-only path reads an
        // `OrbitalSystem` — a separate call, so it is asserted separately.
        let models: Vec<&str> = added["satellite"]["perturbations"]
            .as_array()
            .expect("perturbations is an array")
            .iter()
            .map(|m| m.as_str().expect("a model name"))
            .collect();
        assert!(
            models.contains(&"gravity_gradient"),
            "a controlled satellite with attitude reports its disturbance models: {models:?}"
        );

        // The new satellite's first state message — the one the add itself
        // broadcast — carries what every later one does: the acceleration
        // breakdown and a torque per model.
        let first_state = add_time_state
            .expect("should receive state messages for the dynamically added controlled satellite");
        assert!(
            first_state["accelerations"]["gravity"].as_f64().is_some(),
            "the first sample should carry the acceleration breakdown: {first_state}"
        );
        let torques = first_state["torques"]
            .as_array()
            .unwrap_or_else(|| panic!("the first sample should carry torques: {first_state}"));
        assert!(
            torques
                .iter()
                .any(|t| t["model"] == "gravity_gradient" && t["torque_body_nm"].is_array()),
            "a torque per model, the gravity gradient among them: {first_state}"
        );

        // The uploaded controller runs: starting off its target, it commands
        // the wheels, and a later sample shows wheel momentum.
        read_until(&mut read, "a later state with wheel momentum", 400, |m| {
            m["type"] == "state"
                && m["entity_path"] == "/world/sat/dynamic-sat"
                && m["t"].as_f64().is_some_and(|t| t > t_added)
                && m["attitude"]["rw_momentum"]
                    .as_array()
                    .is_some_and(|h| h.iter().any(|x| x.as_f64().is_some_and(|x| x != 0.0)))
        })
        .await;
    })
    .await;

    result.expect("test timed out");
}

/// An `add_satellite` naming its controller by a path is refused, and the
/// server never opens the path: the path is a FIFO nobody writes to, and
/// opening it held the manager in a read that never returned before this was
/// refused. The simulation keeps streaming and a new connection is answered.
#[cfg(unix)]
#[tokio::test]
async fn an_add_naming_a_controller_path_is_refused_without_opening_it() {
    let Some(wasm_path) = pd_rw_guest_wasm() else {
        return;
    };
    let cfg_file = write_controlled_config(&wasm_path);
    let cfg_path = cfg_file.path().to_string_lossy().to_string();
    let server = Server::spawn(&["--config", &cfg_path]);
    let dir = tempfile::tempdir().expect("temp dir");
    let fifo = writerless_fifo(&dir);

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let (mut write, mut read) = server.connect().await;
        let _state = read_until_type(&mut read, "state").await;

        let path = fifo.display().to_string();
        let sat = controlled_satellite("fifo-sat", serde_json::json!({ "path": path }));
        send_json(&mut write, &add_satellite(sat)).await;
        let reply = add_reply(&mut read).await;
        assert_eq!(reply["type"], "error", "{reply}");
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.starts_with("add_satellite.controller: `path` is not accepted over WebSocket"),
            "{message}"
        );

        // Still running: the initial satellite streams on.
        let after = read_until_type(&mut read, "state").await;
        assert_eq!(after["entity_path"], "/world/sat/initial-sat");

        let (_write2, mut read2) = server.connect().await;
        let info = next_json(&mut read2).await;
        assert_eq!(info["type"], "info", "a new connection is answered: {info}");
    })
    .await;

    result.expect("test timed out");
}

/// A `start_simulation` naming its controller by a path is refused without the
/// path being opened, and the server stays idle and answering.
#[cfg(unix)]
#[tokio::test]
async fn a_start_naming_a_controller_path_is_refused_without_opening_it() {
    // Skipped with the others, so a checkout without the guest runs none of
    // this file rather than a part of it.
    if pd_rw_guest_wasm().is_none() {
        return;
    }
    let server = Server::spawn(&[]);
    let dir = tempfile::tempdir().expect("temp dir");
    let fifo = writerless_fifo(&dir);

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let (mut write, mut read) = server.connect().await;
        let status = next_json(&mut read).await;
        assert_eq!(status["state"], "idle", "{status}");

        let path = fifo.display().to_string();
        let sat = controlled_satellite("fifo-sat", serde_json::json!({ "path": path }));
        send_json(
            &mut write,
            &serde_json::json!({
                "type": "start_simulation",
                "config": { "dt": 0.1, "epoch": "2024-01-01T00:00:00Z", "satellites": [sat] },
            }),
        )
        .await;
        let reply = next_json(&mut read).await;
        assert_eq!(reply["type"], "error", "{reply}");
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.starts_with("satellites[0].controller: `path` is not accepted over WebSocket"),
            "{message}"
        );
        assert!(
            !message.contains("ctrl.fifo"),
            "the path is not echoed: {message}"
        );

        let (_write2, mut read2) = server.connect().await;
        let status = next_json(&mut read2).await;
        assert_eq!(
            status["state"], "idle",
            "a new connection is answered: {status}"
        );
    })
    .await;

    result.expect("test timed out");
}

/// An idle server starts a controlled simulation from a component its client
/// uploaded, and a second connection cannot name that component without
/// sending it itself.
#[tokio::test]
async fn an_uploaded_component_starts_a_simulation_on_its_own_connection() {
    let Some(wasm_path) = pd_rw_guest_wasm() else {
        return;
    };
    let server = Server::spawn(&[]);
    let wasm = std::fs::read(&wasm_path).expect("read the guest");

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let (mut write, mut read) = server.connect().await;
        let status = next_json(&mut read).await;
        assert_eq!(status["state"], "idle", "{status}");

        write
            .send(Message::Binary(wasm.clone().into()))
            .await
            .expect("failed to send the component");
        let uploaded = next_json(&mut read).await;
        assert_eq!(uploaded["type"], "controller_uploaded", "{uploaded}");
        let sha256 = uploaded["sha256"].as_str().expect("a digest").to_string();

        let sat = controlled_satellite("uploaded-sat", serde_json::json!({ "sha256": sha256 }));
        send_json(
            &mut write,
            &serde_json::json!({
                "type": "start_simulation",
                "config": {
                    "dt": 0.1,
                    "output_interval": 1.0,
                    "stream_interval": 1.0,
                    "epoch": "2024-01-01T00:00:00Z",
                    "satellites": [sat],
                },
            }),
        )
        .await;
        let info = read_until(&mut read, "info or error", 50, |m| {
            m["type"] == "info" || m["type"] == "error"
        })
        .await;
        assert_eq!(info["type"], "info", "{info}");
        let state = read_until_type(&mut read, "state").await;
        assert_eq!(state["entity_path"], "/world/sat/uploaded-sat");

        // Another connection has uploaded nothing, so the digest names nothing
        // there.
        let (mut write2, mut read2) = server.connect().await;
        let other = controlled_satellite("other-sat", serde_json::json!({ "sha256": sha256 }));
        send_json(&mut write2, &add_satellite(other)).await;
        let reply = add_reply(&mut read2).await;
        assert_eq!(reply["type"], "error", "{reply}");
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.contains("names no component sent on this connection"),
            "{message}"
        );
    })
    .await;

    result.expect("test timed out");
}
