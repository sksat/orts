//! CLI E2E: `--plugin-backend=sync` and `--plugin-backend=async`
//! must produce byte-identical CSV output for the same config.
//!
//! Exercises `orts run` end-to-end, including config parsing,
//! WasmPluginCache, backend selection, and the full controlled
//! simulation loop. The guest wasm for pd-rw-control must have been
//! built beforehand; tests skip cleanly if it is missing.

#![cfg(feature = "plugin-wasm-async")]

use std::io::Write;
use std::process::Command;

/// Build a temporary orts TOML config pointing at an absolute path to
/// the pd-rw-control guest. Returns `None` if the guest is missing.
fn build_config() -> Option<tempfile::NamedTempFile> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm"
    ));
    if !wasm_path.exists() {
        eprintln!(
            "WASM not found: {}\nBuild: cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-pd-rw-control --release",
            wasm_path.display()
        );
        return None;
    }
    let wasm_path_str = wasm_path.display().to_string();

    let toml = format!(
        r#"body = "earth"
dt = 0.1
output_interval = 1.0
duration = 10.0
epoch = "2024-01-01T00:00:00Z"

[[satellites]]
id = "controlled-sat"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500
initial_quaternion = [0.966, 0, 0.259, 0]
initial_angular_velocity = [0.02, -0.01, 0.015]

[satellites.controller]
type = "wasm"
path = "{wasm_path_str}"

[satellites.controller.config]
kp = 1.0
kd = 2.0
sample_period = 0.1

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_momentum = 1.0
max_torque = 0.5
"#
    );

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("tempfile");
    file.write_all(toml.as_bytes()).expect("write toml");
    Some(file)
}

/// Resolve the `orts` binary path.
///
/// Checks `ORTS_BIN` first so CI jobs can inject a pre-built binary
/// from an artifact (mirrors the viewer-e2e pattern: download the
/// `orts` artifact, then run this test with `ORTS_BIN=./bin/orts`).
/// Falls back to `CARGO_BIN_EXE_orts`, which Cargo fills in when
/// running `cargo test -p orts-cli --test plugin_backend_e2e`
/// locally.
fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

fn run_cli(config_path: &str, backend: &str) -> Vec<u8> {
    let binary = orts_binary();
    let output = Command::new(&binary)
        .args([
            "run",
            "--config",
            config_path,
            "--plugin-backend",
            backend,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {binary}: {e}"));
    assert!(
        output.status.success(),
        "orts run --plugin-backend={backend} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn plugin_backend_sync_matches_async_csv_output() {
    let Some(cfg) = build_config() else {
        return;
    };
    let path = cfg.path().to_string_lossy().to_string();

    let sync_out = run_cli(&path, "sync");
    let async_out = run_cli(&path, "async");

    assert!(!sync_out.is_empty(), "sync output empty");
    assert_eq!(
        sync_out, async_out,
        "CSV output must be byte-identical between sync and async backends"
    );
}
