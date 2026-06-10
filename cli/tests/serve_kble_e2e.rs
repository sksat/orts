//! E2E: the stream-io bridge through a **real `kble` process**.
//!
//! Topology (kble routes between two of its `ws://` plugs):
//!
//! ```text
//! orts serve (bridge endpoint)  ◀─ws─  kble  ─ws▶  this test (WS server)
//!        FSW: framed-commander                      plays the ground tool
//! ```
//!
//! The test uplinks a `SYNC|LEN|payload|CRC16` framed command through kble
//! and expects the FSW's framed reply back through kble — proving the
//! virtual-harness integration end-to-end with the actual kble binary.
//!
//! Soft-skips when the framed-commander guest WASM or the `kble` binary is
//! missing. CI downloads the prebuilt musl `kble` (pinned release); locally
//! `cargo install kble` (or set `KBLE_BIN`).

#![cfg(feature = "plugin-wasm-async")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

const SYNC: [u8; 2] = [0xEB, 0x90];

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

/// Locate the kble binary: `KBLE_BIN` env, or `kble` on `PATH`.
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
    eprintln!(
        "kble binary not found (PATH / KBLE_BIN).\n\
         Install: cargo install kble — or download a release binary.\n\
         Skipping real-kble e2e test."
    );
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
            "WASM not found: {}\n\
             Build: cd plugin-sdk/examples && cargo +1.91.0 component build \
             -p orts-example-plugin-stream-framed-commander --release\n\
             Skipping real-kble e2e test.",
            wasm_path.display()
        );
        None
    }
}

/// Distinct port range from the other serve E2Es (19000/20000/21000..).
fn test_port() -> u16 {
    let pid = std::process::id();
    22000 + (pid % 500) as u16 * 2
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

/// kble spaghetti: wire orts' bridge endpoint to this test's WS server,
/// bidirectionally (two unidirectional links).
fn write_spaghetti(orts_port: u16, peer_port: u16) -> tempfile::NamedTempFile {
    let yaml = format!(
        r#"plugs:
  orts: ws://127.0.0.1:{orts_port}/stream/sat0/comlink
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

fn spawn_serve(port: u16, config_path: &str) -> ChildGuard {
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
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut notified = false;
        for line in reader.lines() {
            let Ok(line) = line else { break };
            eprintln!("[server stderr] {line}");
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
    ChildGuard(child)
}

// ─── framing (mirror of the guest's wire format) ────────────────

fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn build_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut body = Vec::new();
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(payload);
    let crc = crc16_ccitt(&body);
    let mut frame = Vec::new();
    frame.extend_from_slice(&SYNC);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

fn parse_frame(bytes: &[u8]) -> Option<Vec<u8>> {
    let pos = bytes.windows(2).position(|w| w == SYNC)?;
    let rest = &bytes[pos..];
    if rest.len() < 4 {
        return None;
    }
    let len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
    if rest.len() < 4 + len + 2 {
        return None;
    }
    let crc_calc = crc16_ccitt(&rest[2..4 + len]);
    let crc_rx = u16::from_be_bytes([rest[4 + len], rest[5 + len]]);
    (crc_calc == crc_rx).then(|| rest[4..4 + len].to_vec())
}

// ─────────────────────────── test ──────────────────────────────

#[tokio::test]
async fn framed_command_round_trips_through_real_kble() {
    let Some(wasm) = framed_commander_wasm() else {
        return;
    };
    let Some(kble) = kble_binary() else {
        return;
    };

    let config = write_config(&wasm);
    let orts_port = test_port();
    let peer_port = orts_port + 1;
    let _serve = spawn_serve(orts_port, config.path().to_str().unwrap());

    // This test is the second kble plug: a plain WS server that kble
    // connects to (the shape of any external ground tool).
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", peer_port))
        .await
        .expect("bind peer port");

    let spaghetti = write_spaghetti(orts_port, peer_port);
    let mut kble_child = ChildGuard(
        Command::new(&kble)
            .args(["-s", spaghetti.path().to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {kble}: {e}")),
    );
    // Surface kble's own logs in test output.
    if let Some(stderr) = kble_child.0.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("[kble stderr] {line}");
            }
        });
    }

    // kble dials our plug; accept and complete the WS handshake.
    let (stream, _addr) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("kble did not connect within 10 s")
        .expect("accept failed");
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .expect("WS handshake with kble failed");
    let (mut write, mut read) = ws.split();

    // Uplink the framed command through kble.
    write
        .send(Message::Binary(build_frame(b"nadir").into()))
        .await
        .expect("send framed command");

    // Await the FSW's framed reply, reassembling across messages.
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
                other => panic!("kble connection ended unexpectedly: {other:?}"),
            }
        }
    })
    .await
    .expect("no framed reply through kble within 15 s");

    assert_eq!(
        reply,
        b"nadir".to_vec(),
        "framed command must round-trip orts ↔ kble ↔ ground tool"
    );

    kble_child.kill();
}
