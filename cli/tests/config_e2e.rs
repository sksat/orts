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
