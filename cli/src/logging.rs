//! The process-wide logging backend.
//!
//! Two record streams have to reach one place: `log` records from `orts-cli`
//! and `orts` (a WASM plugin's `host-env.log` lines among them), and `tracing`
//! events from the rerun crates on the `.rrd` path. A `tracing` subscriber with
//! `log` bridged into it takes both and adds no dependency — rerun's `re_log`
//! already builds `tracing-subscriber` with these features.
//!
//! `tracing`'s `log` feature (on via axum's default `tower-log`) makes a
//! `tracing` macro emit a `log` record while no subscriber is set. With one set
//! the macros stay on `tracing`, which is why bridging `log` into it cannot
//! recurse — `log-always`, which emits both unconditionally, would.
//!
//! Diagnostics carry a level and go through `log`. A command's own output —
//! `Saved to …`, the contact-window table, the `serve` banner, the `config
//! validate` verdict — stays on `eprintln!`/stdout, where no filter drops it.

use std::io::IsTerminal as _;

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Filter used when `RUST_LOG` says nothing.
///
/// Ours at info, dependencies at warn: an info default across axum, hyper,
/// ureq and wasmtime would bury our own records. Info for ours leaves the
/// `serve` stream-io trail in place *before* a stream goes quiet.
///
/// One directive covers the CLI, the core library and plugin output, because
/// targets are module paths and all three are rooted at `orts` — this crate
/// builds as the `orts` binary, not as `orts_cli`.
const DEFAULT_DIRECTIVES: &str = "warn,orts=info";

/// `RUST_LOG` rather than an `ORTS_*` name of our own: the rerun crates in the
/// same binary read it too, so one variable configures the whole process.
const FILTER_ENV: &str = "RUST_LOG";

/// Takes the value as an argument so the policy is testable without `set_var`
/// (`unsafe` in edition 2024). An empty value falls back as well: `RUST_LOG=`
/// is a cleared variable, not "filter everything out".
fn filter_from_env(rust_log: Option<&str>) -> EnvFilter {
    let directives = match rust_log {
        Some(v) if !v.trim().is_empty() => v,
        _ => DEFAULT_DIRECTIVES,
    };
    // Lossy, so a typo costs that one directive rather than the whole run.
    EnvFilter::builder().parse_lossy(directives)
}

/// The fmt layer's own default checks `NO_COLOR` but not whether the stream is
/// a terminal, so it would write escape codes into a redirected log.
fn use_ansi() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// Installs the backend; call once, before anything worth logging happens.
///
/// Records go to stderr, since stdout carries what a command produces: CSV
/// from `run --output -`, the `--json` summary, and the kble-socket protocol
/// under `serve --stream-stdio`.
pub fn init() {
    let filter = filter_from_env(std::env::var(FILTER_ENV).ok().as_deref());
    let effective_filter = filter.to_string();

    let subscriber = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(use_ansi()),
    );

    // `try_init` also installs the `log` -> `tracing` bridge, capping `log`'s
    // max level at the filter's. That cap is what lets a filtered-out
    // `log::trace!` in the integration loop return without formatting.
    if let Err(e) = subscriber.try_init() {
        // The one diagnostic that cannot go through `log`.
        eprintln!("Warning: could not install the log backend: {e}");
        return;
    }

    // At debug so ordinary runs stay quiet, and through `log` so that a
    // `RUST_LOG=debug` run also shows the bridge is live.
    log::debug!(
        "orts {} starting; log filter: {effective_filter}",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::Layer;
    use tracing_subscriber::Registry;
    use tracing_subscriber::filter::LevelFilter;

    /// The value `try_init` hands to the `log` bridge, which decides where
    /// `log::` call sites can return without formatting their arguments.
    fn max_level(rust_log: Option<&str>) -> Option<LevelFilter> {
        <EnvFilter as Layer<Registry>>::max_level_hint(&filter_from_env(rust_log))
    }

    #[test]
    fn default_directives_are_valid() {
        // `parse`, not `parse_lossy`: a typo in the constant fails here rather
        // than dropping a directive at startup.
        assert!(
            EnvFilter::builder().parse(DEFAULT_DIRECTIVES).is_ok(),
            "DEFAULT_DIRECTIVES must parse: {DEFAULT_DIRECTIVES}"
        );
    }

    /// Info reaches the bridge; debug and trace must not, since `run`'s
    /// downlink records sit there inside the integration loop.
    #[test]
    fn default_admits_info_and_stops_below_it() {
        assert_eq!(max_level(None), Some(LevelFilter::INFO));
    }

    #[test]
    fn rust_log_replaces_the_default() {
        assert_eq!(max_level(Some("warn")), Some(LevelFilter::WARN));
        assert_eq!(max_level(Some("orts=trace")), Some(LevelFilter::TRACE));
        assert_eq!(max_level(Some("off")), Some(LevelFilter::OFF));
    }

    /// Kept in one test: a callsite's `Interest` is cached process-wide, so
    /// spreading these across tests that install different subscribers could
    /// let one test's verdict answer another's.
    #[test]
    fn default_raises_our_crates_and_leaves_dependencies_at_warn() {
        let subscriber = tracing_subscriber::registry().with(filter_from_env(None));
        tracing::dispatcher::with_default(&tracing::Dispatch::new(subscriber), || {
            assert!(
                tracing::enabled!(target: "orts::commands::serve::stream_bridge", tracing::Level::INFO),
                "the CLI's own info records are the stream-io lifecycle trail"
            );
            assert!(
                tracing::enabled!(target: "orts::plugin::wasm::sync_host_state", tracing::Level::INFO),
                "a WASM plugin's host-env.log records carry this target"
            );
            assert!(
                !tracing::enabled!(target: "axum::serve", tracing::Level::INFO),
                "a dependency's info would bury ours"
            );
            assert!(
                tracing::enabled!(target: "axum::serve", tracing::Level::WARN),
                "a dependency's warning still has to reach the user"
            );
            assert!(
                !tracing::enabled!(target: "orts::commands::run", tracing::Level::DEBUG),
                "run's per-tick downlink records sit at debug and must stay off"
            );
        });
    }

    /// `--help` is the only place a user can discover the default.
    #[test]
    fn help_text_quotes_the_default_filter() {
        assert!(
            crate::cli::AFTER_HELP.contains(DEFAULT_DIRECTIVES),
            "--help should quote `{DEFAULT_DIRECTIVES}`"
        );
    }

    /// Filtering on the package name `orts_cli`, or renaming the binary, would
    /// match nothing and silently discard every record again.
    #[test]
    fn our_records_are_rooted_at_the_filtered_target() {
        let root = module_path!()
            .split("::")
            .next()
            .expect("a module path has at least one segment");
        assert_eq!(root, "orts", "this binary's crate name");
        assert!(
            DEFAULT_DIRECTIVES.contains(&format!("{root}=")),
            "{DEFAULT_DIRECTIVES} must raise the level for `{root}`"
        );
    }

    /// `RUST_LOG=` (cleared, not unset) means "default", not "silence".
    #[test]
    fn empty_rust_log_falls_back_to_the_default() {
        assert_eq!(max_level(Some("")), Some(LevelFilter::INFO));
        assert_eq!(max_level(Some("   ")), Some(LevelFilter::INFO));
    }
}
