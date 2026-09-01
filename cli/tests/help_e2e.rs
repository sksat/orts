//! E2E tests for discoverability: `orts --help` carries copy-pasteable
//! examples covering the main agent-facing workflows.

use std::process::Command;

#[test]
fn test_top_level_help_has_examples() {
    let out = Command::new(env!("CARGO_BIN_EXE_orts"))
        .arg("--help")
        .output()
        .expect("run orts --help");
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);

    assert!(
        help.contains("Examples:"),
        "--help should include an Examples section:\n{help}"
    );
    // The examples should cover the key recently-added workflows so an agent
    // reading --help discovers them.
    assert!(help.contains("orts run"), "examples should show `orts run`");
    assert!(
        help.contains("--json"),
        "examples should show the --json run summary workflow"
    );
    assert!(
        help.contains("orts config validate"),
        "examples should show `orts config validate`"
    );
}

/// The log filter is only reachable through the environment, so `--help` is
/// where someone (or an agent) has to be able to find it.
#[test]
fn test_top_level_help_documents_the_log_filter() {
    let out = Command::new(env!("CARGO_BIN_EXE_orts"))
        .arg("--help")
        .output()
        .expect("run orts --help");
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);

    assert!(
        help.contains("Environment:"),
        "--help should include an Environment section:\n{help}"
    );
    assert!(
        help.contains("RUST_LOG"),
        "--help should name the log filter variable:\n{help}"
    );
    assert!(
        help.contains("warn,orts=info"),
        "--help should state the default filter, so the starting point is known:\n{help}"
    );
}
