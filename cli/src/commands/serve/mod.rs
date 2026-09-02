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

use clap::Parser;

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
    reject_unhonored_sim_args(sim)?;
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
fn reject_unhonored_sim_args(sim: &SimArgs) -> Result<(), CmdError> {
    let unhonored = unhonored_sim_args(sim);
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
/// The plugin backend flags are deliberately absent: `PluginBackendOverrides`
/// applies them to every `SimParams` the manager builds, whoever started the
/// simulation.
///
/// Presence is decided by comparing against what the flag-less command line
/// would mean rather than by asking whether the flag appeared (that needs the
/// `ArgMatches` this layer never sees). The question is whether dropping the
/// value changes anything: `--dt 10` asks for the default, `--dt 1` does not,
/// and `--output-interval` equal to `dt` asks for the fallback that
/// `from_sim_args` would have picked anyway. The blind spot is a flag written
/// with its default value alongside a `--config` that sets a different one;
/// silence there is the pre-existing behavior, not a new one.
fn unhonored_sim_args(sim: &SimArgs) -> Vec<&'static str> {
    let default = SimArgs::try_parse_from(["orts"])
        .expect("every SimArgs field is optional or has a default");
    // `from_sim_args`: output_interval falls back to dt, stream_interval to
    // output_interval.
    let output_interval = sim.output_interval.unwrap_or(sim.dt);
    // `SimParams::from_sim_args` clamps the stream interval into
    // `[min(dt, output_interval), output_interval]`, so a written value outside
    // that range resolves to the same interval a bare command line would give.
    // Comparing the written value would refuse `serve --stream-interval 20`
    // (bare defaults clamp both to 10), which changes nothing when dropped.
    let clamp_stream = |v: f64| v.clamp(sim.dt.min(output_interval), output_interval);
    [
        ("--body", sim.body != default.body),
        ("--dt", sim.dt != default.dt),
        (
            "--output-interval",
            sim.output_interval.is_some_and(|v| v != sim.dt),
        ),
        (
            "--stream-interval",
            sim.stream_interval
                .is_some_and(|v| clamp_stream(v) != output_interval),
        ),
        ("--epoch", sim.epoch.is_some()),
        ("--duration", sim.duration.is_some()),
        ("--integrator", sim.integrator != default.integrator),
        ("--atol", sim.atol != default.atol),
        ("--rtol", sim.rtol != default.rtol),
        ("--atmosphere", sim.atmosphere != default.atmosphere),
        ("--f107", sim.f107 != default.f107),
        ("--ap", sim.ap != default.ap),
        ("--space-weather", sim.space_weather.is_some()),
    ]
    .into_iter()
    .filter_map(|(flag, differs)| differs.then_some(flag))
    .collect()
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

    // Spawn simulation manager
    let mgr_tx = tx.clone();
    let plugin_overrides = manager::PluginBackendOverrides::from_sim_args(sim);
    if has_explicit_sim_args(sim) && initial_config.is_none() {
        // Legacy path: build SimParams from CLI args directly.
        // from_sim_args already populates plugin_backend_choice /
        // threshold, but we still pass the overrides so that any
        // later delegate to simulation_manager (after a terminate +
        // restart) honors them too.
        // Same reason as in `run`: this path skips `SimConfig::validate`.
        crate::commands::run::validate_sim_args(sim)?;
        let params = Arc::new(SimParams::from_sim_args(sim, true));
        crate::satellite::ensure_unique_ids(&params.satellites)?;
        tokio::spawn(manager::simulation_manager_with_params(
            params,
            plugin_overrides,
            cmd_rx,
            mgr_tx,
            texture_request_tx.clone(),
            Arc::clone(&bridge),
        ));
    } else {
        tokio::spawn(manager::simulation_manager(
            initial_config,
            plugin_overrides,
            cmd_rx,
            mgr_tx,
            texture_request_tx.clone(),
            Arc::clone(&bridge),
        ));
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
        has_explicit_sim_args, parse_stream_stdio, reject_unhonored_sim_args, unhonored_sim_args,
    };
    use crate::cli::SimArgs;
    use clap::Parser;

    fn args(extra: &[&str]) -> SimArgs {
        let mut argv = vec!["orts"];
        argv.extend_from_slice(extra);
        SimArgs::try_parse_from(argv).expect("valid args")
    }

    /// The refusal message, or `None` when the args are accepted.
    fn refusal(extra: &[&str]) -> Option<String> {
        reject_unhonored_sim_args(&args(extra))
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

    #[test]
    fn bare_serve_has_no_sim_args_to_honor() {
        let sim = args(&[]);
        assert!(!has_explicit_sim_args(&sim));
        assert!(unhonored_sim_args(&sim).is_empty());
        assert!(refusal(&[]).is_none());
    }

    #[test]
    fn tuning_args_are_reported_by_flag_name() {
        assert_eq!(unhonored_sim_args(&args(&["--dt", "1"])), vec!["--dt"]);
        assert_eq!(
            unhonored_sim_args(&args(&["--dt", "1", "--output-interval", "10"])),
            vec!["--dt", "--output-interval"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--body", "mars"])),
            vec!["--body"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--epoch", "2024-03-20T12:00:00Z"])),
            vec!["--epoch"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--duration", "600"])),
            vec!["--duration"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--integrator", "rk4", "--rtol", "1e-6"])),
            vec!["--integrator", "--rtol"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--atmosphere", "nrlmsise00", "--f107", "200"])),
            vec!["--atmosphere", "--f107"]
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--space-weather", "auto"])),
            vec!["--space-weather"]
        );
    }

    /// A value equal to what the flag-less command line would mean changes
    /// nothing, so there is nothing to refuse — the point of the check is
    /// dropped *meaning*, not dropped text. For the two interval flags that
    /// baseline is their documented fallback, not a literal default value.
    #[test]
    fn args_that_ask_for_the_default_are_not_reported() {
        assert!(unhonored_sim_args(&args(&["--dt", "10"])).is_empty());
        assert!(unhonored_sim_args(&args(&["--body", "earth"])).is_empty());
        assert!(unhonored_sim_args(&args(&["--integrator", "dp45"])).is_empty());
        // output_interval falls back to dt, stream_interval to output_interval.
        assert!(unhonored_sim_args(&args(&["--output-interval", "10"])).is_empty());
        assert!(
            unhonored_sim_args(&args(&[
                "--output-interval",
                "30",
                "--stream-interval",
                "30"
            ])) == vec!["--output-interval"]
        );
        assert!(unhonored_sim_args(&args(&["--stream-interval", "10"])).is_empty());
    }

    /// The plugin backend flags survive into a client-started simulation via
    /// `PluginBackendOverrides`, so they must not be refused.
    #[test]
    fn plugin_backend_args_are_honored_when_idle() {
        assert!(
            unhonored_sim_args(&args(&[
                "--plugin-backend",
                "sync",
                "--plugin-backend-threshold",
                "64",
            ]))
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

    /// A stream interval the clamp erases is accepted; one it keeps is refused.
    ///
    /// `SimParams::from_sim_args` clamps the value into
    /// `[min(dt, output_interval), output_interval]`, so `--stream-interval 20`
    /// against the bare defaults resolves to the same 10 s a bare command line
    /// gives. Refusing it would fail a command that changes nothing.
    #[test]
    fn a_stream_interval_the_clamp_erases_is_accepted() {
        // Bare defaults: dt = output_interval = 10, so the clamp range is
        // [10, 10] and any written value lands on 10.
        assert_eq!(
            unhonored_sim_args(&args(&["--stream-interval", "20"])),
            Vec::<&str>::new()
        );
        assert_eq!(
            unhonored_sim_args(&args(&["--stream-interval", "0.001"])),
            Vec::<&str>::new()
        );

        // With room between dt and output_interval, a value inside the range
        // survives the clamp and does change the resolved parameters.
        assert_eq!(
            unhonored_sim_args(&args(&[
                "--dt",
                "1",
                "--output-interval",
                "10",
                "--stream-interval",
                "5"
            ])),
            vec!["--dt", "--output-interval", "--stream-interval"]
        );
        // Above the range it is clamped back to `output_interval`, which is
        // where a bare `--dt 1 --output-interval 10` would leave it anyway.
        assert_eq!(
            unhonored_sim_args(&args(&[
                "--dt",
                "1",
                "--output-interval",
                "10",
                "--stream-interval",
                "50"
            ])),
            vec!["--dt", "--output-interval"]
        );
    }
}
