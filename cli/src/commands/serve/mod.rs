pub mod protocol;
pub mod compute;

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use orts_integrator::State;
use orts_orbits::orbital_system::OrbitalSystem;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::cli::{SimArgs, IntegratorChoice};
use crate::satellite::{SatelliteSpec, SatelliteInfo};
use crate::sim::core::{accel_breakdown, build_orbital_system, make_history_state};
use crate::sim::params::SimParams;

use protocol::{ClientMessage, HistoryBuffer, WsMessage};
use compute::{compute_output_chunk, compute_output_chunk_adaptive, state_message};

pub fn run_server(sim: &SimArgs, port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(sim, port));
}

async fn async_server(sim: &SimArgs, port: u16) {
    let params = Arc::new(SimParams::from_sim_args(sim, true));
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    eprintln!("WebSocket server listening on ws://localhost:{port}");

    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let history = Arc::new(tokio::sync::RwLock::new(HistoryBuffer::new(5000, data_dir, params.mu)));

    let (tx, _rx) = broadcast::channel::<String>(256);

    // Shared list of serialized simulation_terminated JSON messages.
    // The simulation loop appends here when a satellite terminates;
    // handle_connection replays them to late-connecting clients.
    let terminated_events: Arc<tokio::sync::RwLock<Vec<String>>> =
        Arc::new(tokio::sync::RwLock::new(Vec::new()));

    let sim_tx = tx.clone();
    let sim_params = Arc::clone(&params);
    let sim_history = Arc::clone(&history);
    let sim_terminated = Arc::clone(&terminated_events);
    tokio::spawn(async move {
        simulation_loop(sim_params, sim_tx, sim_history, sim_terminated).await;
    });

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        eprintln!("New connection from {peer}");

        // Subscribe before spawning handler (no lost messages)
        let rx = tx.subscribe();
        let client_params = Arc::clone(&params);
        let client_history = Arc::clone(&history);
        let client_terminated = Arc::clone(&terminated_events);

        tokio::spawn(async move {
            handle_connection(stream, rx, client_params, client_history, client_terminated).await;
        });
    }
}

/// Per-satellite simulation state tracked across loop iterations.
pub struct SatSimState {
    pub spec: SatelliteSpec,
    pub system: OrbitalSystem,
    pub state: State,
    pub t: f64,
    pub orbit_end_t: f64,
    pub next_stream_t: f64,
    pub next_save_t: f64,
    pub terminated: bool,
}

async fn simulation_loop(
    params: Arc<SimParams>,
    tx: broadcast::Sender<String>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
    terminated_events: Arc<tokio::sync::RwLock<Vec<String>>>,
) {
    let dt = params.dt;

    // Batch N stream intervals into a single compute chunk.
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;

    // Wall-clock pacing: target sim speed ratio.
    let wall_per_sim_sec = ((dt / 100.0).max(0.01)) / params.stream_interval;
    let chunk_wall_time =
        std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    // Initialize per-satellite state
    let mut sat_states: Vec<SatSimState> = params.satellites.iter().map(|spec| {
        let initial = spec.initial_state(params.mu);
        SatSimState {
            spec: spec.clone(),
            system: build_orbital_system(&params.body, params.mu, params.epoch, spec, params.atmosphere, params.f107, params.ap, params.space_weather_provider.as_ref()),
            state: initial,
            t: 0.0,
            orbit_end_t: spec.period,
            next_stream_t: params.stream_interval,
            next_save_t: params.output_interval,
            terminated: false,
        }
    }).collect();

    // Emit initial states for all satellites
    {
        let mut h = history.write().await;
        for ss in &sat_states {
            let accels = accel_breakdown(&ss.system, 0.0, &ss.state);
            let hs = make_history_state(&ss.spec.id, 0.0, &ss.state.position, &ss.state.velocity, params.mu, accels.clone());
            h.push(hs);
            let msg = state_message(&ss.spec.id, 0.0, &ss.state, params.mu, accels);
            let _ = tx.send(msg);
        }
    }

    loop {
        let chunk_start = tokio::time::Instant::now();
        let mut all_outputs: Vec<crate::sim::core::HistoryState> = Vec::new();

        for ss in &mut sat_states {
            if ss.terminated {
                continue;
            }

            // Each satellite advances by exactly chunk_sim_time, handling
            // orbit boundaries within the loop so all satellites stay in sync.
            let target_t = ss.t + chunk_sim_time;

            // Skip orbit boundary reset when perturbations are active
            // (orbit is no longer periodic with J2)
            let has_perturbations = params.body.properties().j2.is_some();

            while ss.t < target_t - 1e-9 {
                // Check orbit boundary → reset (only for unperturbed 2-body)
                if !has_perturbations && ss.t >= ss.orbit_end_t - 1e-9 {
                    ss.state = ss.spec.initial_state(params.mu);
                    ss.orbit_end_t = ss.t + ss.spec.period;
                }

                // With perturbations, orbit is not periodic; propagate continuously
                let sub_end = if has_perturbations {
                    target_t
                } else {
                    target_t.min(ss.orbit_end_t)
                };

                let atm_alt = params.body.properties().atmosphere_altitude;
                let (outputs, new_state, new_t, termination) = match params.integrator {
                    IntegratorChoice::Rk4 => compute_output_chunk(
                        &ss.spec.id,
                        &ss.system,
                        ss.state.clone(),
                        ss.t,
                        sub_end,
                        dt,
                        params.stream_interval,
                        &mut ss.next_stream_t,
                        atm_alt,
                    ),
                    IntegratorChoice::Dp45 => compute_output_chunk_adaptive(
                        &ss.spec.id,
                        &ss.system,
                        ss.state.clone(),
                        ss.t,
                        sub_end,
                        dt,
                        &params.tolerances,
                        params.stream_interval,
                        &mut ss.next_stream_t,
                        atm_alt,
                    ),
                };

                ss.state = new_state;
                ss.t = new_t;

                // Save output_interval-aligned states to history
                {
                    let mut h = history.write().await;
                    for out in &outputs {
                        if out.t >= ss.next_save_t - 1e-9 {
                            h.push(out.clone());
                            ss.next_save_t += params.output_interval;
                        }
                    }
                }

                all_outputs.extend(outputs);

                // Handle termination
                if let Some(reason) = termination {
                    eprintln!(
                        "Simulation terminated for {} at t={:.2}s: {}",
                        ss.spec.id, ss.t, reason
                    );
                    let msg = serde_json::to_string(&WsMessage::SimulationTerminated {
                        satellite_id: ss.spec.id.clone(),
                        t: ss.t,
                        reason: reason.to_string(),
                    })
                    .expect("failed to serialize termination message");
                    let _ = tx.send(msg.clone());
                    terminated_events.write().await.push(msg);
                    ss.terminated = true;
                    break;
                }
            }
        }

        // Sort all outputs by time for interleaved sending
        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        if !all_outputs.is_empty() {
            let send_interval = chunk_wall_time / all_outputs.len() as u32;
            for out in &all_outputs {
                let send_start = tokio::time::Instant::now();
                let msg = serde_json::to_string(&WsMessage::State {
                    satellite_id: out.satellite_id.clone(),
                    t: out.t,
                    position: out.position,
                    velocity: out.velocity,
                    semi_major_axis: out.semi_major_axis,
                    eccentricity: out.eccentricity,
                    inclination: out.inclination,
                    raan: out.raan,
                    argument_of_periapsis: out.argument_of_periapsis,
                    true_anomaly: out.true_anomaly,
                    accelerations: out.accelerations.clone(),
                })
                .expect("failed to serialize state");
                let _ = tx.send(msg);

                let send_elapsed = send_start.elapsed();
                if send_elapsed < send_interval {
                    tokio::time::sleep(send_interval - send_elapsed).await;
                }
            }
        } else {
            let elapsed = chunk_start.elapsed();
            if elapsed < chunk_wall_time {
                tokio::time::sleep(chunk_wall_time - elapsed).await;
            }
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    params: Arc<SimParams>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
    terminated_events: Arc<tokio::sync::RwLock<Vec<String>>>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // 1. Send info message
    let satellites_info: Vec<SatelliteInfo> = params.satellites.iter().map(|s| {
        let system = build_orbital_system(&params.body, params.mu, params.epoch, s, params.atmosphere, params.f107, params.ap, params.space_weather_provider.as_ref());
        SatelliteInfo {
            id: s.id.clone(),
            name: s.name.clone(),
            altitude: s.altitude(&params.body),
            period: s.period,
            perturbations: system.perturbation_names().into_iter().map(String::from).collect(),
        }
    }).collect();
    let info = WsMessage::Info {
        mu: params.mu,
        dt: params.dt,
        output_interval: params.output_interval,
        stream_interval: params.stream_interval,
        central_body: serde_json::to_value(&params.body)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string(),
        central_body_radius: params.body.properties().radius,
        epoch_jd: params.epoch.map(|e| e.jd()),
        satellites: satellites_info,
    };
    let info_json = serde_json::to_string(&info).expect("failed to serialize info message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(info_json.into()))
        .await
        .is_err()
    {
        return;
    }

    // 1b. Replay termination events for satellites that terminated before this client connected
    {
        let terminated = terminated_events.read().await;
        for event_json in terminated.iter() {
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    event_json.clone().into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // 2. Send overview history (downsampled)
    let all_states = history.read().await.load_all();
    let overview = HistoryBuffer::downsample(&all_states, 1000);
    let history_msg = WsMessage::History { states: overview };
    let history_json =
        serde_json::to_string(&history_msg).expect("failed to serialize history message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(
            history_json.into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // 3. Spawn background detail sender
    let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(async move {
        let chunk_size = 1000;
        for chunk in all_states.chunks(chunk_size) {
            let msg = WsMessage::HistoryDetail {
                states: chunk.to_vec(),
            };
            let json = serde_json::to_string(&msg).expect("failed to serialize detail chunk");
            if detail_tx.send(json).await.is_err() {
                return; // Client disconnected
            }
        }
        let complete = serde_json::to_string(&WsMessage::HistoryDetailComplete)
            .expect("failed to serialize detail complete");
        let _ = detail_tx.send(complete).await;
    });

    // 4. Main loop: multiplex broadcast (real-time) + detail (background) + client messages
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_sender
                            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Client lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            detail = detail_rx.recv() => {
                if let Some(json) = detail
                    && ws_sender
                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                        .await
                        .is_err()
                {
                    break;
                }
                // None means detail sender finished — just continue with broadcast only
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, satellite_id } => {
                                    let mut states = history.read().await.query_range(t_min, t_max, max_points);
                                    if let Some(ref sid) = satellite_id {
                                        states.retain(|s| s.satellite_id == *sid);
                                    }
                                    let resp = WsMessage::QueryRangeResponse { t_min, t_max, states };
                                    let json = serde_json::to_string(&resp)
                                        .expect("failed to serialize query_range_response");
                                    if ws_sender
                                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        break;
                    }
                }
            }
        }
    }

    eprintln!("Client disconnected");
}
