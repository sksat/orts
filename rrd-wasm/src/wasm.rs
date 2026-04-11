//! WASM bindings for RRD decoding.

use wasm_bindgen::prelude::*;

/// Parse an RRD file from bytes and return the decoded data as a JS object.
///
/// The returned value has the shape `{ metadata: RrdMetadata, rows: RrdRow[] }`.
/// Keplerian elements are NOT computed here — use arika WASM for that.
#[wasm_bindgen]
pub fn parse_rrd(bytes: &[u8]) -> Result<JsValue, JsError> {
    let data =
        crate::decode_rrd(std::io::Cursor::new(bytes)).map_err(|e| JsError::new(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&data).map_err(|e| JsError::new(&e.to_string()))
}
