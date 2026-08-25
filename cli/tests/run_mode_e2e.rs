//! E2E tests for `orts run`'s dynamics-mode selection.
//!
//! `run` and `serve` share one mode rule (`crate::sim::mode::select_sim_mode`),
//! so a config that gets attitude dynamics under `serve` gets them under `run`
//! too, and a config no mode can honor is rejected instead of being partly
//! ignored.

use std::process::Command;

fn orts() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orts"))
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "orts-e2e-run-mode-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `config` into a fresh directory and run `orts run --config … --format csv`
/// with the data on stdout. Returns the process output.
fn run_config(tag: &str, config: &str) -> std::process::Output {
    let dir = unique_dir(tag);
    let path = dir.join("orts.toml");
    std::fs::write(&path, config).unwrap();
    let out = orts()
        .args([
            "run",
            "--config",
            path.to_str().unwrap(),
            "--output",
            "-",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts");
    std::fs::remove_dir_all(&dir).ok();
    out
}

fn header_line(stdout: &str) -> &str {
    stdout
        .lines()
        .find(|l| l.starts_with("# t[s]"))
        .unwrap_or_else(|| panic!("no CSV header in output:\n{stdout}"))
}

fn data_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect()
}

/// The README quick-start config: `[satellites.attitude]` + `sensors` +
/// `[satellites.reaction_wheels]`, no controller. It must parse, run, and
/// output the propagated attitude — before this it silently produced an
/// orbit-only CSV.
#[test]
fn readme_quickstart_config_outputs_attitude() {
    let config = r#"
body = "earth"
dt = 0.01
duration = 120.0

[[satellites]]
id = "sat-1"
sensors = ["gyroscope", "star_tracker"]

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10, 10, 10]
mass = 500

[satellites.reaction_wheels]
type = "three_axis"
inertia = 0.01
max_momentum = 1.0
max_torque = 0.5
"#;
    let out = run_config("readme", config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stderr}");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let header = header_line(&stdout);
    for col in ["qw", "qx", "qy", "qz", "wx", "wy", "wz"] {
        assert!(
            header.split(',').any(|c| c == col),
            "attitude column '{col}' missing from header: {header}"
        );
    }

    // Every data row carries the attitude columns as well.
    let ncols = header.trim_start_matches("# ").split(',').count();
    for line in data_lines(&stdout).iter().take(3) {
        assert_eq!(
            line.split(',').count(),
            ncols,
            "row does not match the header width: {line}"
        );
    }

    // `sensors` / `reaction_wheels` need a controller; the run says so rather
    // than dropping them silently.
    assert!(
        stderr.contains("`sensors`") && stderr.contains("`reaction_wheels`"),
        "no warning about the un-honored keys: {stderr}"
    );
}

/// The propagated quaternion is a real integration result, not a logged
/// constant: with an asymmetric inertia tensor the coupled gravity-gradient
/// torque spins the body up, and the quaternion stays a unit quaternion.
#[test]
fn spacecraft_mode_integrates_attitude() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 3000.0
output_interval = 100.0

[[satellites]]
id = "sat-1"

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10.0, 40.0, 80.0]
mass = 500
initial_quaternion = [0.9238795325112867, 0.0, 0.3826834323650898, 0.0]
initial_angular_velocity = [0.0, 0.0, 0.0]
"#;
    let out = run_config("gg", config);
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let header = header_line(&stdout);
    let cols: Vec<&str> = header.trim_start_matches("# ").split(',').collect();
    let idx = |name: &str| {
        cols.iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("column '{name}' missing: {header}"))
    };
    let (iqw, iqx, iqy, iqz) = (idx("qw"), idx("qx"), idx("qy"), idx("qz"));
    let (iwx, iwy, iwz) = (idx("wx"), idx("wy"), idx("wz"));

    let rows = data_lines(&stdout);
    assert!(rows.len() > 10, "expected many rows, got {}", rows.len());

    let field =
        |line: &str, i: usize| -> f64 { line.split(',').nth(i).unwrap().trim().parse().unwrap() };

    // Unit-norm quaternion is an invariant of rigid-body attitude
    // integration; it holds without any external reference data.
    let mut max_w = 0.0_f64;
    let mut max_dq = 0.0_f64;
    let first = rows[0];
    for line in &rows {
        let norm = (field(line, iqw).powi(2)
            + field(line, iqx).powi(2)
            + field(line, iqy).powi(2)
            + field(line, iqz).powi(2))
        .sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-6,
            "quaternion norm {norm} drifted off 1: {line}"
        );
        max_w = max_w.max(
            field(line, iwx)
                .abs()
                .max(field(line, iwy).abs())
                .max(field(line, iwz).abs()),
        );
        for i in [iqw, iqx, iqy, iqz] {
            max_dq = max_dq.max((field(line, i) - field(first, i)).abs());
        }
    }
    // Gravity gradient torques an asymmetric body away from rest.
    assert!(
        max_w > 1e-6,
        "angular velocity never left zero (max |w| = {max_w}); the attitude \
         state is not being integrated"
    );
    assert!(
        max_dq > 1e-3,
        "quaternion never moved (max delta = {max_dq})"
    );
}

/// An orbit-only config keeps the 13-column CSV contract that `orts convert`
/// and the viewer consume. Pins the mode rule against widening: a config
/// without `[satellites.attitude]` must not fall into spacecraft mode.
#[test]
fn orbit_only_config_keeps_thirteen_columns() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "sat-1"

[satellites.orbit]
type = "circular"
altitude = 400
"#;
    let out = run_config("orbit-only", config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        header_line(&stdout),
        "# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]",
    );
    for line in data_lines(&stdout) {
        assert_eq!(
            line.split(',').count(),
            13,
            "orbit-only row is not 13 columns: {line}"
        );
    }
    assert!(
        !stderr.contains("Warning:"),
        "a plain orbit-only config should not warn: {stderr}"
    );
}

/// `[[command]]` entries are delivered by the control loop, so a fleet with no
/// controller cannot honor them. Rejected instead of dropped.
#[test]
fn command_timeline_without_controller_is_rejected() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "sat-1"

[satellites.orbit]
type = "circular"
altitude = 400

[[command]]
t = 10.0
sat = "sat-1"
kind = "orts.cmd.set-mode.v1"
args = { mode = "safe" }
"#;
    let out = run_config("cmd-no-ctrl", config);
    assert!(
        !out.status.success(),
        "a command timeline without a controller must fail, stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[[command]]") && stderr.contains("controller"),
        "unhelpful rejection message: {stderr}"
    );
}

/// stream-io streams are pumped by `orts serve`'s realtime loop; `orts run`
/// has no transport for them, so declaring one is rejected instead of leaving
/// the endpoints dead.
#[test]
fn declared_streams_are_rejected_by_run() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400 }
streams = ["comlink"]
"#;
    let out = run_config("streams", config);
    assert!(
        !out.status.success(),
        "declared streams must fail under run, stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("orts serve") && stderr.contains("streams"),
        "unhelpful rejection message: {stderr}"
    );
}

/// Attitude has to be all-or-nothing across the fleet — the same rule `serve`
/// enforces at engine construction.
#[test]
fn mixed_attitude_config_is_rejected() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "with-attitude"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }

[[satellites]]
id = "without"
orbit = { type = "circular", altitude = 500 }
"#;
    let out = run_config("mixed-att", config);
    assert!(!out.status.success(), "mixed attitude must be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Mixed attitude config"),
        "unexpected message: {stderr}"
    );
}

/// The control loop steps the whole fleet or none of it, so a controller on
/// only some satellites would never run. Rejected rather than ignored.
#[test]
fn mixed_controller_config_is_rejected() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "controlled"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
controller = { type = "wasm", path = "does-not-exist.wasm" }

[[satellites]]
id = "uncontrolled"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
"#;
    let out = run_config("mixed-ctrl", config);
    assert!(!out.status.success(), "mixed controller must be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Mixed controller config"),
        "unexpected message: {stderr}"
    );
}

/// `--json` reports the un-honored keys in the machine-readable summary, not
/// only on stderr, so an agent driving the CLI can see them.
#[test]
fn json_summary_lists_unhonored_keys() {
    let dir = unique_dir("json-warn");
    let config_path = dir.join("orts.toml");
    let csv_path = dir.join("out.csv");
    std::fs::write(
        &config_path,
        r#"
body = "earth"
dt = 1.0
duration = 60.0

[[satellites]]
id = "sat-1"
sensors = ["gyroscope"]
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [10, 10, 10], mass = 500 }
"#,
    )
    .unwrap();

    let out = orts()
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--output",
            csv_path.to_str().unwrap(),
            "--format",
            "csv",
            "--json",
        ])
        .output()
        .expect("failed to execute orts");
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is the JSON summary");
    let warnings = summary["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "got: {warnings:?}");
    assert!(
        warnings[0].as_str().unwrap().contains("`sensors`"),
        "got: {warnings:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
