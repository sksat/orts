//! E2E tests for the `orts serve` argument contract.
//!
//! `serve` with no orbit and no config comes up idle and builds its
//! `SimParams` from the client's `start_simulation`, so sim args like `--dt`
//! never reach a simulation. They used to be dropped without a word; they are
//! now refused by name.

use futures_util::StreamExt;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Ports derived from the pid, offset per test to avoid collisions. The
/// 24000 band is this file's own: the other serve E2E suites run in separate
/// processes (so a different pid) and claim 19000-23999.
fn test_port(offset: u16) -> u16 {
    24000 + (std::process::id() % 500) as u16 * 4 + offset
}

/// Spawn `orts serve` with stderr captured into a shared buffer, plus a
/// channel that fires on the last line of the startup banner and the handle
/// of the reader thread (join it before reading the buffer to be sure stderr
/// was drained to EOF).
fn spawn(
    args: &[&str],
) -> (
    Child,
    Arc<Mutex<Vec<String>>>,
    mpsc::Receiver<()>,
    std::thread::JoinHandle<()>,
) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_orts"))
        .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
        .arg("serve")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn orts serve");

    let stderr = child.stderr.take().expect("failed to capture stderr");
    let collected = Arc::new(Mutex::new(Vec::<String>::new()));
    let (tx, rx) = mpsc::channel::<()>();
    let sink = Arc::clone(&collected);
    let reader = std::thread::spawn(move || {
        let mut signalled = false;
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("[server stderr] {line}");
            sink.lock().unwrap().push(line.clone());
            // Last line of the startup banner, printed whether or not a
            // simulation is started.
            if !signalled && line.contains("WebSocket endpoint") {
                let _ = tx.send(());
                signalled = true;
            }
        }
        if !signalled {
            let _ = tx.send(());
        }
    });
    (child, collected, rx, reader)
}

/// Spawn, wait for the startup banner, and return the child plus everything
/// stderr printed by then.
fn spawn_and_wait_for_startup(args: &[&str]) -> (Child, Vec<String>) {
    let (child, collected, rx, _reader) = spawn(args);
    rx.recv_timeout(Duration::from_secs(15))
        .expect("server produced no startup banner within 15 seconds");
    // The manager task prints its idle notice just after the banner; give it
    // a moment so its absence means "started a simulation", not "too early".
    std::thread::sleep(Duration::from_millis(1500));
    let lines = collected.lock().unwrap().clone();
    (child, lines)
}

/// Run `orts serve` expecting it to refuse the args and exit, and return
/// (exit code, stderr).
///
/// Waits with a deadline instead of `Command::output()`: a `serve` that does
/// *not* refuse keeps serving forever, and that regression must fail the test
/// rather than hang the suite.
fn run_expecting_exit(args: &[&str]) -> (Option<i32>, String) {
    let (mut child, collected, _rx, reader) = spawn(args);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait().expect("failed to poll orts serve") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                child.kill().ok();
                child.wait().ok();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    // The child has exited (or been killed), so its end of the pipe is
    // closed: the reader thread reaches EOF and returns. Joining it before
    // reading the buffer is what makes the message assertions deterministic.
    reader.join().expect("stderr reader thread panicked");
    let stderr = collected.lock().unwrap().join("\n");
    let status = status.unwrap_or_else(|| {
        panic!("orts serve did not exit within 20s; it accepted the args:\n{stderr}")
    });
    (status.code(), stderr)
}

/// Sim args with nothing to apply them to are a usage error (exit 2), and the
/// message names every arg that could not be honored.
#[test]
fn serve_rejects_sim_args_with_no_simulation_to_apply_them_to() {
    let (code, stderr) = run_expecting_exit(&[
        "--dt",
        "1",
        "--output-interval",
        "10",
        "--port",
        &test_port(0).to_string(),
    ]);

    assert_eq!(code, Some(2), "expected a usage error\nstderr: {stderr}");
    assert!(
        stderr.contains("--dt") && stderr.contains("--output-interval"),
        "the error must name the args that were not honored:\n{stderr}"
    );
    assert!(
        stderr.contains("--sat"),
        "the error must say how to make the args apply:\n{stderr}"
    );
}

/// A non-Earth body is the case that would silently run Earth physics, so it
/// is refused too rather than reaching the sim.
#[test]
fn serve_rejects_a_body_it_cannot_apply() {
    let (code, stderr) =
        run_expecting_exit(&["--body", "mars", "--port", &test_port(1).to_string()]);

    assert_eq!(code, Some(2), "expected a usage error\nstderr: {stderr}");
    assert!(
        stderr.contains("--body"),
        "the error must name --body:\n{stderr}"
    );
}

/// Bare `orts serve` keeps coming up idle: the check must not turn the
/// documented no-argument invocation into an error.
#[test]
fn bare_serve_still_comes_up_idle() {
    let (mut child, lines) = spawn_and_wait_for_startup(&["--port", &test_port(2).to_string()]);
    child.kill().ok();
    child.wait().ok();

    let joined = lines.join("\n");
    assert!(
        joined.contains("Server listening on"),
        "server did not start:\n{joined}"
    );
    assert!(
        joined.contains("idle, waiting for start_simulation"),
        "server should be idle:\n{joined}"
    );
}

/// With an orbit the args reach the simulation, so the server must accept
/// them — and actually apply them.
///
/// Read back through the `info` message the server sends on connect rather
/// than through the startup banner: the banner is printed before the manager
/// task even builds its `SimParams`, so it cannot tell "started with dt = 1"
/// from "died right after printing".
#[tokio::test]
async fn serve_with_an_orbit_honors_the_sim_args() {
    let port = test_port(3);
    let (mut child, lines) = spawn_and_wait_for_startup(&[
        "--sat",
        "altitude=400,id=argtest",
        "--dt",
        "1",
        "--output-interval",
        "10",
        "--port",
        &port.to_string(),
    ]);
    let joined = lines.join("\n");
    assert!(
        joined.contains("Server listening on"),
        "server did not start:\n{joined}"
    );
    assert!(
        !joined.contains("idle, waiting for start_simulation"),
        "an explicit orbit must auto-start the simulation:\n{joined}"
    );

    let info = tokio::time::timeout(Duration::from_secs(20), async {
        let url = format!("ws://localhost:{port}/ws");
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("failed to connect");
        let (_write, mut read) = ws.split();
        let msg = read
            .next()
            .await
            .expect("expected the info message, got end of stream")
            .expect("error reading the info message");
        serde_json::from_str::<serde_json::Value>(msg.to_text().expect("info is not text"))
            .expect("info is not JSON")
    })
    .await;
    child.kill().ok();
    child.wait().ok();

    let info = info.expect("timed out waiting for the info message");
    assert_eq!(info["type"], "info");
    assert_eq!(info["dt"], 1.0, "--dt was not applied: {info}");
    assert_eq!(
        info["output_interval"], 10.0,
        "--output-interval was not applied: {info}"
    );
    assert_eq!(
        info["satellites"][0]["id"], "/world/sat/argtest",
        "--sat was not applied: {info}"
    );
}
