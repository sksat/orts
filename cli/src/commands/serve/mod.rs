pub mod compute;
mod connection;
mod engine;
mod history;
mod manager;
pub mod protocol;
#[cfg(feature = "viewer")]
pub(crate) mod spa;
mod stream_bridge;
pub(crate) mod textures;

use std::sync::Arc;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

use crate::cli::SimArgs;
use crate::commands::CmdError;
use crate::sim::params::SimParams;

use manager::SimCommand;
use stream_bridge::StreamBridge;
use textures::TextureCache;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
    cmd_tx: mpsc::Sender<SimCommand>,
    textures: Arc<TextureCache>,
    /// stream-io bridge endpoints (binary WS per declared stream).
    bridge: Arc<StreamBridge>,
    /// The (sat, stream) wired to stdio via `--stream-stdio`, if any. Its
    /// WS endpoint is reserved (answers 409) — one transport per stream.
    reserved_stdio: Option<stream_bridge::StreamKey>,
}

pub fn run_server(sim: &SimArgs, port: u16, stream_stdio: Option<&str>) -> Result<(), CmdError> {
    // Parse + reject malformed flags before starting the runtime so a typo
    // fails fast instead of surfacing as a dead endpoint later.
    let stdio_key = match stream_stdio {
        Some(s) => Some(
            parse_stream_stdio(s)
                .map_err(|e| CmdError::usage(format!("--stream-stdio {s}: {e}")))?,
        ),
        None => None,
    };
    // Sim args nothing will read are a usage error, in the same spirit as the
    // config `[[command]]` rejection below: the server would otherwise come
    // up having silently dropped every one of them.
    reject_unhonored_sim_args(sim, &WrittenFlags::from_this_process())?;
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CmdError::failure(format!("creating the tokio runtime: {e}")))?;
    rt.block_on(async_server(sim, port, stdio_key))
}

/// Parse a `--stream-stdio` value of the form `sat/stream` (both halves
/// non-empty, exactly one `/` — they are endpoint path segments).
fn parse_stream_stdio(s: &str) -> Result<stream_bridge::StreamKey, String> {
    match s.split_once('/') {
        Some((sat, stream)) if !sat.is_empty() && !stream.is_empty() && !stream.contains('/') => {
            Ok((sat.to_string(), stream.to_string()))
        }
        _ => Err("expected SAT/STREAM with non-empty halves".to_string()),
    }
}

/// Detect whether CLI args specify an explicit simulation configuration.
///
/// Only a config file or an orbit describes a simulation to start; the
/// remaining sim args (`--dt`, `--body`, `--integrator`, …) tune one that
/// something else describes.
fn has_explicit_sim_args(sim: &SimArgs) -> bool {
    sim.config.is_some() || sim.has_orbit_args()
}

/// Refuse sim args that nothing on the way to a `SimParams` will read.
///
/// The tuning args reach a simulation only through
/// [`SimParams::from_sim_args`], i.e. the CLI-orbit path. The other two paths
/// ignore them completely: an idle server takes its parameters from the
/// client's `start_simulation`, and `--config` builds them with
/// `SimParams::from_config`. Both used to drop every flag in silence — the
/// documented `serve --dt 1 --output-interval 10` served forever without ever
/// starting a simulation.
fn reject_unhonored_sim_args(sim: &SimArgs, written: &WrittenFlags) -> Result<(), CmdError> {
    // First the flags no `serve` path reads: an orbit on the command line
    // makes the rest honorable, and these stay dropped.
    let always: Vec<&str> = ALWAYS_UNHONORED
        .iter()
        .map(|(_, flag)| *flag)
        .filter(|flag| written.was_written(flag))
        .collect();
    if !always.is_empty() {
        return Err(CmdError::usage(format!(
            "serve cannot honor {}: it runs plugins through a deterministic cache \
             (`WasmPluginCache::new()`) and never resolves a mode. Drop the flag, or run \
             the simulation with `orts run`, which does.",
            always.join(", ")
        )));
    }
    let unhonored = unhonored_sim_args(sim, written);
    if unhonored.is_empty() {
        return Ok(());
    }
    let flags = unhonored.join(", ");
    match &sim.config {
        // A config describes the whole simulation, so the command line has
        // nowhere to put these. (The `--plugin-backend*` flags do get applied
        // on top, which is why they are not on the list.)
        Some(path) => Err(CmdError::usage(format!(
            "{flags} cannot be honored: `serve --config {path}` builds its simulation from the \
             config alone. Set the value in the config instead, or drop the flag."
        ))),
        None if !sim.has_orbit_args() => Err(CmdError::usage(format!(
            "{flags} cannot be honored: without an orbit or a config, `serve` comes up idle and \
             takes its simulation parameters from the client's start_simulation. Give it a \
             simulation to apply them to (--sat altitude=400, --tle, --omm, --norad-id, or \
             --config), or drop them."
        ))),
        // The CLI-orbit path: `SimParams::from_sim_args` reads all of them.
        None => Ok(()),
    }
}

/// The sim args that only [`SimParams::from_sim_args`] reads, named as they
/// were written on the command line.
///
/// `--plugin-backend` and `--plugin-backend-threshold` are deliberately absent:
/// `PluginBackendOverrides` applies those to every `SimParams` the manager
/// builds, whoever started the simulation. It does not carry
/// `--plugin-backend-async-mode`, which reaches a simulation only through
/// `SimParams::from_sim_args`, so that one is named like the rest.
///
/// A flag counts because it was written, not because its value differs from the
/// default. This check only runs where nothing reads these values at all — a
/// `--config` builds them with `SimParams::from_config`, and an idle server
/// takes them from the client's `start_simulation` — so whatever was written
/// is dropped, default-valued or not. Comparing values missed exactly that:
/// `serve --config cfg.toml --atol 1e-10` writes the default, read as absent,
/// and the config's `atol` ran without a word.
/// Which tuning flags the caller wrote, whatever value they wrote.
///
/// A comparison against the default cannot answer that question: `serve --atol
/// 1e-10` writes the default, so the value is the same as a bare command
/// line's and the flag reads as absent. With a config supplying a different
/// `atol`, the config's value is what runs — and the guard that exists to say
/// so stayed quiet.
///
/// The flags carrying an `Option` need none of this: their `None` already
/// means the caller left them out.
#[derive(Debug, Default)]
struct WrittenFlags(Vec<&'static str>);

impl WrittenFlags {
    /// The flags on this process's own command line.
    fn from_this_process() -> Self {
        Self::written_in(std::env::args_os())
    }

    /// The flags written in one whole command line, `serve` and all.
    ///
    /// The arguments are parsed a second time, because the first parse happens
    /// inside `parse_with_license_notice` and does not hand back the matches
    /// that carry each value's source. A second parse of an argv the same
    /// command already accepted does not fail. Where it somehow does — or
    /// where the command line names another subcommand — nothing is reported
    /// as written, so the guard names nothing and `serve` starts as it would
    /// have before this check existed.
    fn written_in<I, T>(argv: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        use clap::CommandFactory;

        let Ok(matches) = crate::cli::Cli::command().try_get_matches_from(argv) else {
            return Self::default();
        };
        match matches.subcommand_matches("serve") {
            Some(serve) => Self::from_matches(serve),
            None => Self::default(),
        }
    }

    /// The flags written in one set of matches, whose ids are the field names
    /// `SimArgs` derives them from.
    fn from_matches(matches: &clap::ArgMatches) -> Self {
        let written = VALUE_FLAGS
            .iter()
            .chain(ALWAYS_UNHONORED.iter())
            .filter(|(id, _)| {
                matches!(
                    matches.value_source(id),
                    Some(clap::parser::ValueSource::CommandLine)
                )
            })
            .map(|(_, flag)| *flag)
            .collect();
        Self(written)
    }

    fn was_written(&self, flag: &str) -> bool {
        self.0.contains(&flag)
    }
}

/// The flags whose value has a default, paired with the id `SimArgs` gives it.
///
/// Every one of them used to be read as absent when its value happened to
/// equal the default.
const VALUE_FLAGS: [(&str, &str); 9] = [
    ("body", "--body"),
    ("dt", "--dt"),
    ("integrator", "--integrator"),
    ("atol", "--atol"),
    ("rtol", "--rtol"),
    ("root_t_tolerance", "--root-t-tolerance"),
    ("atmosphere", "--atmosphere"),
    ("f107", "--f107"),
    ("ap", "--ap"),
];

/// Flags no `serve` path reads, whatever else the command line says.
///
/// `--plugin-backend-async-mode` is the only one so far. `ServeEngine` builds
/// its plugin cache with `WasmPluginCache::new()`, the deterministic mode, and
/// never asks `SimParams::resolve_async_mode` — so the mode is dropped even on
/// the CLI-orbit path, where every other tuning flag is read.
const ALWAYS_UNHONORED: [(&str, &str); 1] =
    [("plugin_backend_async_mode", "--plugin-backend-async-mode")];

fn unhonored_sim_args(sim: &SimArgs, written: &WrittenFlags) -> Vec<&'static str> {
    let optional = [
        ("--output-interval", sim.output_interval.is_some()),
        ("--stream-interval", sim.stream_interval.is_some()),
        ("--epoch", sim.epoch.is_some()),
        ("--duration", sim.duration.is_some()),
        ("--space-weather", sim.space_weather.is_some()),
        ("--gravity-field", sim.gravity_field.is_some()),
        ("--gravity-degree", sim.gravity_degree.is_some()),
        ("--gravity-order", sim.gravity_order.is_some()),
        ("--eop", sim.eop.is_some()),
        ("--frame", sim.frame_arg.is_some()),
    ];
    // In a fixed order, so the message reads the same way twice.
    let mut named: Vec<&'static str> = Vec::new();
    for (flag, given) in VALUE_FLAGS
        .iter()
        .map(|(_, flag)| (*flag, written.was_written(flag)))
        .chain(optional)
    {
        if given {
            named.push(flag);
        }
    }
    named
}

/// The largest control message `/ws` will read.
///
/// Every inbound frame is parsed into a `serde_json::Value` so the keys nothing
/// reads can be named, and the tree costs several times the bytes on the wire.
/// axum's default ceiling is 64 MiB per message, which is far past anything this
/// endpoint has to carry: measured, a `start_simulation` runs about 256 bytes per
/// satellite, so 250 KiB for a fleet of 1000 and 2.5 MiB for 10000. 8 MiB leaves
/// room for a fleet larger than any this simulator runs while keeping what one
/// unauthenticated client can make the server hold to something bounded.
///
/// Outbound messages are unaffected: this bounds what is read.
const MAX_CONTROL_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    let cmd_tx = state.cmd_tx.clone();
    ws.max_message_size(MAX_CONTROL_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            connection::handle_connection(socket, rx, cmd_tx).await;
            eprintln!("Client disconnected");
        })
}

/// Binary WS endpoint for a declared `stream-io` stream — the shape of a
/// kble `ws://` plug. 404 for undeclared `(sat, stream)` pairs; 409 for a
/// stream reserved by `--stream-stdio` (one transport per stream).
async fn stream_ws_handler(
    ws: WebSocketUpgrade,
    AxumPath((sat, stream)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> axum::response::Response {
    if state
        .reserved_stdio
        .as_ref()
        .is_some_and(|(s, n)| *s == sat && *n == stream)
    {
        return (
            StatusCode::CONFLICT,
            format!("stream {sat}/{stream} is reserved for the stdio plug (--stream-stdio)\n"),
        )
            .into_response();
    }
    let Some(endpoint) = state.bridge.lookup(&sat, &stream) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no such stream endpoint: {sat}/{stream}\n"),
        )
            .into_response();
    };
    ws.on_upgrade(move |socket| stream_bridge::handle_stream_socket(socket, endpoint, sat, stream))
        .into_response()
}

async fn async_server(
    sim: &SimArgs,
    port: u16,
    stdio_key: Option<stream_bridge::StreamKey>,
) -> Result<(), CmdError> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| CmdError::failure(format!("binding to {addr}: {e}")))?;

    let actual_port = listener.local_addr().unwrap().port();

    let (tx, _rx) = broadcast::channel::<String>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<SimCommand>(16);

    // Determine initial config: if CLI args specify simulation, auto-start.
    let initial_config = if has_explicit_sim_args(sim) {
        match sim.config.as_ref() {
            Some(config_path) => Some(crate::config::load_config_reporting_unread_keys(
                std::path::Path::new(config_path),
            )?),
            None => None,
        }
    } else {
        None
    };

    // `orts serve` does not drive config `[[command]]` timelines (run-only);
    // reject loudly instead of silently dropping scheduled uplinks.
    if let Some(cfg) = &initial_config {
        cfg.ensure_serve_supported().map_err(CmdError::usage)?;
    }

    // With an explicit config, a `--stream-stdio` typo would otherwise be a
    // silent forever-retry; validate the declaration up front. (Without a
    // config the sim starts later via WS, so the stdio task just waits.)
    if let (Some(cfg), Some((sat, stream))) = (&initial_config, &stdio_key) {
        let body = crate::satellite::parse_body(&cfg.body);
        let declared = cfg.satellites.iter().enumerate().any(|(i, s)| {
            let spec = s.to_satellite_spec(i, body, body.properties().mu);
            spec.id == *sat && spec.streams.iter().any(|n| n == stream)
        });
        if !declared {
            return Err(CmdError::usage(format!(
                "--stream-stdio {sat}/{stream} is not declared in the config (streams = [...])"
            )));
        }
    }

    let texture_cache = Arc::new(TextureCache::new());
    let texture_request_tx =
        textures::spawn_texture_downloader(Arc::clone(&texture_cache), tx.clone());
    let bridge = Arc::new(StreamBridge::new());

    // The flag spelling of the frame gate; a config's `frame =` was refused by
    // `ensure_serve_supported` above, with the same reason. Accepting `gcrs`
    // would run the ERA-only frame behind an explicit request for the IAU
    // 2006 one.
    if let Some(why) = sim.frame().serve_refusal() {
        return Err(CmdError::usage(format!(
            "--frame {} is not supported by `orts serve`: {why}",
            sim.frame().as_str()
        )));
    }

    // The initial simulation (`--config`, or orbit arguments on the legacy
    // path) gets its `SimParams` built here, on the main task, so a bad
    // `[gravity_field]` / `--gravity-field` file is a fatal configuration
    // error. Inside the spawned manager it would only kill that task and
    // leave the HTTP / WebSocket server up with nobody behind the command
    // channel.
    let mgr_tx = tx.clone();
    let plugin_overrides = manager::PluginBackendOverrides::from_sim_args(sim);
    let initial_params = if let Some(cfg) = &initial_config {
        let mut params = SimParams::from_config(cfg).map_err(CmdError::failure)?;
        plugin_overrides.apply(&mut params);
        Some(params)
    } else if has_explicit_sim_args(sim) {
        // Legacy path: build SimParams from CLI args directly.
        // from_sim_args already populates plugin_backend_choice /
        // threshold, but we still pass the overrides so that any
        // later delegate to simulation_manager (after a terminate +
        // restart) honors them too.
        // Same reason as in `run`: this path skips `SimConfig::validate`.
        crate::commands::run::validate_sim_args(sim)?;
        Some(SimParams::from_sim_args(sim, true).map_err(CmdError::failure)?)
    } else {
        None
    };
    match initial_params {
        Some(params) => {
            let params = Arc::new(params);
            crate::satellite::ensure_unique_ids(&params.satellites)?;
            tokio::spawn(manager::simulation_manager_with_params(
                params,
                plugin_overrides,
                cmd_rx,
                mgr_tx,
                texture_request_tx.clone(),
                Arc::clone(&bridge),
            ));
        }
        None => {
            tokio::spawn(manager::simulation_manager(
                None,
                plugin_overrides,
                cmd_rx,
                mgr_tx,
                texture_request_tx.clone(),
                Arc::clone(&bridge),
            ));
        }
    }

    // The stdio plug task drives stdin/stdout with the kble-socket protocol
    // and signals shutdown when the peer (the kble harness that spawned us)
    // closes the connection.
    let shutdown_rx = stdio_key.clone().map(|(sat, stream)| {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(stream_bridge::run_stdio_plug(
            Arc::clone(&bridge),
            sat,
            stream,
            shutdown_tx,
        ));
        shutdown_rx
    });

    let state = AppState {
        tx,
        cmd_tx,
        textures: texture_cache,
        bridge,
        reserved_stdio: stdio_key,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/stream/{sat}/{stream}", get(stream_ws_handler))
        .route(
            "/textures/{filename}",
            get(textures::texture_handler).with_state(Arc::clone(&state.textures)),
        );

    #[cfg(feature = "viewer")]
    let app = app.fallback(spa::spa_handler);

    let app = app.with_state(state);

    // Announced only once every rejection above is behind us. This banner is
    // what callers wait on to mean "the endpoint is up" — `cli/tests/ws_e2e.rs`
    // matches the first line, the Playwright specs read the port out of the
    // `ws://` one — so printing it before the config is loaded turned a
    // rejected config into a connection that is refused with no explanation.
    eprintln!("Server listening on http://localhost:{actual_port}");
    #[cfg(feature = "viewer")]
    eprintln!("Viewer:             http://localhost:{actual_port}/");
    eprintln!("WebSocket endpoint: ws://localhost:{actual_port}/ws");

    match shutdown_rx {
        Some(rx) => {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                    eprintln!("stdio plug closed; shutting down");
                })
                .await
        }
        None => axum::serve(listener, app).await,
    }
    .map_err(|e| CmdError::failure(format!("server error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::{
        WrittenFlags, has_explicit_sim_args, parse_stream_stdio, reject_unhonored_sim_args,
        unhonored_sim_args,
    };
    use crate::cli::SimArgs;
    use clap::Parser;

    fn args(extra: &[&str]) -> SimArgs {
        let mut argv = vec!["orts"];
        argv.extend_from_slice(extra);
        SimArgs::try_parse_from(argv).expect("valid args")
    }

    /// What the same argv wrote, which is what the guard asks about.
    fn written(extra: &[&str]) -> WrittenFlags {
        use clap::CommandFactory;

        let mut argv = vec!["orts"];
        argv.extend_from_slice(extra);
        let matches = SimArgs::command()
            .try_get_matches_from(argv)
            .expect("valid args");
        WrittenFlags::from_matches(&matches)
    }

    /// The refusal message, or `None` when the args are accepted.
    fn refusal(extra: &[&str]) -> Option<String> {
        reject_unhonored_sim_args(&args(extra), &written(extra))
            .err()
            .map(|e| e.to_string())
    }

    #[test]
    fn parse_stream_stdio_accepts_sat_slash_stream() {
        assert_eq!(
            parse_stream_stdio("sat0/comlink"),
            Ok(("sat0".to_string(), "comlink".to_string()))
        );
    }

    #[test]
    fn parse_stream_stdio_rejects_malformed_values() {
        assert!(parse_stream_stdio("nodelimiter").is_err());
        assert!(parse_stream_stdio("/comlink").is_err());
        assert!(parse_stream_stdio("sat0/").is_err());
        assert!(parse_stream_stdio("sat0/a/b").is_err());
    }

    /// The path the process itself takes: the whole command line, through
    /// `Cli`, with `serve`'s own matches pulled out of it.
    ///
    /// The other cases build matches from `SimArgs` alone, which would keep
    /// passing if the ids under `serve` were spelled differently or the
    /// subcommand were pulled out wrongly.
    #[test]
    fn the_whole_command_line_is_read_the_way_the_process_reads_it() {
        let written = WrittenFlags::written_in(["orts", "serve", "--atol", "1e-10"]);
        assert!(
            written.was_written("--atol"),
            "--atol is written, whatever its value: {written:?}"
        );
        assert!(!written.was_written("--dt"), "and --dt is not: {written:?}");

        // Another subcommand's flags are not serve's.
        let elsewhere = WrittenFlags::written_in(["orts", "run", "--atol", "1e-10"]);
        assert!(
            !elsewhere.was_written("--atol"),
            "run's flags are read by run: {elsewhere:?}"
        );
    }

    /// The plugin async mode is refused even where every other tuning flag is
    /// honored.
    ///
    /// An orbit on the command line makes `SimParams::from_sim_args` the path,
    /// and it reads all of them — except this one, which reaches no plugin:
    /// `ServeEngine` builds its cache with `WasmPluginCache::new()`.
    #[test]
    fn the_plugin_async_mode_is_refused_on_every_serve_path() {
        for extra in [
            vec!["--plugin-backend-async-mode", "throughput"],
            vec![
                "--sat",
                "altitude=400",
                "--plugin-backend-async-mode",
                "throughput",
            ],
            vec![
                "--config",
                "mission.toml",
                "--plugin-backend-async-mode",
                "deterministic",
            ],
        ] {
            let msg = refusal(&extra).unwrap_or_else(|| panic!("{extra:?} must be refused"));
            assert!(
                msg.contains("--plugin-backend-async-mode"),
                "{extra:?} names the flag: {msg}"
            );
        }
    }

    /// A flag written with the value it already had is still a flag the server
    /// will not honor.
    ///
    /// This is what a comparison against the default cannot see: with a config
    /// supplying a different `atol`, `serve --config cfg.toml --atol 1e-10`
    /// runs the config's value while the command line asked for something, and
    /// the guard used to stay quiet about it.
    #[test]
    fn a_flag_written_with_the_default_value_is_still_named() {
        let default = SimArgs::try_parse_from(["orts"]).expect("valid args");
        for (flag, value) in [
            ("--dt", format!("{}", default.dt)),
            ("--atol", format!("{}", default.atol)),
            ("--rtol", format!("{}", default.rtol)),
            ("--body", default.body.clone()),
            ("--f107", format!("{}", default.f107)),
            ("--ap", format!("{}", default.ap)),
            (
                "--root-t-tolerance",
                format!("{}", default.root_t_tolerance),
            ),
        ] {
            let extra = [flag, value.as_str()];
            let named = unhonored_sim_args(&args(&extra), &written(&extra));
            assert!(
                named.contains(&flag),
                "{flag} {value} is named although it matches the default: {named:?}"
            );
        }
    }

    #[test]
    fn bare_serve_has_no_sim_args_to_honor() {
        let sim = args(&[]);
        assert!(!has_explicit_sim_args(&sim));
        assert!(unhonored_sim_args(&sim, &written(&[])).is_empty());
        assert!(refusal(&[]).is_none());
    }

    #[test]
    fn tuning_args_are_reported_by_flag_name() {
        assert_eq!(
            unhonored_sim_args(&args(&["--dt", "1"]), &written(&["--dt", "1"])),
            vec!["--dt"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--dt", "1", "--output-interval", "10"]),
                &written(&["--dt", "1", "--output-interval", "10"])
            ),
            vec!["--dt", "--output-interval"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--body", "mars"]), &written(&["--body", "mars"])),
            vec!["--body"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--epoch", "2024-03-20T12:00:00Z"]),
                &written(&["--epoch", "2024-03-20T12:00:00Z"])
            ),
            vec!["--epoch"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--duration", "600"]),
                &written(&["--duration", "600"])
            ),
            vec!["--duration"]
        );
        // Every path that walks with boundaries takes the search from
        // `SimParams`, and `serve` idle or with `--config` never builds one
        // from these args: a written tolerance would be dropped in silence.
        assert_eq!(
            unhonored_sim_args(
                &args(&["--root-t-tolerance", "1e-6"]),
                &written(&["--root-t-tolerance", "1e-6"])
            ),
            vec!["--root-t-tolerance"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--integrator", "rk4", "--rtol", "1e-6"]),
                &written(&["--integrator", "rk4", "--rtol", "1e-6"])
            ),
            vec!["--integrator", "--rtol"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--atmosphere", "nrlmsise00", "--f107", "200"]),
                &written(&["--atmosphere", "nrlmsise00", "--f107", "200"])
            ),
            vec!["--atmosphere", "--f107"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--space-weather", "auto"]),
                &written(&["--space-weather", "auto"])
            ),
            vec!["--space-weather"]
        );
    }

    /// A value equal to what a bare command line would mean is still named.
    ///
    /// This expectation is the opposite of the one this test carried before
    /// ([#522](https://github.com/sksat/orts/issues/522)): the old rule asked
    /// whether dropping the value changes anything, and answered from the
    /// value. But the check only runs where nothing reads these values — a
    /// `--config` builds them itself, an idle server takes them from its
    /// client — so what was written is dropped either way, and with a config
    /// setting something else the two are not even the same number.
    #[test]
    fn a_value_equal_to_the_default_is_still_named() {
        assert_eq!(
            unhonored_sim_args(&args(&["--dt", "10"]), &written(&["--dt", "10"])),
            vec!["--dt"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--body", "earth"]), &written(&["--body", "earth"])),
            vec!["--body"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--integrator", "dp45"]),
                &written(&["--integrator", "dp45"])
            ),
            vec!["--integrator"]
        );
        // The fallback `output_interval` would have taken is `dt`, and writing
        // it is still writing it.
        assert_eq!(
            unhonored_sim_args(
                &args(&["--output-interval", "10"]),
                &written(&["--output-interval", "10"])
            ),
            vec!["--output-interval"]
        );
        assert_eq!(
            unhonored_sim_args(
                &args(&["--output-interval", "30", "--stream-interval", "30"]),
                &written(&["--output-interval", "30", "--stream-interval", "30"])
            ),
            vec!["--output-interval", "--stream-interval"]
        );
    }
    /// The plugin backend flags survive into a client-started simulation via
    /// `PluginBackendOverrides`, so they must not be refused.
    #[test]
    fn plugin_backend_args_are_honored_when_idle() {
        assert!(
            unhonored_sim_args(
                &args(&[
                    "--plugin-backend",
                    "sync",
                    "--plugin-backend-threshold",
                    "64",
                ]),
                &written(&[
                    "--plugin-backend",
                    "sync",
                    "--plugin-backend-threshold",
                    "64",
                ])
            )
            .is_empty()
        );
        assert!(
            refusal(&[
                "--plugin-backend",
                "sync",
                "--plugin-backend-threshold",
                "64",
            ])
            .is_none()
        );
    }

    /// The CLI-orbit path is the one `SimParams::from_sim_args` serves, so
    /// there the same args are honored.
    #[test]
    fn an_orbit_makes_the_tuning_args_honorable() {
        let sim = args(&["--sat", "altitude=800", "--dt", "1"]);
        assert!(has_explicit_sim_args(&sim));
        assert!(refusal(&["--sat", "altitude=800", "--dt", "1"]).is_none());
    }

    /// Idle: the message names every dropped flag and how to make it apply.
    #[test]
    fn idle_serve_refuses_tuning_args_by_name() {
        let msg = refusal(&["--dt", "1", "--output-interval", "60"]).expect("must be refused");
        assert!(msg.contains("--dt"), "{msg}");
        assert!(msg.contains("--output-interval"), "{msg}");
        assert!(msg.contains("--sat"), "{msg}");
        assert!(msg.contains("start_simulation"), "{msg}");
    }

    /// `--config` builds the whole `SimParams` by itself, so a tuning arg
    /// alongside it is dropped just as silently as in the idle case. The
    /// message points at the config rather than at `--sat`.
    #[test]
    fn config_serve_refuses_tuning_args_it_cannot_apply() {
        let msg = refusal(&["--config", "mission.toml", "--dt", "1"]).expect("must be refused");
        assert!(msg.contains("--dt"), "{msg}");
        assert!(msg.contains("mission.toml"), "{msg}");
        assert!(
            refusal(&["--config", "mission.toml"]).is_none(),
            "a bare --config must still be accepted"
        );
    }

    /// A stream interval is named whatever the clamp would make of it.
    ///
    /// `SimParams::from_sim_args` clamps the value into
    /// `[min(dt, output_interval), output_interval]`, so `--stream-interval 20`
    /// against the bare defaults resolves to the same 10 s a bare command line
    /// gives — but `from_sim_args` is not what runs here, and the value written
    /// reaches nothing. This too is the opposite of what the test asserted
    /// before ([#522](https://github.com/sksat/orts/issues/522)).
    #[test]
    fn a_stream_interval_is_named_whatever_the_clamp_would_do() {
        for value in ["20", "0.001", "5"] {
            let extra = ["--stream-interval", value];
            assert_eq!(
                unhonored_sim_args(&args(&extra), &written(&extra)),
                vec!["--stream-interval"],
                "--stream-interval {value} is written, so it is named"
            );
        }
        // And alongside an output interval, both are named.
        let extra = [
            "--dt",
            "1",
            "--output-interval",
            "10",
            "--stream-interval",
            "5",
        ];
        assert_eq!(
            unhonored_sim_args(&args(&extra), &written(&extra)),
            vec!["--dt", "--output-interval", "--stream-interval"]
        );
    }

    #[test]
    fn a_non_finite_interval_is_named_rather_than_panicking() {
        for extra in [
            vec!["--output-interval", "NaN", "--stream-interval", "1"],
            vec!["--dt", "NaN", "--stream-interval", "1"],
            vec!["--output-interval", "inf", "--stream-interval", "1"],
            vec!["--stream-interval", "NaN"],
            // The clamp would fold these into the default and read them as
            // inert; `validate_time_params` refuses them, so they are named.
            vec!["--stream-interval", "inf"],
            vec!["--stream-interval", "0"],
        ] {
            let named = unhonored_sim_args(&args(&extra), &written(&extra));
            assert!(
                named.contains(&"--stream-interval"),
                "{extra:?} names the stream interval: {named:?}"
            );
        }
        // The value itself is refused a moment later, by the shared check.
        assert!(
            crate::commands::run::validate_sim_args(&args(&["--dt", "NaN"])).is_err(),
            "a non-finite dt is refused"
        );
    }

    /// The gravity-field flags reach a simulation only through
    /// `from_sim_args`, so `serve --config` must name them rather than run the
    /// zonal model behind an explicit `--gravity-field`.
    #[test]
    fn gravity_field_flags_are_named_when_unhonored() {
        assert_eq!(
            unhonored_sim_args(
                &args(&[
                    "--gravity-field",
                    "x.gfc",
                    "--gravity-degree",
                    "8",
                    "--gravity-order",
                    "8",
                ]),
                &written(&[
                    "--gravity-field",
                    "x.gfc",
                    "--gravity-degree",
                    "8",
                    "--gravity-order",
                    "8",
                ])
            ),
            vec!["--gravity-field", "--gravity-degree", "--gravity-order"]
        );
        let err = refusal(&["--config", "mission.toml", "--gravity-field", "x.gfc"])
            .expect("serve --config must refuse the flag");
        assert!(err.contains("--gravity-field"), "{err}");
    }

    /// `serve` propagates in `SimpleEci` only, so `--frame gcrs` is named as
    /// unhonored rather than served in the other frame.
    #[test]
    fn serve_names_the_frame_flag_when_it_cannot_honor_it() {
        assert_eq!(
            unhonored_sim_args(
                &args(&["--frame", "gcrs", "--eop", "zero"]),
                &written(&["--frame", "gcrs", "--eop", "zero"])
            ),
            vec!["--eop", "--frame"]
        );
        let err = refusal(&["--config", "mission.toml", "--frame", "gcrs"])
            .expect("serve --config must refuse the flag");
        assert!(err.contains("--frame"), "{err}");
    }
}
