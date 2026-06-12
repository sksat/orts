pub mod compute;
mod connection;
mod history;
mod manager;
pub mod protocol;
#[cfg(feature = "viewer")]
pub(crate) mod spa;
mod stream_bridge;
pub(crate) mod textures;

use std::sync::Arc;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

use crate::cli::SimArgs;
use crate::sim::params::SimParams;

use manager::SimCommand;
use stream_bridge::StreamBridge;
use textures::TextureCache;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
    textures: Arc<TextureCache>,
    /// stream-io bridge endpoints (binary WS per declared stream).
    bridge: Arc<StreamBridge>,
    /// The (sat, stream) wired to stdio via `--stream-stdio`, if any. Its
    /// WS endpoint is reserved (answers 409) — one transport per stream.
    reserved_stdio: Option<stream_bridge::StreamKey>,
}

pub fn run_server(sim: &SimArgs, port: u16, stream_stdio: Option<&str>) {
    // Parse + reject malformed flags before starting the runtime so a typo
    // fails fast instead of surfacing as a dead endpoint later.
    let stdio_key = stream_stdio.map(|s| {
        parse_stream_stdio(s).unwrap_or_else(|e| panic!("Error: --stream-stdio {s}: {e}"))
    });
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(sim, port, stdio_key));
}

/// Parse a `--stream-stdio` value of the form `sat/stream` (both halves
/// non-empty, exactly one `/` — they are endpoint path segments).
fn parse_stream_stdio(s: &str) -> Result<stream_bridge::StreamKey, String> {
    match s.split_once('/') {
        Some((sat, stream)) if !sat.is_empty() && !stream.is_empty() && !stream.contains('/') => {
            Ok((sat.to_string(), stream.to_string()))
        }
        _ => Err("expected SAT/STREAM with non-empty halves".to_string()),
    }
}

/// Detect whether CLI args specify an explicit simulation configuration.
fn has_explicit_sim_args(sim: &SimArgs) -> bool {
    sim.config.is_some() || sim.has_orbit_args()
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    let cmd_tx = state.cmd_tx.clone();
    ws.on_upgrade(move |socket| async move {
        connection::handle_connection(socket, rx, cmd_tx).await;
        eprintln!("Client disconnected");
    })
}

/// Binary WS endpoint for a declared `stream-io` stream — the shape of a
/// kble `ws://` plug. 404 for undeclared `(sat, stream)` pairs; 409 for a
/// stream reserved by `--stream-stdio` (one transport per stream).
async fn stream_ws_handler(
    ws: WebSocketUpgrade,
    AxumPath((sat, stream)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> axum::response::Response {
    if state
        .reserved_stdio
        .as_ref()
        .is_some_and(|(s, n)| *s == sat && *n == stream)
    {
        return (
            StatusCode::CONFLICT,
            format!("stream {sat}/{stream} is reserved for the stdio plug (--stream-stdio)\n"),
        )
            .into_response();
    }
    let Some(endpoint) = state.bridge.lookup(&sat, &stream) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no such stream endpoint: {sat}/{stream}\n"),
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| stream_bridge::handle_stream_socket(socket, endpoint, sat, stream))
        .into_response()
}

async fn async_server(sim: &SimArgs, port: u16, stdio_key: Option<stream_bridge::StreamKey>) {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    let actual_port = listener.local_addr().unwrap().port();
    eprintln!("Server listening on http://localhost:{actual_port}");
    #[cfg(feature = "viewer")]
    eprintln!("Viewer:             http://localhost:{actual_port}/");
    eprintln!("WebSocket endpoint: ws://localhost:{actual_port}/ws");

    let (tx, _rx) = broadcast::channel::<String>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<SimCommand>(16);

    // Determine initial config: if CLI args specify simulation, auto-start.
    let initial_config = if has_explicit_sim_args(sim) {
        sim.config.as_ref().map(|config_path| {
            crate::config::SimConfig::load(std::path::Path::new(config_path))
                .unwrap_or_else(|e| panic!("Error: {e}"))
        })
    } else {
        None
    };

    // `orts serve` does not drive config `[[command]]` timelines (run-only);
    // reject loudly instead of silently dropping scheduled uplinks.
    if let Some(cfg) = &initial_config {
        cfg.ensure_serve_supported()
            .unwrap_or_else(|e| panic!("Error: {e}"));
    }

    // With an explicit config, a `--stream-stdio` typo would otherwise be a
    // silent forever-retry; validate the declaration up front. (Without a
    // config the sim starts later via WS, so the stdio task just waits.)
    if let (Some(cfg), Some((sat, stream))) = (&initial_config, &stdio_key) {
        let body = crate::satellite::parse_body(&cfg.body);
        let declared = cfg.satellites.iter().enumerate().any(|(i, s)| {
            let spec = s.to_satellite_spec(i, body, body.properties().mu);
            spec.id == *sat && spec.streams.iter().any(|n| n == stream)
        });
        if !declared {
            panic!(
                "Error: --stream-stdio {sat}/{stream} is not declared in the config (streams = [...])"
            );
        }
    }

    let texture_cache = Arc::new(TextureCache::new());
    let texture_request_tx =
        textures::spawn_texture_downloader(Arc::clone(&texture_cache), tx.clone());
    let bridge = Arc::new(StreamBridge::new());

    // Spawn simulation manager
    let mgr_tx = tx.clone();
    let plugin_overrides = manager::PluginBackendOverrides::from_sim_args(sim);
    if has_explicit_sim_args(sim) && initial_config.is_none() {
        // Legacy path: build SimParams from CLI args directly.
        // from_sim_args already populates plugin_backend_choice /
        // threshold, but we still pass the overrides so that any
        // later delegate to simulation_manager (after a terminate +
        // restart) honors them too.
        let params = Arc::new(SimParams::from_sim_args(sim, true));
        tokio::spawn(manager::simulation_manager_with_params(
            params,
            plugin_overrides,
            cmd_rx,
            mgr_tx,
            texture_request_tx.clone(),
            Arc::clone(&bridge),
        ));
    } else {
        tokio::spawn(manager::simulation_manager(
            initial_config,
            plugin_overrides,
            cmd_rx,
            mgr_tx,
            texture_request_tx.clone(),
            Arc::clone(&bridge),
        ));
    }

    // The stdio plug task drives stdin/stdout with the kble-socket protocol
    // and signals shutdown when the peer (the kble harness that spawned us)
    // closes the connection.
    let shutdown_rx = stdio_key.clone().map(|(sat, stream)| {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(stream_bridge::run_stdio_plug(
            Arc::clone(&bridge),
            sat,
            stream,
            shutdown_tx,
        ));
        shutdown_rx
    });

    let state = AppState {
        tx,
        cmd_tx,
        textures: texture_cache,
        bridge,
        reserved_stdio: stdio_key,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/stream/{sat}/{stream}", get(stream_ws_handler))
        .route(
            "/textures/{filename}",
            get(textures::texture_handler).with_state(Arc::clone(&state.textures)),
        );

    #[cfg(feature = "viewer")]
    let app = app.fallback(spa::spa_handler);

    let app = app.with_state(state);

    match shutdown_rx {
        Some(rx) => {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                    eprintln!("stdio plug closed; shutting down");
                })
                .await
                .expect("server error");
        }
        None => axum::serve(listener, app).await.expect("server error"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_stream_stdio;

    #[test]
    fn parse_stream_stdio_accepts_sat_slash_stream() {
        assert_eq!(
            parse_stream_stdio("sat0/comlink"),
            Ok(("sat0".to_string(), "comlink".to_string()))
        );
    }

    #[test]
    fn parse_stream_stdio_rejects_malformed_values() {
        assert!(parse_stream_stdio("nodelimiter").is_err());
        assert!(parse_stream_stdio("/comlink").is_err());
        assert!(parse_stream_stdio("sat0/").is_err());
        assert!(parse_stream_stdio("sat0/a/b").is_err());
    }
}
