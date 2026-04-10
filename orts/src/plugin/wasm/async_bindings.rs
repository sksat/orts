//! Async host-side bindings for `wit/v0/orts.wit`.
//!
//! Parallel to [`sync_bindings`](super::sync_bindings), but with
//! `imports: { default: async }` / `exports: { default: async }` so
//! that host imports (`wait-tick`, `send-command`, `log`, ...) are
//! `async fn`s and the guest export `run` returns a `Future`. This is
//! required by the fiber-based async backend where a single OS
//! thread multiplexes many satellite controllers.
//!
//! Types generated here are **not binary-compatible** with the sync
//! `bindings.rs` output: they live in a separate module tree and each
//! side has its own `convert::{sync,async}` submodule that bridges
//! between the plugin-layer Rust types and the WIT records.

wasmtime::component::bindgen!({
    path: "wit/v0/orts.wit",
    world: "plugin",
    imports: { default: async },
    exports: { default: async },
});
