use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
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

/// Read the next WebSocket message as parsed JSON.
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

/// Read messages until we find one with the given type, returning it.
/// Collects intermediate messages in a Vec.
async fn read_until_type(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    msg_type: &str,
    max_messages: usize,
) -> (serde_json::Value, Vec<serde_json::Value>) {
    let mut others = Vec::new();
    for _ in 0..max_messages {
        let msg = next_json(read).await;
        if msg["type"] == msg_type {
            return (msg, others);
        }
        others.push(msg);
    }
    panic!("did not receive message type '{msg_type}' within {max_messages} messages");
}

#[tokio::test]
async fn test_websocket_info_and_state_messages() {
    let port = test_port();
    let mut server = Server::spawn(port);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");

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
        let info = next_json(&mut read).await;
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
        let output_interval = info["output_interval"].as_f64().unwrap();
        assert!(
            (output_interval - dt).abs() < f64::EPSILON,
            "expected default output_interval to equal dt ({dt}), got {output_interval}"
        );

        // --- Second message: must be "history" ---
        let history = next_json(&mut read).await;
        assert_eq!(
            history["type"], "history",
            "second message must be 'history'"
        );
        assert!(
            history["states"].is_array(),
            "history must have 'states' array"
        );

        // --- Subsequent messages: must include "state" messages ---
        // (may also include history_detail interleaved)
        let (first_state, _) = read_until_type(&mut read, "state", 50).await;
        assert!(first_state["t"].is_f64(), "state.t must be a number");
        let position = first_state["position"].as_array().unwrap();
        assert_eq!(position.len(), 3);
        let velocity = first_state["velocity"].as_array().unwrap();
        assert_eq!(velocity.len(), 3);

        let pos: Vec<f64> = position.iter().map(|v| v.as_f64().unwrap()).collect();
        let r = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
        assert!(
            r > 6000.0 && r < 7500.0,
            "position magnitude {r:.1} km is out of expected range [6000, 7500]"
        );

        // Read 2 more state messages
        for _ in 0..2 {
            let (state, _) = read_until_type(&mut read, "state", 50).await;
            assert_eq!(state["type"], "state");
        }
    })
    .await;

    server.kill();
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

        // Client 1: info → history
        let info1 = next_json(&mut read1).await;
        assert_eq!(info1["type"], "info", "client 1 must get info message");
        let hist1 = next_json(&mut read1).await;
        assert_eq!(hist1["type"], "history", "client 1 must get history message");

        // Connect second client while the first is still connected.
        let (ws2, _) = connect_async(&url)
            .await
            .expect("client 2 failed to connect");
        let (_write2, mut read2) = ws2.split();

        // Client 2: info → history
        let info2 = next_json(&mut read2).await;
        assert_eq!(info2["type"], "info", "client 2 must get info message");
        let hist2 = next_json(&mut read2).await;
        assert_eq!(hist2["type"], "history", "client 2 must get history message");

        // Both clients should receive state messages
        let (s1, _) = read_until_type(&mut read1, "state", 50).await;
        assert_eq!(s1["type"], "state", "client 1 must get state message");

        let (s2, _) = read_until_type(&mut read2, "state", 50).await;
        assert_eq!(s2["type"], "state", "client 2 must get state message");
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_history_on_connect() {
    let port = test_port() + 2;
    let mut server = Server::spawn(port);

    // Wait for simulation to accumulate some states
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");
        let (ws, _) = connect_async(&url)
            .await
            .expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history → state
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");

        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");
        let states = history["states"].as_array().unwrap();
        assert!(
            !states.is_empty(),
            "history should have accumulated states after 3 seconds"
        );

        // Verify each history state has required fields
        for (i, state) in states.iter().enumerate() {
            assert!(state["t"].is_f64(), "history state {i}: t must be a number");
            assert_eq!(
                state["position"].as_array().unwrap().len(),
                3,
                "history state {i}: position must have 3 elements"
            );
            assert_eq!(
                state["velocity"].as_array().unwrap().len(),
                3,
                "history state {i}: velocity must have 3 elements"
            );
        }

        // State messages should follow
        let (state, _) = read_until_type(&mut read, "state", 50).await;
        assert_eq!(state["type"], "state");
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_history_grows_over_time() {
    let port = test_port() + 3;
    let mut server = Server::spawn(port);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");

        // Connect client A immediately
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (ws_a, _) = connect_async(&url)
            .await
            .expect("client A failed to connect");
        let (_write_a, mut read_a) = ws_a.split();

        let _info_a = next_json(&mut read_a).await;
        let hist_a = next_json(&mut read_a).await;
        let len_a = hist_a["states"].as_array().unwrap().len();

        // Wait for more data to accumulate
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Connect client B
        let (ws_b, _) = connect_async(&url)
            .await
            .expect("client B failed to connect");
        let (_write_b, mut read_b) = ws_b.split();

        let _info_b = next_json(&mut read_b).await;
        let hist_b = next_json(&mut read_b).await;
        let len_b = hist_b["states"].as_array().unwrap().len();

        assert!(
            len_b > len_a,
            "history should grow over time: len_a={len_a}, len_b={len_b}"
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_history_detail_follows() {
    let port = test_port() + 4;
    let mut server = Server::spawn(port);

    // Wait for data to accumulate
    tokio::time::sleep(Duration::from_secs(2)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");
        let (ws, _) = connect_async(&url)
            .await
            .expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");

        // Read messages until we find history_detail_complete
        let mut found_detail = false;
        let mut found_complete = false;
        let mut found_state = false;
        for _ in 0..200 {
            let msg = next_json(&mut read).await;
            match msg["type"].as_str().unwrap() {
                "history_detail" => {
                    found_detail = true;
                    let states = msg["states"].as_array().unwrap();
                    assert!(!states.is_empty(), "detail chunk should not be empty");
                }
                "history_detail_complete" => {
                    found_complete = true;
                    if found_state {
                        break;
                    }
                    // Continue to collect state messages after detail complete
                }
                "state" => {
                    found_state = true;
                    if found_complete {
                        break;
                    }
                }
                other => {
                    panic!("unexpected message type: {other}");
                }
            }
        }

        assert!(found_detail, "should receive at least one history_detail");
        assert!(found_complete, "should receive history_detail_complete");
        assert!(
            found_state,
            "should receive state messages (before or after detail)"
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_overview_arrives_fast() {
    let port = test_port() + 5;
    let mut server = Server::spawn(port);

    // Wait for substantial data accumulation
    tokio::time::sleep(Duration::from_secs(5)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");
        let (ws, _) = connect_async(&url)
            .await
            .expect("failed to connect");
        let (_write, mut read) = ws.split();

        let _info = next_json(&mut read).await;
        let start = std::time::Instant::now();
        let history = next_json(&mut read).await;
        let elapsed = start.elapsed();

        assert_eq!(history["type"], "history");
        assert!(
            elapsed.as_millis() < 500,
            "overview should arrive within 500ms, took {}ms",
            elapsed.as_millis()
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_query_range() {
    let port = test_port() + 6;
    let mut server = Server::spawn(port);

    // Wait for data to accumulate
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = format!("ws://localhost:{port}");
        let (ws, _) = connect_async(&url)
            .await
            .expect("failed to connect");
        let (mut write, mut read) = ws.split();

        // info → history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");
        let history_states = history["states"].as_array().unwrap();
        assert!(!history_states.is_empty(), "need accumulated history");

        // Determine a valid time range from the history
        let first_t = history_states[0]["t"].as_f64().unwrap();
        let last_t = history_states[history_states.len() - 1]["t"].as_f64().unwrap();

        // Send query_range request
        let query = serde_json::json!({
            "type": "query_range",
            "t_min": first_t,
            "t_max": last_t,
            "max_points": 50
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                query.to_string().into(),
            ))
            .await
            .expect("failed to send query_range");

        // Read messages until we get the query_range_response
        let (response, _) = read_until_type(&mut read, "query_range_response", 100).await;
        assert_eq!(response["type"], "query_range_response");
        assert!(response["t_min"].is_f64());
        assert!(response["t_max"].is_f64());

        let resp_states = response["states"].as_array().unwrap();
        assert!(
            !resp_states.is_empty(),
            "query_range_response should have states"
        );
        assert!(
            resp_states.len() <= 50,
            "should respect max_points limit, got {}",
            resp_states.len()
        );

        // Verify all returned states are within the requested range
        for state in resp_states {
            let t = state["t"].as_f64().unwrap();
            assert!(
                t >= first_t - 1e-9 && t <= last_t + 1e-9,
                "state t={t} is outside range [{first_t}, {last_t}]"
            );
        }
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// Verify that state `t` values are monotonically increasing across orbit boundaries.
/// The server must NOT reset t to 0 at the start of each orbit period.
#[tokio::test]
async fn test_websocket_monotonic_time_across_orbits() {
    let port = test_port() + 7;
    let mut server = Server::spawn(port);

    // Wait long enough for more than one full orbit (~55s wall time at default params).
    // 65 seconds ensures the second orbit has started and t resets would be visible.
    tokio::time::sleep(Duration::from_secs(65)).await;

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let url = format!("ws://localhost:{port}");
        let (ws, _) = connect_async(&url)
            .await
            .expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let _period = info["period"].as_f64().unwrap();

        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");
        let states = history["states"].as_array().unwrap();

        // With enough wait time, history should contain data beyond one orbit.
        // After the monotonic-time fix, max_t > period. Before the fix,
        // t resets to 0, so max_t == period but monotonicity fails below.
        assert!(
            !states.is_empty(),
            "history should have accumulated states after waiting"
        );

        // Verify all history t values are monotonically increasing.
        // This is the core assertion: if the server resets t at orbit boundaries,
        // we'll see t jump from ~period back to ~0.
        let mut prev_t = f64::NEG_INFINITY;
        for (i, state) in states.iter().enumerate() {
            let t = state["t"].as_f64().unwrap();
            assert!(
                t >= prev_t,
                "history t values must be monotonically increasing: \
                 state[{i}].t={t} < state[{}].t={prev_t}",
                i - 1
            );
            prev_t = t;
        }

        // Collect live state messages and verify monotonicity among them.
        // Note: the first live state may overlap with the history tail due to
        // subscribe-before-snapshot timing, so we don't compare against history max_t.
        let mut last_state_t = f64::NEG_INFINITY;
        for i in 0..10 {
            let (state, _) = read_until_type(&mut read, "state", 50).await;
            let t = state["t"].as_f64().unwrap();
            assert!(
                t >= last_state_t,
                "live state t values must be monotonically increasing: \
                 state[{i}].t={t} < previous {last_state_t}"
            );
            last_state_t = t;
        }
    })
    .await;

    server.kill();
    result.expect("test timed out after 60 seconds");
}
