pub mod compute;
mod connection;
mod controller_upload;
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
    /// Controller component bytes all `/ws` connections hold together.
    upload_budget: Arc<controller_upload::UploadBudget>,
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
    // A sim arg nothing would read never gets this far: `SimArgs` declares
    // which flags conflict with `--config` and which need an orbit, and clap
    // refuses the command line before `run_server` is called.
    let plugin_overrides = manager::PluginBackendOverrides::from_sim_args(sim);
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CmdError::failure(format!("creating the tokio runtime: {e}")))?;
    rt.block_on(async_server(sim, port, stdio_key, plugin_overrides))
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
/// Outbound messages are unaffected: this bounds what is read. A binary
/// message, a controller component, has its own limit,
/// [`controller_upload::MAX_CONTROLLER_COMPONENT_BYTES`].
const MAX_CONTROL_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// What the socket reads before handing a message over: the larger of the two
/// limits, each of which the connection then applies to its own kind. A
/// message past this closes the connection without a reply.
const MAX_WS_MESSAGE_BYTES: usize =
    if MAX_CONTROL_MESSAGE_BYTES > controller_upload::MAX_CONTROLLER_COMPONENT_BYTES {
        MAX_CONTROL_MESSAGE_BYTES
    } else {
        controller_upload::MAX_CONTROLLER_COMPONENT_BYTES
    };

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    let cmd_tx = state.cmd_tx.clone();
    let upload_budget = Arc::clone(&state.upload_budget);
    ws.max_message_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            connection::handle_connection(socket, rx, cmd_tx, upload_budget).await;
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
    plugin_overrides: manager::PluginBackendOverrides,
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
        // The id and streams are the config's own, so no spec is built: that
        // would fetch a NORAD satellite's TLE only to read these two.
        let declared = cfg
            .satellites
            .iter()
            .enumerate()
            .any(|(i, s)| s.resolved_id(i) == *sat && s.streams.iter().any(|n| n == stream));
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
    let initial_params = if let Some(cfg) = &initial_config {
        let mut params = SimParams::from_config(cfg).map_err(CmdError::failure)?;
        plugin_overrides.apply(&mut params);
        Some(params)
    } else if has_explicit_sim_args(sim) {
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
        upload_budget: controller_upload::UploadBudget::new(
            controller_upload::MAX_UPLOADED_BYTES_PER_SERVER,
        ),
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
    use super::{has_explicit_sim_args, parse_stream_stdio};
    use crate::cli::{Cli, PluginAsyncModeChoice, SimArgs};
    use crate::sim::params::SimParams;
    use clap::Parser;
    use clap::error::ErrorKind;

    fn args(extra: &[&str]) -> SimArgs {
        let mut argv = vec!["orts"];
        argv.extend_from_slice(extra);
        SimArgs::try_parse_from(argv).expect("valid args")
    }

    /// How clap answers `orts serve <extra>`: `None` when it accepts the
    /// command line, else the kind of refusal and the message it prints.
    ///
    /// The refusal is clap's own. `SimArgs` declares which flags conflict with
    /// `--config` and which need an orbit, and clap checks those relations
    /// against the flags written on the command line, so a flag left at its
    /// default never counts and a flag written with its default value does.
    ///
    /// The message is split where it means different things: the error — up
    /// to the `Usage:` line — says what is wrong, and the usage line after it
    /// echoes every flag the command line wrote, refused or not.
    fn refusal(extra: &[&str]) -> Option<(ErrorKind, Refusal)> {
        let mut argv = vec!["orts", "serve"];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv).err().map(|e| {
            let full = e.render().to_string();
            let (error, usage) = match full.split_once("Usage:") {
                Some((error, usage)) => (error.to_string(), usage.to_string()),
                None => (full.clone(), String::new()),
            };
            (e.kind(), Refusal { error, usage })
        })
    }

    /// A refusal's message, split into what is wrong and the echoed usage.
    #[derive(Debug, PartialEq)]
    struct Refusal {
        error: String,
        usage: String,
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

    /// A command line mixing a dropped flag with a carried one names only the
    /// dropped one.
    ///
    /// `--plugin-backend-async-mode` rides `PluginBackendOverrides` into
    /// whatever simulation the server runs, so `--atol` is the only flag a
    /// `--config` run leaves nowhere to put — and the caller hears about that
    /// one alone rather than being sent to drop a flag that works.
    #[test]
    fn a_carried_flag_is_not_named_beside_a_dropped_one() {
        let (kind, msg) = refusal(&[
            "--config",
            "mission.toml",
            "--plugin-backend-async-mode",
            "deterministic",
            "--atol",
            "1e-10",
        ])
        .expect("--atol still has nowhere to go");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(
            msg.error.contains("--atol"),
            "the dropped flag is named: {msg:?}"
        );
        assert!(
            !msg.error.contains("--plugin-backend-async-mode"),
            "the carried one is not: {msg:?}"
        );
        assert!(
            msg.error.contains("--config"),
            "beside the flag that leaves no room for it: {msg:?}"
        );
    }

    /// A CLI-orbit serve runs deterministic until the flag is written.
    ///
    /// The flag has no clap default, because the two commands that read it
    /// differ: `run` runs `throughput` when it is left out and `serve`
    /// `deterministic`. `SimParams::from_sim_args` resolves the absent flag
    /// for the command that called it, so a `serve` whose params come from the
    /// command line lands where a config-built one does.
    #[test]
    fn a_cli_orbit_serve_stays_deterministic_until_the_flag_is_written() {
        let bare = args(&["--sat", "altitude=400"]);
        assert_eq!(
            bare.plugin_backend_async_mode, None,
            "a flag left out reaches the params as nothing asked"
        );
        let params = SimParams::from_sim_args(&bare, true).expect("valid sim args");
        assert_eq!(
            params.plugin_backend_async_mode,
            PluginAsyncModeChoice::Deterministic,
            "an absent flag leaves this server where it has always run"
        );

        let asked = args(&[
            "--sat",
            "altitude=400",
            "--plugin-backend-async-mode",
            "throughput",
        ]);
        let params = SimParams::from_sim_args(&asked, true).expect("valid sim args");
        assert_eq!(
            params.plugin_backend_async_mode,
            PluginAsyncModeChoice::Throughput,
            "and a written one is what the server runs"
        );

        // `run` resolves the same absent flag the other way.
        let params = SimParams::from_sim_args(&bare, false).expect("valid sim args");
        assert_eq!(
            params.plugin_backend_async_mode,
            PluginAsyncModeChoice::Throughput,
            "run fans its control steps out when nothing asked otherwise"
        );
    }

    /// The plugin async mode is honored however the server was started.
    ///
    /// `PluginBackendOverrides` carries it into every `SimParams` the manager
    /// builds, and `ServeEngine` builds its plugin cache with the mode, so no
    /// way of starting `serve` drops it.
    #[test]
    fn the_plugin_async_mode_is_honored_on_every_serve_path() {
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
            assert_eq!(refusal(&extra), None, "{extra:?} starts");
        }
    }

    /// A flag written with the value it already had is still a flag the server
    /// will not honor.
    ///
    /// This is what a comparison against the default cannot see: with a config
    /// supplying a different `atol`, `serve --config cfg.toml --atol 1e-10`
    /// runs the config's value while the command line asked for something.
    /// clap counts the flag because it was written, whatever its value.
    #[test]
    fn a_flag_written_with_the_default_value_is_still_named() {
        let default = args(&[]);
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
            let extra = ["--config", "cfg.toml", flag, value.as_str()];
            let (kind, msg) = refusal(&extra).unwrap_or_else(|| {
                panic!("{flag} {value} is refused although it matches the default")
            });
            assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
            assert!(msg.error.contains(flag), "{flag} is named: {msg:?}");
        }
    }

    #[test]
    fn bare_serve_has_no_sim_args_to_honor() {
        let sim = args(&[]);
        assert!(!has_explicit_sim_args(&sim));
        assert_eq!(refusal(&[]), None);
    }

    /// Idle, a tuning flag has no simulation to apply to, and clap names it
    /// beside the orbit it would need.
    #[test]
    fn tuning_args_are_reported_by_flag_name() {
        for extra in [
            vec!["--dt", "1"],
            vec!["--dt", "1", "--output-interval", "10"],
            vec!["--body", "mars"],
            vec!["--epoch", "2024-03-20T12:00:00Z"],
            vec!["--duration", "600"],
            // Every path that walks with boundaries takes the search from
            // `SimParams`, and an idle `serve` never builds one from these
            // args: a written tolerance would be dropped in silence.
            vec!["--root-t-tolerance", "1e-6"],
            vec!["--integrator", "rk4", "--rtol", "1e-6"],
            vec!["--atmosphere", "nrlmsise00", "--f107", "200"],
            vec!["--space-weather", "auto"],
        ] {
            let (kind, msg) = refusal(&extra).unwrap_or_else(|| panic!("{extra:?} is refused"));
            assert_eq!(kind, ErrorKind::MissingRequiredArgument, "{msg:?}");
            // The error names the orbit the flags need; the usage line after
            // it shows them in the command line that needed one.
            assert!(msg.error.contains("--sat"), "the orbit is named: {msg:?}");
            for flag in extra.iter().filter(|a| a.starts_with("--")) {
                assert!(msg.usage.contains(flag), "{flag} is in the usage: {msg:?}");
            }
        }
    }

    /// A value equal to what a bare command line would mean is still named.
    ///
    /// The old rule asked whether dropping the value changes anything, and
    /// answered from the value
    /// ([#522](https://github.com/sksat/orts/issues/522)). But these values
    /// are read only where the command line describes the orbit, so what was
    /// written anywhere else is dropped either way.
    #[test]
    fn a_value_equal_to_the_default_is_still_named() {
        for extra in [
            vec!["--dt", "10"],
            vec!["--body", "earth"],
            vec!["--integrator", "dp45"],
            // The fallback `output_interval` would have taken is `dt`, and
            // writing it is still writing it.
            vec!["--output-interval", "10"],
            vec!["--output-interval", "30", "--stream-interval", "30"],
        ] {
            let (kind, msg) = refusal(&extra).unwrap_or_else(|| panic!("{extra:?} is refused"));
            assert_eq!(kind, ErrorKind::MissingRequiredArgument, "{msg:?}");
            for flag in extra.iter().filter(|a| a.starts_with("--")) {
                assert!(msg.usage.contains(flag), "{flag} is in the usage: {msg:?}");
            }
        }
    }

    /// The plugin backend flags survive into a client-started simulation via
    /// `PluginBackendOverrides`, so they must not be refused.
    #[test]
    fn plugin_backend_args_are_honored_when_idle() {
        assert_eq!(
            refusal(&[
                "--plugin-backend",
                "sync",
                "--plugin-backend-threshold",
                "64",
            ]),
            None
        );
    }

    /// The CLI-orbit path is the one `SimParams::from_sim_args` serves, so
    /// there the same args are honored.
    #[test]
    fn an_orbit_makes_the_tuning_args_honorable() {
        let sim = args(&["--sat", "altitude=800", "--dt", "1"]);
        assert!(has_explicit_sim_args(&sim));
        assert_eq!(refusal(&["--sat", "altitude=800", "--dt", "1"]), None);
    }

    /// Idle: the message names every dropped flag and the orbit that would
    /// make it apply.
    ///
    /// clap's message stops at the orbit. That a client can instead set these
    /// through `start_simulation` is in `serve --help`, which the message
    /// points to.
    #[test]
    fn idle_serve_refuses_tuning_args_by_name() {
        let (kind, msg) =
            refusal(&["--dt", "1", "--output-interval", "60"]).expect("must be refused");
        assert_eq!(kind, ErrorKind::MissingRequiredArgument, "{msg:?}");
        assert!(msg.usage.contains("--dt"), "{msg:?}");
        assert!(msg.usage.contains("--output-interval"), "{msg:?}");
        assert!(msg.error.contains("--sat"), "{msg:?}");
        assert!(msg.usage.contains("--help"), "{msg:?}");

        use clap::CommandFactory;
        let help = Cli::command()
            .find_subcommand_mut("serve")
            .expect("serve is a subcommand")
            .render_long_help()
            .to_string();
        assert!(
            help.contains("start_simulation"),
            "serve --help says where an idle server takes its settings from"
        );
    }

    /// `--config` builds the whole `SimParams` by itself, so a tuning arg
    /// alongside it is dropped just as silently as in the idle case. The
    /// message names the two flags that cannot go together.
    #[test]
    fn config_serve_refuses_tuning_args_it_cannot_apply() {
        let (kind, msg) =
            refusal(&["--config", "mission.toml", "--dt", "1"]).expect("must be refused");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(msg.error.contains("--dt"), "{msg:?}");
        assert!(msg.error.contains("--config"), "{msg:?}");
        assert_eq!(
            refusal(&["--config", "mission.toml"]),
            None,
            "a bare --config must still be accepted"
        );
    }

    /// An interval is refused for being written, whatever its value.
    ///
    /// The check never reads the value: clap looks at which flags appear, so a
    /// value `SimParams::from_sim_args` would clamp into the default (`20`,
    /// `0.001`, `5` against the bare defaults), one that is not finite, and
    /// one of zero are all the same written flag. The values themselves are
    /// refused later, by the shared time-parameter check, where they are read.
    #[test]
    fn an_interval_is_refused_for_being_written_whatever_its_value() {
        for value in ["20", "0.001", "5", "NaN", "inf", "0"] {
            let extra = ["--config", "cfg.toml", "--stream-interval", value];
            let (kind, msg) =
                refusal(&extra).unwrap_or_else(|| panic!("--stream-interval {value} is refused"));
            assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
            assert!(msg.error.contains("--stream-interval"), "{msg:?}");
        }
        assert!(
            crate::commands::run::validate_sim_args(&args(&[
                "--sat",
                "altitude=400",
                "--dt",
                "NaN"
            ]))
            .is_err(),
            "a non-finite dt is refused where it is read"
        );
    }

    /// The gravity-field flags reach a simulation only through
    /// `from_sim_args`, so `serve --config` must name them rather than run the
    /// zonal model behind an explicit `--gravity-field`.
    #[test]
    fn gravity_field_flags_are_named_when_unhonored() {
        let (_, msg) = refusal(&[
            "--gravity-field",
            "x.gfc",
            "--gravity-degree",
            "8",
            "--gravity-order",
            "8",
        ])
        .expect("an idle serve must refuse the flags");
        for flag in ["--gravity-field", "--gravity-degree", "--gravity-order"] {
            assert!(msg.usage.contains(flag), "{flag} is in the usage: {msg:?}");
        }
        let (kind, msg) = refusal(&["--config", "mission.toml", "--gravity-field", "x.gfc"])
            .expect("serve --config must refuse the flag");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(msg.error.contains("--gravity-field"), "{msg:?}");
    }

    /// `--frame` and `--eop` reach a simulation only through
    /// `from_sim_args`, like the other tuning flags.
    #[test]
    fn serve_names_the_frame_flag_when_it_cannot_honor_it() {
        let (_, msg) =
            refusal(&["--frame", "gcrs", "--eop", "zero"]).expect("an idle serve must refuse them");
        assert!(msg.usage.contains("--frame"), "{msg:?}");
        assert!(msg.usage.contains("--eop"), "{msg:?}");
        let (kind, msg) = refusal(&["--config", "mission.toml", "--frame", "gcrs"])
            .expect("serve --config must refuse the flag");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(msg.error.contains("--frame"), "{msg:?}");
    }

    /// An orbit next to `--config` is refused, the way a tuning flag is.
    ///
    /// `serve` builds from the config when it has one and never reads the
    /// orbit, so `--sat` there used to be dropped in silence.
    #[test]
    fn an_orbit_next_to_a_config_is_refused() {
        let (kind, msg) = refusal(&["--config", "mission.toml", "--sat", "altitude=400"])
            .expect("the orbit has nowhere to go");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(msg.error.contains("--sat"), "{msg:?}");
    }

    /// Two orbits are refused as a usage error (#551). `serve` builds a
    /// command-line orbit with `SimParams::from_sim_args`, which used to panic
    /// on the pair.
    #[test]
    fn two_orbits_are_refused() {
        let (kind, msg) = refusal(&["--sat", "altitude=400", "--norad-id", "25544"])
            .expect("two orbits cannot both be run");
        assert_eq!(kind, ErrorKind::ArgumentConflict, "{msg:?}");
        assert!(
            msg.error.contains("--sat") && msg.error.contains("--norad-id"),
            "{msg:?}"
        );
    }
}
