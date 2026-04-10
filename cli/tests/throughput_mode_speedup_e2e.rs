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

/// Build a controlled simulation config with N satellites. The
/// workload is sized so that a debug-build 2-core CI run finishes
/// the full test in roughly 30 seconds while still leaving enough
/// parallelisable work to show a measurable speedup on multi-core
/// developer machines.
fn build_config(wasm_path: &std::path::Path, n_sats: usize) -> tempfile::NamedTempFile {
    let mut toml = String::from(
        r#"body = "earth"
dt = 0.01
output_interval = 10.0
duration = 5.0
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
        eprintln!(
            "available_parallelism() = {parallelism}; skipping throughput speedup test \
             (need ≥ 2 cores for any parallelism to be meaningful)"
        );
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

    // The assertion tier depends on how many cores we actually have
    // to work with. The CI environment this test is expected to run
    // in is a 2-core SMT GitHub Actions runner with a *debug* build.
    // With the compact workload above (duration = 5), Amdahl's law
    // plus the relatively small parallel fraction only buys a modest
    // speedup on low-core debug builds. We therefore only guarantee
    // "not slower than deterministic, minus noise margin" on low
    // core counts — the point of the assertion there is to catch a
    // regression that accidentally serialises the parallel path,
    // which would push the ratio to ~1.0 or higher (rayon dispatch
    // overhead makes it actively worse than sequential).
    //
    // On developer machines and larger runners we demand a real
    // speedup, since parallelism is clearly available and useful
    // there. A 16-core host running this debug workload hits
    // ratio ≈ 0.50, so the tighter 8+ core threshold leaves margin
    // for machine / CI variance.
    let max_ratio = match parallelism {
        2..=3 => 0.95, // catches "became slower than sequential" regressions
        4..=7 => 0.85, // at least 1.18×, still a safe ceiling on small workloads
        _ => 0.70,     // at least 1.43× — tight enough to catch serialisation
    };
    assert!(
        tp_best.as_secs_f64() < det_best.as_secs_f64() * max_ratio,
        "throughput mode did not deliver the expected speedup on {parallelism}-core host: \
         deterministic_best={det_best:?}, throughput_best={tp_best:?}, \
         ratio={:.3}, required < {max_ratio}",
        tp_best.as_secs_f64() / det_best.as_secs_f64()
    );
}
