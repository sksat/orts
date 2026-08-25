//! Error type returned by plugin-layer operations.
//!
//! `PluginError` unifies failure modes across the various backends
//! (Native / WASM / Rhai / ...). Only a handful of variants exist so
//! far; dedicated guest-runtime cases (Trap, OutOfFuel, OutOfMemory,
//! GuestPanic, Marshal, ApiVersionMismatch) are planned.
//!
//! The shape matches the landmines identified in the wasmtime runtime
//! survey: every failure path should be
//! distinguishable so the host can decide per-case whether to halt the
//! simulation, fall back to the last command, or switch the controller
//! to a safemode.

use thiserror::Error;

/// Errors produced by a plugin-layer controller or actuator bridge.
///
/// Dedicated variants for the WASM backend (`Trap`, `OutOfFuel`,
/// `OutOfMemory`, `GuestPanic`, `Marshal`, `ApiVersionMismatch`) are
/// planned. The current `Runtime(String)` catch-all will shrink as
/// those land.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginError {
    /// A guest returned a command containing NaN / Inf. The host must
    /// never forward such a command to the actuator layer because the
    /// downstream ODE integration would propagate the non-finite value
    /// into the whole 14-D spacecraft state.
    #[error("plugin returned a non-finite command: {0}")]
    BadCommand(String),

    /// The actuator bridge was asked to apply a command field that the
    /// current `ActuatorBundle` does not have a target for (e.g. a
    /// `magnetic_moment` command when no magnetorquer is configured).
    #[error("actuator for {command} is not configured")]
    MissingActuator {
        /// Human-readable label of the command variant.
        command: &'static str,
    },

    /// The host could not load / init a guest.
    #[error("plugin init failed: {0}")]
    Init(String),

    /// The controller does not implement the requested optional
    /// operation (e.g. `snapshot_state` / `restore_state` on a native
    /// controller that has no serializable internal state).
    #[error("operation '{0}' not supported by this controller")]
    UnsupportedOperation(&'static str),

    /// Catch-all for backends that have richer error taxonomies than
    /// the ones listed above. The planned dedicated variants will
    /// replace most uses of this over time.
    #[error("plugin runtime error: {0}")]
    Runtime(String),
}
