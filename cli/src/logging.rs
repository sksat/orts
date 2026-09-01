//! The process-wide logging backend.
//!
//! Two record streams have to land in one place. `orts-cli` and `orts` emit
//! `log` records — including every line a WASM plugin writes through the WIT
//! `host-env.log` import, which the host forwards as a `log` record — while
//! the rerun crates that own the `.rrd` recording path emit `tracing` events.
//! Setting a `tracing` subscriber and bridging `log` into it puts both in one
//! place, and adds no dependency: rerun's `re_log` already compiles
//! `tracing-subscriber` with the features used here into every build.
//!
//! A `log`-only backend would see rerun's diagnostics too, but only by
//! accident: `tracing`'s `log` feature (enabled here through tower, not by
//! this crate) makes a `tracing` macro emit a `log` record while no subscriber
//! is set, and that fallback drops spans and fields. Since a subscriber *is*
//! set here, the macros take their `tracing` branch instead — which is also
//! why bridging `log` into `tracing` cannot recurse. `log-always`, the feature
//! that would emit both unconditionally, is off.
//!
//! What belongs here and what does not: a diagnostic carries a level and is
//! addressed to whoever is debugging (`stream-io socket error on …`,
//! `simulation halted: …`), so it goes through `log`. A command's own output is
//! not a diagnostic — `Saved to …`, the contact-window table, the `serve`
//! banner and the `config validate` verdict are the result the user asked for,
//! and they stay on `eprintln!`/stdout where no filter can drop them.

use std::io::IsTerminal as _;

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// The filter applied when `RUST_LOG` says nothing.
///
/// Our own code at info, everything else at warn. The dependency tree can emit
/// records from axum, hyper, tungstenite, ureq and wasmtime, and an info
/// default across all of them would bury our own diagnostics. Info for ours is
/// what makes the `serve` stream-io lifecycle (plug attached, displaced, peer
/// connected) show up *before* something goes wrong — raising the level after a
/// stream has gone quiet is too late to explain it.
///
/// One directive covers everything of ours because a record's target is its
/// module path, and both roots are `orts`: this crate builds as the `orts`
/// binary (`[[bin]] name`), not as `orts_cli`, and the core library is `orts`
/// too. That single root also carries plugin output, since a guest's
/// `host-env.log` records take the target of the host module that forwards
/// them. [`our_records_are_rooted_at_the_filtered_target`] holds the filter to
/// that name.
///
/// [`our_records_are_rooted_at_the_filtered_target`]: tests::our_records_are_rooted_at_the_filtered_target
const DEFAULT_DIRECTIVES: &str = "warn,orts=info";

/// The environment variable the filter is read from. `RUST_LOG` rather than an
/// `ORTS_*` name of our own: the rerun crates in the same binary already read
/// it, so one variable configures the whole process.
const FILTER_ENV: &str = "RUST_LOG";

/// Builds the filter from a `RUST_LOG` value, falling back to
/// [`DEFAULT_DIRECTIVES`].
///
/// Takes the value as an argument rather than reading the environment so the
/// policy is testable without mutating process-global state (`set_var` is
/// `unsafe` in edition 2024).
///
/// An unset *or empty* `RUST_LOG` falls back: `RUST_LOG=` is what a shell
/// leaves behind when a variable is cleared, and reading that as "filter
/// everything out" would silence the CLI for a caller who meant "default".
fn filter_from_env(rust_log: Option<&str>) -> EnvFilter {
    let directives = match rust_log {
        Some(v) if !v.trim().is_empty() => v,
        _ => DEFAULT_DIRECTIVES,
    };
    // Lossy: an unparsable directive is reported on stderr and skipped, so a
    // typo in `RUST_LOG` costs that directive rather than the whole run.
    EnvFilter::builder().parse_lossy(directives)
}

/// Colour only when stderr is a terminal.
///
/// The fmt layer's own default checks `NO_COLOR` but not whether the stream is
/// a terminal, so it would write escape codes into a redirected log file.
fn use_ansi() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// Installs the backend. Call once, before anything worth logging happens.
///
/// Records go to **stderr**: stdout is reserved for what a command produces —
/// CSV from `run --output -`, the `--json` summary, and the kble-socket binary
/// protocol under `serve --stream-stdio`, which a log line would corrupt.
pub fn init() {
    let filter = filter_from_env(std::env::var(FILTER_ENV).ok().as_deref());
    // Before the filter is moved into the subscriber: "which filter am I
    // actually running with" is the first question when an expected record
    // does not show up, and `RUST_LOG` is not always visible to whoever is
    // reading the log.
    let effective_filter = filter.to_string();

    let subscriber = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(use_ansi()),
    );

    // `try_init` also installs the `log` -> `tracing` bridge (tracing-subscriber's
    // `tracing-log` feature), and does it *after* setting the global default so
    // it can cap `log`'s max level at the filter's. That cap is what lets a
    // filtered-out `log::trace!` inside the integration loop return without
    // formatting its arguments.
    if let Err(e) = subscriber.try_init() {
        // The one diagnostic that cannot go through `log`.
        eprintln!("Warning: could not install the log backend: {e}");
        return;
    }

    // Deliberately at debug, so the default filter keeps it out of every
    // ordinary run. It goes through `log` rather than `tracing` so that a
    // `RUST_LOG=debug` run also demonstrates the `log` bridge is live.
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

    /// The global max level the filter admits. This is the value `try_init`
    /// hands to the `log` bridge, so it decides which `log::` call sites can
    /// return without formatting their arguments.
    fn max_level(rust_log: Option<&str>) -> Option<LevelFilter> {
        <EnvFilter as Layer<Registry>>::max_level_hint(&filter_from_env(rust_log))
    }

    #[test]
    fn default_directives_are_valid() {
        // `parse`, not `parse_lossy`, so a typo in the constant fails here
        // instead of silently dropping a directive at startup.
        assert!(
            EnvFilter::builder().parse(DEFAULT_DIRECTIVES).is_ok(),
            "DEFAULT_DIRECTIVES must parse: {DEFAULT_DIRECTIVES}"
        );
    }

    /// Info has to reach the bridge, since our crates log there; debug and
    /// trace must not, because `run`'s downlink records sit at those levels
    /// inside the integration loop.
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

    /// The policy the default filter exists to express, checked per target.
    ///
    /// Kept in one test on a thread-local dispatcher: a callsite's `Interest`
    /// is cached process-wide, so spreading these across tests that install
    /// different subscribers could let one test's verdict answer another's.
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

    /// `--help` quotes the default filter, and it is the only place a user can
    /// discover it. Changing the constant has to change the help text too.
    #[test]
    fn help_text_quotes_the_default_filter() {
        assert!(
            crate::cli::AFTER_HELP.contains(DEFAULT_DIRECTIVES),
            "--help should quote `{DEFAULT_DIRECTIVES}`"
        );
    }

    /// The filter names `orts` because that is what our record targets are
    /// rooted at. Renaming the binary, or filtering on the package name
    /// `orts_cli` instead, would match nothing and silently discard every
    /// record again — the exact failure this module exists to fix.
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

    /// `RUST_LOG=` (cleared, not unset) must mean "default", not "silence".
    #[test]
    fn empty_rust_log_falls_back_to_the_default() {
        assert_eq!(max_level(Some("")), Some(LevelFilter::INFO));
        assert_eq!(max_level(Some("   ")), Some(LevelFilter::INFO));
    }
}
