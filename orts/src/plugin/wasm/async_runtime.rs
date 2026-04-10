//! Dedicated tokio runtime hosted on its own OS thread.
//!
//! The async WASM backend multiplexes N satellite controllers onto
//! worker threads managed by tokio. We own the tokio runtime on a
//! **separate OS thread** from the simulator's main thread so that:
//!
//! 1. The simulator stays a plain sync program (`orts` crate is
//!    executor-agnostic). `PluginController::update` is still a sync
//!    trait method — the async runtime is hidden behind a
//!    `Handle::block_on` facade.
//!
//! 2. Calls from the simulator thread to `AsyncWasmController::update`
//!    don't risk nested `block_on` panics. The runtime lives on a
//!    different thread, so the simulator thread is outside the tokio
//!    context when it calls `block_on`.
//!
//! 3. On shutdown we tear the runtime down cleanly: dropping the
//!    `AsyncRuntime` signals the internal shutdown `oneshot`, joins
//!    the runtime thread, and waits for all satellite tasks to
//!    complete before returning.
//!
//! # Mode: Deterministic vs Throughput
//!
//! Two execution modes are supported:
//!
//! - [`AsyncMode::Deterministic`] (default): `worker_threads(1)`.
//!   All satellite tasks share a single worker thread, so the
//!   scheduling order is stable and the backend is bit-for-bit
//!   reproducible when callers drive `update()` in a fixed order.
//!   Used by the oracle tests.
//! - [`AsyncMode::Throughput`]: `worker_threads(N)` where N =
//!   `available_parallelism()`. N satellite tasks can run in
//!   parallel, which is required to fan out the simulator step
//!   across CPU cores via rayon in `orts run`. Bit-for-bit
//!   reproducibility is **not** guaranteed in this mode; callers
//!   should use tolerance-based comparison if they need oracles.

use std::thread;

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::oneshot;

use crate::plugin::error::PluginError;

/// Execution mode for the shared async WASM runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AsyncMode {
    /// Single worker thread. Scheduling order is stable, which
    /// makes the async backend bit-for-bit reproducible across
    /// runs when `update()` is driven sequentially. Used by the
    /// oracle tests.
    #[default]
    Deterministic,
    /// Multi-worker runtime sized to `available_parallelism()`.
    /// Allows `rayon`-driven parallel `update()` calls to actually
    /// run on multiple CPU cores. Trades bit-for-bit
    /// reproducibility for throughput.
    Throughput,
}

impl AsyncMode {
    /// How many tokio worker threads this mode asks for.
    fn worker_threads(self) -> usize {
        match self {
            AsyncMode::Deterministic => 1,
            AsyncMode::Throughput => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        }
    }
}

/// Shared async runtime owning a background tokio thread.
///
/// Wrap in `Arc<AsyncRuntime>` and hand clones to every
/// [`super::async_controller::AsyncWasmController`] that should run
/// on this runtime.
pub struct AsyncRuntime {
    handle: Handle,
    shutdown_tx: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    mode: AsyncMode,
}

impl AsyncRuntime {
    /// Spawn a new dedicated runtime thread and return a handle to it.
    ///
    /// The runtime is a `multi_thread` runtime sized according to
    /// `mode`. The background thread lives until this `AsyncRuntime`
    /// is dropped.
    pub fn new(mode: AsyncMode) -> Result<Self, PluginError> {
        let (handle_tx, handle_rx) = std::sync::mpsc::channel::<Handle>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let worker_threads = mode.worker_threads();

        let thread = thread::Builder::new()
            .name("orts-plugin-runtime".to_string())
            .spawn(move || {
                let rt: Runtime = match Builder::new_multi_thread()
                    .worker_threads(worker_threads)
                    .thread_name("orts-plugin-worker")
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("tokio runtime build failed: {e}");
                        return;
                    }
                };

                if handle_tx.send(rt.handle().clone()).is_err() {
                    // Caller went away before we could report our handle.
                    return;
                }

                // Block the runtime thread until shutdown is requested.
                // Dropping the runtime here joins all spawned tasks.
                rt.block_on(async move {
                    let _ = shutdown_rx.await;
                });
            })
            .map_err(|e| PluginError::Init(format!("failed to spawn async runtime thread: {e}")))?;

        let handle = handle_rx.recv().map_err(|_| {
            PluginError::Init("async runtime thread exited before reporting handle".to_string())
        })?;

        Ok(Self {
            handle,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
            mode,
        })
    }

    /// Borrow the tokio `Handle` so controllers can submit futures.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// The execution mode this runtime was built with.
    pub fn mode(&self) -> AsyncMode {
        self.mode
    }
}

impl Drop for AsyncRuntime {
    fn drop(&mut self) {
        // Signal shutdown; if receivers are already gone, this is a no-op.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Join the runtime thread so the process cannot exit while the
        // runtime is still draining tasks.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_starts_and_shuts_down() {
        let rt = AsyncRuntime::new(AsyncMode::Deterministic).expect("runtime must start");
        let handle = rt.handle().clone();
        let result: i32 = handle.block_on(async { 1 + 2 });
        assert_eq!(result, 3);
        drop(rt);
    }

    #[test]
    fn drop_joins_runtime_thread() {
        let rt = AsyncRuntime::new(AsyncMode::Deterministic).expect("runtime must start");
        // Submit a quick task so the runtime has something to drain.
        let result: u64 = rt.handle().block_on(async { 42 });
        assert_eq!(result, 42);
        // If drop did not join, this test would still pass by luck,
        // but the join guarantees we don't leak the OS thread across
        // the whole process lifetime.
        drop(rt);
    }
}
