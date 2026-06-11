//! Host-side mirror of the `stream-framed-commander` guest's wire format,
//! shared by the stream-bridge E2E tests (`SYNC|LEN|PAYLOAD|CRC16`, CRC over
//! `LEN ++ PAYLOAD`).

pub const SYNC: [u8; 2] = [0xEB, 0x90];

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, MSB-first, no final xor).
pub fn crc16_ccitt(bytes: &[u8]) -> u16 {
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

pub fn build_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut body = Vec::new();
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(payload);
    let crc = crc16_ccitt(&body);
    let mut frame = Vec::new();
    frame.extend_from_slice(&SYNC);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// Parse the first complete frame in `bytes`, returning its payload if the
/// CRC checks out.
pub fn parse_frame(bytes: &[u8]) -> Option<Vec<u8>> {
    let pos = bytes.windows(2).position(|w| w == SYNC)?;
    let rest = &bytes[pos..];
    if rest.len() < 4 {
        return None;
    }
    let len = u16::from_be_bytes([rest[2], rest[3]]) as usize;
    if rest.len() < 4 + len + 2 {
        return None;
    }
    let crc_calc = crc16_ccitt(&rest[2..4 + len]);
    let crc_rx = u16::from_be_bytes([rest[4 + len], rest[5 + len]]);
    (crc_calc == crc_rx).then(|| rest[4..4 + len].to_vec())
}
