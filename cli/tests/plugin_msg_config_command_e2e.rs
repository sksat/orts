//! CLI E2E: `orts run` with a config `[[command]]` timeline.
//!
//! Exercises the config command transport end-to-end through the real
//! binary: TOML parsing (`[[command]]`), `CommandSchedule`, the run
//! loop's per-tick `drain_due` → `controller.deliver`, the guest (FSW)
//! processing the uplinked set-mode command, and the downlink drain
//! (`take_outbound`). Asserts the run completes without error.
//!
//! Behavioral correctness of the command effect (the FSW actually
//! switching mode, accept/reject) is covered by the orts-level E2E
//! `orts/tests/plugin_msg_commandable_mode.rs`; this test guards the
//! CLI wiring (config → schedule → run loop) against regressions.
//!
//! Skips cleanly when the guest wasm has not been built.

#![cfg(feature = "plugin-wasm")]

use std::io::Write;
use std::process::Command;

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

/// Build a temp config with a commandable-mode FSW and a timed set-mode
/// command. Returns `None` if the guest wasm is missing.
fn build_config() -> Option<tempfile::NamedTempFile> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_commandable_mode_ff.wasm"
    ));
    if !wasm_path.exists() {
        eprintln!(
            "WASM not found: {}\nBuild: cd plugin-sdk/examples && cargo +1.91.0 component build -p orts-example-plugin-commandable-mode-ff --release",
            wasm_path.display()
        );
        return None;
    }
    let wasm = wasm_path.display();

    // Near-zero initial rate + a gyroscope so the FSW's nadir gate sees a
    // settled spacecraft and accepts the set-mode command.
    let toml = format!(
        r#"body = "earth"
dt = 0.5
output_interval = 1.0
duration = 3.0
epoch = "2024-01-01T00:00:00Z"

[[satellites]]
id = "commanded"
sensors = ["gyroscope"]

[satellites.orbit]
type = "circular"
altitude = 500

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 100
initial_quaternion = [1, 0, 0, 0]
initial_angular_velocity = [0.001, 0, 0]

[satellites.controller]
type = "wasm"
path = "{wasm}"

[[command]]
t = 1.0
sat = "commanded"
kind = "orts.cmd.set-mode.v1"
args = {{ mode = "nadir" }}
"#
    );

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("create temp config");
    file.write_all(toml.as_bytes()).expect("write config");
    Some(file)
}

#[test]
fn run_with_config_command_completes() {
    let Some(cfg) = build_config() else {
        return;
    };
    let path = cfg.path().to_string_lossy().to_string();

    let output = Command::new(orts_binary())
        .args([
            "run",
            "--config",
            &path,
            "--plugin-backend",
            "sync",
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts");

    assert!(
        output.status.success(),
        "orts run with [[command]] failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    // CSV produced → the controlled simulation actually ran to completion
    // with the command timeline wired in.
    assert!(!output.stdout.is_empty(), "expected CSV output, got none");
}

#[test]
fn run_rejects_command_for_unknown_satellite() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let wasm_path = std::path::PathBuf::from(format!(
        "{manifest_dir}/../plugin-sdk/examples/target/wasm32-wasip1/release/orts_example_plugin_commandable_mode_ff.wasm"
    ));
    if !wasm_path.exists() {
        return;
    }
    let wasm = wasm_path.display();
    // Command targets a satellite id that does not exist → run must fail.
    let toml = format!(
        r#"body = "earth"
dt = 0.5
duration = 1.0
epoch = "2024-01-01T00:00:00Z"

[[satellites]]
id = "commanded"
sensors = ["gyroscope"]

[satellites.orbit]
type = "circular"
altitude = 500

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 100

[satellites.controller]
type = "wasm"
path = "{wasm}"

[[command]]
t = 0.5
sat = "does-not-exist"
kind = "orts.cmd.set-mode.v1"
args = {{ mode = "nadir" }}
"#
    );
    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("temp config");
    file.write_all(toml.as_bytes()).expect("write");
    let path = file.path().to_string_lossy().to_string();

    let output = Command::new(orts_binary())
        .args(["run", "--config", &path, "--plugin-backend", "sync"])
        .output()
        .expect("execute orts");

    assert!(
        !output.status.success(),
        "orts run should fail for a command targeting an unknown satellite"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does-not-exist"),
        "stderr should name the unknown satellite"
    );
}
