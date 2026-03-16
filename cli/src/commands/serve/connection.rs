use std::ops::ControlFlow;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::history::HistoryBuffer;
use super::manager::{SimCommand, SimStatusResponse};
use super::protocol::{ClientMessage, WsMessage};

pub(super) type WsSender = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    tokio_tungstenite::tungstenite::Message,
>;
pub(super) type WsReceiver =
    futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>>;

pub(super) async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver): (WsSender, WsReceiver) = ws_stream.split();

    // 1. Query current status from the manager
    let (status_tx, status_rx) = oneshot::channel();
    if cmd_tx
        .send(SimCommand::GetStatus { respond: status_tx })
        .await
        .is_err()
    {
        return;
    }
    let status = match status_rx.await {
        Ok(s) => s,
        Err(_) => return,
    };

    let is_paused = matches!(status, SimStatusResponse::Paused { .. });

    match status {
        SimStatusResponse::Idle => {
            let idle_msg = serde_json::to_string(&WsMessage::Status {
                state: "idle".to_string(),
            })
            .expect("failed to serialize status");
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    idle_msg.into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
        SimStatusResponse::Running {
            info_json,
            terminated_events,
            history_states,
        }
        | SimStatusResponse::Paused {
            info_json,
            terminated_events,
            history_states,
        } => {
            // Send info
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    info_json.into(),
                ))
                .await
                .is_err()
            {
                return;
            }

            // If paused, send status so the client knows immediately
            if is_paused {
                let paused_msg = serde_json::to_string(&WsMessage::Status {
                    state: "paused".to_string(),
                })
                .expect("failed to serialize status");
                if ws_sender
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        paused_msg.into(),
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // Replay terminated events
            for event_json in &terminated_events {
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

            // Send overview history
            let overview = HistoryBuffer::downsample(&history_states, 1000);
            let history_msg = WsMessage::History { states: overview };
            let history_json =
                serde_json::to_string(&history_msg).expect("failed to serialize history");
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    history_json.into(),
                ))
                .await
                .is_err()
            {
                return;
            }

            // Send full detail in background
            let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<String>(16);
            tokio::spawn(async move {
                let chunk_size = 1000;
                for chunk in history_states.chunks(chunk_size) {
                    let msg = WsMessage::HistoryDetail {
                        states: chunk.to_vec(),
                    };
                    let json =
                        serde_json::to_string(&msg).expect("failed to serialize detail chunk");
                    if detail_tx.send(json).await.is_err() {
                        return;
                    }
                }
                let complete = serde_json::to_string(&WsMessage::HistoryDetailComplete)
                    .expect("failed to serialize detail complete");
                let _ = detail_tx.send(complete).await;
            });

            main_loop(
                &mut ws_sender,
                &mut ws_receiver,
                &mut rx,
                &cmd_tx,
                Some(&mut detail_rx),
            )
            .await;
            eprintln!("Client disconnected");
            return;
        }
    }

    // Idle client: main loop (waiting for start_simulation or other messages)
    main_loop(&mut ws_sender, &mut ws_receiver, &mut rx, &cmd_tx, None).await;
    eprintln!("Client disconnected");
}

/// Send a command to the simulation manager, await the response, and send
/// an error message back to the client if the command failed.
/// Returns `ControlFlow::Break(())` if the connection should be closed.
async fn dispatch_command<T>(
    cmd_tx: &mpsc::Sender<SimCommand>,
    ws_sender: &mut WsSender,
    make_cmd: impl FnOnce(oneshot::Sender<Result<T, String>>) -> SimCommand,
) -> ControlFlow<()> {
    let (resp_tx, resp_rx) = oneshot::channel();
    if cmd_tx.send(make_cmd(resp_tx)).await.is_err() {
        return ControlFlow::Break(());
    }
    match resp_rx.await {
        Ok(Ok(_)) => ControlFlow::Continue(()),
        Ok(Err(e)) => {
            let err_msg = serde_json::to_string(&WsMessage::Error { message: e })
                .expect("failed to serialize error");
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    err_msg.into(),
                ))
                .await
                .is_err()
            {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
        Err(_) => ControlFlow::Break(()),
    }
}

async fn main_loop(
    ws_sender: &mut WsSender,
    ws_receiver: &mut WsReceiver,
    rx: &mut broadcast::Receiver<String>,
    cmd_tx: &mpsc::Sender<SimCommand>,
    mut detail_rx: Option<&mut tokio::sync::mpsc::Receiver<String>>,
) {
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
            detail = async {
                if let Some(ref mut drx) = detail_rx {
                    drx.recv().await
                } else {
                    std::future::pending::<Option<String>>().await
                }
            } => {
                if let Some(json) = detail {
                    if ws_sender
                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                } else {
                    // Detail sender finished
                    detail_rx = None;
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            let result = match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, satellite_id } => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::QueryRange {
                                        t_min, t_max, max_points, satellite_id, respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    if let Ok(states) = resp_rx.await {
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
                                    ControlFlow::Continue(())
                                }
                                ClientMessage::StartSimulation { config } => {
                                    dispatch_command(cmd_tx, ws_sender, |respond| {
                                        SimCommand::Start { config, respond }
                                    }).await
                                }
                                ClientMessage::PauseSimulation => {
                                    dispatch_command(cmd_tx, ws_sender, |respond| {
                                        SimCommand::Pause { respond }
                                    }).await
                                }
                                ClientMessage::ResumeSimulation => {
                                    dispatch_command(cmd_tx, ws_sender, |respond| {
                                        SimCommand::Resume { respond }
                                    }).await
                                }
                                ClientMessage::TerminateSimulation => {
                                    dispatch_command(cmd_tx, ws_sender, |respond| {
                                        SimCommand::Terminate { respond }
                                    }).await
                                }
                                ClientMessage::AddSatellite { satellite } => {
                                    dispatch_command(cmd_tx, ws_sender, |respond| {
                                        SimCommand::AddSatellite { satellite, respond }
                                    }).await
                                }
                            };
                            if result.is_break() {
                                break;
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
}
