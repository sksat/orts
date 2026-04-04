use std::ops::ControlFlow;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::manager::{SimCommand, SimStatusResponse};
use super::protocol::{ClientMessage, WsMessage};

type WsSender = futures_util::stream::SplitSink<WebSocket, Message>;
type WsReceiver = futures_util::stream::SplitStream<WebSocket>;

pub(super) async fn handle_connection(
    socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
) {
    let (mut ws_sender, mut ws_receiver): (WsSender, WsReceiver) = socket.split();

    // 1. Query current status from the manager. The manager returns a
    //    bounded downsampled history overview regardless of how long the
    //    simulation has been running; any windowed detail is the client's
    //    concern via subsequent `query_range` requests.
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
                .send(Message::Text(idle_msg.into()))
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
                .send(Message::Text(info_json.into()))
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
                    .send(Message::Text(paused_msg.into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // Replay terminated events
            for event_json in &terminated_events {
                if ws_sender
                    .send(Message::Text(event_json.clone().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // Send the bounded history overview. The manager returned an
            // incrementally-maintained, downsampled summary of the full
            // history from memory (no disk I/O, no time-range parameter)
            // — clients that want higher resolution for a specific
            // display window issue follow-up `query_range` requests.
            let history_msg = WsMessage::History {
                states: history_states,
            };
            let history_json =
                serde_json::to_string(&history_msg).expect("failed to serialize history");
            if ws_sender
                .send(Message::Text(history_json.into()))
                .await
                .is_err()
            {
                return;
            }

            main_loop(&mut ws_sender, &mut ws_receiver, &mut rx, &cmd_tx).await;
            return;
        }
    }

    // Idle client: main loop (waiting for start_simulation or other messages)
    main_loop(&mut ws_sender, &mut ws_receiver, &mut rx, &cmd_tx).await;
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
            if ws_sender.send(Message::Text(err_msg.into())).await.is_err() {
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
) {
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_sender
                            .send(Message::Text(text.into()))
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
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            let result = match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, entity_path } => {
                                    let (resp_tx, resp_rx) = oneshot::channel();
                                    if cmd_tx.send(SimCommand::QueryRange {
                                        t_min, t_max, max_points, entity_path, respond: resp_tx,
                                    }).await.is_err() {
                                        break;
                                    }
                                    if let Ok(states) = resp_rx.await {
                                        let resp = WsMessage::QueryRangeResponse { t_min, t_max, states };
                                        let json = serde_json::to_string(&resp)
                                            .expect("failed to serialize query_range_response");
                                        if ws_sender
                                            .send(Message::Text(json.into()))
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
