//! The CLI installs a `log` backend, so `log::` call sites in `orts-cli` and
//! `orts` — the WASM guest's `host-env.log` records among them — actually
//! reach the user instead of being discarded by `log`'s no-op logger.
//!
//! These drive the real binary, so they cover what a unit test on the filter
//! cannot: that the backend is installed at all, that records go to stderr
//! rather than stdout, and that `RUST_LOG` reaches it. Which target gets which
//! level is pinned in `logging.rs`'s unit tests.
//!
//! The record used here is the startup line from `logging::init`, because it is
//! the only one a plain `orts run` emits: every other call site needs either a
//! plugin controller (a built guest component) or a live `serve` peer.

use std::process::Output;

/// Substring of the startup record. Not the whole line: the assertions should
/// survive a change of timestamp or field layout.
const STARTUP_RECORD: &str = "log filter:";

fn run(rust_log: Option<&str>) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_orts"));
    cmd.args([
        "run",
        "--sat",
        "altitude=400",
        "--output",
        "-",
        "--format",
        "csv",
    ]);
    match rust_log {
        Some(v) => cmd.env("RUST_LOG", v),
        // An inherited RUST_LOG would decide the outcome instead of the
        // default filter under test.
        None => cmd.env_remove("RUST_LOG"),
    };
    cmd.output().expect("failed to execute orts")
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A `log::` record reaches stderr, carrying its level and target — a `serve`
/// operator needs to know which component spoke, and the CLI's targets are
/// what `RUST_LOG` selects on.
#[test]
fn log_records_reach_stderr_with_level_and_target() {
    let out = run(Some("debug"));
    let stderr = stderr_of(&out);
    assert!(
        out.status.success(),
        "run failed: stderr={stderr}, stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        stderr.contains(STARTUP_RECORD),
        "expected a log record on stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("DEBUG"),
        "expected the record to carry its level, got: {stderr}"
    );
    assert!(
        stderr.contains("orts::logging"),
        "expected the record to carry its target, got: {stderr}"
    );
}

/// `RUST_LOG` decides, in both directions, without a rebuild — and the
/// built-in default is a real filter rather than "everything through".
#[test]
fn rust_log_selects_which_records_are_emitted() {
    for filter in ["error", "warn", "off", "orts=info"] {
        let out = run(Some(filter));
        let stderr = stderr_of(&out);
        assert!(out.status.success(), "run failed under RUST_LOG={filter}");
        assert!(
            !stderr.contains(STARTUP_RECORD),
            "RUST_LOG={filter} is above debug, so the startup record should be \
             filtered out, got: {stderr}"
        );
    }

    for filter in ["debug", "trace", "orts=debug"] {
        let out = run(Some(filter));
        let stderr = stderr_of(&out);
        assert!(out.status.success(), "run failed under RUST_LOG={filter}");
        assert!(
            stderr.contains(STARTUP_RECORD),
            "RUST_LOG={filter} admits debug, so the startup record should be \
             emitted, got: {stderr}"
        );
    }
}

/// The default filter stops below info, so an ordinary run gains no debug
/// chatter from installing a backend.
#[test]
fn the_default_filter_stops_below_info() {
    let out = run(None);
    let stderr = stderr_of(&out);
    assert!(out.status.success(), "run failed: {stderr}");
    assert!(
        !stderr.contains(STARTUP_RECORD),
        "the default filter should not admit debug records, got: {stderr}"
    );
}

/// The effective filter is reported, so "why do I see no logs" is answerable
/// from the log itself.
#[test]
fn the_startup_record_names_the_filter_in_effect() {
    let stderr = stderr_of(&run(Some("orts=debug")));
    assert!(
        stderr.contains("orts=debug"),
        "expected the record to name the filter in effect, got: {stderr}"
    );

    // Every directive, not just the one that let this record through: the
    // point is to show why *other* records are missing.
    let stderr = stderr_of(&run(Some("error,orts=debug")));
    for directive in ["error", "orts=debug"] {
        assert!(
            stderr.contains(directive),
            "expected the reported filter to name `{directive}`, got: {stderr}"
        );
    }
}

/// stdout carries what the command produces and nothing else: `orts serve
/// --stream-stdio` puts a binary protocol there and `run --json` a parseable
/// summary, so a log record on stdout would corrupt both.
#[test]
fn log_records_stay_off_stdout() {
    let out = run(Some("trace"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("# orts simulation"),
        "expected CSV data on stdout, got: {stdout}"
    );
    for marker in [STARTUP_RECORD, "DEBUG", "TRACE", "INFO"] {
        assert!(
            !stdout.contains(marker),
            "a log record reached stdout ({marker}), got: {stdout}"
        );
    }
}

/// Records are written unstyled when stderr is not a terminal, so a redirected
/// log or a grep over it does not have to step over escape codes.
#[test]
fn records_are_unstyled_when_stderr_is_not_a_terminal() {
    let stderr = stderr_of(&run(Some("debug")));
    assert!(
        stderr.contains(STARTUP_RECORD),
        "precondition: expected a record to inspect, got: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1b}'),
        "expected no ANSI escapes on a piped stderr, got: {stderr:?}"
    );
}
