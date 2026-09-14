use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;

use orts::record::entity_path::EntityPath;
use orts::record::rerun_export::{RrdData, load_rrd_data};

use crate::commands::serve::protocol::{ClientMessage, WsMessage};
use crate::satellite::SatelliteInfo;
use crate::sim::core::{HistoryState, ModelLoads, downsample_states, make_history_state};

/// Pre-loaded replay data shared across connections.
struct ReplayData {
    info_json: String,
    /// States grouped by entity_path, each sorted by t. Ordered by path, so
    /// everything derived from it — the Info satellite list, the entity
    /// `estimate_dt` reads, the overview's order among equal timestamps — is
    /// the same on every run.
    states_by_entity: BTreeMap<String, Vec<HistoryState>>,
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
    let central_body = meta.body_name.as_deref().unwrap_or("earth").to_lowercase();

    // Convert RRD rows to HistoryState, grouped by entity_path
    let mut states_by_entity: BTreeMap<String, Vec<HistoryState>> = BTreeMap::new();

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
            rw_momentum: None,
        });

        let hs = make_history_state(
            entity_path.clone(),
            row.t,
            &pos,
            &vel,
            mu,
            body_radius,
            // An `.rrd` carries the per-model torque columns `orts run` writes,
            // but `RrdRow` does not decode them and the replay advertises no
            // perturbations, so the charts would stay hidden even if it did.
            // Tracked separately; a file without them reads as no torque.
            ModelLoads::accelerations(HashMap::new()),
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

    let satellites = satellite_infos(&states_by_entity, body_radius, &rrd, meta.period);

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

/// One `SatelliteInfo` per entity, in entity-path order.
///
/// `period` comes from the `orts.OrbitalPeriod` static the run logs on each
/// satellite's own path. A recording made before that existed carries only
/// `meta/sim/period`, which is the first satellite *in config order* — replay
/// lists satellites in entity-path order, so that number is attributed only
/// when the recording holds one satellite and the two orders cannot disagree.
/// Otherwise the period is 0, which is what every satellite got before.
fn satellite_infos(
    states_by_entity: &BTreeMap<String, Vec<HistoryState>>,
    body_radius: f64,
    statics: &RrdData,
    recording_period: Option<f64>,
) -> Vec<SatelliteInfo> {
    let single = states_by_entity.len() == 1;
    states_by_entity
        .iter()
        .map(|(ep_str, states)| {
            let ep = EntityPath::parse(ep_str);
            let first = &states[0];
            let r_mag =
                (first.position[0].powi(2) + first.position[1].powi(2) + first.position[2].powi(2))
                    .sqrt();
            let period = statics
                .static_scalar(ep_str, "period")
                .or(if single { recording_period } else { None })
                .unwrap_or(0.0);
            SatelliteInfo {
                id: ep.to_string(),
                name: Some(ep.name().to_string()),
                altitude: r_mag - body_radius,
                period,
                perturbations: vec![],
                shape: None,
            }
        })
        .collect()
}

/// Estimate dt from the median time step of the first entity by path.
fn estimate_dt(states_by_entity: &BTreeMap<String, Vec<HistoryState>>) -> f64 {
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
    by_entity: &BTreeMap<String, Vec<HistoryState>>,
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

    let state = ReplayAppState { data, ws_tx };

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

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ReplayAppState>,
) -> impl IntoResponse {
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

    // 2. Send bounded overview history (per-entity downsampled). We no
    //    longer stream a full-resolution detail dump on connect — if a client
    //    needs higher resolution for a specific range, it issues a targeted
    //    `query_range` request (same contract as `orts serve`).
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

    // 3. Main loop: forward texture_ready + handle client messages
    loop {
        tokio::select! {
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
            let mut by_entity: BTreeMap<String, Vec<HistoryState>> = BTreeMap::new();
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
            ModelLoads::default(),
            None,
        )
    }

    /// Eight paths, so an arbitrary order is essentially never the sorted one.
    fn eight_entities(interval_of_first: f64) -> BTreeMap<String, Vec<HistoryState>> {
        let mut by_entity = BTreeMap::new();
        for i in (0..8).rev() {
            let path = format!("/world/sat/s{i}");
            // the first path by order gets its own sample interval, the rest another
            let step = if i == 0 { interval_of_first } else { 7.0 };
            let states = (0..3)
                .map(|k| make_test_state(&path, k as f64 * step))
                .collect();
            by_entity.insert(path, states);
        }
        by_entity
    }

    /// An `RrdData` carrying only the statics a test needs.
    fn rrd_with_statics(pairs: &[(&str, f64)]) -> RrdData {
        let mut data = RrdData::default();
        for (key, value) in pairs {
            data.statics.insert((*key).to_string(), *value);
        }
        data
    }

    fn two_entities() -> BTreeMap<String, Vec<HistoryState>> {
        let mut by_entity = BTreeMap::new();
        for path in ["/world/sat/b", "/world/sat/a"] {
            by_entity.insert(
                path.to_string(),
                vec![make_test_state(path, 0.0), make_test_state(path, 60.0)],
            );
        }
        by_entity
    }

    #[test]
    fn each_satellite_reports_the_period_recorded_on_its_own_path() {
        // A recording-wide period as well: the per-satellite statics win.
        let statics = rrd_with_statics(&[
            ("world/sat/a/period", 5500.0),
            ("world/sat/b/period", 7000.0),
        ]);
        let infos = satellite_infos(&two_entities(), 6378.137, &statics, Some(1234.0));
        let periods: Vec<(String, f64)> = infos.into_iter().map(|i| (i.id, i.period)).collect();
        assert_eq!(
            periods,
            vec![
                ("/world/sat/a".to_string(), 5500.0),
                ("/world/sat/b".to_string(), 7000.0)
            ]
        );
    }

    #[test]
    fn a_recording_wide_period_is_not_handed_to_one_of_several_satellites() {
        // `meta/sim/period` is the first satellite in config order, which is not
        // knowable from the RRD: with two satellites it is attributed to none.
        let infos = satellite_infos(&two_entities(), 6378.137, &RrdData::default(), Some(7000.0));
        assert!(
            infos.iter().all(|i| i.period == 0.0),
            "a period was attributed without evidence: {:?}",
            infos.iter().map(|i| (&i.id, i.period)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_single_satellite_recording_keeps_using_the_recording_wide_period() {
        // One satellite: config order and entity-path order cannot disagree, so
        // an older recording still reports its orbit.
        let mut by_entity = BTreeMap::new();
        by_entity.insert(
            "/world/sat/only".to_string(),
            vec![make_test_state("/world/sat/only", 0.0)],
        );
        let infos = satellite_infos(&by_entity, 6378.137, &RrdData::default(), Some(5828.5));
        assert_eq!(infos.len(), 1);
        assert!(
            (infos[0].period - 5828.5).abs() < 1e-9,
            "{}",
            infos[0].period
        );
    }

    #[test]
    fn the_info_message_lists_satellites_in_entity_path_order() {
        let by_entity = eight_entities(60.0);
        let ids: Vec<String> = satellite_infos(&by_entity, 6378.137, &RrdData::default(), None)
            .into_iter()
            .map(|s| s.id)
            .collect();
        let expected: Vec<String> = (0..8).map(|i| format!("/world/sat/s{i}")).collect();
        assert_eq!(
            ids, expected,
            "the order a client sees must not vary per run"
        );
    }

    #[test]
    fn estimate_dt_reads_the_first_entity_by_path() {
        let dt = estimate_dt(&eight_entities(60.0));
        assert!(
            (dt - 60.0).abs() < 1e-9,
            "expected the interval of /world/sat/s0, got {dt}"
        );
    }

    /// Eight entities sampled at the same instants, inserted in reverse.
    fn eight_entities_same_instants() -> BTreeMap<String, Vec<HistoryState>> {
        let mut by_entity = BTreeMap::new();
        for i in (0..8).rev() {
            let path = format!("/world/sat/s{i}");
            let states = (0..3)
                .map(|k| make_test_state(&path, k as f64 * 60.0))
                .collect();
            by_entity.insert(path, states);
        }
        by_entity
    }

    fn paths_at(states: &[HistoryState], t: f64) -> Vec<String> {
        states
            .iter()
            .filter(|s| s.t == t)
            .map(|s| s.entity_path.to_string())
            .collect()
    }

    #[test]
    fn the_overview_breaks_ties_in_entity_path_order() {
        // The merge is a stable sort by t, so the order among equal t is the
        // order the entities were iterated in.
        let overview = downsample_per_entity(&eight_entities_same_instants(), 64);
        let expected: Vec<String> = (0..8).map(|i| format!("/world/sat/s{i}")).collect();
        assert_eq!(
            paths_at(&overview, 0.0),
            expected,
            "samples sharing a timestamp must come out in a fixed order"
        );
    }

    #[test]
    fn a_downsampled_query_range_breaks_ties_in_entity_path_order() {
        let by_entity = eight_entities_same_instants();
        let mut all_states: Vec<HistoryState> =
            by_entity.values().flat_map(|v| v.iter().cloned()).collect();
        all_states.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        let data = ReplayData {
            info_json: String::new(),
            states_by_entity: by_entity,
            all_states,
            central_body: "earth".to_string(),
        };
        let range = handle_query_range(&data, 0.0, 120.0, Some(64), None);
        let expected: Vec<String> = (0..8).map(|i| format!("/world/sat/s{i}")).collect();
        assert_eq!(paths_at(&range, 60.0), expected);
    }

    #[test]
    fn estimate_dt_from_states() {
        let mut by_entity = BTreeMap::new();
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
        let mut by_entity = BTreeMap::new();
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
        let mut by_entity = BTreeMap::new();
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
