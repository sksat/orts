//! E2E test: `--plugin-backend-async-mode=throughput` is measurably
//! faster than `deterministic` for a controlled simulation with many
//! satellites.
//!
//! This is a **regression guard**: the throughput mode's whole reason
//! to exist is that `run_controlled_simulation` fans the per-satellite
//! `step_controlled` calls out across CPU cores via rayon. If someone
//! later accidentally serializes that path (e.g. by locking a shared
//! resource), this test should catch it.
//!
//! The test runs `orts run` twice against the same config, once per
//! mode, and asserts that throughput completes in at most 80 % of
//! the deterministic wall-clock time. That leaves plenty of slack
//! for 2-core CI runners; the real speedup on a 16-core host is
//! closer to 7×.
//!
//! Skips cleanly when:
//! - the pd-rw-control guest WASM is not built
//! - the host reports fewer than 2 available cores (so parallelism
//!   gives nothing to measure against)

#![cfg(feature = "plugin-wasm-async")]

use std::io::Write;
use std::process::Command;
use std::time::Instant;

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

fn pd_rw_guest_wasm() -> Option<std::path::PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm"
    ));
    if wasm_path.exists() {
        Some(wasm_path)
    } else {
        eprintln!(
            "WASM not found: {}\nSkipping throughput-speedup E2E.",
            wasm_path.display()
        );
        None
    }
}

/// Build a controlled simulation config with N satellites. Deliberately
/// uses a fine `dt` and a long-enough duration so that the
/// parallelisable per-tick work dominates the one-shot startup cost
/// (Cranelift compile, task spawn). Otherwise Amdahl's law limits
/// the observable speedup on low-core-count CI runners.
fn build_config(wasm_path: &std::path::Path, n_sats: usize) -> tempfile::NamedTempFile {
    let mut toml = String::from(
        r#"body = "earth"
dt = 0.01
output_interval = 10.0
duration = 20.0
epoch = "2024-01-01T00:00:00Z"
"#,
    );
    for i in 0..n_sats {
        let alt = 400 + i;
        toml.push_str(&format!(
            r#"
[[satellites]]
id = "sat-{i}"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = {alt}

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500
initial_quaternion = [0.966, 0, 0.259, 0]
initial_angular_velocity = [0.02, -0.01, 0.015]

[satellites.controller]
type = "wasm"
path = "{path}"

[satellites.controller.config]
kp = 1.0
kd = 2.0
sample_period = 0.1

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_momentum = 1.0
max_torque = 0.5
"#,
            path = wasm_path.display()
        ));
    }

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("tempfile");
    file.write_all(toml.as_bytes()).expect("write toml");
    file
}

fn run_once(config_path: &str, mode: &str) -> (std::time::Duration, Vec<u8>) {
    let binary = orts_binary();
    let start = Instant::now();
    let output = Command::new(&binary)
        .args([
            "run",
            "--config",
            config_path,
            "--plugin-backend",
            "async",
            "--plugin-backend-async-mode",
            mode,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {binary}: {e}"));
    let elapsed = start.elapsed();
    assert!(
        output.status.success(),
        "orts run --plugin-backend-async-mode={mode} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    (elapsed, output.stdout)
}

#[test]
fn throughput_mode_is_faster_than_deterministic() {
    let Some(wasm_path) = pd_rw_guest_wasm() else {
        return;
    };
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if parallelism < 2 {
        eprintln!("available_parallelism() = {parallelism}; skipping throughput speedup test");
        return;
    }

    let cfg = build_config(&wasm_path, 50);
    let path = cfg.path().to_string_lossy().to_string();

    // Warm up cargo cache / Cranelift compile / filesystem so the
    // first invocation does not pay an outsized cost.
    let _warmup = run_once(&path, "deterministic");

    // Take the **best** (minimum) wall time from a few runs per mode
    // instead of a single measurement. Shared CI runners are noisy
    // — CPU throttling, neighbouring jobs, and scheduler jitter can
    // all inflate a single sample by 2× or more. `min` is the
    // conventional noise-reducing statistic for timing tests because
    // contention can only push a run slower, never faster.
    const RUNS: usize = 3;
    let mut det_times = Vec::with_capacity(RUNS);
    let mut tp_times = Vec::with_capacity(RUNS);
    let mut det_out = Vec::new();
    let mut tp_out = Vec::new();
    for _ in 0..RUNS {
        let (t, out) = run_once(&path, "deterministic");
        det_times.push(t);
        det_out = out;
    }
    for _ in 0..RUNS {
        let (t, out) = run_once(&path, "throughput");
        tp_times.push(t);
        tp_out = out;
    }
    let det_best = *det_times.iter().min().unwrap();
    let tp_best = *tp_times.iter().min().unwrap();
    eprintln!("deterministic runs: {det_times:?}, best={det_best:?}");
    eprintln!("throughput runs:    {tp_times:?}, best={tp_best:?}");
    eprintln!(
        "throughput_best / deterministic_best = {:.3}",
        tp_best.as_secs_f64() / det_best.as_secs_f64()
    );

    // Each satellite's `step_controlled` is independent (no shared
    // mutable state between satellites), so parallelising the outer
    // loop must not change the floating-point result. This keeps
    // throughput mode usable wherever determinism matters, and
    // catches any future regression that accidentally introduces
    // order-sensitive side effects.
    assert_eq!(
        det_out, tp_out,
        "deterministic and throughput modes must produce byte-identical CSV output; \
         any divergence means someone introduced cross-satellite shared state"
    );

    // Expected speedup depends on core count. Amdahl's law puts the
    // parallelisable fraction of this workload at roughly 80 %
    // (measured locally with `duration = 20`), which gives:
    //
    //   2 cores → ~1.67× (ratio ~0.60)
    //   4 cores → ~2.50× (ratio ~0.40)
    //   8 cores → ~3.33× (ratio ~0.30)
    //  16 cores → ~4.00× (ratio ~0.25)
    //
    // We require "at least 2×" whenever the host has ≥ 4 cores, and
    // a looser "at least 1.4×" on 2-3 core runners so that the GitHub
    // Actions free-tier runner (2 cores) still gets a meaningful
    // assertion. CI scheduler noise is absorbed by the min-of-N
    // sampling above.
    let max_ratio = if parallelism >= 4 { 0.50 } else { 0.70 };
    assert!(
        tp_best.as_secs_f64() < det_best.as_secs_f64() * max_ratio,
        "throughput mode did not deliver the expected speedup on {parallelism}-core host: \
         deterministic_best={det_best:?}, throughput_best={tp_best:?}, \
         ratio={:.3}, required < {max_ratio}",
        tp_best.as_secs_f64() / det_best.as_secs_f64()
    );
}
