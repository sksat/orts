//! E2E: the stream-io stdio plug through a **real `kble` process**.
//!
//! Topology — kble spawns orts itself, as a real harness would:
//!
//! ```text
//! kble ─exec:─▶ orts serve --stream-stdio sat0/comlink   (kble-socket over stdio)
//!   └──ws──▶ this test (WS server)                        plays the ground tool
//! ```
//!
//! A `SYNC|LEN|payload|CRC16` framed command is uplinked through kble into
//! the orts child's stdin, deframed by the FSW, and the framed reply comes
//! back through the child's stdout — exercising the kble-socket stdio
//! protocol end-to-end. The reserved stream's WS endpoint must answer 409.
//!
//! Soft-skips when the kble binary (PATH / KBLE_BIN) or the guest WASM is
//! missing (same as serve_kble_e2e).

#![cfg(feature = "plugin-wasm-async")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
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

fn kble_binary() -> Option<String> {
    if let Ok(path) = std::env::var("KBLE_BIN") {
        return Some(path);
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("kble");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    eprintln!("kble binary not found (PATH / KBLE_BIN); skipping stdio e2e test.");
    None
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
            "WASM not found: {} — build the framed-commander guest first; skipping.",
            wasm_path.display()
        );
        None
    }
}

/// Distinct port range from the other serve E2Es (19000/20000/21000/22000..).
fn test_port() -> u16 {
    let pid = std::process::id();
    23000 + (pid % 500) as u16 * 2
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

/// kble passes the `exec:` URL path to `sh -c` without percent-decoding, so
/// a command line with spaces cannot be written inline — wrap it in a script.
/// Returns a closed [`tempfile::TempPath`]: keeping the write handle open
/// would make the kernel refuse to exec it (ETXTBSY).
fn write_launcher(orts_bin: &str, config_path: &str, orts_port: u16) -> tempfile::TempPath {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        "#!/bin/sh\nexec {orts_bin} serve --port {orts_port} --config {config_path} \
         --stream-stdio sat0/comlink\n"
    );
    let mut file = tempfile::Builder::new()
        .suffix(".sh")
        .tempfile()
        .expect("tempfile");
    file.write_all(script.as_bytes()).expect("write script");
    let mut perms = file.as_file().metadata().expect("metadata").permissions();
    perms.set_mode(0o755);
    file.as_file().set_permissions(perms).expect("chmod");
    file.into_temp_path()
}

/// kble spaghetti: kble spawns orts via the launcher as an `exec:` plug
/// (stdio carries the stream) and dials this test's WS server as the peer.
fn write_spaghetti(launcher_path: &str, peer_port: u16) -> tempfile::NamedTempFile {
    let yaml = format!(
        r#"plugs:
  orts: "exec:{launcher_path}"
  peer: ws://127.0.0.1:{peer_port}/
links:
  orts: peer
  peer: orts
"#
    );
    let mut file = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .expect("tempfile");
    file.write_all(yaml.as_bytes()).expect("write yaml");
    file
}

struct ChildGuard(std::process::Child);

impl ChildGuard {
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

#[tokio::test]
async fn framed_command_round_trips_through_kble_exec_stdio() {
    let Some(wasm) = framed_commander_wasm() else {
        return;
    };
    let Some(kble) = kble_binary() else {
        return;
    };

    let config = write_config(&wasm);
    let orts_port = test_port();
    let peer_port = orts_port + 1;

    // This test is the second kble plug: a plain WS server kble dials.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", peer_port))
        .await
        .expect("bind peer port");

    let launcher = write_launcher(&orts_binary(), config.path().to_str().unwrap(), orts_port);
    let spaghetti = write_spaghetti(launcher.to_str().unwrap(), peer_port);
    let mut kble_child = ChildGuard(
        Command::new(&kble)
            .args(["-s", spaghetti.path().to_str().unwrap()])
            .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {kble}: {e}")),
    );
    // Surface kble's (and the inherited orts child's) logs in test output.
    if let Some(stderr) = kble_child.0.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("[kble stderr] {line}");
            }
        });
    }

    // kble dials our plug as soon as it starts.
    let (stream, _addr) = tokio::time::timeout(Duration::from_secs(15), listener.accept())
        .await
        .expect("kble did not connect within 15 s")
        .expect("accept failed");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("WS handshake with kble failed");
    let (mut write, mut read) = ws.split();

    // Uplink the framed command. The orts child may still be starting its
    // simulation; bytes buffer in its stdin until the stdio plug attaches,
    // so sending early is safe.
    write
        .send(Message::Binary(build_frame(b"nadir").into()))
        .await
        .expect("send framed command");

    // Await the FSW's framed reply through kble (allow for orts startup +
    // WASM compilation + the 1 s realtime tick).
    let mut rxbuf: Vec<u8> = Vec::new();
    let reply = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    rxbuf.extend_from_slice(&bytes);
                    if let Some(payload) = parse_frame(&rxbuf) {
                        return payload;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("kble connection ended unexpectedly: {other:?}"),
            }
        }
    })
    .await
    .expect("no framed reply through kble within 30 s");
    assert_eq!(
        reply,
        b"nadir".to_vec(),
        "framed command must round-trip kble ↔ orts stdio ↔ FSW"
    );

    // The stdio-wired stream is exclusive: its WS endpoint must answer 409.
    let url = format!("ws://127.0.0.1:{orts_port}/stream/sat0/comlink");
    let err = tokio_tungstenite::connect_async(&url)
        .await
        .expect_err("the stdio-reserved stream must reject WS connections");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(
                response.status(),
                tokio_tungstenite::tungstenite::http::StatusCode::CONFLICT,
                "reserved stream must answer 409"
            );
        }
        other => panic!("expected an HTTP 409 rejection, got: {other:?}"),
    }

    kble_child.kill();
}
