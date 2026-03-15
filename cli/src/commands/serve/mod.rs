pub mod protocol;
pub mod compute;
mod manager;
mod connection;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

use crate::cli::SimArgs;
use crate::sim::params::SimParams;

use manager::SimCommand;
use connection::handle_connection;

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

async fn async_server(sim: &SimArgs, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    let actual_port = listener.local_addr().unwrap().port();
    eprintln!("WebSocket server listening on ws://localhost:{actual_port}");

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
        tokio::spawn(manager::simulation_manager_with_params(params, cmd_rx, mgr_tx));
    } else {
        tokio::spawn(manager::simulation_manager(initial_config, cmd_rx, mgr_tx));
    }

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        eprintln!("New connection from {peer}");
        let rx = tx.subscribe();
        let client_cmd_tx = cmd_tx.clone();

        tokio::spawn(async move {
            handle_connection(stream, rx, client_cmd_tx).await;
        });
    }
}
