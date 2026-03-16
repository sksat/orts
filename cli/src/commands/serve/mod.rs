pub mod compute;
mod connection;
mod history;
mod manager;
pub mod protocol;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

use crate::cli::SimArgs;
use crate::sim::params::SimParams;

use manager::SimCommand;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
}

pub fn run_server(sim: &SimArgs, port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(sim, port));
}

/// Detect whether CLI args specify an explicit simulation configuration.
fn has_explicit_sim_args(sim: &SimArgs) -> bool {
    sim.config.is_some()
        || !sim.sats.is_empty()
        || sim.tle.is_some()
        || sim.tle_line1.is_some()
        || sim.norad_id.is_some()
        // --altitude with non-default value
        || (sim.altitude - 400.0).abs() > 1e-9
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    let cmd_tx = state.cmd_tx.clone();
    ws.on_upgrade(move |socket| async move {
        connection::handle_connection(socket, rx, cmd_tx).await;
        eprintln!("Client disconnected");
    })
}

async fn async_server(sim: &SimArgs, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    let actual_port = listener.local_addr().unwrap().port();
    eprintln!("Server listening on http://localhost:{actual_port}");
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

    // Spawn simulation manager
    let mgr_tx = tx.clone();
    if has_explicit_sim_args(sim) && initial_config.is_none() {
        // Legacy path: build SimParams from CLI args directly
        let params = Arc::new(SimParams::from_sim_args(sim, true));
        tokio::spawn(manager::simulation_manager_with_params(
            params, cmd_rx, mgr_tx,
        ));
    } else {
        tokio::spawn(manager::simulation_manager(initial_config, cmd_rx, mgr_tx));
    }

    let state = AppState { tx, cmd_tx };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    axum::serve(listener, app).await.expect("server error");
}
