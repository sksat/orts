//! Smoke test: verify `Handle::block_on` works from inside a
//! `spawn_blocking` closure on a different runtime.
//!
//! This is the scenario created by `serve`: its sim loop is moved to
//! `spawn_blocking` on the serve runtime (Commit 1), and inside that
//! closure it calls `PluginController::update` on `AsyncWasmController`
//! which internally uses `Handle::block_on` on the **plugin** runtime
//! (different runtime instance).
//!
//! If this panics, the async backend is unusable inside serve and we
//! have to refactor the outer controller to avoid `block_on`
//! altogether (e.g. std::sync::mpsc + spawn_blocking bridge).

#![cfg(feature = "plugin-wasm-async")]

use tokio::runtime::Builder;

#[test]
fn block_on_cross_runtime_from_spawn_blocking() {
    // rt_outer simulates serve's tokio runtime.
    let rt_outer = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("outer runtime");

    // rt_inner simulates the plugin AsyncRuntime (worker_threads=1).
    let rt_inner = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("inner runtime");
    let inner_handle = rt_inner.handle().clone();

    // Outer: run an async task. Inside, spawn_blocking. Inside that
    // blocking closure, call Handle::block_on on rt_inner.
    let result = rt_outer.block_on(async move {
        tokio::task::spawn_blocking(move || {
            // This is the critical call: cross-runtime block_on from
            // a spawn_blocking thread spawned by a different runtime.
            inner_handle.block_on(async { 42 })
        })
        .await
        .expect("spawn_blocking must not panic")
    });

    assert_eq!(result, 42);
}

/// Calling `Handle::block_on` directly from within an async task
/// (NOT spawn_blocking) panics. This is the scenario that callers
/// must avoid when using `AsyncWasmController`.
#[test]
#[should_panic(expected = "Cannot start a runtime from within a runtime")]
fn block_on_from_async_task_panics() {
    let rt_outer = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("outer runtime");

    let rt_inner = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("inner runtime");
    let inner_handle = rt_inner.handle().clone();

    rt_outer.block_on(async move {
        // This panics: block_on from within a tokio task of any runtime.
        let _ = inner_handle.block_on(async { 42 });
    });
}
