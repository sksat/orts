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

/// How many characters of a parse error go back to the client.
///
/// 200 leaves room for the whole of every error this server produces from a
/// message of its own shape.
const ERROR_DETAIL_LIMIT: usize = 200;

/// A parse error, cut to something bounded.
///
/// Serde names the field it could not read, and over `/ws` that name is the
/// client's own text. Measured: a 100 KB unknown key inside an orbit block
/// yields a 100 KB error, which would then be serialized and sent back — the
/// inbound limit turned into an outbound allocation. The head of the message
/// says which field and what was expected, which is the part anyone reads.
///
/// The position is appended when serde has one. On this path it does not: the
/// error comes from replaying a buffered `type`-tagged block, so `line` and
/// `column` are both 0 (measured), and a top-level syntax error carries its
/// position in the text already, well inside the limit.
fn client_error_detail(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let Some((cut, _)) = text.char_indices().nth(ERROR_DETAIL_LIMIT) else {
        return text;
    };
    let mut out = text[..cut].to_string();
    out.push('…');
    if e.line() != 0 {
        out.push_str(&format!(" (line {}, column {})", e.line(), e.column()));
    }
    out
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
                        // A key nothing reads is named on the server's stderr
                        // and the message still runs, the policy a config file
                        // gets: a client built against a newer `orts` keeps
                        // working here, and an operator can see what was
                        // dropped. `start_simulation` carries a whole config, so
                        // a typo there would otherwise be silent.
                        //
                        // The message is read from the text, not from a
                        // `serde_json::Value`: a `Value`'s map keeps the last of
                        // two members with one name, so
                        // `{…,"config":{"dt":1},"config":{"dt":99}}` would run
                        // with 99 and no word about it, where serde refuses a
                        // duplicate field outright. The tree for the warning
                        // pass is then built only for a message that was read,
                        // and `MAX_CONTROL_MESSAGE_BYTES` bounds what that costs.
                        let parsed = serde_json::from_str::<ClientMessage>(&text);
                        if parsed.is_ok()
                            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                        {
                            {
                                let unread = crate::config::unread_client_message_keys(&value);
                                for key in &unread.named {
                                    log::warn!(
                                        "client message: nothing reads `{}`; its value is ignored",
                                        crate::config::printable_key(key)
                                    );
                                }
                                if unread.unnamed > 0 {
                                    log::warn!(
                                        "client message: and {} more keys nothing reads",
                                        unread.unnamed
                                    );
                                }
                            }
                        }
                        // A message this server cannot read is answered, not
                        // dropped. A `type`-tagged block refuses an unknown key
                        // (nothing can report one there), and dropping the error
                        // left the client waiting on a reply that never came.
                        if let Err(e) = &parsed {
                            let json = serde_json::to_string(&WsMessage::Error {
                                message: format!(
                                    "could not read the message: {}",
                                    client_error_detail(e)
                                ),
                            })
                            .expect("failed to serialize error");
                            if ws_sender.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        if let Ok(client_msg) = parsed {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The error a client gets back is bounded, however long its key was.
    ///
    /// Serde puts the field name in the message, and over `/ws` that name is
    /// whatever the client sent. Measured before this was cut: a 100 KB unknown
    /// key inside an orbit block produced a 100 KB error, which the server then
    /// serialized and sent back.
    #[test]
    fn a_client_error_is_bounded_however_long_the_key() {
        let key = "z".repeat(100_000);
        let msg = format!(
            "{{\"type\":\"start_simulation\",\"config\":{{\"satellites\":[{{\"id\":\"a\",\
             \"orbit\":{{\"type\":\"circular\",\"altitude\":400,\"{key}\":1}}}}]}}}}"
        );
        let e = serde_json::from_str::<ClientMessage>(&msg)
            .err()
            .expect("an unknown key in a `type`-tagged block is refused");
        assert!(
            e.to_string().len() > 100_000,
            "the raw error carries the whole key, which is what needs cutting"
        );

        let detail = client_error_detail(&e);
        assert!(
            detail.chars().count() <= ERROR_DETAIL_LIMIT + 40,
            "bounded: {} chars",
            detail.chars().count()
        );
        assert!(
            detail.starts_with("unknown field"),
            "the head still says what went wrong: {detail:.60}"
        );
        assert!(detail.ends_with('…'), "and says it was cut: {detail:.60}");
    }

    /// An error short enough to fit comes through whole, position and all.
    #[test]
    fn a_short_client_error_keeps_its_position() {
        let e = serde_json::from_str::<ClientMessage>("{\"type\":\"query_range\",")
            .err()
            .expect("truncated JSON is refused");
        let detail = client_error_detail(&e);
        assert_eq!(detail, e.to_string());
        assert!(detail.contains("line 1"), "serde's position: {detail}");
    }
}
