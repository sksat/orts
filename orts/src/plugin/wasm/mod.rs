//! WASM backend for the plugin layer (Phase P1, `plugin-wasm` feature).
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
//! the `Config::target("pulley64")` setup and DESIGN.md Phase P for
//! the rationale (interface/format layer split, two-stage feature
//! `plugin-wasm` vs `plugin-wasm-runtime-only`).
//!
//! [wasm-component-model]: https://component-model.bytecodealliance.org/

pub mod bindings;
pub mod cache;
pub mod controller;
pub mod convert;
pub mod engine;
pub mod host_state;

pub use cache::WasmPluginCache;
pub use controller::WasmController;
pub use engine::WasmEngine;
