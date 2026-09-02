//! E2E tests for `orts run`'s dynamics-mode selection.
//!
//! `run` and `serve` share one mode rule (`crate::sim::mode::select_sim_mode`),
//! so a config that gets attitude dynamics under `serve` gets them under `run`
//! too, and a config no mode can honor is rejected instead of being partly
//! ignored.

use std::process::Command;

/// Honor `ORTS_BIN` so the plugin-backend CI job, which downloads a prebuilt
/// binary instead of building one, can run these tests too.
fn orts() -> Command {
    if let Ok(path) = std::env::var("ORTS_BIN") {
        return Command::new(path);
    }
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

/// `[satellites.attitude]` + `sensors` + `[satellites.reaction_wheels]` with no
/// controller must parse, run, and output the propagated attitude — before this
/// it silently produced an orbit-only CSV. The README's quick start adds a
/// controller on top of this shape; `readme_quickstart_config_runs_controlled`
/// covers that config as it ships.
#[test]
fn attitude_without_controller_outputs_attitude() {
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
///
/// `diag(10, 40, 45)` satisfies `I1 + I2 >= I3` with 11% to spare, so it is a
/// tensor some mass distribution has. The initial 45° about y puts the body's
/// radial direction at `(1/√2, 0, 1/√2)`, where the difference between `Ix` and
/// `Iz` produces the largest gravity-gradient torque: `|τy| = 6.7e-5 N·m` and
/// `|ω̇y| = 1.7e-6 rad/s²` at `t = 0`. The span is short on purpose — the
/// thresholds below are cleared within the first hundred seconds, and a longer
/// run would fold the nonlinear response into the same assertion.
#[test]
fn spacecraft_mode_integrates_attitude() {
    let config = r#"
body = "earth"
dt = 1.0
duration = 200.0
output_interval = 10.0

[[satellites]]
id = "sat-1"

[satellites.orbit]
type = "circular"
altitude = 400

[satellites.attitude]
inertia_diag = [10.0, 40.0, 45.0]
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

/// Extract the README quick-start TOML block, with the controller's cwd-relative
/// `path` rewritten to an absolute one so the test does not depend on its cwd.
///
/// Reading the README rather than restating it is deliberate: the quick-start
/// config has shipped un-runnable before, once for a missing required field,
/// which a copy in this file would not have caught.
fn readme_quickstart_toml(wasm: &std::path::Path) -> String {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md"))
        .expect("README.md is readable");
    let after = readme
        .split_once("Example config (`orts.toml`)")
        .expect("README has the quick-start config section")
        .1;
    let block = after
        .split("```toml")
        .nth(1)
        .expect("a toml code block follows")
        .split("```")
        .next()
        .expect("the toml block is closed");
    let mut rewritten = 0;
    let toml = block
        .lines()
        .map(|l| {
            let key = l.split('=').next().unwrap_or_default().trim();
            if key == "path" {
                rewritten += 1;
                format!("path = {:?}", wasm.to_str().unwrap())
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Without this the relative path in the README would resolve by accident
    // when the tests happen to run from the repository root, and the rewrite
    // silently covering nothing would look like a pass.
    assert_eq!(
        rewritten, 1,
        "expected exactly one `path` to rewrite in the quick-start config: {toml}"
    );
    toml
}

/// Resolve the pd-rw-control guest WASM the README points at, or `None` if it
/// has not been built.
fn pd_rw_guest_wasm() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../plugin-sdk/examples/target/wasm32-wasip1/release/",
        "orts_example_plugin_pd_rw_control.wasm"
    ));
    if path.exists() {
        return Some(path);
    }
    eprintln!(
        "WASM not found: {}\n\
         Build: cargo +1.91.0 component build --release \
         --manifest-path plugin-sdk/examples/pd-rw-control/Cargo.toml\n\
         Skipping the README quick-start e2e test.",
        path.display()
    );
    None
}

/// A `[satellites.controller]` with no `config` table starts the guest on its
/// own defaults. The omitted table arrives as JSON `null`, whose `to_string()`
/// is `"null"` — which the guest cannot parse as its config struct however many
/// `#[serde(default)]`s it carries.
#[test]
fn controller_without_config_table_uses_guest_defaults() {
    let Some(wasm) = pd_rw_guest_wasm() else {
        return;
    };
    let config = format!(
        r#"
body = "earth"
dt = 0.1
duration = 10.0

[[satellites]]
id = "sat-1"
sensors = ["gyroscope", "star_tracker"]
orbit = {{ type = "circular", altitude = 400 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 500 }}
reaction_wheels = {{ type = "three_axis", inertia = 0.01, max_momentum = 1.0, max_torque = 0.5 }}

[satellites.controller]
type = "wasm"
path = {:?}
"#,
        wasm.to_str().unwrap()
    );
    let out = run_config("ctrl-no-config", &config);
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A large unnormalized quaternion runs to completion, with a finite attitude
/// throughout.
///
/// What keeps it finite is the load-time normalization in
/// `AttitudeConfig::normalized_initial_quaternion`: integrated raw, a
/// quaternion this large grows until its sum of squares overflows, and every
/// attitude row from that point on is meaningless while the run still reports
/// success. Replacing that division by a pass-through fails this test on the
/// first data row, so that is the behaviour it pins — the `project` guard in
/// `OdeState` is reached with a finite norm here and is covered by the unit
/// tests beside it.
///
/// The span is short on purpose. `initial_angular_velocity` of 1e4 rad/s is
/// what makes this expensive: the adaptive solver sizes its steps to that rate,
/// so cost scales with the simulated span, and 30 s of it took over six minutes
/// in CI. Two seconds gives 21 rows, twice what the row-count assertion below
/// wants, and detection is immediate — 23 ms with the normalization dropped.
#[test]
fn a_large_unnormalized_quaternion_still_integrates() {
    let config = r#"
body = "earth"
dt = 0.1
duration = 2.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [1, 1, 1], mass = 500, initial_quaternion = [1e150, 0, 0, 0], initial_angular_velocity = [1e4, 0, 0] }
"#;
    let out = run_config("huge-quat", config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stderr}");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let header = header_line(&stdout);
    let cols: Vec<&str> = header.trim_start_matches("# ").split(',').collect();
    // Looked up by name one at a time: the quaternion columns are not
    // necessarily adjacent, so slicing four wide from `qw` reads whatever
    // happens to follow it.
    let quat: Vec<usize> = ["qw", "qx", "qy", "qz"]
        .iter()
        .map(|name| {
            cols.iter()
                .position(|c| c == name)
                .unwrap_or_else(|| panic!("column '{name}' missing: {header}"))
        })
        .collect();

    let mut rows = 0;
    for line in data_lines(&stdout) {
        let fields: Vec<&str> = line.split(',').collect();
        let norm_sq: f64 = quat
            .iter()
            .map(|i| {
                let f = fields[*i];
                let v: f64 = f.parse().unwrap_or_else(|e| panic!("{f:?}: {e}"));
                assert!(v.is_finite(), "non-finite quaternion component in: {line}");
                v * v
            })
            .sum();
        // The failure mode this pins is the all-zero quaternion: integrated
        // raw, the components grow until their sum of squares overflows and the
        // post-step projection divides each by infinity. How closely the norm
        // tracks 1 is a separate question — the adaptive solvers do not project
        // between steps — so this asks only that the attitude still names one.
        // `is_finite` on the sum too: each component can be finite while their
        // squares overflow, and `inf > 0.5` would wave that through.
        assert!(
            norm_sq.is_finite() && norm_sq > 0.5,
            "quaternion no longer names an attitude (norm² = {norm_sq}) in: {line}"
        );
        rows += 1;
    }
    assert!(
        rows > 10,
        "expected the whole span to be recorded, got {rows}"
    );
}

/// An attitude config that cannot be integrated is rejected before the run
/// starts, in controlled mode too.
///
/// The controlled path builds its satellites inside
/// `build_controlled_satellite` rather than through the spacecraft path, so a
/// check placed in the latter left this entry point uncovered.
///
/// The controller path is never loaded — the config is refused first — so this
/// names a `.wasm` that does not exist rather than requiring the guest to be
/// built. That keeps the test running everywhere: were the check removed, the
/// run would fail on the missing plugin instead, with a different message and
/// exit code, and the assertions below would still catch it.
#[test]
fn controlled_run_rejects_unintegrable_attitude() {
    let config = r#"
body = "earth"
dt = 0.1
duration = 10.0

[[satellites]]
id = "sat-1"
orbit = { type = "circular", altitude = 400 }
attitude = { inertia_diag = [nan, 10, 10], mass = 500 }

[satellites.controller]
type = "wasm"
path = "definitely-not-a-plugin.wasm"
"#;
    let out = run_config("ctrl-nan-inertia", config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "run should have failed: {stderr}");
    assert!(
        stderr.contains("attitude") && stderr.contains("inertia_diag"),
        "got: {stderr}"
    );
}

/// The README quick-start config, as it ships, runs in controlled mode: the
/// wheels are commanded, so nothing is reported as un-honored.
#[test]
fn readme_quickstart_config_runs_controlled() {
    let Some(wasm) = pd_rw_guest_wasm() else {
        return;
    };
    let config = readme_quickstart_toml(&wasm);
    assert!(
        config.contains("[satellites.controller]"),
        "the README quick start no longer declares a controller: {config}"
    );

    let out = run_config("readme-quickstart", &config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stderr}");
    assert!(
        !stderr.contains("declared without `[satellites.controller]`"),
        "the shipped config warns about un-honored keys: {stderr}"
    );

    // The wheel-command and wheel-momentum columns only exist when the control
    // loop runs, so they are what distinguishes controlled from spacecraft mode
    // — the attitude columns alone appear in both.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let header = header_line(&stdout);
    for col in [
        "qw", "qx", "qy", "qz", "wx", "wy", "wz", "rw_tx", "rw_ty", "rw_tz", "rw_hx", "rw_hy",
        "rw_hz",
    ] {
        assert!(
            header.split(',').any(|c| c == col),
            "column '{col}' missing from header: {header}"
        );
    }
}

/// `--gravity-field` reaches the recorded metadata: the CSV header carries the
/// field's GM (EGM-class 398600.4415), not WGS-84's 398600.4418, and the run
/// completes with the field installed.
#[test]
fn test_gravity_field_flag_sets_mu_to_the_fields_gm() {
    let gfc = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tobari/tests/fixtures/orekit_geopotential_70x70.gfc"
    );
    let out = orts()
        .args([
            "run",
            "--sat",
            "altitude=570,id=a",
            "--epoch",
            "2024-03-20T12:00:00Z",
            "--duration",
            "120",
            "--dt",
            "10",
            "--gravity-field",
            gfc,
            "--gravity-degree",
            "8",
            "--output",
            "-",
            "--format",
            "csv",
        ])
        .output()
        .expect("failed to execute orts");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stderr}");
    assert!(
        stdout
            .lines()
            .any(|l| l.trim() == "# mu = 398600.4415 km^3/s^2"),
        "CSV metadata should carry the field's GM:\n{stdout}"
    );
    assert!(!data_lines(&stdout).is_empty(), "no data rows:\n{stdout}");
}

/// A `[gravity_field]` whose file does not exist is a clean error at the
/// command line — the exit is non-zero, the message names the path, and
/// nothing panicked on the way.
#[test]
fn test_missing_gravity_field_file_is_a_clean_error() {
    let out = run_config(
        "gravity-field-missing",
        r#"
[gravity_field]
path = "/nonexistent/EGM2008.gfc"

[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 570 }
"#,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "run should fail: {stderr}");
    assert!(stderr.contains("/nonexistent/EGM2008.gfc"), "{stderr}");
    assert!(
        !stderr.contains("panicked"),
        "should be an error, not a panic: {stderr}"
    );
}

/// `run --config` builds from the config alone, so a gravity flag next to it
/// is refused rather than dropped.
#[test]
fn test_run_config_refuses_gravity_flags() {
    let dir = unique_dir("gravity-flag-with-config");
    let path = dir.join("orts.toml");
    std::fs::write(
        &path,
        "[[satellites]]\nid = \"a\"\norbit = { type = \"circular\", altitude = 570 }\n",
    )
    .unwrap();
    let out = orts()
        .args([
            "run",
            "--config",
            path.to_str().unwrap(),
            "--gravity-field",
            "x.gfc",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");
    std::fs::remove_dir_all(&dir).ok();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(
        stderr.contains("--gravity-field cannot be honored"),
        "{stderr}"
    );
}

/// The EOP series shipped for the `Gcrs` oracle tests, reused here so the
/// `--frame gcrs` path runs against real Earth orientation.
const EOP_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../orts/tests/fixtures/finals2000A.sample"
);

/// The final position of a run, from the last CSV data row.
fn final_position(stdout: &str) -> [f64; 3] {
    let last = data_lines(stdout)
        .last()
        .unwrap_or_else(|| panic!("no data rows:\n{stdout}"))
        .to_string();
    let cols: Vec<f64> = last
        .split(',')
        .skip(1)
        .take(3)
        .map(|c| c.trim().parse().expect("numeric position column"))
        .collect();
    [cols[0], cols[1], cols[2]]
}

fn one_orbit_config(frame_and_eop: &str) -> String {
    format!(
        r#"
epoch = "2024-03-20T12:00:00Z"
dt = 10.0
duration = 5400.0
output_interval = 600.0
{frame_and_eop}

[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 570, inclination = 97.6, raan = 40 }}
"#
    )
}

/// `frame = "gcrs"` propagates through the IAU 2006 chain and says so in the
/// recording, and it is not the same dynamics as the ERA-only default: the
/// ~0.1° pole offset moves the state by hundreds of metres to kilometres over
/// one orbit. (The metre-level agreement with Orekit is pinned at the library
/// level by `orts/tests/oracle_geopotential.rs`; this is the CLI wiring.)
#[test]
fn test_frame_gcrs_runs_and_differs_from_simple_eci() {
    let gcrs = run_config(
        "frame-gcrs",
        &one_orbit_config(&format!("frame = \"gcrs\"\neop = \"{EOP_FIXTURE}\"")),
    );
    let stderr = String::from_utf8_lossy(&gcrs.stderr);
    assert!(gcrs.status.success(), "gcrs run failed: {stderr}");
    let gcrs_out = String::from_utf8_lossy(&gcrs.stdout);
    assert!(
        gcrs_out.lines().any(|l| l.trim() == "# frame = gcrs"),
        "the recording should name the frame:\n{gcrs_out}"
    );

    let simple = run_config("frame-simple", &one_orbit_config(""));
    assert!(simple.status.success());
    let simple_out = String::from_utf8_lossy(&simple.stdout);
    assert!(
        simple_out
            .lines()
            .any(|l| l.trim() == "# frame = simple-eci"),
        "the default frame should be recorded too:\n{simple_out}"
    );

    let a = final_position(&gcrs_out);
    let b = final_position(&simple_out);
    let sep = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    assert!(
        (0.05..50.0).contains(&sep),
        "gcrs and simple-eci should differ by 0.05..50 km after one orbit, got {sep} km"
    );
}

/// `gcrs` without EOP, on a non-Earth body, or with `eop` but no `gcrs` is
/// refused with a reason instead of quietly falling back.
#[test]
fn test_frame_gcrs_preconditions_are_reported() {
    let cases = [
        ("frame = \"gcrs\"", "needs Earth Orientation Parameters"),
        (
            "body = \"moon\"\nframe = \"gcrs\"\neop = \"zero\"",
            "Earth-only",
        ),
        ("eop = \"zero\"", "only used by frame"),
    ];
    for (i, (extra, needle)) in cases.iter().enumerate() {
        let out = run_config(&format!("frame-bad-{i}"), &one_orbit_config(extra));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{extra} should be refused: {stderr}");
        assert!(stderr.contains(needle), "{extra}: {stderr}");
    }
}

/// The same rules apply to the flags, and a missing EOP file is a clean error.
#[test]
fn test_frame_flags_are_validated_and_eop_file_errors_are_clean() {
    let out = orts()
        .args([
            "run",
            "--sat",
            "altitude=570,id=a",
            "--epoch",
            "2024-03-20T12:00:00Z",
            "--duration",
            "600",
            "--frame",
            "gcrs",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(
        stderr.contains("needs Earth Orientation Parameters"),
        "{stderr}"
    );

    let out = orts()
        .args([
            "run",
            "--sat",
            "altitude=570,id=a",
            "--epoch",
            "2024-03-20T12:00:00Z",
            "--duration",
            "600",
            "--frame",
            "gcrs",
            "--eop",
            "/nonexistent/finals2000A.all",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("/nonexistent/finals2000A.all"), "{stderr}");
    assert!(
        !stderr.contains("panicked"),
        "should be an error, not a panic: {stderr}"
    );
}

/// A `--frame` next to a config is refused, like the gravity flags: the config
/// is the whole simulation there.
#[test]
fn test_run_config_refuses_frame_flags() {
    let dir = unique_dir("frame-flag-with-config");
    let path = dir.join("orts.toml");
    std::fs::write(&path, one_orbit_config("")).unwrap();
    let out = orts()
        .args([
            "run",
            "--config",
            path.to_str().unwrap(),
            "--frame",
            "gcrs",
            "--output",
            "-",
        ])
        .output()
        .expect("failed to execute orts");
    std::fs::remove_dir_all(&dir).ok();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("--frame cannot be honored"), "{stderr}");
}
