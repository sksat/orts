//! E2E tests for the `orts config` subcommand group (example / validate).

use std::process::Command;

fn orts() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orts"))
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("orts-e2e-config-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `config example` (default TOML) prints a config whose round-trip through
/// `config validate` succeeds — the printed example is always valid.
#[test]
fn test_config_example_toml_is_valid() {
    let out = orts()
        .args(["config", "example"])
        .output()
        .expect("run config example");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let toml = String::from_utf8_lossy(&out.stdout);
    assert!(toml.contains("body ="), "example missing body key:\n{toml}");
    assert!(
        toml.contains("[[satellites]]"),
        "example missing satellites table:\n{toml}"
    );

    let dir = unique_dir("ex-toml");
    let path = dir.join("example.toml");
    std::fs::write(&path, toml.as_bytes()).unwrap();
    let v = orts()
        .args(["config", "validate", path.to_str().unwrap()])
        .output()
        .expect("run config validate");
    assert!(
        v.status.success(),
        "example TOML failed validation: stderr={}",
        String::from_utf8_lossy(&v.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `config example --format json` prints valid JSON that also validates.
#[test]
fn test_config_example_json_is_valid() {
    let out = orts()
        .args(["config", "example", "--format", "json"])
        .output()
        .expect("run config example --format json");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("example is not valid JSON: {e}\n{stdout}"));
    assert!(
        v["satellites"].is_array(),
        "JSON example missing satellites array"
    );

    let dir = unique_dir("ex-json");
    let path = dir.join("example.json");
    std::fs::write(&path, stdout.as_bytes()).unwrap();
    let r = orts()
        .args(["config", "validate", path.to_str().unwrap()])
        .output()
        .expect("validate json example");
    assert!(
        r.status.success(),
        "JSON example failed validation: stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `config example --format yaml` validates too.
#[test]
fn test_config_example_yaml_is_valid() {
    let out = orts()
        .args(["config", "example", "--format", "yaml"])
        .output()
        .expect("run config example --format yaml");
    assert!(out.status.success());
    let yaml = String::from_utf8_lossy(&out.stdout);

    let dir = unique_dir("ex-yaml");
    let path = dir.join("example.yaml");
    std::fs::write(&path, yaml.as_bytes()).unwrap();
    let r = orts()
        .args(["config", "validate", path.to_str().unwrap()])
        .output()
        .expect("validate yaml example");
    assert!(
        r.status.success(),
        "YAML example failed validation: stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `config validate <ok> --json` reports a structured ok verdict on stdout.
#[test]
fn test_config_validate_ok_json() {
    let dir = unique_dir("ok");
    let path = dir.join("ok.toml");
    std::fs::write(
        &path,
        "body = \"earth\"\ndt = 10.0\n\n[[satellites]]\nid = \"a\"\n\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500.0\n",
    )
    .unwrap();

    let out = orts()
        .args(["config", "validate", path.to_str().unwrap(), "--json"])
        .output()
        .expect("validate ok --json");
    assert!(out.status.success(), "expected exit 0 for a valid config");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("validate --json should print JSON");
    assert_eq!(v["status"], "ok");
    std::fs::remove_dir_all(&dir).ok();
}

/// `config validate <bad> --json` exits non-zero with a structured error on
/// stdout; without `--json` the error is human-readable on stderr.
#[test]
fn test_config_validate_bad() {
    let dir = unique_dir("bad");
    let path = dir.join("bad.toml");
    // Unknown orbit type → parse/validation failure.
    std::fs::write(
        &path,
        "body = \"earth\"\n\n[[satellites]]\nid = \"a\"\n\n[satellites.orbit]\ntype = \"banana\"\n",
    )
    .unwrap();

    // --json: structured error on stdout, exit 2.
    let out = orts()
        .args(["config", "validate", path.to_str().unwrap(), "--json"])
        .output()
        .expect("validate bad --json");
    assert!(
        !out.status.success(),
        "expected non-zero exit for bad config"
    );
    assert_eq!(out.status.code(), Some(2), "expected exit code 2");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("validate --json should print JSON even on error");
    assert_eq!(v["status"], "error");
    assert!(v["error"].is_string(), "error message should be present");

    // Without --json: human error on stderr, stdout empty.
    let out2 = orts()
        .args(["config", "validate", path.to_str().unwrap()])
        .output()
        .expect("validate bad");
    assert!(!out2.status.success());
    assert!(
        out2.stdout.is_empty(),
        "stdout should be empty without --json"
    );
    assert!(
        !String::from_utf8_lossy(&out2.stderr).is_empty(),
        "expected a human error on stderr"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A syntactically valid config with an unknown central body is rejected — it
/// would otherwise panic later in `orts run`.
#[test]
fn test_config_validate_rejects_unknown_body() {
    let dir = unique_dir("body");
    let path = dir.join("body.toml");
    std::fs::write(
        &path,
        "body = \"pluto\"\n\n[[satellites]]\nid = \"a\"\n\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500.0\n",
    )
    .unwrap();

    let out = orts()
        .args(["config", "validate", path.to_str().unwrap(), "--json"])
        .output()
        .expect("validate unknown body");
    assert_eq!(out.status.code(), Some(2), "unknown body should exit 2");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("json verdict");
    assert_eq!(v["status"], "error");
    assert!(
        v["error"].as_str().unwrap().contains("pluto"),
        "error should name the bad body: {}",
        v["error"]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A malformed epoch is rejected up front rather than panicking at run time.
#[test]
fn test_config_validate_rejects_bad_epoch() {
    let dir = unique_dir("epoch");
    let path = dir.join("epoch.toml");
    std::fs::write(
        &path,
        "body = \"earth\"\nepoch = \"not-a-date\"\n\n[[satellites]]\nid = \"a\"\n\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500.0\n",
    )
    .unwrap();

    let out = orts()
        .args(["config", "validate", path.to_str().unwrap()])
        .output()
        .expect("validate bad epoch");
    assert_eq!(out.status.code(), Some(2), "bad epoch should exit 2");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("epoch"),
        "error should mention epoch"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// An attitude config the simulation would refuse must not validate as OK.
///
/// `config validate` reports on `SimConfig::load`, so a check applied only at
/// the run and serve entry points would leave this command calling a config
/// valid that both of them then reject.
#[test]
fn test_config_validate_rejects_unintegrable_attitude() {
    for (tag, attitude) in [
        ("nan-inertia", "inertia_diag = [nan, 10, 10]\nmass = 500"),
        (
            "huge-inertia",
            "inertia_diag = [1e200, 1e200, 1e200]\nmass = 500\n\
             inertia_off_diag = [1e200, 1e200, 1e200]",
        ),
        ("zero-mass", "inertia_diag = [10, 10, 10]\nmass = 0"),
        (
            "zero-quat",
            "inertia_diag = [10, 10, 10]\nmass = 500\ninitial_quaternion = [0, 0, 0, 0]",
        ),
    ] {
        let dir = unique_dir(tag);
        let path = dir.join("bad.toml");
        std::fs::write(
            &path,
            format!(
                "body = \"earth\"\ndt = 1.0\n\n[[satellites]]\nid = \"a\"\n\
                 orbit = {{ type = \"circular\", altitude = 500 }}\n\n\
                 [satellites.attitude]\n{attitude}\n"
            ),
        )
        .unwrap();

        let out = orts()
            .args(["config", "validate", path.to_str().unwrap(), "--json"])
            .output()
            .expect("validate --json");
        assert_eq!(out.status.code(), Some(2), "{tag}: expected exit code 2");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
            .expect("validate --json should print JSON even on error");
        assert_eq!(v["status"], "error", "{tag}");
        let msg = v["error"].as_str().unwrap_or_default();
        assert!(msg.contains("attitude"), "{tag}: got {msg}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// A rejected config must not be announced as a running server.
///
/// `serve` binds before it loads the config, and its startup banner is what
/// every harness waits on: `cli/tests/ws_e2e.rs` matches `Server listening
/// on`, the Playwright specs read the port out of the `WebSocket endpoint`
/// line. Printing the banner ahead of the rejection left the caller connecting
/// to a socket that was already closing, so a rejected value surfaced as a
/// connection failure instead of as its own message.
#[test]
fn serve_does_not_announce_a_port_for_a_rejected_config() {
    let dir = unique_dir("serve-reject");
    let path = dir.join("unknown-value.json");
    // A known key holding a value the simulation cannot honor: `atmosphere` is
    // the model to use, and `none` names no model. (An unknown *key* is a
    // warning, so it would not stop `serve` and could not test the ordering.)
    std::fs::write(
        &path,
        br#"{"atmosphere":"none","dt":1.0,"satellites":[{"id":"a","orbit":{"type":"circular","altitude":400}}]}"#,
    )
    .unwrap();

    // Not `output()`: were the rejection itself to regress, `serve` would run
    // forever and the test would hang rather than report which assertion broke.
    let mut child = orts()
        .args(["serve", "--port", "0", "--config", path.to_str().unwrap()])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn serve with an unknown config key");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll serve") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                child.kill().ok();
                child.wait().ok();
                std::fs::remove_dir_all(&dir).ok();
                panic!("serve kept running for 30s on a config it should have rejected");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };

    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().expect("piped stderr"), &mut stderr)
        .expect("read serve stderr");

    assert!(!status.success(), "serve accepted it: {stderr}");
    assert!(
        stderr.contains("unknown atmosphere 'none'"),
        "the rejection should name the value it cannot honor: {stderr}"
    );
    // Each banner line separately: the harnesses wait on different ones.
    for line in ["Server listening", "Viewer:", "WebSocket endpoint"] {
        assert!(
            !stderr.contains(line),
            "a rejected config printed the '{line}' banner line: {stderr}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `config validate --json` carries the keys nothing read, and still says ok.
///
/// The lower-level loader tests cover the collection; this covers the shape a
/// caller reads, which could otherwise disappear while they keep passing.
#[test]
fn config_validate_json_carries_the_unread_keys() {
    let dir = unique_dir("validate-warnings");
    let path = dir.join("typo.toml");
    std::fs::write(
        &path,
        "dt = 1.0\nduraton = 100.0\n\n[[satellites]]\nid = \"a\"\naltitide = 400\n\
         [satellites.orbit]\ntype = \"circular\"\naltitude = 400\n",
    )
    .unwrap();

    let out = orts()
        .args(["config", "validate", "--json"])
        .arg(&path)
        .output()
        .expect("run config validate");

    assert!(
        out.status.success(),
        "an unread key is a warning, so validate exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let verdict: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("json: {e}\n{stdout}"));
    assert_eq!(verdict["status"], "ok");
    let warnings = verdict["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("a warnings array: {stdout}"));
    let text: Vec<&str> = warnings.iter().filter_map(|w| w.as_str()).collect();
    assert_eq!(text.len(), 2, "both keys: {text:?}");
    assert!(
        text.iter().any(|w| w.contains("`duraton`")),
        "the top-level key: {text:?}"
    );
    assert!(
        text.iter().any(|w| w.contains("`satellites.0.altitide`")),
        "the nested key, named by its path: {text:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
