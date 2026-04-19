# orts-plugin-sdk

SDK for writing orts WASM plugin guests (callback and main-loop styles).

Provides WIT bindings, procedural macros, and helpers for building
WASM Component Model plugins that expose attitude / orbital controllers
to the [orts](https://github.com/sksat/orts) simulation host runtime.

Bindings are generated at compile time via `wit_bindgen::generate!()` and
re-exported as `orts_plugin_sdk::bindings`. Plugin crates use these
directly — no per-crate binding generation or `cargo-component` metadata
is required.

## Usage

See the example plugin crates under [`examples/`](examples/) for concrete usage:

- `orts-example-plugin-bdot-finite-diff`
- `orts-example-plugin-pd-rw-control`
- `orts-example-plugin-pd-rw-unloading`
- `orts-example-plugin-detumble-nadir`

