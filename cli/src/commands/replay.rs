use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;

use orts::record::entity_path::EntityPath;
use orts::record::rerun_export::load_rrd_data;

use crate::commands::serve::protocol::{ClientMessage, WsMessage};
use crate::satellite::SatelliteInfo;
use crate::sim::core::{downsample_states, make_history_state, HistoryState};

/// Pre-loaded replay data shared across connections.
struct ReplayData {
    info_json: String,
    /// States grouped by entity_path, each sorted by t.
    states_by_entity: HashMap<String, Vec<HistoryState>>,
    /// All states merged and sorted by t.
    all_states: Vec<HistoryState>,
    /// Central body name (e.g. "earth") for texture downloads.
    central_body: String,
}

/// Shared state for the replay axum server.
#[derive(Clone)]
struct ReplayAppState {
    data: Arc<ReplayData>,
    ws_tx: tokio::sync::broadcast::Sender<String>,
}

pub fn run_replay(input: &str, port: u16) {
    let data = load_replay_data(input);
    let data = Arc::new(data);

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(data, port));
}

fn load_replay_data(path: &str) -> ReplayData {
    let rrd = load_rrd_data(path).unwrap_or_else(|e| {
        eprintln!("Error reading {path}: {e}");
        std::process::exit(1);
    });

    let meta = &rrd.metadata;
    let mu = meta.mu.unwrap_or(398600.4418);
    let body_radius = meta.body_radius.unwrap_or(6378.137);
    let central_body = meta
        .body_name
        .as_deref()
        .unwrap_or("earth")
        .to_lowercase();

    // Convert RRD rows to HistoryState, grouped by entity_path
    let mut states_by_entity: HashMap<String, Vec<HistoryState>> = HashMap::new();

    for row in &rrd.rows {
        let entity_path = row
            .entity_path
            .as_deref()
            .map(EntityPath::parse)
            .unwrap_or_else(|| EntityPath::parse("/world/sat/default"));

        let pos = nalgebra::Vector3::new(row.x, row.y, row.z);
        let vel = nalgebra::Vector3::new(row.vx, row.vy, row.vz);

        let attitude = row.quaternion.map(|q| crate::sim::core::AttitudePayload {
            quaternion_wxyz: q,
            angular_velocity_body: row.angular_velocity.unwrap_or([0.0; 3]),
            source: crate::sim::core::AttitudeSource::Propagated,
        });

        let hs = make_history_state(
            entity_path.clone(),
            row.t,
            &pos,
            &vel,
            mu,
            body_radius,
            HashMap::new(),
            attitude,
        );

        states_by_entity
            .entry(entity_path.to_string())
            .or_default()
            .push(hs);
    }

    // Sort each entity's states by time
    for states in states_by_entity.values_mut() {
        states.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    }

    // Merge all states sorted by time
    let mut all_states: Vec<HistoryState> = states_by_entity
        .values()
        .flat_map(|v| v.iter().cloned())
        .collect();
    all_states.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

    // Build info message
    let entity_count = states_by_entity.len();
    let dt = estimate_dt(&states_by_entity);

    let satellites: Vec<SatelliteInfo> = states_by_entity
        .keys()
        .map(|ep_str| {
            let ep = EntityPath::parse(ep_str);
            let states = &states_by_entity[ep_str];
            let first = &states[0];
            let r_mag =
                (first.position[0].powi(2) + first.position[1].powi(2) + first.position[2].powi(2))
                    .sqrt();
            SatelliteInfo {
                id: ep.to_string(),
                name: Some(ep.name().to_string()),
                altitude: r_mag - body_radius,
                period: 0.0,
                perturbations: vec![],
            }
        })
        .collect();

    let info_msg = WsMessage::Info {
        mu,
        dt,
        output_interval: dt,
        stream_interval: dt,
        central_body: central_body.clone(),
        central_body_radius: body_radius,
        epoch_jd: meta.epoch_jd,
        satellites,
    };
    let info_json = serde_json::to_string(&info_msg).expect("failed to serialize info");

    eprintln!(
        "Loaded {} entities, {} total states from {path}",
        entity_count,
        all_states.len()
    );

    ReplayData {
        info_json,
        states_by_entity,
        all_states,
        central_body,
    }
}

/// Estimate dt from median time step within the first entity.
fn estimate_dt(states_by_entity: &HashMap<String, Vec<HistoryState>>) -> f64 {
    for states in states_by_entity.values() {
        if states.len() >= 2 {
            let mut dts: Vec<f64> = states
                .windows(2)
                .map(|w| w[1].t - w[0].t)
                .filter(|dt| *dt > 0.0)
                .collect();
            if !dts.is_empty() {
                dts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                return dts[dts.len() / 2];
            }
        }
    }
    10.0 // fallback
}

/// Build per-entity downsampled overview, then merge.
fn build_overview(data: &ReplayData, max_points: usize) -> Vec<HistoryState> {
    downsample_per_entity(&data.states_by_entity, max_points)
}

/// Downsample each entity independently, then merge. Guarantees total <= max_points.
fn downsample_per_entity(
    by_entity: &HashMap<String, Vec<HistoryState>>,
    max_points: usize,
) -> Vec<HistoryState> {
    let entity_count = by_entity.len().max(1);
    let per_entity = (max_points / entity_count).max(2);

    let mut result: Vec<HistoryState> = Vec::new();
    for states in by_entity.values() {
        result.extend(downsample_states(states, per_entity));
    }
    result.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

    // Clamp if per_entity minimum (2) caused overshoot
    if result.len() > max_points {
        result = downsample_states(&result, max_points);
    }
    result
}

async fn async_server(data: Arc<ReplayData>, port: u16) {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    let actual_port = listener.local_addr().unwrap().port();
    eprintln!("Replay server listening on http://localhost:{actual_port}");
    eprintln!("WebSocket endpoint: ws://localhost:{actual_port}/ws");

    let texture_cache = Arc::new(crate::commands::serve::textures::TextureCache::new());

    // Broadcast channel for forwarding texture_ready messages to WebSocket clients
    let (ws_tx, _) = tokio::sync::broadcast::channel::<String>(64);

    // Spawn high-res texture downloader (same as orts serve)
    let texture_request_tx = crate::commands::serve::textures::spawn_texture_downloader(
        Arc::clone(&texture_cache),
        ws_tx.clone(),
    );
    // Request textures for the central body and any secondary bodies
    let mut bodies = vec![data.central_body.clone()];
    for entity_path in data.states_by_entity.keys() {
        let ep = EntityPath::parse(entity_path);
        // Non-satellite entities (e.g. /world/moon) are secondary bodies
        if !entity_path.starts_with("/world/sat/") {
            bodies.push(ep.name().to_string());
        }
    }
    bodies.dedup();
    let _ = texture_request_tx.send(bodies).await;

    let state = ReplayAppState {
        data,
        ws_tx,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route(
            "/textures/{filename}",
            get(crate::commands::serve::textures::texture_handler)
                .with_state(Arc::clone(&texture_cache)),
        )
        .with_state(state);

    #[cfg(feature = "viewer")]
    let app = app.fallback(crate::commands::serve::spa::spa_handler);

    axum::serve(listener, app).await.expect("server error");
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<ReplayAppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handle_connection(socket, state.data, state.ws_tx).await;
        eprintln!("Client disconnected");
    })
}

async fn handle_connection(
    socket: WebSocket,
    data: Arc<ReplayData>,
    ws_tx: tokio::sync::broadcast::Sender<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut texture_rx = ws_tx.subscribe();

    // 1. Send info
    if sender
        .send(Message::Text(data.info_json.clone().into()))
        .await
        .is_err()
    {
        return;
    }

    // 2. Send overview history (per-entity downsampled)
    let overview = build_overview(&data, 1000);
    let history_msg =
        serde_json::to_string(&WsMessage::History { states: overview }).expect("serialize");
    if sender
        .send(Message::Text(history_msg.into()))
        .await
        .is_err()
    {
        return;
    }

    // 3. Send full detail in chunks
    let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<String>(16);
    let all_states = data.all_states.clone();
    tokio::spawn(async move {
        for chunk in all_states.chunks(1000) {
            let msg = WsMessage::HistoryDetail {
                states: chunk.to_vec(),
            };
            let json = serde_json::to_string(&msg).expect("serialize");
            if detail_tx.send(json).await.is_err() {
                return;
            }
        }
        let complete =
            serde_json::to_string(&WsMessage::HistoryDetailComplete).expect("serialize");
        let _ = detail_tx.send(complete).await;
    });

    // 4. Main loop: send detail chunks + forward texture_ready + handle client messages
    loop {
        tokio::select! {
            detail = detail_rx.recv() => {
                match detail {
                    Some(json) => {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        // All detail sent, continue listening for query_range
                    }
                }
            }
            texture_msg = texture_rx.recv() => {
                if let Ok(json) = texture_msg
                    && sender.send(Message::Text(json.into())).await.is_err()
                {
                    break;
                }
            }
            ws_msg = receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, entity_path } => {
                                    let states = handle_query_range(&data, t_min, t_max, max_points, entity_path.as_ref());
                                    let resp = WsMessage::QueryRangeResponse { t_min, t_max, states };
                                    let json = serde_json::to_string(&resp).expect("serialize");
                                    if sender.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                                _ => {
                                    // Replay mode: start/pause/resume/terminate/add_satellite not supported
                                    let err = WsMessage::Error {
                                        message: "not supported in replay mode".to_string(),
                                    };
                                    let json = serde_json::to_string(&err).expect("serialize");
                                    if sender.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }
}

fn handle_query_range(
    data: &ReplayData,
    t_min: f64,
    t_max: f64,
    max_points: Option<usize>,
    entity_path: Option<&EntityPath>,
) -> Vec<HistoryState> {
    let filtered: Vec<&HistoryState> = data
        .all_states
        .iter()
        .filter(|s| s.t >= t_min && s.t <= t_max)
        .filter(|s| entity_path.is_none_or(|ep| s.entity_path == *ep))
        .collect();

    match max_points {
        Some(mp) => {
            let owned: Vec<HistoryState> = filtered.into_iter().cloned().collect();
            let mut by_entity: HashMap<String, Vec<HistoryState>> = HashMap::new();
            for s in owned {
                by_entity
                    .entry(s.entity_path.to_string())
                    .or_default()
                    .push(s);
            }
            downsample_per_entity(&by_entity, mp)
        }
        None => filtered.into_iter().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_state(entity_path: &str, t: f64) -> HistoryState {
        make_history_state(
            EntityPath::parse(entity_path),
            t,
            &nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            398600.4418,
            6378.137,
            HashMap::new(),
            None,
        )
    }

    #[test]
    fn estimate_dt_from_states() {
        let mut by_entity = HashMap::new();
        by_entity.insert(
            "/world/sat/test".to_string(),
            vec![
                make_test_state("/world/sat/test", 0.0),
                make_test_state("/world/sat/test", 60.0),
                make_test_state("/world/sat/test", 120.0),
            ],
        );
        let dt = estimate_dt(&by_entity);
        assert!((dt - 60.0).abs() < 1e-9);
    }

    #[test]
    fn overview_downsamples_per_entity() {
        let mut by_entity = HashMap::new();
        let sat_states: Vec<HistoryState> = (0..100)
            .map(|i| make_test_state("/world/sat/apollo11", i as f64 * 60.0))
            .collect();
        let moon_states: Vec<HistoryState> = (0..100)
            .map(|i| make_test_state("/world/moon", i as f64 * 60.0))
            .collect();

        let mut all_states = sat_states.clone();
        all_states.extend(moon_states.clone());
        all_states.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        by_entity.insert("/world/sat/apollo11".to_string(), sat_states);
        by_entity.insert("/world/moon".to_string(), moon_states);

        let data = ReplayData {
            info_json: String::new(),
            states_by_entity: by_entity,
            all_states,
            central_body: "earth".to_string(),
        };

        let overview = build_overview(&data, 20);
        // Should have points from both entities
        let sat_count = overview
            .iter()
            .filter(|s| s.entity_path == EntityPath::parse("/world/sat/apollo11"))
            .count();
        let moon_count = overview
            .iter()
            .filter(|s| s.entity_path == EntityPath::parse("/world/moon"))
            .count();
        assert!(sat_count > 0, "overview should include satellite data");
        assert!(moon_count > 0, "overview should include moon data");
        assert!(overview.len() <= 20, "overview should respect max_points");
    }

    #[test]
    fn query_range_filters_and_downsamples() {
        let mut by_entity = HashMap::new();
        let states: Vec<HistoryState> = (0..100)
            .map(|i| make_test_state("/world/sat/test", i as f64 * 10.0))
            .collect();
        by_entity.insert("/world/sat/test".to_string(), states.clone());

        let data = ReplayData {
            info_json: String::new(),
            states_by_entity: by_entity,
            all_states: states,
            central_body: "earth".to_string(),
        };

        // Filter to t=[200, 500]
        let result = handle_query_range(&data, 200.0, 500.0, Some(10), None);
        assert!(result.len() <= 10);
        for s in &result {
            assert!(s.t >= 200.0 && s.t <= 500.0);
        }
    }
}
