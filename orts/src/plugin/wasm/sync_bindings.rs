//! Host-side type-safe bindings for `wit/v0/orts.wit`.
//!
//! `wasmtime::component::bindgen!` parses the WIT world at compile
//! time and emits:
//!
//! - Rust record types (`Vec3`, `Quat`, `SpacecraftState`, `Command`,
//!   `Observation`, ...) matching the WIT records / variants.
//! - A trait (`PluginImports`) that the host must implement to
//!   provide the `host-env` interface (`log`, `magnetic-field-eci`).
//! - A typed handle (`Plugin`) that wraps a `wasmtime::component::Instance`
//!   and exposes the guest's `controller` interface
//!   (`sample-period-s`, `init`, `initial-command`, `update`,
//!   `current-mode`) as strongly-typed methods.
//!
//! `WasmController` (`sync_controller.rs`) wires these bindings up
//! and provides the host `env` implementation.
//!
//! The emitted types live under `orts::plugin::wasm::bindings::*`;
//! downstream code in `orts::plugin::wasm` re-exports whatever it
//! needs. The wit-bindgen output is **private to this module** so
//! that the public plugin-layer API (in `orts::plugin`) never leaks
//! generated types to downstream crates.

// Path is relative to the crate root (`orts/Cargo.toml`), so
// `wit/v0/orts.wit` resolves to `orts/wit/v0/orts.wit`. The WIT
// definition lives inside the crate rather than at the workspace
// root so the crate is self-contained and the `bindgen!` macro does
// not reach out of its own source tree.
//
// This module runs guests synchronously on the Pulley interpreter
// (wasmtime default is blocking, no `async` opt-in needed); the
// async backend has its own bindings.
//
// `trappable_imports: true` may become necessary so that
// host `host-env` functions (log, magnetic-field-eci) can return
// `Result<_, wasmtime::Error>` and trap the guest cleanly on
// failure instead of propagating a panic.
wasmtime::component::bindgen!({
    path: "wit/v0/orts.wit",
    world: "plugin",
});
