//! Shared cache of compiled WASM plugins keyed by file path or by the
//! SHA-256 of a component given as bytes.
//!
//! Compiling a WASM Component (running Cranelift) is the most expensive
//! step of constructing a plugin-backed satellite. When a simulation
//! builds many satellites that share the same controller WASM file, we
//! don't want to recompile the component N times. This cache holds:
//!
//! - a single shared sync [`WasmEngine`] (Pulley target),
//! - optionally, a single shared async [`WasmEngine`] and
//!   [`AsyncRuntime`] (feature `plugin-wasm-async`),
//! - per-path (or per-digest, for a [`ComponentBytes`]) compiled sync +
//!   async [`Component`]s and their pre-linked instances.
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
//!         arika::body::KnownBody::Earth,
//!     )?;
//!     // use ctrl ...
//! }
//! # Ok(()) }
//! ```
//!
//! The first call for a given path compiles the component and prepares
//! the linker; subsequent calls reuse both. Building 1000 satellites
//! with a shared cache takes ~seconds instead of ~minutes.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmtime::component::Component;

use super::component_bytes::ComponentBytes;
use super::engine::WasmEngine;
use super::limits::GuestLimits;
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
/// by file path, or by digest for a component given as bytes.
pub struct WasmPluginCache {
    sync_engine: Arc<WasmEngine>,
    sync_plugins: HashMap<PluginKey, CachedSyncPlugin>,
    /// The limits every controller this cache builds runs under.
    limits: GuestLimits,

    /// Execution mode used when the `AsyncRuntime` is lazily created.
    /// Set at construction and immutable afterwards; the runtime is
    /// only built once, so the mode is locked in after first async use.
    #[cfg(feature = "plugin-wasm-async")]
    async_mode: AsyncMode,
    #[cfg(feature = "plugin-wasm-async")]
    async_state: Option<AsyncCacheState>,
}

/// Which cache entry a plugin is.
///
/// Paths and digests are separate namespaces: a file whose name spells a
/// digest is not the component with that digest.
#[derive(Clone, PartialEq, Eq, Hash)]
enum PluginKey {
    Path(PathBuf),
    Sha256([u8; 32]),
}

/// Where a plugin's bytes come from.
#[derive(Clone, Copy)]
enum Source<'a> {
    Path(&'a Path),
    Bytes(&'a ComponentBytes),
}

impl<'a> Source<'a> {
    fn key(self) -> PluginKey {
        match self {
            Source::Path(path) => PluginKey::Path(path.to_path_buf()),
            Source::Bytes(component) => PluginKey::Sha256(*component.sha256()),
        }
    }

    /// How an error names the plugin.
    fn describe(self) -> String {
        match self {
            Source::Path(path) => format!("'{}'", path.display()),
            Source::Bytes(component) => format!("component sha256 {}", component.sha256_hex()),
        }
    }

    /// The bytes to compile. Only a path touches the filesystem.
    fn read(self) -> Result<Cow<'a, [u8]>, PluginError> {
        match self {
            Source::Path(path) => std::fs::read(path).map(Cow::Owned).map_err(|e| {
                PluginError::Init(format!("cannot read WASM at '{}': {e}", path.display()))
            }),
            Source::Bytes(component) => Ok(Cow::Borrowed(component.bytes())),
        }
    }
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
    plugins: HashMap<PluginKey, AsyncPluginPreBuilt>,
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
            limits: GuestLimits::default(),
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
            limits: GuestLimits::default(),
            async_mode,
            async_state: None,
        })
    }

    /// The mode this cache will build its `AsyncRuntime` in.
    ///
    /// The runtime is started on first async use, so this is what the cache
    /// was created with rather than a running runtime's mode. A caller that
    /// hands the choice down from a command line reads it back here to check
    /// the cache it built carries what was asked for.
    #[cfg(feature = "plugin-wasm-async")]
    pub fn async_mode(&self) -> AsyncMode {
        self.async_mode
    }

    /// Build every controller from now on under `limits` instead of the
    /// default [`GuestLimits`].
    pub fn with_guest_limits(mut self, limits: GuestLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The limits the controllers this cache builds run under.
    pub fn guest_limits(&self) -> GuestLimits {
        self.limits
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
        body: arika::body::KnownBody,
    ) -> Result<WasmController, PluginError> {
        self.build_sync_controller_with_streams(path, label, config, Vec::new(), body)
    }

    /// As [`build_sync_controller`](Self::build_sync_controller) but wired
    /// to the given declared `stream-io` streams (kble bridge).
    pub fn build_sync_controller_with_streams(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
    ) -> Result<WasmController, PluginError> {
        let limits = self.limits;
        let pre = self.get_or_load_sync(Source::Path(path))?;
        WasmController::new_with_limits(pre, label, config, stream_names, body, limits)
    }

    /// As [`build_sync_controller_with_streams`](Self::build_sync_controller_with_streams)
    /// for a component given as bytes, cached by its SHA-256.
    ///
    /// Nothing is read from the filesystem. Two [`ComponentBytes`] holding the
    /// same bytes share one compilation.
    pub fn build_sync_controller_from_bytes_with_streams(
        &mut self,
        component: &ComponentBytes,
        label: &str,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
    ) -> Result<WasmController, PluginError> {
        let limits = self.limits;
        let pre = self.get_or_load_sync(Source::Bytes(component))?;
        WasmController::new_with_limits(pre, label, config, stream_names, body, limits)
    }

    fn get_or_load_sync(
        &mut self,
        source: Source<'_>,
    ) -> Result<&PluginPre<HostState>, PluginError> {
        let key = source.key();
        if !self.sync_plugins.contains_key(&key) {
            let bytes = source.read()?;
            let component = Component::new(self.sync_engine.inner(), &bytes).map_err(|e| {
                PluginError::Init(format!(
                    "WASM compile failed for {}: {e}",
                    source.describe()
                ))
            })?;
            let pre = WasmController::prepare(&self.sync_engine, &component)?;
            self.sync_plugins
                .insert(key.clone(), CachedSyncPlugin { component, pre });
        }
        Ok(&self
            .sync_plugins
            .get(&key)
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
        body: arika::body::KnownBody,
    ) -> Result<AsyncWasmController, PluginError> {
        self.build_async_controller_with_streams(path, label, config, Vec::new(), body)
    }

    /// As [`build_async_controller`](Self::build_async_controller) but wired
    /// to the given declared `stream-io` streams (kble bridge).
    pub fn build_async_controller_with_streams(
        &mut self,
        path: &Path,
        label: &str,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
    ) -> Result<AsyncWasmController, PluginError> {
        let limits = self.limits;
        let built = self.get_or_load_async(Source::Path(path))?;
        AsyncWasmController::new_with_limits(built, label, config, stream_names, body, limits)
    }

    /// As [`build_async_controller_with_streams`](Self::build_async_controller_with_streams)
    /// for a component given as bytes, cached by its SHA-256.
    ///
    /// Nothing is read from the filesystem. Two [`ComponentBytes`] holding the
    /// same bytes share one compilation.
    pub fn build_async_controller_from_bytes_with_streams(
        &mut self,
        component: &ComponentBytes,
        label: &str,
        config: &str,
        stream_names: Vec<String>,
        body: arika::body::KnownBody,
    ) -> Result<AsyncWasmController, PluginError> {
        let limits = self.limits;
        let built = self.get_or_load_async(Source::Bytes(component))?;
        AsyncWasmController::new_with_limits(built, label, config, stream_names, body, limits)
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

    fn get_or_load_async(
        &mut self,
        source: Source<'_>,
    ) -> Result<&AsyncPluginPreBuilt, PluginError> {
        self.ensure_async_state()?;
        let state = self.async_state.as_mut().unwrap();
        let key = source.key();
        if !state.plugins.contains_key(&key) {
            let bytes = source.read()?;
            let component = Component::new(state.engine.inner(), &bytes).map_err(|e| {
                PluginError::Init(format!(
                    "async WASM compile failed for {}: {e}",
                    source.describe()
                ))
            })?;
            let built = AsyncPluginPreBuilt::new(&state.engine, &state.runtime, &component)?;
            state.plugins.insert(key.clone(), built);
        }
        Ok(state.plugins.get(&key).expect("just inserted if missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PD_RW_CONFIG: &str = r#"{"kp":1.0,"kd":2.0,"sample_period":0.1}"#;

    /// The pd-rw-control guest, or `None` (with a note) on a checkout that
    /// has not built it.
    fn pd_rw_guest() -> Option<(PathBuf, Vec<u8>)> {
        let path = PathBuf::from(format!(
            "{}/../plugin-sdk/examples/target/wasm32-wasip1/release/\
             orts_example_plugin_pd_rw_control.wasm",
            env!("CARGO_MANIFEST_DIR")
        ));
        match std::fs::read(&path) {
            Ok(bytes) => Some((path, bytes)),
            Err(_) => {
                eprintln!(
                    "WASM not found: {}\nBuild: cd plugin-sdk/examples && \
                     cargo +1.91.0 component build --release -p orts-example-plugin-pd-rw-control",
                    path.display()
                );
                None
            }
        }
    }

    /// Two `ComponentBytes` holding the same bytes compile once, and the same
    /// component read from its file is a separate entry.
    ///
    /// The digest names the entry, so separately received copies of one
    /// component share a compilation, as satellites sharing one controller
    /// file do.
    #[test]
    fn the_same_bytes_compile_once_and_a_path_is_its_own_entry() {
        let Some((path, bytes)) = pd_rw_guest() else {
            return;
        };
        let mut cache = WasmPluginCache::new().expect("a cache needs no plugin file");
        let first = ComponentBytes::new(bytes.clone());
        let second = ComponentBytes::new(bytes);
        for (label, component) in [("a", &first), ("b", &second)] {
            cache
                .build_sync_controller_from_bytes_with_streams(
                    component,
                    label,
                    PD_RW_CONFIG,
                    Vec::new(),
                    arika::body::KnownBody::Earth,
                )
                .expect("the pd-rw guest builds from its bytes");
        }
        assert_eq!(cache.sync_plugins.len(), 1, "one digest, one compilation");

        cache
            .build_sync_controller(&path, "c", PD_RW_CONFIG, arika::body::KnownBody::Earth)
            .expect("the pd-rw guest builds from its file");
        assert_eq!(
            cache.sync_plugins.len(),
            2,
            "a path and a digest are separate entries"
        );
    }

    /// A component given as bytes that does not compile is named by digest,
    /// and nothing is read from disk to find out.
    #[test]
    fn bytes_that_do_not_compile_are_named_by_digest() {
        let mut cache = WasmPluginCache::new().expect("a cache needs no plugin file");
        let garbage = ComponentBytes::new(b"\0asm\x0d\x00\x01\x00not a component".to_vec());
        let err = cache
            .build_sync_controller_from_bytes_with_streams(
                &garbage,
                "a",
                "",
                Vec::new(),
                arika::body::KnownBody::Earth,
            )
            .err()
            .expect("garbage after the preamble does not compile");
        let text = err.to_string();
        assert!(
            text.contains(&format!("component sha256 {}", garbage.sha256_hex())),
            "{text}"
        );
        assert!(!text.contains("cannot read WASM"), "{text}");
        assert!(
            cache.sync_plugins.is_empty(),
            "a failed compile is not kept"
        );
    }
}
