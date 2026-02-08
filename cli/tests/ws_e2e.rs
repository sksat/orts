use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::connect_async;

/// Pick a port unlikely to collide with other processes.
fn test_port() -> u16 {
    let pid = std::process::id();
    19000 + (pid % 1000) as u16
}

/// A running server with its child process and stderr drain thread.
struct Server {
    child: std::process::Child,
    /// Join handle for the thread that drains stderr (keeps the pipe alive).
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    /// Spawn the CLI binary in WebSocket server mode.
    /// Blocks until the server prints its "listening" message to stderr.
    fn spawn(port: u16) -> Self {
        let binary = env!("CARGO_BIN_EXE_orts-cli");
        let mut child = Command::new(binary)
            .args(["serve", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn orts-cli");

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let (tx, rx) = mpsc::channel::<()>();

        // Spawn a thread to read stderr. This keeps the pipe open for the entire
        // lifetime of the server process, preventing broken-pipe crashes.
        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut notified = false;
            for line in reader.lines() {
                let line = line.expect("failed to read stderr line");
                eprintln!("[server stderr] {line}");
                if !notified && line.contains("WebSocket server listening") {
                    let _ = tx.send(());
                    notified = true;
                }
            }
            // If the server never printed the ready message, notify anyway so
            // the test doesn't hang.
            if !notified {
                let _ = tx.send(());
            }
        });

        // Wait for the "listening" signal from the stderr reader thread.
        rx.recv_timeout(Duration::from_secs(10))
            .expect("server did not print 'listening' message within 10 seconds");

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

#[tokio::test]
async fn test_websocket_info_and_state_messages() {
    let port = test_port();
    let mut server = Server::spawn(port);

    // Give the server a moment to fully enter its accept loop after printing the
    // ready message.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Wrap the actual test logic in a timeout so it cannot hang forever.
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");

        // Retry connection a few times to handle any residual startup race.
        let mut ws_stream = None;
        for attempt in 0..20 {
            match connect_async(&url).await {
                Ok((stream, _response)) => {
                    ws_stream = Some(stream);
                    break;
                }
                Err(e) => {
                    if attempt == 19 {
                        panic!("failed to connect to WebSocket server after 20 attempts: {e}");
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        let ws_stream = ws_stream.unwrap();

        let (_write, mut read) = ws_stream.split();

        // --- First message: must be "info" ---
        let msg = read
            .next()
            .await
            .expect("expected info message, got end of stream")
            .expect("error reading info message");

        let info_text = msg.into_text().expect("info message is not text");
        let info: serde_json::Value =
            serde_json::from_str(&info_text).expect("info message is not valid JSON");

        assert_eq!(info["type"], "info", "first message type must be 'info'");
        assert!(info["mu"].is_f64(), "info.mu must be a number");
        assert!(info["altitude"].is_f64(), "info.altitude must be a number");
        assert!(info["period"].is_f64(), "info.period must be a number");
        assert!(info["dt"].is_f64(), "info.dt must be a number");
        assert!(
            info["output_interval"].is_f64(),
            "info.output_interval must be a number"
        );
        assert_eq!(info["central_body"], "earth");
        assert!(
            info["central_body_radius"].is_f64(),
            "info.central_body_radius must be a number"
        );

        // Sanity-check default values (altitude=400, dt=10).
        let altitude = info["altitude"].as_f64().unwrap();
        assert!(
            (altitude - 400.0).abs() < f64::EPSILON,
            "expected default altitude 400, got {altitude}"
        );
        let dt = info["dt"].as_f64().unwrap();
        assert!(
            (dt - 10.0).abs() < f64::EPSILON,
            "expected default dt 10, got {dt}"
        );
        // output_interval defaults to dt when not specified.
        let output_interval = info["output_interval"].as_f64().unwrap();
        assert!(
            (output_interval - dt).abs() < f64::EPSILON,
            "expected default output_interval to equal dt ({dt}), got {output_interval}"
        );

        // --- Subsequent messages: must be "state" ---
        let required_state_count = 3;
        for i in 0..required_state_count {
            let msg = read
                .next()
                .await
                .unwrap_or_else(|| panic!("expected state message #{i}, got end of stream"))
                .unwrap_or_else(|e| panic!("error reading state message #{i}: {e}"));

            let text = msg.into_text().expect("state message is not text");
            let state: serde_json::Value =
                serde_json::from_str(&text).expect("state message is not valid JSON");

            assert_eq!(
                state["type"], "state",
                "message #{i} type must be 'state'"
            );
            assert!(state["t"].is_f64(), "state.t must be a number");

            // position and velocity must be arrays of 3 numbers
            let position = state["position"]
                .as_array()
                .unwrap_or_else(|| panic!("state #{i}: position must be an array"));
            assert_eq!(
                position.len(),
                3,
                "state #{i}: position must have 3 elements"
            );
            for (j, val) in position.iter().enumerate() {
                assert!(
                    val.is_f64(),
                    "state #{i}: position[{j}] must be a number"
                );
            }

            let velocity = state["velocity"]
                .as_array()
                .unwrap_or_else(|| panic!("state #{i}: velocity must be an array"));
            assert_eq!(
                velocity.len(),
                3,
                "state #{i}: velocity must have 3 elements"
            );
            for (j, val) in velocity.iter().enumerate() {
                assert!(
                    val.is_f64(),
                    "state #{i}: velocity[{j}] must be a number"
                );
            }

            // The position magnitude should be roughly Earth radius + 400 km = ~6771 km.
            let pos: Vec<f64> = position.iter().map(|v| v.as_f64().unwrap()).collect();
            let r = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
            assert!(
                r > 6000.0 && r < 7500.0,
                "state #{i}: position magnitude {r:.1} km is out of expected range [6000, 7500]"
            );
        }
    })
    .await;

    // Kill the server process regardless of test outcome.
    server.kill();

    // Propagate timeout or assertion failures.
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_multiple_clients() {
    let port = test_port() + 1;
    let mut server = Server::spawn(port);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");

        // Connect first client.
        let (ws1, _) = connect_async(&url)
            .await
            .expect("client 1 failed to connect");
        let (_write1, mut read1) = ws1.split();

        // First client should receive info message.
        let msg1 = read1
            .next()
            .await
            .expect("client 1: expected info message")
            .expect("client 1: error reading info");
        let info1: serde_json::Value =
            serde_json::from_str(&msg1.into_text().unwrap()).unwrap();
        assert_eq!(info1["type"], "info", "client 1 must get info message");

        // Connect second client while the first is still connected.
        let (ws2, _) = connect_async(&url)
            .await
            .expect("client 2 failed to connect");
        let (_write2, mut read2) = ws2.split();

        // Second client should also receive its own info message.
        let msg2 = read2
            .next()
            .await
            .expect("client 2: expected info message")
            .expect("client 2: error reading info");
        let info2: serde_json::Value =
            serde_json::from_str(&msg2.into_text().unwrap()).unwrap();
        assert_eq!(info2["type"], "info", "client 2 must get info message");

        // Both clients should receive state messages.
        let state1 = read1
            .next()
            .await
            .expect("client 1: expected state message")
            .expect("client 1: error reading state");
        let s1: serde_json::Value =
            serde_json::from_str(&state1.into_text().unwrap()).unwrap();
        assert_eq!(s1["type"], "state", "client 1 must get state message");

        let state2 = read2
            .next()
            .await
            .expect("client 2: expected state message")
            .expect("client 2: error reading state");
        let s2: serde_json::Value =
            serde_json::from_str(&state2.into_text().unwrap()).unwrap();
        assert_eq!(s2["type"], "state", "client 2 must get state message");
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}
