//! CLI E2E: WASM plugin から出力された thruster throttle command が
//! `orts run` の controlled simulation loop で実際に軌道に反映されるか。
//!
//! `transfer-burn-with-tcm` plugin を build しておく必要がある。未 build の
//! 場合はテストを skip する (既存の `plugin_backend_e2e` と同じパターン)。
//!
//! このテストは sim 時間 (100 s) 内で遷移 burn が完了しないパラメータを
//! 選び、FirstBurn 中の continuous thrust で軌道速度に意味のある差分が
//! 現れることを検証する。完全な Hohmann + TCM シーケンスの動作確認は
//! README の長時間 demo に譲る（姿勢トラッキングが別途必要になるため、
//! E2E では扱わない）。

#![cfg(feature = "plugin-wasm")]

use std::io::Write;
use std::process::Command;

const THRUSTER_WASM_REL: &str = "/../plugin-sdk/examples/target/wasm32-wasip2/release/orts_example_plugin_transfer_burn_with_tcm.wasm";

fn wasm_path() -> Option<std::path::PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let p = std::path::PathBuf::from(format!("{manifest_dir}{THRUSTER_WASM_REL}"));
    if p.exists() { Some(p) } else { None }
}

fn orts_binary() -> String {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return path;
    }
    option_env!("CARGO_BIN_EXE_orts")
        .map(str::to_owned)
        .expect("neither ORTS_BIN nor CARGO_BIN_EXE_orts is set")
}

fn build_config(wasm_path: &std::path::Path, with_thruster: bool) -> tempfile::NamedTempFile {
    let wasm = wasm_path.display();
    let thruster_block = if with_thruster {
        r#"
[satellites.thruster]
dry_mass = 400.0

[[satellites.thruster.thrusters]]
thrust_n = 10.0
isp_s = 230.0
direction_body = [1.0, 0.0, 0.0]
"#
    } else {
        ""
    };

    let toml = format!(
        r#"body = "earth"
dt = 0.1
output_interval = 10.0
duration = 100.0
epoch = "2024-01-01T00:00:00Z"

[[satellites]]
id = "thrust-sat"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = 500

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500
initial_quaternion = [1.0, 0, 0, 0]
initial_angular_velocity = [0, 0, 0]

[satellites.controller]
type = "wasm"
path = "{wasm}"

[satellites.controller.config]
# 遷移先高度を十分高くして、100 s では FirstBurn を抜けない設定にする。
target_altitude_km = 10000.0
mu_km3_s2 = 398600.4418
deadband_km = 1.0
num_thrusters = 1
sample_period = 1.0
{thruster_block}
"#
    );

    let mut file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("tempfile");
    file.write_all(toml.as_bytes()).expect("write toml");
    file
}

fn run_cli(config_path: &str) -> String {
    let binary = orts_binary();
    let output = Command::new(&binary)
        .args([
            "run",
            "--config",
            config_path,
            "--output",
            "stdout",
            "--format",
            "csv",
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {binary}: {e}"));
    assert!(
        output.status.success(),
        "orts run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is valid utf-8")
}

/// Parse the last CSV row's vx column (index 4) from `orts run` output.
fn last_vx(csv: &str) -> f64 {
    let last = csv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("t["))
        .next_back()
        .expect("at least one data row");
    let cols: Vec<&str> = last.split(',').collect();
    cols[4]
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("vx not a float: {:?}", cols[4]))
}

/// Thrust plugin must cause a detectable velocity delta compared to
/// an otherwise-identical no-thruster run.
///
/// Setup: +X body-frame thruster, 10 N, 500 kg, identity attitude,
/// 100 s run. Plugin stays in FirstBurn throughout (target 10000 km
/// alt is unreachable in the test window), so it fires continuously
/// at throttle=1. Expected Δvx ≈ F/m × t_effective. Since the first
/// tick emits at t=sample_period=1s, effective burn ≈ 99 s →
/// Δv ≈ 0.02 m/s² × 99 s ≈ 1.98 m/s ≈ 1.98e-3 km/s.
///
/// 上下両方 assert しておく (片側だけだと「10 倍推力」「二重適用」を
/// 見逃すため — smart-friend review 指摘)。
#[test]
fn thruster_plugin_changes_velocity() {
    let Some(wasm) = wasm_path() else {
        eprintln!(
            "SKIP: transfer-burn-with-tcm wasm missing. Build with:\n  \
             cd plugin-sdk/examples && cargo build -p orts-example-plugin-transfer-burn-with-tcm \
             --target wasm32-wasip2 --release"
        );
        return;
    };

    let cfg_thrust = build_config(&wasm, true);
    let cfg_no_thrust = build_config(&wasm, false);

    let csv_thrust = run_cli(&cfg_thrust.path().to_string_lossy());
    let csv_no_thrust = run_cli(&cfg_no_thrust.path().to_string_lossy());

    let vx_thrust = last_vx(&csv_thrust);
    let vx_no_thrust = last_vx(&csv_no_thrust);
    let dvx = vx_thrust - vx_no_thrust;

    // 期待値 ≈ 1.98e-3 km/s。上下幅で sanity check。
    assert!(
        (1.5e-3..2.5e-3).contains(&dvx),
        "expected Δvx around 2e-3 km/s from constant +X thrust (FirstBurn phase), got {dvx:.6e} km/s\n\
         vx_thrust={vx_thrust:.6}, vx_no_thrust={vx_no_thrust:.6}"
    );
}
