use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;

/// How long a server gets to announce its endpoint after being spawned.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// A running server with its child process and stderr drain thread. Killed
/// on drop, so a failing assertion does not leave it listening.
struct Server {
    child: std::process::Child,
    /// The port the server bound. It asks the OS for a free one (`--port 0`),
    /// so a server left over from another run cannot answer in its place.
    port: u16,
    /// Join handle for the thread that drains stderr (keeps the pipe alive).
    _stderr_thread: std::thread::JoinHandle<()>,
}

impl Server {
    /// Spawn the CLI binary in WebSocket server mode.
    /// Blocks until the server announces its WebSocket endpoint on stderr.
    /// Uses explicit `--sat` args to avoid CelesTrak network dependency.
    fn spawn() -> Self {
        Self::spawn_with_sats(&["altitude=400,id=test"])
    }

    /// Spawn the CLI binary with custom satellite configurations.
    fn spawn_with_sats(sats: &[&str]) -> Self {
        Self::spawn_with_sats_and_env(sats, &[])
    }

    /// Spawn with custom satellite configurations and extra environment
    /// variables. With no `sats` the server starts idle.
    fn spawn_with_sats_and_env(sats: &[&str], env: &[(&str, &str)]) -> Self {
        Self::spawn_with(sats, env, &[])
    }

    /// `extra` goes on the `serve` command line after the satellites.
    fn spawn_with(sats: &[&str], env: &[(&str, &str)], extra: &[&str]) -> Self {
        let binary = env!("CARGO_BIN_EXE_orts");
        let mut args = vec!["serve".to_string(), "--port".to_string(), "0".to_string()];
        for sat in sats {
            args.push("--sat".to_string());
            args.push(sat.to_string());
        }
        args.extend(extra.iter().map(|a| a.to_string()));
        let mut child = Command::new(binary)
            .env("ORTS_DISABLE_TEXTURE_DOWNLOAD", "1")
            .envs(env.iter().copied())
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn orts");

        let stderr = child.stderr.take().expect("failed to capture stderr");
        let (tx, rx) = mpsc::channel::<Option<u16>>();

        // Spawn a thread to read stderr. This keeps the pipe open for the entire
        // lifetime of the server process, preventing broken-pipe crashes.
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
            // The server exited without announcing an endpoint (a rejected
            // config, a failed bind): fail the spawn instead of letting the
            // test connect to whatever else is listening.
            if !notified {
                let _ = tx.send(None);
            }
        });

        // `Server` (and its `Drop`) does not exist yet, so a failed start
        // kills the child here; otherwise it would outlive the test.
        let port = match rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Some(port)) => port,
            failure => {
                let _ = child.kill();
                let _ = child.wait();
                match failure {
                    Err(_) => panic!("server did not announce its WebSocket endpoint in time"),
                    _ => panic!("server exited or announced an unreadable WebSocket endpoint"),
                }
            }
        };

        Server {
            child,
            port,
            _stderr_thread: stderr_thread,
        }
    }

    /// Spawn in idle mode (no --sat args, no --config).
    fn spawn_idle() -> Self {
        Self::spawn_idle_with_env(&[])
    }

    /// Spawn in idle mode with extra environment variables.
    fn spawn_idle_with_env(env: &[(&str, &str)]) -> Self {
        Self::spawn_with_sats_and_env(&[], env)
    }

    fn ws_url(&self) -> String {
        format!("ws://localhost:{}/ws", self.port)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
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
    let mut server = Server::spawn();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();

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

        // First message: must be "info"
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info", "first message type must be 'info'");
        assert!(info["mu"].is_f64(), "info.mu must be a number");
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
        // New protocol: satellites array
        let satellites = info["satellites"]
            .as_array()
            .expect("info must have satellites array");
        assert!(!satellites.is_empty(), "satellites array must not be empty");
        // At least the SSO satellite should be present
        let first_sat = &satellites[0];
        assert!(
            first_sat["altitude"].is_f64(),
            "satellite must have altitude"
        );
        assert!(first_sat["period"].is_f64(), "satellite must have period");
        assert!(first_sat["id"].is_string(), "satellite must have id");

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

        // Second message: must be "history"
        let history = next_json(&mut read).await;
        assert_eq!(
            history["type"], "history",
            "second message must be 'history'"
        );
        assert!(
            history["states"].is_array(),
            "history must have 'states' array"
        );

        // Subsequent messages: must include "state" messages
        // (may also include history_detail interleaved)
        let (first_state, _) = read_until_type(&mut read, "state", 50).await;
        assert!(first_state["t"].is_f64(), "state.t must be a number");
        assert!(
            first_state["entity_path"].is_string(),
            "state must have entity_path"
        );
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

        // Verify Keplerian elements are present in state messages
        assert!(
            first_state["semi_major_axis"].is_f64(),
            "state must include semi_major_axis"
        );
        assert!(
            first_state["eccentricity"].is_f64(),
            "state must include eccentricity"
        );
        assert!(
            first_state["inclination"].is_f64(),
            "state must include inclination"
        );
        assert!(first_state["raan"].is_f64(), "state must include raan");
        assert!(
            first_state["argument_of_periapsis"].is_f64(),
            "state must include argument_of_periapsis"
        );
        assert!(
            first_state["true_anomaly"].is_f64(),
            "state must include true_anomaly"
        );

        // Sanity check: semi-major axis should be near orbit radius for circular orbit
        let sma = first_state["semi_major_axis"].as_f64().unwrap();
        assert!(
            sma > 6000.0 && sma < 7500.0,
            "semi_major_axis {sma:.1} km out of expected range"
        );
        let ecc = first_state["eccentricity"].as_f64().unwrap();
        assert!(
            ecc < 0.01,
            "eccentricity {ecc} should be near zero for circular orbit"
        );

        // Verify Keplerian elements are present in history states too
        let history_states = history["states"].as_array().unwrap();
        if !history_states.is_empty() {
            let first_hist = &history_states[0];
            assert!(
                first_hist["semi_major_axis"].is_f64(),
                "history state must include semi_major_axis"
            );
            assert!(
                first_hist["eccentricity"].is_f64(),
                "history state must include eccentricity"
            );
            assert!(
                first_hist["inclination"].is_f64(),
                "history state must include inclination"
            );
        }

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
    let mut server = Server::spawn();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();

        // Connect first client.
        let (ws1, _) = connect_async(&url)
            .await
            .expect("client 1 failed to connect");
        let (_write1, mut read1) = ws1.split();

        // Client 1: info → history
        let info1 = next_json(&mut read1).await;
        assert_eq!(info1["type"], "info", "client 1 must get info message");
        let hist1 = next_json(&mut read1).await;
        assert_eq!(
            hist1["type"], "history",
            "client 1 must get history message"
        );

        // Connect second client while the first is still connected.
        let (ws2, _) = connect_async(&url)
            .await
            .expect("client 2 failed to connect");
        let (_write2, mut read2) = ws2.split();

        // Client 2: info → history
        let info2 = next_json(&mut read2).await;
        assert_eq!(info2["type"], "info", "client 2 must get info message");
        let hist2 = next_json(&mut read2).await;
        assert_eq!(
            hist2["type"], "history",
            "client 2 must get history message"
        );

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
    let mut server = Server::spawn();

    // Wait for simulation to accumulate some states
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
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
    let mut server = Server::spawn();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();

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

/// After the initial bounded overview, the server transitions directly to
/// the live state stream. The old contract spawned a background detail
/// replay (`history_detail` chunks + `history_detail_complete` marker); that
/// machinery has been removed, so those message types must never appear on
/// the wire. The dedicated assertion lives in `test_websocket_no_history_detail_sent`;
/// this test doubles as a smoke check that the `info → history → state`
/// handshake still works end-to-end after the simplification.
#[tokio::test]
async fn test_websocket_info_history_state_handshake() {
    let mut server = Server::spawn();

    // Wait for data to accumulate
    tokio::time::sleep(Duration::from_secs(2)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history → (eventually) state
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");

        // Every message from here on must be a state (or another protocol
        // message), never history_detail*.
        let mut found_state = false;
        for _ in 0..100 {
            let msg = next_json(&mut read).await;
            let ty = msg["type"].as_str().unwrap();
            assert_ne!(ty, "history_detail", "history_detail must not be emitted");
            assert_ne!(
                ty, "history_detail_complete",
                "history_detail_complete must not be emitted"
            );
            if ty == "state" {
                found_state = true;
                break;
            }
        }
        assert!(found_state, "should receive a live state message");
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// After removing the unbounded full-resolution `HistoryDetail` background replay,
/// a fresh client must only receive `info` + `history` (downsampled overview) before
/// the live `state` stream begins. No `history_detail` / `history_detail_complete`
/// messages should ever arrive.
/// After removing the unbounded full-resolution `HistoryDetail` background replay,
/// a fresh client must only receive `info` + `history` (downsampled overview) before
/// the live `state` stream begins. No `history_detail` / `history_detail_complete`
/// messages should ever arrive, even after observing many subsequent frames.
#[tokio::test]
async fn test_websocket_no_history_detail_sent() {
    // Use dt=1 output_interval=1 so history accumulates fast enough that the
    // old code path would actually chunk detail replay (rather than ship an
    // empty HistoryDetailComplete marker immediately).
    let server = Server::spawn_with(
        &["altitude=400,id=test"],
        &[],
        &["--dt", "1", "--output-interval", "1"],
    );

    // Let several history outputs accumulate.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");
        let history_states = history["states"].as_array().unwrap();
        assert!(
            !history_states.is_empty(),
            "precondition: history overview should be non-empty so the old code \
             path would have actually queued detail chunks"
        );

        // Collect messages for a meaningful window, not stopping early on the
        // first `state` message. If the old background HistoryDetail pipeline
        // is still wired up, at least one `history_detail` or
        // `history_detail_complete` frame will appear in the first N messages.
        let mut seen_state = false;
        for _ in 0..100 {
            let msg = next_json(&mut read).await;
            let ty = msg["type"].as_str().unwrap();
            assert_ne!(
                ty, "history_detail",
                "history_detail must not be sent (removed by design)"
            );
            assert_ne!(
                ty, "history_detail_complete",
                "history_detail_complete must not be sent (removed by design)"
            );
            if ty == "state" {
                seen_state = true;
            }
        }
        assert!(
            seen_state,
            "should have observed at least one live state message in the sample"
        );
    })
    .await;

    drop(server);
    result.expect("test timed out after 20 seconds");
}

/// The connect-time history overview must be bounded regardless of how long
/// the simulation has been running. This is the core regression test for the
/// "viewer blank after reload" problem on long-running sims — the server's
/// handshake cost and the wire payload must both stay constant in sim
/// duration. Any client-side time-range display concern is handled via
/// follow-up `query_range` requests, not baked into the handshake.
#[tokio::test]
async fn test_websocket_history_overview_payload_is_bounded() {
    // dt=1 output_interval=1 accumulates 1 history point per wall-clock second.
    let server = Server::spawn_with(
        &["altitude=400,id=test"],
        &[],
        &["--dt", "1", "--output-interval", "1"],
    );

    // Let the sim accumulate well past the server's overview cap (1000
    // points). The sim loop often produces bursts of >1 output per wall
    // second, so 8 seconds is enough to comfortably exceed the cap on any
    // reasonable machine while keeping the test fast.
    tokio::time::sleep(Duration::from_secs(8)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_write, mut read) = ws.split();

        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");

        let history = next_json(&mut read).await;
        assert_eq!(history["type"], "history");
        let states = history["states"].as_array().expect("states array");

        // Core assertion: payload is bounded by the server-side cap, no
        // matter how many history points the simulation has accumulated.
        // The old code path returned ~N points where N scaled with sim
        // duration — regressing back to that would blow past this limit.
        assert!(
            states.len() <= 1000,
            "history overview must be bounded to OVERVIEW_MAX_POINTS (1000), got {}",
            states.len()
        );
    })
    .await;

    drop(server);
    result.expect("test timed out after 30 seconds");
}

#[tokio::test]
async fn test_websocket_overview_arrives_fast() {
    let mut server = Server::spawn();

    // Wait for substantial data accumulation
    tokio::time::sleep(Duration::from_secs(5)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
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
    let mut server = Server::spawn();

    // Wait for data to accumulate
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
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
        let last_t = history_states[history_states.len() - 1]["t"]
            .as_f64()
            .unwrap();

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
    let mut server = Server::spawn();

    // Wait long enough for more than one full orbit (~55s wall time at default params).
    // 65 seconds ensures the second orbit has started and t resets would be visible.
    tokio::time::sleep(Duration::from_secs(65)).await;

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_write, mut read) = ws.split();

        // info → history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let _period = info["satellites"][0]["period"].as_f64().unwrap();

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

        // Collect live state messages and verify per-satellite monotonicity.
        // With multi-satellite, states from different satellites are interleaved,
        // so we track last_t per entity_path.
        let mut last_t_per_sat: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        for i in 0..10 {
            let (state, _) = read_until_type(&mut read, "state", 50).await;
            let t = state["t"].as_f64().unwrap();
            let sid = state["entity_path"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let prev = last_t_per_sat
                .entry(sid.clone())
                .or_insert(f64::NEG_INFINITY);
            assert!(
                t >= *prev,
                "live state t for {sid} must be monotonically increasing: \
                 state[{i}].t={t} < previous {}",
                *prev
            );
            *prev = t;
        }
    })
    .await;

    server.kill();
    result.expect("test timed out after 60 seconds");
}

/// Verify that a late-connecting client receives `simulation_terminated` events
/// for satellites that terminated before the client connected.
#[tokio::test]
async fn test_websocket_terminated_replay_on_late_connect() {
    // altitude=50 is below Earth's atmosphere (100 km Kármán line)
    // → immediate atmospheric entry termination
    let mut server = Server::spawn_with_sats(&["altitude=50,id=low", "altitude=800,id=high"]);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();

        // An observer waits until the termination has reached the clients.
        // The engine adds the event to its replay list before it broadcasts
        // it, so once the observer has it (live or replayed) the list holds
        // it. The client below connects after the broadcast went out, so it
        // can only get the event from the replay.
        {
            let (ws, _) = connect_async(&url).await.expect("failed to connect");
            let (_write, mut read) = ws.split();
            let (terminated, _) = read_until_type(&mut read, "simulation_terminated", 200).await;
            assert_eq!(terminated["entity_path"], "/world/sat/low");
        }

        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_write, mut read) = ws.split();

        // First message: info
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");
        let sats = info["satellites"].as_array().unwrap();
        assert_eq!(sats.len(), 2, "should have two satellites");

        // Should receive simulation_terminated for "low" among early messages
        // (after info, before or interleaved with history/state)
        let (terminated, _) = read_until_type(&mut read, "simulation_terminated", 50).await;
        assert_eq!(
            terminated["entity_path"], "/world/sat/low",
            "should receive termination for 'low' satellite"
        );
        assert!(
            terminated["reason"]
                .as_str()
                .unwrap()
                .contains("atmospheric"),
            "reason should mention atmospheric entry: {}",
            terminated["reason"]
        );

        // Subsequent state messages should only be for "high" satellite
        for _ in 0..5 {
            let (state, _) = read_until_type(&mut read, "state", 50).await;
            assert_eq!(
                state["entity_path"], "/world/sat/high",
                "only 'high' satellite should still stream state messages"
            );
        }
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// Verify that a server started without --sat args enters idle mode,
/// and that a client can start the simulation via start_simulation message.
#[tokio::test]
async fn test_websocket_idle_then_start_simulation() {
    let mut server = Server::spawn_idle();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        // Should receive status: idle
        let status = next_json(&mut read).await;
        assert_eq!(status["type"], "status");
        assert_eq!(status["state"], "idle");

        // Send start_simulation
        let config = serde_json::json!({
            "type": "start_simulation",
            "config": {
                "body": "earth",
                "dt": 10.0,
                "satellites": [
                    { "id": "test", "orbit": { "type": "circular", "altitude": 400.0 } }
                ]
            }
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                config.to_string().into(),
            ))
            .await
            .expect("failed to send start_simulation");

        // Should receive info message (via broadcast)
        let (info, _) = read_until_type(&mut read, "info", 10).await;
        assert_eq!(info["type"], "info");
        assert!(info["satellites"].as_array().unwrap().len() >= 1);

        // Should receive state messages
        let (state, _) = read_until_type(&mut read, "state", 100).await;
        assert_eq!(state["type"], "state");
        assert!(state["t"].as_f64().is_some());
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// Verify that add_satellite works on a running simulation.
#[tokio::test]
async fn test_websocket_add_satellite() {
    let mut server = Server::spawn();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        // Read info + history
        let info = next_json(&mut read).await;
        assert_eq!(info["type"], "info");

        let _history = next_json(&mut read).await;

        // Wait a bit for simulation to run
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Send add_satellite
        let add_sat = serde_json::json!({
            "type": "add_satellite",
            "id": "new-sat",
            "name": "Dynamically Added",
            "orbit": { "type": "circular", "altitude": 600.0 }
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                add_sat.to_string().into(),
            ))
            .await
            .expect("failed to send add_satellite");

        // Should receive satellite_added among the messages
        let (added, _) = read_until_type(&mut read, "satellite_added", 200).await;
        assert_eq!(added["type"], "satellite_added");
        assert_eq!(added["satellite"]["id"], "/world/sat/new-sat");
        assert!(added["t"].as_f64().is_some());

        // Should now receive state messages for the new satellite
        let mut found_new_sat_state = false;
        for _ in 0..200 {
            let msg = next_json(&mut read).await;
            if msg["type"] == "state" && msg["entity_path"] == "/world/sat/new-sat" {
                found_new_sat_state = true;
                break;
            }
        }
        assert!(
            found_new_sat_state,
            "should receive state messages for new satellite"
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// A `start_simulation` the server cannot read is answered, and the socket lives.
///
/// A `type`-tagged block still refuses an unknown key, so this message fails to
/// deserialize. Dropping that error left the client waiting on a reply that
/// never arrives.
#[tokio::test]
async fn test_websocket_unreadable_message_is_answered() {
    let mut server = Server::spawn_idle();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        let status = next_json(&mut read).await;
        assert_eq!(status["state"], "idle");

        // `inclinaton` is a typo for `inclination`, inside the `type`-tagged
        // orbit block, which is the one place an unknown key is refused.
        let typo = serde_json::json!({
            "type": "start_simulation",
            "config": {
                "dt": 10.0,
                "satellites": [
                    { "id": "test", "orbit": {
                        "type": "circular", "altitude": 400.0, "inclinaton": 51.6
                    } }
                ]
            }
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                typo.to_string().into(),
            ))
            .await
            .expect("failed to send start_simulation");

        let (error, _) = read_until_type(&mut read, "error", 10).await;
        let message = error["message"].as_str().expect("an error message");
        assert!(
            message.contains("inclinaton"),
            "the message names the key it could not read: {message}"
        );

        // The same connection still serves a message the server can read.
        let good = serde_json::json!({
            "type": "start_simulation",
            "config": {
                "dt": 10.0,
                "satellites": [
                    { "id": "test", "orbit": { "type": "circular", "altitude": 400.0 } }
                ]
            }
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                good.to_string().into(),
            ))
            .await
            .expect("failed to send start_simulation");

        let (info, _) = read_until_type(&mut read, "info", 10).await;
        assert!(
            !info["satellites"]
                .as_array()
                .expect("satellites")
                .is_empty()
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// Environment that makes every HTTP(S) request an `orts` process sends fail.
///
/// Each proxy variable ureq reads points at port 9 (discard), where no proxy
/// listens. ureq takes the first of `ALL_PROXY`, `all_proxy`, `HTTPS_PROXY`,
/// `https_proxy`, `HTTP_PROXY`, `http_proxy` that parses, so an inherited
/// `ALL_PROXY` would win over the other five if it were left in place. The
/// `NO_PROXY` bypass lists are emptied for the same reason. A test that fails
/// a fetch this way needs no network and does not depend on having none.
const UNREACHABLE_PROXY_ENV: &[(&str, &str)] = &[
    ("ALL_PROXY", "http://127.0.0.1:9"),
    ("all_proxy", "http://127.0.0.1:9"),
    ("HTTPS_PROXY", "http://127.0.0.1:9"),
    ("https_proxy", "http://127.0.0.1:9"),
    ("HTTP_PROXY", "http://127.0.0.1:9"),
    ("http_proxy", "http://127.0.0.1:9"),
    ("NO_PROXY", ""),
    ("no_proxy", ""),
];

/// Send one JSON message on a split WebSocket writer.
async fn send_json(
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
    message: serde_json::Value,
) {
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string().into(),
        ))
        .await
        .expect("failed to send");
}

/// Read messages until an `info` or an `error`, the two replies a
/// `start_simulation` brings, and return it.
async fn next_info_or_error(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> serde_json::Value {
    loop {
        let msg = next_json(read).await;
        if msg["type"] == "info" || msg["type"] == "error" {
            return msg;
        }
    }
}

/// The satellite ids an `info` message lists.
fn info_satellite_ids(info: &serde_json::Value) -> Vec<String> {
    info["satellites"]
        .as_array()
        .expect("satellites")
        .iter()
        .map(|sat| sat["id"].as_str().expect("an id").to_string())
        .collect()
}

fn circular_start() -> serde_json::Value {
    serde_json::json!({
        "type": "start_simulation",
        "config": {
            "dt": 10.0,
            "satellites": [
                { "id": "test", "orbit": { "type": "circular", "altitude": 400.0 } }
            ]
        }
    })
}

/// A `start_simulation` whose simulation cannot be built is answered with an
/// `error` to the client that sent it, and the server starts the next one
/// (#554, #555).
///
/// The server cannot fetch `space_weather = "auto"`. `validate_sim_config`
/// accepts `"auto"` without fetching it (a path is refused there, #556), so
/// the failure comes from `SimParams::from_config`. The manager used to panic
/// there, after which the server closed every connection, new ones included,
/// until it was restarted (#554). After that was fixed, the request was still
/// acknowledged before the simulation was built, so the client that sent it
/// got neither an `error` nor an `info` (#555). The fetch fails through
/// `UNREACHABLE_PROXY_ENV`.
///
/// `fetch_default` answers from `$HOME/.cache/orts/SW-Last5Years.txt` when
/// that file is under a day old, without the request, so the server gets an
/// empty `HOME` of its own.
#[tokio::test]
async fn test_websocket_unbuildable_start_leaves_the_server_usable() {
    let home_dir = tempfile::tempdir().expect("a temporary HOME");
    let home = home_dir.path().to_str().expect("a UTF-8 temp path");
    let mut env = UNREACHABLE_PROXY_ENV.to_vec();
    env.push(("HOME", home));
    let mut server = Server::spawn_idle_with_env(&env);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();
        assert_eq!(next_json(&mut read).await["state"], "idle");
        // A second client, which sends nothing.
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_bystander_write, mut bystander) = ws.split();
        assert_eq!(next_json(&mut bystander).await["state"], "idle");

        // The failing start, then a valid one on the same connection. The
        // connection sends the second only after the server has answered the
        // first, and the manager takes its commands in order, so the second
        // reaches the manager after the first has failed.
        let mut failing = circular_start();
        failing["config"]["space_weather"] = "auto".into();
        send_json(&mut write, failing).await;
        let mut next = circular_start();
        next["config"]["satellites"][0]["id"] = "next".into();
        send_json(&mut write, next).await;

        let reply = next_info_or_error(&mut read).await;
        assert_eq!(
            reply["type"], "error",
            "the failing start is answered: {reply}"
        );
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.contains("Failed to fetch space weather data from CelesTrak"),
            "{message}"
        );

        // The simulation that starts is the second one.
        let reply = next_info_or_error(&mut read).await;
        assert_eq!(reply["type"], "info", "{reply}");
        assert_eq!(info_satellite_ids(&reply), ["/world/sat/next"]);

        // The client that sent nothing gets the `info` of the simulation that
        // started, and nothing about the one that failed.
        let seen = next_info_or_error(&mut bystander).await;
        assert_eq!(seen["type"], "info", "{seen}");
        assert_eq!(info_satellite_ids(&seen), ["/world/sat/next"]);

        // A new connection is still answered.
        let (ws, _) = connect_async(&url).await.expect("failed to reconnect");
        let (_write, mut read) = ws.split();
        read_until_type(&mut read, "info", 10).await;
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// A `start_simulation` the engine refuses is answered to the client that
/// sent it, and to no other (#555).
///
/// A satellite with `streams` and no controller passes `validate_sim_config`
/// and `SimParams::from_config`, and `ServeEngine::build` refuses it: only a
/// controller pumps a stream. The refusal used to be broadcast, so every
/// connected client got an `error` for a request it had not sent.
#[tokio::test]
async fn test_websocket_start_the_engine_refuses_is_answered_to_its_sender() {
    let mut server = Server::spawn_idle();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();
        assert_eq!(next_json(&mut read).await["state"], "idle");
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (_bystander_write, mut bystander) = ws.split();
        assert_eq!(next_json(&mut bystander).await["state"], "idle");

        // As in `test_websocket_unbuildable_start_leaves_the_server_usable`,
        // the second start reaches the manager after the first has failed.
        let mut refused = circular_start();
        refused["config"]["satellites"][0]["streams"] = serde_json::json!(["uart0"]);
        send_json(&mut write, refused).await;
        let mut next = circular_start();
        next["config"]["satellites"][0]["id"] = "next".into();
        send_json(&mut write, next).await;

        let reply = next_info_or_error(&mut read).await;
        assert_eq!(reply["type"], "error", "{reply}");
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.contains("no satellite has a controller"),
            "{message}"
        );
        let reply = next_info_or_error(&mut read).await;
        assert_eq!(reply["type"], "info", "{reply}");
        assert_eq!(info_satellite_ids(&reply), ["/world/sat/next"]);

        let seen = next_info_or_error(&mut bystander).await;
        assert_eq!(
            seen["type"], "info",
            "a client that sent no request gets no error for it: {seen}"
        );
        assert_eq!(info_satellite_ids(&seen), ["/world/sat/next"]);
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// After the simulation `orts serve` started from its command line is
/// terminated, a `start_simulation` that cannot be built is answered as well
/// (#555).
///
/// That server runs its first simulation outside the manager an idle server
/// runs, and hands it the starts that come after a terminate, so the reply
/// takes a different path to the client. The failure is the
/// `space_weather = "auto"` fetch of
/// `test_websocket_unbuildable_start_leaves_the_server_usable`.
#[tokio::test]
async fn test_websocket_unbuildable_start_after_terminate_is_answered() {
    let home_dir = tempfile::tempdir().expect("a temporary HOME");
    let home = home_dir.path().to_str().expect("a UTF-8 temp path");
    let mut env = UNREACHABLE_PROXY_ENV.to_vec();
    env.push(("HOME", home));
    let mut server = Server::spawn_with_sats_and_env(&["altitude=400,id=test"], &env);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();
        read_until_type(&mut read, "info", 10).await;

        send_json(
            &mut write,
            serde_json::json!({ "type": "terminate_simulation" }),
        )
        .await;
        // States keep streaming until the terminate lands.
        let (status, _) = read_until_type(&mut read, "status", 500).await;
        assert_eq!(status["state"], "idle", "{status}");

        let mut failing = circular_start();
        failing["config"]["space_weather"] = "auto".into();
        send_json(&mut write, failing).await;
        let mut next = circular_start();
        next["config"]["satellites"][0]["id"] = "next".into();
        send_json(&mut write, next).await;

        let reply = next_info_or_error(&mut read).await;
        assert_eq!(
            reply["type"], "error",
            "the failing start is answered: {reply}"
        );
        let message = reply["message"].as_str().expect("an error message");
        assert!(
            message.contains("Failed to fetch space weather data from CelesTrak"),
            "{message}"
        );
        let reply = next_info_or_error(&mut read).await;
        assert_eq!(reply["type"], "info", "{reply}");
        assert_eq!(info_satellite_ids(&reply), ["/world/sat/next"]);
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// An `add_satellite` with a TLE that does not parse is refused, and the
/// simulation keeps running (#554).
///
/// `SatelliteConfig::validate` does not parse the lines, so the malformed TLE
/// used to reach `to_satellite_spec` and panic in the manager: the running
/// simulation was lost and every connection closed.
#[tokio::test]
async fn test_websocket_add_with_a_malformed_tle_is_refused() {
    let mut server = Server::spawn();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();
        read_until_type(&mut read, "info", 10).await;

        send_json(
            &mut write,
            serde_json::json!({
                "type": "add_satellite",
                "id": "bad",
                "orbit": { "type": "tle", "line1": "1 x", "line2": "2 y" }
            }),
        )
        .await;
        // State messages keep streaming, so the error arrives among them.
        let (error, _) = read_until_type(&mut read, "error", 500).await;
        let message = error["message"].as_str().expect("an error message");
        assert!(message.contains("invalid TLE"), "{message}");

        // The simulation is still running, on this connection and a new one.
        read_until_type(&mut read, "state", 500).await;
        let (ws, _) = connect_async(&url).await.expect("failed to reconnect");
        let (_write, mut read) = ws.split();
        let (info, _) = read_until_type(&mut read, "info", 10).await;
        assert_eq!(info["satellites"].as_array().expect("satellites").len(), 1);
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// A NORAD satellite the server cannot fetch is refused at `start_simulation`
/// (#554).
///
/// Building a satellite's spec fetches a NORAD orbit's TLE. A failed fetch
/// used to panic in `validate_sim_config`, which built the specs before
/// `SimParams::from_config` built them again; they are built once now, in
/// `from_config`, and its error is the reply. The fetch fails through
/// `UNREACHABLE_PROXY_ENV`.
#[tokio::test]
async fn test_websocket_norad_start_the_server_cannot_fetch_is_refused() {
    let mut server = Server::spawn_idle_with_env(UNREACHABLE_PROXY_ENV);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();
        assert_eq!(next_json(&mut read).await["state"], "idle");

        send_json(
            &mut write,
            serde_json::json!({
                "type": "start_simulation",
                "config": {
                    "dt": 10.0,
                    "satellites": [
                        { "id": "iss", "orbit": { "type": "norad", "norad_id": 25544 } }
                    ]
                }
            }),
        )
        .await;
        let (error, _) = read_until_type(&mut read, "error", 10).await;
        let message = error["message"].as_str().expect("an error message");
        assert!(message.contains("NORAD ID 25544"), "{message}");

        // The same connection still starts a simulation the server can build.
        send_json(&mut write, circular_start()).await;
        let (info, _) = read_until_type(&mut read, "info", 10).await;
        assert_eq!(info["satellites"].as_array().expect("satellites").len(), 1);
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// A `start_simulation` that asks for realtime runs in realtime on a server
/// started without `--realtime`: the idle status says the server's default is
/// accelerated, the `info` says realtime, and the states keep to the wall
/// clock instead of running 100x ahead of it.
#[tokio::test]
async fn test_websocket_start_simulation_asks_for_realtime() {
    let mut server = Server::spawn_idle();

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        let status = next_json(&mut read).await;
        assert_eq!(status["state"], "idle");
        assert_eq!(status["default_pacing"], "accelerated");

        let mut start = circular_start();
        start["config"]["dt"] = 1.0.into();
        start["pacing"] = "realtime".into();
        send_json(&mut write, start).await;
        let info = next_info_or_error(&mut read).await;
        assert_eq!(info["type"], "info", "{info}");
        assert_eq!(info["pacing"], "realtime");

        // Accelerated, dt = 1 runs 100 sim s per wall s, so two wall seconds
        // would reach t = 200. Realtime reaches about 2 (slack for start-up
        // and scheduling).
        let started = std::time::Instant::now();
        let mut latest_t = 0.0_f64;
        while started.elapsed() < Duration::from_secs(2) {
            let msg = next_json(&mut read).await;
            if msg["type"] == "state" {
                latest_t = latest_t.max(msg["t"].as_f64().expect("t"));
            }
        }
        assert!(
            (1.0..=4.0).contains(&latest_t),
            "t = {latest_t} after 2 wall seconds"
        );
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}

/// `orts serve --realtime` says so in its idle status, and a
/// `start_simulation` that names no pacing runs at it; one that names
/// accelerated runs accelerated.
#[tokio::test]
async fn test_websocket_realtime_server_default_and_override() {
    let mut server = Server::spawn_with(&[], &[], &["--realtime"]);

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let url = server.ws_url();
        let (ws, _) = connect_async(&url).await.expect("failed to connect");
        let (mut write, mut read) = ws.split();

        let status = next_json(&mut read).await;
        assert_eq!(status["state"], "idle");
        assert_eq!(status["default_pacing"], "realtime");

        send_json(&mut write, circular_start()).await;
        let info = next_info_or_error(&mut read).await;
        assert_eq!(info["pacing"], "realtime", "{info}");

        send_json(
            &mut write,
            serde_json::json!({ "type": "terminate_simulation" }),
        )
        .await;
        let (idle, _) = read_until_type(&mut read, "status", 100).await;
        assert_eq!(idle["state"], "idle");

        let mut start = circular_start();
        start["pacing"] = "accelerated".into();
        send_json(&mut write, start).await;
        let info = next_info_or_error(&mut read).await;
        assert_eq!(info["pacing"], "accelerated", "{info}");
    })
    .await;

    server.kill();
    result.expect("test timed out after 30 seconds");
}
