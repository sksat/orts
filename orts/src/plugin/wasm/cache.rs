//! Shared cache of compiled WASM plugins keyed by file path.
//!
//! Compiling a WASM Component (running Cranelift) is the most expensive
//! step of constructing a plugin-backed satellite. When a simulation
//! builds many satellites that share the same controller WASM file, we
//! don't want to recompile the component N times. This cache holds:
//!
//! - a single shared sync [`WasmEngine`] (Pulley target),
//! - optionally, a single shared async [`WasmEngine`] and
//!   [`AsyncRuntime`] (feature `plugin-wasm-async`),
//! - per-path compiled sync + async [`Component`]s and their
//!   pre-linked instances.
//!
//! Typical usage:
//!
//! ```no_run
//! # use orts::plugin::wasm::WasmPluginCache;
//! # fn main() -> Result<(), orts::plugin::PluginError> {
//! let mut cache = WasmPluginCache::new()?;
//! for i in 0..1000 {
//!     let ctrl = cache.build_sync_controller(
//!         "plugin-sdk/examples/bdot-finite-diff/target/wasm32-wasip1/release/guest.wasm".as_ref(),
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

#[cfg(feature = "plugin-wasm-async")]
use super::async_controller::{AsyncPluginPreBuilt, AsyncWasmController};
#[cfg(feature = "plugin-wasm-async")]
use super::async_runtime::{AsyncMode, AsyncRuntime};

/// Cache of compiled WASM plugins and their pre-linked instances.
///
/// Holds a single sync `WasmEngine` and, when the
/// `plugin-wasm-async` feature is enabled, a single async
/// `WasmEngine` + `AsyncRuntime` that are created lazily on first
/// async use. Plugin components are compiled per backend and cached
/// by file path.
pub struct WasmPluginCache {
    sync_engine: Arc<WasmEngine>,
    sync_plugins: HashMap<PathBuf, CachedSyncPlugin>,

    /// Execution mode used when the `AsyncRuntime` is lazily created.
    /// Set at construction and immutable afterwards; the runtime is
    /// only built once, so the mode is locked in after first async use.
    #[cfg(feature = "plugin-wasm-async")]
    async_mode: AsyncMode,
    #[cfg(feature = "plugin-wasm-async")]
    async_state: Option<AsyncCacheState>,
}

/// A compiled component and its pre-linked sync instance, kept alive
/// together so that the pre stays valid.
struct CachedSyncPlugin {
    /// Kept alive for the pre to reference.
    #[allow(dead_code)]
    component: Component,
    pre: PluginPre<HostState>,
}

#[cfg(feature = "plugin-wasm-async")]
struct AsyncCacheState {
    engine: Arc<WasmEngine>,
    runtime: Arc<AsyncRuntime>,
    plugins: HashMap<PathBuf, AsyncPluginPreBuilt>,
}

impl WasmPluginCache {
    /// Create a new empty cache with a fresh sync Pulley-target
    /// engine. When `plugin-wasm-async` is enabled the async runtime
    /// defaults to [`AsyncMode::Deterministic`]; use
    /// [`new_with_async_mode`](Self::new_with_async_mode) to opt into
    /// the throughput-optimised variant.
    ///
    /// The async engine and runtime are **not** created here even
    /// when the `plugin-wasm-async` feature is enabled — they are
    /// started lazily on the first call to
    /// [`build_async_controller`](Self::build_async_controller).
    pub fn new() -> Result<Self, PluginError> {
        let sync_engine = Arc::new(WasmEngine::new_sync()?);
        Ok(Self {
            sync_engine,
            sync_plugins: HashMap::new(),
            #[cfg(feature = "plugin-wasm-async")]
            async_mode: AsyncMode::Deterministic,
            #[cfg(feature = "plugin-wasm-async")]
            async_state: None,
        })
    }

    /// Create a new cache that, on first async use, will build an
    /// `AsyncRuntime` in the given [`AsyncMode`].
    #[cfg(feature = "plugin-wasm-async")]
    pub fn new_with_async_mode(async_mode: AsyncMode) -> Result<Self, PluginError> {
        let sync_engine = Arc::new(WasmEngine::new_sync()?);
        Ok(Self {
            sync_engine,
            sync_plugins: HashMap::new(),
            async_mode,
            async_state: None,
        })
    }

    /// Borrow the underlying shared sync engine.
    pub fn sync_engine(&self) -> &Arc<WasmEngine> {
        &self.sync_engine
    }

    /// Build a sync controller for the plugin at `path`, reusing the
    /// cached component + pre-link if available.
    ///
    /// On first call for a given path, this reads the WASM bytes,
    /// compiles them to a `Component`, and prepares a `PluginPre`.
    /// Subsequent calls for the same path skip all three steps.
    pub fn build_sync_controller(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
    ) -> Result<WasmController, PluginError> {
        let pre = self.get_or_load_sync(path)?;
        WasmController::new(pre, label, config)
    }

    /// Legacy alias for [`build_sync_controller`](Self::build_sync_controller).
    pub fn build_controller(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
    ) -> Result<WasmController, PluginError> {
        self.build_sync_controller(path, label, config)
    }

    fn get_or_load_sync(&mut self, path: &Path) -> Result<&PluginPre<HostState>, PluginError> {
        if !self.sync_plugins.contains_key(path) {
            let bytes = std::fs::read(path).map_err(|e| {
                PluginError::Init(format!("cannot read WASM at '{}': {e}", path.display()))
            })?;
            let component = Component::new(self.sync_engine.inner(), &bytes).map_err(|e| {
                PluginError::Init(format!("WASM compile failed for '{}': {e}", path.display()))
            })?;
            let pre = WasmController::prepare(&self.sync_engine, &component)?;
            self.sync_plugins
                .insert(path.to_path_buf(), CachedSyncPlugin { component, pre });
        }
        Ok(&self
            .sync_plugins
            .get(path)
            .expect("just inserted if missing")
            .pre)
    }
}

#[cfg(feature = "plugin-wasm-async")]
impl WasmPluginCache {
    /// Build an async controller for the plugin at `path`.
    ///
    /// On first async use, this also creates the shared async engine
    /// and the background `AsyncRuntime` thread. On subsequent calls
    /// for the same path the cached compiled component + pre-link
    /// are reused.
    pub fn build_async_controller(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
    ) -> Result<AsyncWasmController, PluginError> {
        let built = self.get_or_load_async(path)?;
        AsyncWasmController::new(built, label, config)
    }

    /// Borrow the async engine, creating it if this is the first
    /// async use. Public so that callers that need direct access
    /// (e.g. tests) can reuse the same engine.
    pub fn async_engine(&mut self) -> Result<&Arc<WasmEngine>, PluginError> {
        self.ensure_async_state()?;
        Ok(&self.async_state.as_ref().unwrap().engine)
    }

    /// Borrow the async runtime, creating it if this is the first
    /// async use.
    pub fn async_runtime(&mut self) -> Result<&Arc<AsyncRuntime>, PluginError> {
        self.ensure_async_state()?;
        Ok(&self.async_state.as_ref().unwrap().runtime)
    }

    fn ensure_async_state(&mut self) -> Result<(), PluginError> {
        if self.async_state.is_none() {
            let engine = Arc::new(WasmEngine::new_async()?);
            let runtime = Arc::new(AsyncRuntime::new(self.async_mode)?);
            self.async_state = Some(AsyncCacheState {
                engine,
                runtime,
                plugins: HashMap::new(),
            });
        }
        Ok(())
    }

    fn get_or_load_async(&mut self, path: &Path) -> Result<&AsyncPluginPreBuilt, PluginError> {
        self.ensure_async_state()?;
        let state = self.async_state.as_mut().unwrap();
        if !state.plugins.contains_key(path) {
            let bytes = std::fs::read(path).map_err(|e| {
                PluginError::Init(format!("cannot read WASM at '{}': {e}", path.display()))
            })?;
            let component = Component::new(state.engine.inner(), &bytes).map_err(|e| {
                PluginError::Init(format!(
                    "async WASM compile failed for '{}': {e}",
                    path.display()
                ))
            })?;
            let built = AsyncPluginPreBuilt::new(&state.engine, &state.runtime, &component)?;
            state.plugins.insert(path.to_path_buf(), built);
        }
        Ok(state.plugins.get(path).expect("just inserted if missing"))
    }
}
