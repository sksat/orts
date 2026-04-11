# rrd-wasm

WebAssembly-friendly Rerun RRD decoder for streaming simulation data into
browser viewers.

Wraps the decoder portion of the [Rerun](https://rerun.io) SDK
(`re_log_encoding`, `re_chunk`, `re_log_types`, `re_sdk_types`) with
`wasm-bindgen` so a browser-side TypeScript viewer can parse `.rrd` byte
streams without shelling out to the native Rerun Viewer.

Built as a companion to [orts](https://github.com/sksat/orts) but usable by
any browser tool that needs to read Rerun recordings.

