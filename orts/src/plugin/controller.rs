//! Plugin-layer controller trait.
//!
//! `PluginController` is the host-visible interface that every backend
//! (native Rust / WASM / Rhai / PyO3 / ...) implements. A controller
//! receives an [`Observation`] snapshot at each sample tick and returns
//! a logical [`Command`] to be applied by the host's actuator bridge.
//!
//! This is separate from the existing
//! [`crate::control::DiscreteController`] trait on purpose: the legacy
//! trait is parameterised over a concrete `type Command` (e.g.
//! `Vector3<f64>`) and takes `(attitude, orbit, epoch)` as positional
//! arguments. The plugin trait fixes `Command` to the
//! [`super::Command`] enum and bundles all inputs into a single
//! `Observation` struct so that WASM guests can share the same shape
//! with native implementations. A future phase may unify the two once
//! every native controller has migrated; Phase P0.5 deliberately keeps
//! them side by side so that no existing oracle tests have to change.
//!
//! See DESIGN.md Phase P, D3 ("trait 構造: 既存 DiscreteController を
//! 拡張して 1 trait に統一") for the long-term plan — P0.5 introduces
//! `PluginController` as the forward-compatible target shape.

use super::command::Command;
use super::error::PluginError;
use super::observation::Observation;

/// A controller backend exposed through the plugin layer.
///
/// Implementors are either native Rust controllers (`BdotFiniteDiff`,
/// `InertialPdController`, ...) or guest runtimes wrapping a WASM
/// component / Rhai script / Python callable. In both cases the
/// contract is the same: given an observation, produce a command.
///
/// `Send` is required so individual satellite simulations (each
/// holding its own controller instance) can be driven on worker
/// threads. `Sync` is NOT required — `wasmtime::Store` is `!Sync`, and
/// the orts spacecraft lifecycle is "1 controller = 1 satellite", so
/// there is no legitimate need to share a single controller across
/// threads concurrently.
pub trait PluginController: Send {
    /// Human-readable controller name, used for logging and for the
    /// `current_mode()` reporting channel.
    fn name(&self) -> &str;

    /// Fixed sample period \[s\]. Controllers that need to change
    /// their tick rate dynamically are out of scope for Phase P1.
    fn sample_period(&self) -> f64;

    /// API version of the plugin interface this controller targets.
    ///
    /// The host uses this to detect guest/host mismatches separately
    /// from semver bumps of the containing crate. Default is `1` for
    /// Phase P0.5; Phase P1 will bump this as the WIT evolves.
    fn api_version(&self) -> u32 {
        1
    }

    /// Initialise the controller from a backend-specific configuration
    /// string (e.g. JSON / YAML / TOML blob serialised by the host from
    /// the mission configuration).
    ///
    /// Native controllers typically carry their config in their
    /// constructor and return `Ok(())` here unconditionally. Guest
    /// backends use this to seed internal parameters before the first
    /// `update` call.
    fn init(&mut self, _config: &str) -> Result<(), PluginError> {
        Ok(())
    }

    /// Initial command emitted before the first `update` call.
    ///
    /// This is what the host's [`super::ActuatorBundle`] holds during
    /// the very first zero-order-hold segment (usually a zero moment /
    /// zero throttle / identity quaternion, depending on the actuator).
    fn initial_command(&self) -> Command;

    /// Advance the controller's internal state by one sample tick and
    /// return the command to apply during the next zero-order-hold
    /// segment.
    ///
    /// Returning `Err(PluginError::BadCommand(_))` (or any other
    /// variant) tells the host to halt the simulation: the command
    /// cannot be trusted, and the host should fall back to safemode or
    /// abort rather than propagating bad state into the ODE.
    fn update(&mut self, obs: &Observation<'_>) -> Result<Command, PluginError>;

    /// Currently-active mission mode, if the controller exposes a
    /// mode machine (detumble / nadir-point / burn / ...).
    ///
    /// Native controllers with a fixed mode return `None`. Guest-side
    /// controllers that implement their own state machines return the
    /// mode label for observability (viewer, logs, telemetry).
    fn current_mode(&self) -> Option<&str> {
        None
    }

    /// Serialise the controller's internal state into a byte blob
    /// suitable for hot reload across an identical controller binary.
    ///
    /// Controllers without migratable state (most native ones) return
    /// `None`. Phase P6 will use this for hot-reload of WASM guests;
    /// Phase P0.5 only needs the default implementation.
    ///
    /// **Contract**: a controller that returns `Some(_)` from
    /// `snapshot_state()` MUST accept the resulting bytes in
    /// [`restore_state`](Self::restore_state) without error. Conversely,
    /// a controller that returns `None` from `snapshot_state()` is NOT
    /// required to accept any input in `restore_state()` — callers
    /// should use `snapshot_state().is_some()` as the probe for
    /// hot-reload support, not `restore_state` return values. Calling
    /// `restore_state` on a controller that returned `None` from
    /// `snapshot_state` is a host-side bug and will produce
    /// [`PluginError::UnsupportedOperation`].
    fn snapshot_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// Restore a previously captured internal state blob.
    ///
    /// Returns `PluginError::UnsupportedOperation` by default, which
    /// makes native controllers a no-op for hot reload. See
    /// [`snapshot_state`](Self::snapshot_state) for the full contract.
    fn restore_state(&mut self, _bytes: &[u8]) -> Result<(), PluginError> {
        Err(PluginError::UnsupportedOperation("restore_state"))
    }
}
