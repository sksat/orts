//! E2E tests for the `orts serve` argument contract.
//!
//! `serve` with no orbit and no config comes up idle and builds its
//! `SimParams` from the client's `start_simulation`, so sim args like `--dt`
//! never reach a simulation. They used to be dropped without a word; they are
//! now refused by name.

use futures_util::StreamExt;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Ports derived from the pid, offset per test to avoid collisions. The
/// 24000 band is this file's own: the other serve E2E suites run in separate
/// processes (so a different pid) and claim 19000-23999.
fn test_port(offset: u16) -> u16 {
    24000 + (std::process::id() % 500) as u16 * 4 + offset
}

/// A spawned `orts serve` with its stderr streamed into a shared buffer.
///
/// The child and its reader sit in `Option`s so that both the consuming
/// methods and [`Drop`] can take them. Dropping a `std::process::Child` does
/// not kill the process, so an assertion that panics before `shutdown` or
/// `wait_for_exit` would otherwise leave an idle server holding its port for
/// the rest of the suite.
struct Server {
    child: Option<Child>,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().ok();
            child.wait().ok();
        }
        if let Some(reader) = self.reader.take() {
            // The child is gone, so the pipe is closed and the reader returns.
            // `join` can only fail if the reader itself panicked, which the
            // consuming methods report; a drop during unwinding stays quiet.
            let _ = reader.join();
        }
    }
}

impl Server {
    fn spawn(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_orts"))
            .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
            .arg("serve")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn orts serve");

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&lines);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                eprintln!("[server stderr] {line}");
                sink.lock().unwrap().push(line);
            }
        });
        Server {
            child: Some(child),
            lines,
            reader: Some(reader),
        }
    }

    /// Wait until a collected stderr line contains `needle`.
    ///
    /// Every wait in this file is on a specific line rather than on a fixed
    /// sleep, so no assertion depends on how fast the reader thread happens
    /// to be scheduled.
    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self
                .lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains(needle))
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Wait for the last line of the startup banner, which is printed whether
    /// or not a simulation is started.
    fn wait_for_startup(&self) {
        assert!(
            self.wait_for("WebSocket endpoint", Duration::from_secs(15)),
            "server produced no startup banner within 15 seconds:\n{}",
            self.stderr()
        );
    }

    fn stderr(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    /// Stop the server and drain its stderr to EOF.
    fn shutdown(mut self) -> String {
        let mut child = self.child.take().expect("the server is spawned once");
        let reader = self.reader.take().expect("the server is spawned once");
        child.kill().ok();
        child.wait().ok();
        reader.join().expect("stderr reader thread panicked");
        self.lines.lock().unwrap().join("\n")
    }

    /// Wait for the process to exit on its own and drain stderr to EOF.
    ///
    /// Waits with a deadline instead of `Command::output()`: a `serve` that
    /// does *not* refuse its args keeps serving forever, and that regression
    /// must fail the test rather than hang the suite.
    fn wait_for_exit(mut self, timeout: Duration) -> (Option<i32>, String) {
        let mut child = self.child.take().expect("the server is spawned once");
        let reader = self.reader.take().expect("the server is spawned once");
        let lines = Arc::clone(&self.lines);
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait().expect("failed to poll orts serve") {
                Some(status) => break Some(status),
                None if Instant::now() >= deadline => break None,
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        if status.is_none() {
            child.kill().ok();
            child.wait().ok();
        }
        // The child is gone, so its end of the pipe is closed: the reader
        // thread reaches EOF and returns. Joining before reading the buffer is
        // what makes the message assertions deterministic.
        reader.join().expect("stderr reader thread panicked");
        let stderr = lines.lock().unwrap().join("\n");
        let status = status.unwrap_or_else(|| {
            panic!("orts serve did not exit within {timeout:?}; it accepted the args:\n{stderr}")
        });
        (status.code(), stderr)
    }
}

/// Sim args with nothing to apply them to are a usage error (exit 2), and the
/// message names every arg that could not be honored.
#[test]
fn serve_rejects_sim_args_with_no_simulation_to_apply_them_to() {
    let server = Server::spawn(&[
        "--dt",
        "1",
        "--output-interval",
        "10",
        "--port",
        &test_port(0).to_string(),
    ]);
    let (code, stderr) = server.wait_for_exit(Duration::from_secs(20));

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
    let server = Server::spawn(&["--body", "mars", "--port", &test_port(1).to_string()]);
    let (code, stderr) = server.wait_for_exit(Duration::from_secs(20));

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
    let server = Server::spawn(&["--port", &test_port(2).to_string()]);
    server.wait_for_startup();
    let went_idle = server.wait_for(
        "idle, waiting for start_simulation",
        Duration::from_secs(15),
    );
    let stderr = server.shutdown();

    assert!(stderr.contains("Server listening on"), "{stderr}");
    assert!(went_idle, "server should have gone idle:\n{stderr}");
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
    let server = Server::spawn(&[
        "--sat",
        "altitude=400,id=argtest",
        "--dt",
        "1",
        "--output-interval",
        "10",
        "--port",
        &port.to_string(),
    ]);
    server.wait_for_startup();

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
    let stderr = server.shutdown();

    let info = info.unwrap_or_else(|_| panic!("timed out waiting for the info message:\n{stderr}"));
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
    assert!(
        !stderr.contains("idle, waiting for start_simulation"),
        "an explicit orbit must auto-start the simulation:\n{stderr}"
    );
}
