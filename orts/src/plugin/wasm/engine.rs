//! Shared `wasmtime::Engine` configured for Pulley target execution.
//!
//! A single `WasmEngine` is created per `orts` process and then shared
//! by reference (`Arc<WasmEngine>`) across every satellite controller
//! that needs it. The underlying `wasmtime::Engine` is itself `Send +
//! Sync` and reference-counted internally, so cloning a `WasmEngine`
//! handle is cheap and does not re-initialise Cranelift or the
//! Pulley interpreter tables.
//!
//! ## Deterministic execution
//!
//! The engine is configured with `Config::target("pulley64")`, which
//! tells wasmtime/Cranelift to emit **Pulley bytecode** instead of
//! host-native machine code. Execution then flows through the
//! pure-Rust Pulley interpreter, which does not depend on host
//! register allocation, JIT optimisation passes, or platform-specific
//! SIMD lowering. That is the configuration we rely on to get
//! bit-reproducible plugin behaviour across machines (see DESIGN.md
//! Phase P: "決定論性を config 調整なしで担保").

use wasmtime::{Config, Engine};

use crate::plugin::error::PluginError;

/// Shared wasmtime engine pre-configured for Pulley-target execution.
///
/// `WasmEngine` is intentionally **not** `Clone`. Share an engine
/// across satellite controllers by wrapping it in `Arc<WasmEngine>` —
/// the inner `wasmtime::Engine` is already Arc-based internally, so
/// `Arc<WasmEngine>` is the canonical sharing handle and avoids any
/// confusion between "cheap Arc clone" and "deep value copy".
pub struct WasmEngine {
    inner: Engine,
}

impl WasmEngine {
    /// Create a new engine for the **sync** WASM backend.
    ///
    /// Returns `PluginError::Init` if wasmtime rejects the target
    /// triple (e.g. the binary was compiled without the `pulley`
    /// feature).
    pub fn new_sync() -> Result<Self, PluginError> {
        let mut config = Config::new();
        config
            .target("pulley64")
            .map_err(|err| PluginError::Init(format!("pulley64 target unsupported: {err}")))?;
        let inner = Engine::new(&config)
            .map_err(|err| PluginError::Init(format!("wasmtime Engine::new failed: {err}")))?;
        Ok(Self { inner })
    }

    /// Backwards-compatible alias for [`new_sync`](Self::new_sync).
    pub fn new() -> Result<Self, PluginError> {
        Self::new_sync()
    }

    /// Create a new engine for the **async (fiber)** WASM backend.
    ///
    /// In wasmtime 43 the engine itself has no per-mode flag: async
    /// vs sync invocation is determined by the bindgen variant used
    /// (async bindings call `instantiate_async` / `call_xxx_async`).
    /// We keep a distinct constructor so that the intent is visible
    /// at the call site and so we have a place to add future async
    /// engine tuning (e.g. fuel, epoch deadlines) without disturbing
    /// sync callers.
    #[cfg(feature = "plugin-wasm-async")]
    pub fn new_async() -> Result<Self, PluginError> {
        Self::new_sync()
    }

    /// Access the underlying `wasmtime::Engine`.
    ///
    /// Used by `WasmController` (Phase P1 follow-up) to instantiate
    /// `Component`s and `Store`s against this engine.
    pub fn inner(&self) -> &Engine {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Instance, Module, Store};

    // Minimal core-wasm module that exports `add: (i32, i32) -> i32`.
    // This reuses the Phase P0 smoke-test shape (see
    // `tmp/wasmtime-pulley-smoke/`) but as an inline WAT literal so
    // the test is self-contained and does not depend on any file on
    // disk.
    const ADD_WAT: &str = r#"
        (module
          (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))
    "#;

    #[test]
    fn wasm_engine_compiles_and_runs_pulley_core_module() {
        let engine = WasmEngine::new().expect("Pulley engine must construct on this target");

        // Compile + run a trivial core wasm module through the
        // engine. This validates that:
        // 1. the feature-gated `wasmtime` dependency links,
        // 2. the Pulley target accepts the `wat` input path,
        // 3. Module::new produces a module runnable by the
        //    Pulley interpreter,
        // 4. our `WasmEngine::inner()` hand-off works.
        let module = Module::new(engine.inner(), ADD_WAT).expect("module compile must succeed");
        let mut store = Store::new(engine.inner(), ());
        let instance = Instance::new(&mut store, &module, &[]).expect("instantiation must succeed");
        let add = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "add")
            .expect("module must export add");
        let result = add.call(&mut store, (1, 2)).expect("add must run");
        assert_eq!(result, 3);
    }
}
