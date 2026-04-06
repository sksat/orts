//! Host-side state for the wasmtime `Store` and implementation of
//! the WIT `host-env` import interface.
//!
//! Each satellite's `WasmController` owns a `Store<HostState>`, and
//! the `HostState` is where the guest can reach back into the host
//! via the `host-env` interface imports (`log`, `magnetic-field-eci`).
//!
//! Phase P1-b2 provides a minimal `HostState` with `log` → `tracing`
//! forwarding and a `todo!()` stub for `magnetic-field-eci`. Phase
//! P1-b3 will wire the magnetic field to `tobari::magnetic`.

use super::bindings::orts::plugin::host_env;
use super::bindings::orts::plugin::types as wit;

// The `types` interface has no host functions, but the bindgen-generated
// `add_to_linker` requires a blanket `types::Host` impl for the host state.
impl wit::Host for HostState {}

/// Per-satellite host state stored inside each `wasmtime::Store`.
pub struct HostState {
    /// Human-readable satellite / controller label for log messages.
    pub label: String,
}

/// Required by wasmtime's `bindgen!`-generated `add_to_linker`.
///
/// `HasData` tells wasmtime how to borrow the host state from a
/// `&mut Store<HostState>` for the duration of a host-import call.
/// The standard pattern is `Data<'a> = &'a mut Self`.
impl wasmtime::component::HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

impl HostState {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl host_env::Host for HostState {
    fn log(&mut self, level: host_env::LogLevel, message: String) {
        match level {
            host_env::LogLevel::Trace => log::trace!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Debug => log::debug!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Info => log::info!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Warn => log::warn!("[wasm:{}] {}", self.label, message),
            host_env::LogLevel::Error => log::error!("[wasm:{}] {}", self.label, message),
        }
    }

    fn magnetic_field_eci(&mut self, _position_eci_km: wit::Vec3, _e: wit::Epoch) -> wit::Vec3 {
        // Phase P1-b3 will wire this to tobari::magnetic::TiltedDipole
        // (or a configurable MagneticFieldModel trait object).
        // For now, guests that call this import will get a panic.
        todo!("magnetic-field-eci host import not yet wired (Phase P1-b3)")
    }
}
