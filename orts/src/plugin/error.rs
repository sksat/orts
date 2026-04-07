//! Error type returned by plugin-layer operations.
//!
//! `PluginError` unifies failure modes across the various backends
//! (Native / WASM / Rhai / ...). Phase P0.5 only needs a handful of
//! variants for the Native adapter; later phases will light up the
//! guest-runtime-specific cases (Trap, OutOfFuel, OutOfMemory,
//! GuestPanic, Marshal, ApiVersionMismatch).
//!
//! The shape matches the landmines identified in the Phase P0 research
//! (see DESIGN.md "落とし穴リスト"): every failure path should be
//! distinguishable so the host can decide per-case whether to halt the
//! simulation, fall back to the last command, or switch the controller
//! to a safemode.

use thiserror::Error;

/// Errors produced by a plugin-layer controller or actuator bridge.
///
/// Phase P1 will add dedicated variants for the WASM backend:
/// `Trap`, `OutOfFuel`, `OutOfMemory`, `GuestPanic`, `Marshal`,
/// `ApiVersionMismatch`. The current `Runtime(String)` catch-all
/// will shrink as those land.
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

    /// The host could not load / init a guest (Phase P1+).
    #[error("plugin init failed: {0}")]
    Init(String),

    /// The controller does not implement the requested optional
    /// operation (e.g. `snapshot_state` / `restore_state` on a native
    /// controller that has no serializable internal state).
    #[error("operation '{0}' not supported by this controller")]
    UnsupportedOperation(&'static str),

    /// Catch-all for backends that have richer error taxonomies than
    /// the ones listed above. Phase P1 will replace most uses of this
    /// with dedicated variants (`Trap`, `OutOfFuel`, `OutOfMemory`,
    /// `GuestPanic`, `Marshal`, `ApiVersionMismatch`).
    #[error("plugin runtime error: {0}")]
    Runtime(String),
}
