//! Shared cache of compiled WASM plugins keyed by file path.
//!
//! Compiling a WASM Component (running Cranelift) is the most expensive
//! step of constructing a plugin-backed satellite. When a simulation
//! builds many satellites that share the same controller WASM file, we
//! don't want to recompile the component N times. This cache holds:
//!
//! - a single shared [`WasmEngine`] (Pulley target),
//! - per-path compiled [`Component`]s,
//! - per-path pre-linked [`PluginPre`] instances ready for
//!   [`WasmController::new`].
//!
//! Typical usage:
//!
//! ```no_run
//! # use orts::plugin::wasm::WasmPluginCache;
//! # fn main() -> Result<(), orts::plugin::PluginError> {
//! let mut cache = WasmPluginCache::new()?;
//! for i in 0..1000 {
//!     let ctrl = cache.build_controller(
//!         "plugins/bdot-finite-diff/target/wasm32-wasip1/release/guest.wasm".as_ref(),
//!         &format!("sat{i}"),
//!         "",
//!     )?;
//!     // use ctrl ...
//! }
//! # Ok(()) }
//! ```
//!
//! The first call for a given path compiles the component and prepares
//! the linker; subsequent calls reuse both. Building 1000 satellites
//! with a shared cache takes ~seconds instead of ~minutes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmtime::component::Component;

use super::engine::WasmEngine;
use super::sync_bindings::PluginPre;
use super::sync_controller::WasmController;
use super::sync_host_state::HostState;

use crate::plugin::error::PluginError;

/// Cache of compiled WASM plugins and their pre-linked instances.
///
/// Holds a single `WasmEngine` and a map of `wasm path → cached
/// component + pre-link`. Satellites are built against the cached
/// entries by calling [`WasmPluginCache::build_controller`].
pub struct WasmPluginCache {
    engine: Arc<WasmEngine>,
    plugins: HashMap<PathBuf, CachedPlugin>,
}

/// A compiled component and its pre-linked instance, kept alive
/// together so that the pre stays valid.
struct CachedPlugin {
    /// Kept alive for the pre to reference.
    #[allow(dead_code)]
    component: Component,
    pre: PluginPre<HostState>,
}

impl WasmPluginCache {
    /// Create a new empty cache with a fresh Pulley-target engine.
    pub fn new() -> Result<Self, PluginError> {
        let engine = Arc::new(WasmEngine::new()?);
        Ok(Self {
            engine,
            plugins: HashMap::new(),
        })
    }

    /// Borrow the underlying shared engine.
    pub fn engine(&self) -> &Arc<WasmEngine> {
        &self.engine
    }

    /// Build a controller for the plugin at `path`, reusing the cached
    /// component + pre-link if available.
    ///
    /// On first call for a given path, this reads the WASM bytes,
    /// compiles them to a `Component`, and prepares a `PluginPre`.
    /// Subsequent calls for the same path skip all three steps.
    pub fn build_controller(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
    ) -> Result<WasmController, PluginError> {
        let pre = self.get_or_load(path)?;
        WasmController::new(pre, label, config)
    }

    fn get_or_load(&mut self, path: &Path) -> Result<&PluginPre<HostState>, PluginError> {
        if !self.plugins.contains_key(path) {
            let bytes = std::fs::read(path).map_err(|e| {
                PluginError::Init(format!("cannot read WASM at '{}': {e}", path.display()))
            })?;
            let component = Component::new(self.engine.inner(), &bytes).map_err(|e| {
                PluginError::Init(format!("WASM compile failed for '{}': {e}", path.display()))
            })?;
            let pre = WasmController::prepare(&self.engine, &component)?;
            self.plugins
                .insert(path.to_path_buf(), CachedPlugin { component, pre });
        }
        Ok(&self
            .plugins
            .get(path)
            .expect("just inserted if missing")
            .pre)
    }
}
