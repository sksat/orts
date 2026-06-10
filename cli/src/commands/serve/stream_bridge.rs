//! kble bridge: binary WebSocket endpoints for `stream-io` byte streams.
//!
//! Each declared stream of a controlled satellite is exposed at
//! `ws://…/stream/{sat}/{stream}` as a **binary** WebSocket — the shape of an
//! [`arkedge/kble`](https://github.com/arkedge/kble) `ws://` plug, so kble can
//! wire external tools (framers, ground software, serial bridges) straight to
//! the simulated FSW. orts stays a dumb byte conduit.
//!
//! ## Architecture
//!
//! ```text
//! WS reader task ──append──▶ staging buffer ──take_staged──▶ sim loop
//!                                                       (stream_deliver)
//! WS writer task ◀──mpsc──── push_outbound ◀──stream_take── sim loop
//! ```
//!
//! - **Directory vs data path**: the [`StreamBridge`] map (`RwLock`) is only
//!   for endpoint lookup; per-endpoint state lives behind a short-critical-
//!   section `Mutex` (no `.await` while locked).
//! - **Transient disconnect**: a WS peer dropping does *not* close the
//!   guest-side stream (`stream_close` is never called) — the stream just
//!   goes idle and a reconnect resumes it.
//! - **Last-wins with generation fencing**: a new connection replaces the
//!   active peer; the old connection's late inbound is rejected by
//!   generation mismatch (kble reconnects can race the old socket teardown).
//! - **No byte drops**: the inbound staging buffer is bounded; overflowing it
//!   latches a flag that halts the simulation (mirroring the host-side
//!   `stream-io` overrun contract). Outbound to a *connected but stuck* peer
//!   likewise halts; outbound with *no* peer is discarded (transient policy).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

/// Bound for bytes staged between WS arrival and the next sim tick. Matches
/// the per-direction stream capacity used by the guest-side buffers.
const STAGING_CAPACITY: usize = 1 << 20;

/// Chunks queued towards the WS writer task before the peer counts as stuck.
const PEER_QUEUE_CHUNKS: usize = 64;

/// Key: (satellite id, stream name).
pub type StreamKey = (String, String);

/// Registry shared between the axum handlers (lookup) and the sim loop
/// (registration + pumping).
#[derive(Default)]
pub struct StreamBridge {
    endpoints: RwLock<HashMap<StreamKey, Arc<StreamEndpoint>>>,
}

impl StreamBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install endpoints for the given keys, replacing the previous set.
    /// Old endpoints are marked defunct so their lingering WS tasks shut
    /// down instead of staging bytes nobody will drain (config reload).
    pub fn reset(&self, keys: impl IntoIterator<Item = StreamKey>) {
        let fresh: HashMap<StreamKey, Arc<StreamEndpoint>> = keys
            .into_iter()
            .map(|k| (k, Arc::new(StreamEndpoint::default())))
            .collect();
        let mut map = self.endpoints.write().expect("bridge lock poisoned");
        for ep in map.values() {
            ep.inner.lock().expect("endpoint lock poisoned").defunct = true;
        }
        *map = fresh;
    }

    /// Look up the endpoint for `(sat, stream)`, if declared.
    pub fn lookup(&self, sat: &str, stream: &str) -> Option<Arc<StreamEndpoint>> {
        self.endpoints
            .read()
            .expect("bridge lock poisoned")
            .get(&(sat.to_string(), stream.to_string()))
            .cloned()
    }
}

/// One `(sat, stream)` endpoint.
#[derive(Default)]
pub struct StreamEndpoint {
    inner: Mutex<EndpointInner>,
}

#[derive(Default)]
struct EndpointInner {
    /// Bumped on every `attach_peer`; inbound from older generations is
    /// rejected (fencing against a half-dead predecessor socket).
    generation: u64,
    /// Bytes received from the active peer, awaiting the next sim tick.
    staged: Vec<u8>,
    /// Staging exceeded [`STAGING_CAPACITY`]. Sticky; halts the sim.
    overflowed: bool,
    /// Endpoint belongs to a torn-down sim context (config reload).
    defunct: bool,
    /// Sender towards the active peer's WS writer task.
    peer_tx: Option<mpsc::Sender<Vec<u8>>>,
}

/// Result of staging inbound bytes from a WS reader task.
#[derive(Debug, PartialEq, Eq)]
pub enum InboundPush {
    Ok,
    /// A newer connection took over; the caller should close its socket.
    StaleGeneration,
    /// The sim context owning this endpoint is gone; close the socket.
    Defunct,
}

/// Result of forwarding sim-produced bytes towards the active peer.
#[derive(Debug, PartialEq, Eq)]
pub enum OutboundPush {
    Sent,
    /// No connected peer — bytes discarded (transient-disconnect policy).
    NoPeer,
    /// Peer connected but its queue is full (not draining). The sim must
    /// halt rather than drop or buffer without bound.
    Stuck,
}

impl StreamEndpoint {
    /// Register a new active peer (last-wins). Returns the connection's
    /// generation token and the receiver feeding its WS writer task.
    /// Replacing the previous sender ends the old writer task's `recv()`.
    pub fn attach_peer(&self) -> (u64, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel(PEER_QUEUE_CHUNKS);
        let mut inner = self.inner.lock().expect("endpoint lock poisoned");
        inner.generation += 1;
        inner.peer_tx = Some(tx);
        (inner.generation, rx)
    }

    /// Stage bytes from the WS reader task with generation `generation`.
    pub fn push_inbound(&self, generation: u64, bytes: &[u8]) -> InboundPush {
        let mut inner = self.inner.lock().expect("endpoint lock poisoned");
        if inner.defunct {
            return InboundPush::Defunct;
        }
        if generation != inner.generation {
            return InboundPush::StaleGeneration;
        }
        if inner.staged.len() + bytes.len() > STAGING_CAPACITY {
            // Don't drop bytes — latch overflow; the sim halts on next pump.
            inner.overflowed = true;
        } else {
            inner.staged.extend_from_slice(bytes);
        }
        InboundPush::Ok
    }

    /// Drain everything staged since the last tick. `overflowed == true`
    /// means bytes were lost to the bound — the caller must halt the sim.
    pub fn take_staged(&self) -> (Vec<u8>, bool) {
        let mut inner = self.inner.lock().expect("endpoint lock poisoned");
        (std::mem::take(&mut inner.staged), inner.overflowed)
    }

    /// Forward FSW-produced bytes to the active peer (non-blocking; called
    /// from the sync sim loop).
    pub fn push_outbound(&self, bytes: Vec<u8>) -> OutboundPush {
        let mut inner = self.inner.lock().expect("endpoint lock poisoned");
        let Some(tx) = inner.peer_tx.as_ref() else {
            return OutboundPush::NoPeer;
        };
        match tx.try_send(bytes) {
            Ok(()) => OutboundPush::Sent,
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Writer task ended (peer disconnected): transient — drop the
                // sender and discard, a reconnect will re-attach.
                inner.peer_tx = None;
                OutboundPush::NoPeer
            }
            Err(mpsc::error::TrySendError::Full(_)) => OutboundPush::Stuck,
        }
    }
}

/// Drive one WS connection for `(sat, stream)`: socket → staging (inbound)
/// and writer queue → socket (outbound), until the peer disconnects, a newer
/// connection takes over, or the owning sim context is torn down.
pub async fn handle_stream_socket(
    socket: WebSocket,
    endpoint: Arc<StreamEndpoint>,
    sat: String,
    stream: String,
) {
    let (generation, mut out_rx) = endpoint.attach_peer();
    log::info!("stream-io peer connected: {sat}/{stream} (generation {generation})");
    let (mut ws_tx, mut ws_rx) = socket.split();

    loop {
        tokio::select! {
            msg = ws_rx.next() => match msg {
                Some(Ok(Message::Binary(bytes))) => {
                    match endpoint.push_inbound(generation, &bytes) {
                        InboundPush::Ok => {}
                        InboundPush::StaleGeneration => {
                            log::info!(
                                "stream-io peer superseded: {sat}/{stream} (generation {generation})"
                            );
                            break;
                        }
                        InboundPush::Defunct => break,
                    }
                }
                // kble plugs speak binary only; ignore text. Ping/pong is
                // handled by axum automatically.
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    log::warn!("stream-io socket error on {sat}/{stream}: {e}");
                    break;
                }
            },
            out = out_rx.recv() => match out {
                Some(bytes) => {
                    if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                // Sender replaced (newer peer) or endpoint dropped.
                None => break,
            },
        }
    }
    log::info!("stream-io peer disconnected: {sat}/{stream} (generation {generation})");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_only_registered_keys() {
        let bridge = StreamBridge::new();
        bridge.reset(vec![("sat0".into(), "comlink".into())]);
        assert!(bridge.lookup("sat0", "comlink").is_some());
        assert!(bridge.lookup("sat0", "uart0").is_none());
        assert!(bridge.lookup("sat1", "comlink").is_none());
    }

    #[test]
    fn inbound_stages_and_drains() {
        let ep = StreamEndpoint::default();
        let (generation, _rx) = ep.attach_peer();
        assert_eq!(ep.push_inbound(generation, &[1, 2]), InboundPush::Ok);
        assert_eq!(ep.push_inbound(generation, &[3]), InboundPush::Ok);
        assert_eq!(ep.take_staged(), (vec![1, 2, 3], false));
        assert_eq!(ep.take_staged(), (vec![], false));
    }

    #[test]
    fn staging_overflow_latches_without_dropping_silently() {
        let ep = StreamEndpoint::default();
        let (generation, _rx) = ep.attach_peer();
        ep.push_inbound(generation, &vec![0u8; STAGING_CAPACITY]);
        ep.push_inbound(generation, &[1]); // would exceed → latch
        let (bytes, overflowed) = ep.take_staged();
        assert_eq!(bytes.len(), STAGING_CAPACITY);
        assert!(overflowed, "overflow must be reported, not silent");
    }

    #[test]
    fn new_peer_fences_out_the_old_one() {
        let ep = StreamEndpoint::default();
        let (old_generation, mut old_rx) = ep.attach_peer();
        let (new_generation, _new_rx) = ep.attach_peer();
        assert_ne!(old_generation, new_generation);
        // Old connection's late inbound is rejected.
        assert_eq!(
            ep.push_inbound(old_generation, &[9]),
            InboundPush::StaleGeneration
        );
        // New connection works.
        assert_eq!(ep.push_inbound(new_generation, &[1]), InboundPush::Ok);
        // The old writer's channel is closed (sender replaced + dropped).
        assert!(matches!(
            old_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn outbound_without_peer_is_discarded() {
        let ep = StreamEndpoint::default();
        assert_eq!(ep.push_outbound(vec![1, 2]), OutboundPush::NoPeer);
    }

    #[test]
    fn outbound_to_disconnected_peer_downgrades_to_no_peer() {
        let ep = StreamEndpoint::default();
        let (_generation, rx) = ep.attach_peer();
        drop(rx); // peer writer task ended
        assert_eq!(ep.push_outbound(vec![1]), OutboundPush::NoPeer);
        assert_eq!(ep.push_outbound(vec![2]), OutboundPush::NoPeer);
    }

    #[test]
    fn outbound_to_stuck_peer_reports_stuck() {
        let ep = StreamEndpoint::default();
        let (_generation, _rx) = ep.attach_peer();
        // Fill the writer queue without draining it.
        let mut last = OutboundPush::Sent;
        for _ in 0..=PEER_QUEUE_CHUNKS {
            last = ep.push_outbound(vec![0]);
        }
        assert_eq!(last, OutboundPush::Stuck);
    }

    #[test]
    fn reset_marks_old_endpoints_defunct() {
        let bridge = StreamBridge::new();
        bridge.reset(vec![("sat0".into(), "comlink".into())]);
        let ep = bridge.lookup("sat0", "comlink").unwrap();
        let (generation, _rx) = ep.attach_peer();
        bridge.reset(vec![("sat0".into(), "comlink".into())]);
        // Old Arc still held by a lingering WS task → must report defunct.
        assert_eq!(ep.push_inbound(generation, &[1]), InboundPush::Defunct);
        // The fresh endpoint is a different object and works.
        let fresh = bridge.lookup("sat0", "comlink").unwrap();
        let (g2, _rx2) = fresh.attach_peer();
        assert_eq!(fresh.push_inbound(g2, &[1]), InboundPush::Ok);
    }
}
