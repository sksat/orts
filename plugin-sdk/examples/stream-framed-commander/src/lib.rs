//! Framed command/telemetry over a raw **byte stream** (`stream-io`).
//!
//! Unlike the `msg-io` commandable-mode examples — where the host delivers
//! whole typed *packets* — this FSW speaks its **own wire framing** over a
//! dumb byte conduit, exactly as a real FSW would when wired into the
//! [`kble`](https://github.com/arkedge/kble) virtual harness. orts does not
//! interpret the bytes; this guest owns sync-word search, length parsing,
//! reassembly, and CRC.
//!
//! ## Frame
//!
//! ```text
//! +--------+--------+-----------------+--------+
//! | SYNC   | LEN    | PAYLOAD         | CRC16  |
//! | 0xEB90 | u16 BE | LEN bytes       | u16 BE |
//! +--------+--------+-----------------+--------+
//!            \__________ CRC16-CCITT ________/
//! ```
//!
//! The CRC covers `LEN ++ PAYLOAD`. The payload is an ASCII mode name
//! (`detumble` / `nadir`).
//!
//! ## Why this is a *stream* example, not a packet one
//!
//! A `read` returns only the bytes that arrived this tick, so a single
//! frame routinely **spans multiple ticks** (and multiple frames can arrive
//! in one read). The FSW keeps a reassembly buffer across ticks, extracts
//! whole frames, and resyncs on a bad CRC — the byte-stream realities that
//! `msg-io`'s packet model hides. This is the value byte-stream support
//! adds: integrating an FSW that talks a real framed protocol.
//!
//! On a valid command the FSW applies the mode (gated: `nadir` only once
//! settled, like the msg-io examples) and writes a framed reply carrying
//! the resulting mode — a request/response exchange over one bidirectional
//! stream.

use orts_plugin_sdk::bindings::orts::plugin::types::{Command, TickInput};
use orts_plugin_sdk::stream::{self, StreamRead};
use orts_plugin_sdk::{Plugin, orts_plugin};

/// The single bidirectional comlink stream this FSW is wired to. The host
/// maps this local name to an external endpoint (kble `ws://.../{sat}/comlink`).
const STREAM: &str = "comlink";

/// Frame sync word (EB90, as used by `kble-eb90`).
const SYNC: [u8; 2] = [0xEB, 0x90];

/// Max bytes to pull from the stream per tick.
const READ_CHUNK: u32 = 1024;

/// Largest payload this FSW will accept. A desync onto a false sync word can
/// parse a bogus `LEN` (e.g. 0xFFFF); without a cap the reassembly buffer
/// would grow unbounded waiting for a frame that never completes. On an
/// oversized length we resync (drop the sync word) instead.
const MAX_PAYLOAD: usize = 4096;

/// `nadir` acceptance gate \[rad/s\] — same threshold as the msg-io examples.
const NADIR_RATE_GATE_RAD_S: f64 = 0.05;

struct Controller {
    sample_period: f64,
    mode: String,
    /// Bytes received but not yet consumed into a complete frame. Carries
    /// partial frames across ticks.
    rxbuf: Vec<u8>,
}

impl Plugin<TickInput, Command> for Controller {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(_config: &str) -> Result<Self, String> {
        Ok(Self {
            sample_period: 1.0,
            mode: "detumble".to_string(),
            rxbuf: Vec::new(),
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        // 1) Pull this tick's bytes onto the reassembly buffer. An
        //    `overrun` / `unknown-stream` error is fatal — propagate it so
        //    the host halts the simulation rather than masking lost framing.
        match stream::read(STREAM, READ_CHUNK) {
            Ok(StreamRead::Data(bytes)) => self.rxbuf.extend_from_slice(&bytes),
            Ok(StreamRead::NoData) | Ok(StreamRead::Closed) => {}
            Err(e) => return Err(format!("comlink read failed: {e:?}")),
        }

        // 2) Extract every complete frame currently buffered. Partial
        //    frames stay in `rxbuf` for a later tick.
        while let Some(payload) = self.next_frame() {
            // Payload is an ASCII mode name. Apply it (gated), then reply
            // with a frame carrying the resulting mode.
            if let Ok(target) = core::str::from_utf8(&payload)
                && can_enter(target, input)
            {
                self.mode = target.to_string();
            }
            let reply = build_frame(self.mode.as_bytes());
            if let Err(e) = stream::write(STREAM, &reply) {
                return Err(format!("comlink write failed: {e:?}"));
            }
        }

        // Attitude control is not the point of this example (ZOH hold).
        Ok(None)
    }

    fn current_mode(&self) -> Option<&str> {
        Some(&self.mode)
    }
}

impl Controller {
    /// Pop the next complete frame's payload from `rxbuf`, or `None` if a
    /// full frame is not yet buffered. Drops leading garbage and resyncs
    /// past a frame whose CRC fails.
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            // Find the sync word; discard anything before it.
            match find_sync(&self.rxbuf) {
                Some(0) => {}
                Some(pos) => {
                    self.rxbuf.drain(..pos);
                }
                None => {
                    // No sync yet. Keep a trailing 0xEB in case the sync
                    // word is split across the tick boundary; drop the rest.
                    let keep = self.rxbuf.last() == Some(&SYNC[0]);
                    self.rxbuf.clear();
                    if keep {
                        self.rxbuf.push(SYNC[0]);
                    }
                    return None;
                }
            }
            // Need SYNC(2) + LEN(2) before we know the frame length.
            if self.rxbuf.len() < 4 {
                return None;
            }
            let len = u16::from_be_bytes([self.rxbuf[2], self.rxbuf[3]]) as usize;
            if len > MAX_PAYLOAD {
                // Implausible length — we're desynced. Drop the sync word and
                // resync rather than buffer toward a frame that never arrives.
                self.rxbuf.drain(..2);
                continue;
            }
            let frame_len = 4 + len + 2;
            if self.rxbuf.len() < frame_len {
                // Partial frame — wait for the rest on a later tick.
                return None;
            }
            let crc_calc = crc16_ccitt(&self.rxbuf[2..4 + len]);
            let crc_rx = u16::from_be_bytes([self.rxbuf[4 + len], self.rxbuf[5 + len]]);
            if crc_calc == crc_rx {
                let payload = self.rxbuf[4..4 + len].to_vec();
                self.rxbuf.drain(..frame_len);
                return Some(payload);
            }
            // Bad CRC: drop the sync word and resync from the next byte.
            self.rxbuf.drain(..2);
        }
    }
}

/// Index of the first `SYNC` (0xEB90) occurrence in `buf`.
fn find_sync(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == SYNC)
}

/// Build a framed message: `SYNC | LEN | payload | CRC16(LEN ++ payload)`.
fn build_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut body = Vec::with_capacity(2 + payload.len());
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(payload);
    let crc = crc16_ccitt(&body);

    let mut frame = Vec::with_capacity(2 + body.len() + 2);
    frame.extend_from_slice(&SYNC);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, MSB-first, no final xor).
fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// May the FSW enter `target`? `nadir` only once settled (|ω| < gate);
/// `detumble` always; unknown modes never.
fn can_enter(target: &str, input: &TickInput) -> bool {
    match target {
        "detumble" => true,
        "nadir" => settled(input),
        _ => false,
    }
}

fn settled(input: &TickInput) -> bool {
    match input.sensors.gyroscopes.first() {
        Some(g) => {
            let w2 = g.x * g.x + g.y * g.y + g.z * g.z;
            w2 < NADIR_RATE_GATE_RAD_S * NADIR_RATE_GATE_RAD_S
        }
        None => false,
    }
}

orts_plugin!(Controller, mode);
