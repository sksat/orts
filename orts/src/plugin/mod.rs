//! Pluggable controller backends (Phase P).
//!
//! This module defines the host-side glue between the orts simulator and
//! "guest" controllers that implement spacecraft attitude / thrust / mode
//! logic in external code. The intended use cases are:
//!
//! The plugin layer intentionally ships **only** the host-side
//! interfaces and infrastructure — [`PluginController`], [`Command`],
//! [`Observation`], [`ActuatorBundle`], [`PluginError`]. It does NOT
//! ship any concrete controllers. Test-time reference implementations
//! (e.g. the plugin-layer B-dot used by the P0.5 oracle) live inline
//! as private modules inside each `orts/tests/plugin_bdot_*.rs`
//! integration test binary, so they cannot be mistaken for production
//! controllers and cannot leak into downstream crates that depend on
//! `orts`.
//!
//! - **WASM backend** (Phase P1, feature-gated): loads a WebAssembly
//!   Component guest via `wasmtime` + Pulley interpreter and implements
//!   [`PluginController`] by delegating every `update` call to the guest.
//! - **Script backends** (Phase P2+): Rhai / PyO3 / ...
//!
//! The plugin layer deliberately does NOT modify the existing
//! [`crate::control::DiscreteController`] trait. Native controllers
//! continue to implement `DiscreteController` (unchanged, so existing
//! oracle tests keep passing). A future phase may unify the two traits
//! once a WASM backend is in place; see DESIGN.md Phase P, D3.

pub mod actuators;
pub mod command;
pub mod controller;
pub mod error;
pub mod observation;

pub use actuators::ActuatorBundle;
pub use command::Command;
pub use controller::PluginController;
pub use error::PluginError;
pub use observation::{EnvSnapshot, Observation};
