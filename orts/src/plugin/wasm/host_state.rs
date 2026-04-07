//! Host-side state for the wasmtime `Store` and implementation of
//! the WIT `host-env` import interface.
//!
//! Each satellite's `WasmController` owns a `Store<HostState>`, and
//! the `HostState` is where the guest can reach back into the host
//! via the `host-env` interface imports (`log`, `magnetic-field-eci`).

use nalgebra::Vector3;
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};

use super::bindings::orts::plugin::host_env;
use super::bindings::orts::plugin::types as wit;

// The `types` interface has no host functions, but the bindgen-generated
// `add_to_linker` requires a blanket `types::Host` impl for the host state.
impl wit::Host for HostState {}

/// Per-satellite host state stored inside each `wasmtime::Store`.
///
/// Holds the WASI context (required by Rust std-based guests), the
/// geomagnetic field model, and a human-readable label for logging.
pub struct HostState {
    /// Human-readable satellite / controller label for log messages.
    pub label: String,
    /// Geomagnetic field model used by the `magnetic-field-eci` host
    /// import. Phase P1 defaults to `TiltedDipole::earth()`; Phase
    /// D-5 will replace this with an IGRF spherical-harmonic model
    /// (or a `Box<dyn MagneticFieldModel>` for configurability).
    field: TiltedDipole,
    /// WASI context: provides sandboxed stdio / env / filesystem to
    /// the guest. Our controllers don't use these, but Rust std's
    /// runtime startup requires `wasi:cli/*` imports to be present.
    wasi: wasmtime_wasi::WasiCtx,
    /// Resource table for WASI resources.
    table: wasmtime_wasi::ResourceTable,
}

impl HostState {
    /// Create a new host state with default field model and a
    /// sandboxed (no I/O) WASI context.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            field: TiltedDipole::earth(),
            wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
            table: wasmtime_wasi::ResourceTable::new(),
        }
    }
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Required by wasmtime's `bindgen!`-generated `add_to_linker`.
///
/// `HasData` tells wasmtime how to borrow the host state from a
/// `&mut Store<HostState>` for the duration of a host-import call.
/// The standard pattern is `Data<'a> = &'a mut Self`.
impl wasmtime::component::HasData for HostState {
    type Data<'a> = &'a mut HostState;
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

    fn magnetic_field_eci(&mut self, position_eci_km: wit::Vec3, epoch: wit::Epoch) -> wit::Vec3 {
        let pos = kaname::Eci::new(position_eci_km.x, position_eci_km.y, position_eci_km.z);
        let epoch = kaname::epoch::Epoch::from_jd(epoch.julian_date);
        let b = self.field.field_eci(&pos, &epoch);
        wit::Vec3 {
            x: b.x,
            y: b.y,
            z: b.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::host_env::Host as _;
    use super::*;

    #[test]
    fn magnetic_field_returns_finite_nonzero_for_leo() {
        let mut state = HostState::new("test");
        // LEO position: ~7000 km from Earth centre, on the x-axis.
        let pos = wit::Vec3 {
            x: 7000.0,
            y: 0.0,
            z: 0.0,
        };
        // J2000 epoch.
        let epoch = wit::Epoch {
            julian_date: 2451545.0,
        };
        let b = state.magnetic_field_eci(pos, epoch);
        assert!(b.x.is_finite());
        assert!(b.y.is_finite());
        assert!(b.z.is_finite());
        let magnitude = (b.x * b.x + b.y * b.y + b.z * b.z).sqrt();
        // LEO geomagnetic field magnitude is ~20-60 µT = 2e-5 to 6e-5 T.
        // We assert a slightly wider band (1e-5 to 1e-4) to absorb
        // model variation while still catching conversion errors that
        // would produce wildly wrong values.
        assert!(
            magnitude > 1e-5 && magnitude < 1e-4,
            "expected LEO-range magnetic field (~20-60 µT), got {magnitude:.3e} T"
        );
    }
}
