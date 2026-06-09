//! Backend-agnostic byte-stream buffers for the `stream-io` interface.
//!
//! `stream-io` lets a guest FSW speak a raw byte protocol (its own
//! framing — EB90 / C2A / CCSDS / custom) over named streams, for kble
//! virtual-harness integration. orts is a **dumb byte conduit**: it does
//! not interpret the bytes.
//!
//! This module holds the pure buffer logic, kept free of any wasmtime /
//! bindgen types so it can be unit-tested without a guest. Each WASM
//! backend embeds a [`Streams`] in its host state and maps the outcomes
//! below onto its own generated `stream-read` / `stream-error` WIT types.
//!
//! ## Determinism & tick boundaries
//!
//! Like `msg-io`, inbound bytes are frozen at the tick boundary (the host
//! appends a tick's deliveries before handing control to the guest) and
//! outbound bytes are flushed after the tick. Continuous-UART inter-byte
//! timing is explicitly out of scope; frame-unit protocols are the target.
//!
//! ## Bounded queues, overrun & faults
//!
//! Each direction is bounded. When a queue would exceed capacity the
//! bytes are **not silently dropped** (dropping hides frame corruption and
//! makes downstream analysis impossible): the stream latches `overrun`.
//! Overrun — and other host-side wiring inconsistencies (delivery to an
//! undeclared or already-closed stream) — also latch a **sticky fault** on
//! the whole [`Streams`] set. The host is authoritative for halting: the
//! controller surfaces [`Streams::fault`] as a `PluginError` so the
//! simulation stops even if the guest never touches the affected stream.
//! The guest separately sees `Err(overrun)` on its next `read`/`write` as
//! an immediate signal.

use std::collections::{HashMap, VecDeque};

/// Default per-direction byte capacity for a stream (1 MiB). Overrun is a
/// safety backstop for a stuck peer/guest, not an expected steady state.
/// The harness/config may override this per deployment in a later phase.
pub(super) const DEFAULT_STREAM_CAPACITY: usize = 1 << 20;

/// One tick's inbound delivery for a single stream (outer controller →
/// worker, carried in the tick packet). `closed` marks the peer as gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StreamDelivery {
    pub name: String,
    pub bytes: Vec<u8>,
    pub closed: bool,
}

/// Outcome of a guest `read`, mapped by each backend onto its WIT
/// `result<stream-read, stream-error>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReadOutcome {
    /// Bytes drained from the frozen inbound buffer (length > 0).
    Data(Vec<u8>),
    /// Inbound buffer empty this tick (or `max == 0`); peer still connected.
    NoData,
    /// Peer closed and the inbound buffer is drained — no more bytes.
    Closed,
    /// A queue overran; the stream is unusable. Sticky.
    Overrun,
    /// No such stream is configured.
    Unknown,
}

/// Outcome of a guest `write`, mapped by each backend onto its WIT
/// `result<_, stream-error>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WriteOutcome {
    Ok,
    Overrun,
    Closed,
    Unknown,
}

#[derive(Debug, Default)]
struct StreamState {
    inbound: VecDeque<u8>,
    outbound: Vec<u8>,
    /// Peer has disconnected; no more inbound bytes will arrive.
    closed: bool,
    /// A bounded queue overran. Sticky: once set, read/write report it.
    overrun: bool,
}

/// The set of named byte streams owned by one controller's host state.
#[derive(Debug)]
pub(super) struct Streams {
    map: HashMap<String, StreamState>,
    capacity: usize,
    /// Sticky host-authoritative fault (overrun / wiring inconsistency).
    /// First fault wins; the controller turns this into a `PluginError`
    /// to halt the simulation. `None` while healthy.
    fault: Option<String>,
}

impl Streams {
    /// Build a stream set with the given declared stream names. Streams
    /// must be declared up front (at controller construction) so that a
    /// guest `read`/`write` on a configured-but-idle stream resolves to
    /// `NoData` rather than `Unknown`.
    pub(super) fn new<I, S>(names: I, capacity: usize) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let map = names
            .into_iter()
            .map(|n| (n.into(), StreamState::default()))
            .collect();
        Self {
            map,
            capacity,
            fault: None,
        }
    }

    /// First sticky fault, if any. The host treats `Some(_)` as a reason
    /// to halt the simulation.
    pub(super) fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    fn set_fault(&mut self, msg: String) {
        if self.fault.is_none() {
            self.fault = Some(msg);
        }
    }

    /// Append host-delivered inbound bytes for a declared stream.
    ///
    /// Delivery to an **undeclared** or **already-closed** stream is a host
    /// wiring inconsistency and latches a fault (the host wires only
    /// configured, live streams). Exceeding capacity latches `overrun`
    /// rather than dropping bytes.
    pub(super) fn deliver(&mut self, name: &str, bytes: &[u8]) {
        let cap = self.capacity;
        let Some(s) = self.map.get_mut(name) else {
            self.set_fault(format!("stream-io: delivery to undeclared stream '{name}'"));
            return;
        };
        if s.closed {
            self.set_fault(format!("stream-io: delivery to closed stream '{name}'"));
            return;
        }
        if s.inbound.len() + bytes.len() > cap {
            s.overrun = true;
            self.set_fault(format!("stream-io: inbound overrun on stream '{name}'"));
            return;
        }
        s.inbound.extend(bytes.iter().copied());
    }

    /// Mark a declared stream's peer as closed. A close on an undeclared
    /// stream is ignored (idempotent teardown, not a fault).
    pub(super) fn close(&mut self, name: &str) {
        if let Some(s) = self.map.get_mut(name) {
            s.closed = true;
        }
    }

    /// Drain up to `max` bytes from a stream's frozen inbound buffer.
    /// `max == 0` is a no-op read (returns `NoData`); a guest must request
    /// `> 0` to observe `Data` / `Closed`.
    pub(super) fn read(&mut self, name: &str, max: usize) -> ReadOutcome {
        let Some(s) = self.map.get_mut(name) else {
            return ReadOutcome::Unknown;
        };
        if s.overrun {
            return ReadOutcome::Overrun;
        }
        if max == 0 {
            return ReadOutcome::NoData;
        }
        if !s.inbound.is_empty() {
            let n = max.min(s.inbound.len());
            return ReadOutcome::Data(s.inbound.drain(..n).collect());
        }
        if s.closed {
            return ReadOutcome::Closed;
        }
        ReadOutcome::NoData
    }

    /// Append guest-written bytes to a stream's outbound buffer (flushed
    /// at the next tick boundary). Exceeding capacity latches `overrun`
    /// (both a guest-visible error and a host fault).
    pub(super) fn write(&mut self, name: &str, bytes: &[u8]) -> WriteOutcome {
        let cap = self.capacity;
        let Some(s) = self.map.get_mut(name) else {
            return WriteOutcome::Unknown;
        };
        if s.overrun {
            return WriteOutcome::Overrun;
        }
        if s.closed {
            return WriteOutcome::Closed;
        }
        if s.outbound.len() + bytes.len() > cap {
            s.overrun = true;
            self.set_fault(format!("stream-io: outbound overrun on stream '{name}'"));
            return WriteOutcome::Overrun;
        }
        s.outbound.extend_from_slice(bytes);
        WriteOutcome::Ok
    }

    /// Drain every stream's pending outbound bytes (for flushing to the
    /// outer controller). Only streams with bytes are returned, ordered by
    /// stream name for determinism.
    pub(super) fn drain_outbound(&mut self) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = self
            .map
            .iter_mut()
            .filter(|(_, s)| !s.outbound.is_empty())
            .map(|(name, s)| (name.clone(), std::mem::take(&mut s.outbound)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streams(cap: usize, names: &[&str]) -> Streams {
        Streams::new(names.iter().map(|s| s.to_string()), cap)
    }

    #[test]
    fn read_write_unknown_stream() {
        let mut s = streams(64, &["c2a"]);
        assert_eq!(s.read("nope", 10), ReadOutcome::Unknown);
        assert_eq!(s.write("nope", b"x"), WriteOutcome::Unknown);
        // A guest probing an unknown stream is its own error, not a host fault.
        assert!(s.fault().is_none());
    }

    #[test]
    fn declared_but_empty_is_no_data_not_closed() {
        let mut s = streams(64, &["c2a"]);
        assert_eq!(s.read("c2a", 10), ReadOutcome::NoData);
    }

    #[test]
    fn deliver_then_read_drains_in_order() {
        let mut s = streams(64, &["c2a"]);
        s.deliver("c2a", &[1, 2, 3, 4]);
        assert_eq!(s.read("c2a", 2), ReadOutcome::Data(vec![1, 2]));
        assert_eq!(s.read("c2a", 10), ReadOutcome::Data(vec![3, 4]));
        assert_eq!(s.read("c2a", 10), ReadOutcome::NoData);
    }

    #[test]
    fn read_max_zero_is_no_op() {
        let mut s = streams(64, &["c2a"]);
        s.deliver("c2a", &[1, 2]);
        // max=0 must never yield empty `data` (WIT: data length > 0).
        assert_eq!(s.read("c2a", 0), ReadOutcome::NoData);
        // Bytes are still there for a real read.
        assert_eq!(s.read("c2a", 10), ReadOutcome::Data(vec![1, 2]));
    }

    #[test]
    fn closed_reports_remaining_bytes_then_closed() {
        let mut s = streams(64, &["c2a"]);
        s.deliver("c2a", &[9, 9]);
        s.close("c2a");
        assert_eq!(s.read("c2a", 10), ReadOutcome::Data(vec![9, 9]));
        assert_eq!(s.read("c2a", 10), ReadOutcome::Closed);
    }

    #[test]
    fn write_after_close_is_rejected() {
        let mut s = streams(64, &["c2a"]);
        s.close("c2a");
        assert_eq!(s.write("c2a", b"hi"), WriteOutcome::Closed);
        // guest writing after close is not a host fault.
        assert!(s.fault().is_none());
    }

    #[test]
    fn write_then_drain_outbound_sorted() {
        let mut s = streams(64, &["a", "b"]);
        assert_eq!(s.write("b", b"world"), WriteOutcome::Ok);
        assert_eq!(s.write("a", b"hello"), WriteOutcome::Ok);
        assert_eq!(
            s.drain_outbound(),
            vec![
                ("a".to_string(), b"hello".to_vec()),
                ("b".to_string(), b"world".to_vec()),
            ]
        );
        assert!(s.drain_outbound().is_empty());
    }

    #[test]
    fn inbound_overrun_latches_fault_and_does_not_drop() {
        let mut s = streams(4, &["c2a"]);
        s.deliver("c2a", &[1, 2, 3]);
        s.deliver("c2a", &[4, 5]); // would exceed cap=4 → overrun, no partial drop
        assert_eq!(s.read("c2a", 10), ReadOutcome::Overrun); // guest-visible
        assert_eq!(s.read("c2a", 10), ReadOutcome::Overrun); // sticky
        assert!(s.fault().is_some()); // host-authoritative halt signal
    }

    #[test]
    fn outbound_overrun_latches_fault() {
        let mut s = streams(4, &["c2a"]);
        assert_eq!(s.write("c2a", b"abcd"), WriteOutcome::Ok);
        assert_eq!(s.write("c2a", b"e"), WriteOutcome::Overrun);
        assert!(s.fault().is_some());
    }

    #[test]
    fn deliver_to_undeclared_stream_faults() {
        let mut s = streams(64, &["c2a"]);
        s.deliver("uart0", &[1]); // host wiring bug
        assert!(s.fault().is_some());
    }

    #[test]
    fn deliver_after_close_faults() {
        let mut s = streams(64, &["c2a"]);
        s.close("c2a");
        s.deliver("c2a", &[1]); // host said closed, then delivered more
        assert!(s.fault().is_some());
    }

    #[test]
    fn first_fault_wins() {
        let mut s = streams(4, &["c2a"]);
        s.deliver("c2a", &[1, 2, 3, 4, 5]); // inbound overrun
        let first = s.fault().map(str::to_string);
        s.deliver("nope", &[1]); // undeclared
        assert_eq!(s.fault().map(str::to_string), first); // unchanged
    }
}
