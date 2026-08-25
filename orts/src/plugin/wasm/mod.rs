//! WASM backend for the plugin layer (`plugin-wasm` feature).
//!
//! This module loads guest controllers written as WebAssembly
//! [Components][wasm-component-model] and exposes them through the
//! `PluginController` trait. It is gated behind the `plugin-wasm`
//! cargo feature because the `wasmtime` dependency is heavy; callers
//! that never need WASM plugins pay no binary-size or build-time cost.
//!
//! The wasmtime build is configured for a **Pulley single-backend**
//! execution path (pure-Rust interpreter, no JIT codegen at runtime).
//! Cranelift is still linked because it is needed on the *host* side
//! to compile wasm bytes into Pulley bytecode — see `engine.rs` for
//! the `Config::target("pulley64")` setup and DESIGN.md "WASM backend"
//! for the rationale (a runtime-only feature without Cranelift is
//! planned; see ROADMAP.md).
//!
//! [wasm-component-model]: https://component-model.bytecodealliance.org/

pub mod cache;
pub mod convert;
pub mod engine;
pub mod sync_bindings;
pub mod sync_controller;
pub mod sync_host_state;

/// Backend-agnostic byte-stream buffers shared by the sync and async
/// `stream-io` host implementations.
mod stream_state;

/// Backwards-compatible alias for the sync bindgen output. The async
/// backend (feature `plugin-wasm-async`) brings its own bindings in
/// [`async_bindings`].
pub use sync_bindings as bindings;
/// Backwards-compatible alias. The struct is still named
/// `WasmController` — only the module file was renamed.
pub use sync_controller as controller;
/// Backwards-compatible alias for the host-state module.
pub use sync_host_state as host_state;

pub use cache::WasmPluginCache;
pub use engine::WasmEngine;
pub use sync_controller::WasmController;

#[cfg(feature = "plugin-wasm-async")]
pub mod async_bindings;
#[cfg(feature = "plugin-wasm-async")]
pub mod async_controller;
#[cfg(feature = "plugin-wasm-async")]
pub mod async_host_state;
#[cfg(feature = "plugin-wasm-async")]
pub mod async_runtime;

#[cfg(feature = "plugin-wasm-async")]
pub use async_controller::{AsyncPluginPreBuilt, AsyncWasmController};
#[cfg(feature = "plugin-wasm-async")]
pub use async_runtime::{AsyncMode, AsyncRuntime};
