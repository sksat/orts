use clap::{Parser, Subcommand, ValueEnum};

/// orts CLI — orbital mechanics simulation tool
#[derive(Parser, Debug)]
#[command(name = "orts")]
#[command(after_help = AFTER_HELP)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Copy-pasteable examples and the environment the CLI reads, shown at the end
/// of `orts --help` (and `-h`). Kept here so the most common — and the most
/// agent-relevant — workflows are discoverable without reading the docs.
pub(crate) const AFTER_HELP: &str = "\
Examples:
  # Run a simulation, recording to an .rrd file (the default)
  orts run --sat altitude=400

  # Run from a config file and write CSV to a path (use '-' for stdout)
  orts run --config mission.toml --format csv --output orbit.csv

  # Machine-readable run summary on stdout for scripts/agents
  # (simulation data must go to a file when --json is set)
  orts run --config mission.toml --json --output result.rrd

  # Get a starting config, then validate it
  orts config example > mission.toml
  orts config validate mission.toml

  # Live WebSocket server + embedded 3D viewer at http://localhost:9001
  orts serve --config mission.toml

Environment:
  RUST_LOG   Log filter, default \"warn,orts=info\": orts at info, dependencies
             at warn. Records go to stderr (stdout carries only what the
             command produces), and a WASM plugin's own log output arrives
             under the orts target.
               RUST_LOG=warn            quiet — warnings and errors only
               RUST_LOG=debug           everything, dependencies included
               RUST_LOG=orts=debug      just ours, more detail
  NO_COLOR   Set to any non-empty value to disable styled records. They are
             already unstyled when stderr is not a terminal.
";

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a simulation and save results
    Run {
        #[command(flatten)]
        sim: SimArgs,

        /// Output path for the simulation data. Use "-" (or the legacy
        /// "stdout" alias) to write to standard output. When omitted, the
        /// default is "output.rrd" for --format rrd and standard output for
        /// --format csv.
        #[arg(long)]
        output: Option<String>,

        /// Output data format
        #[arg(long, default_value = "rrd")]
        format: OutputFormat,

        /// Emit a machine-readable run summary as JSON on stdout (status,
        /// per-satellite final state, and the output artifact). Diagnostics
        /// and logs stay on stderr. Because stdout then carries the JSON,
        /// the simulation data must go to a file: combining --json with data
        /// on stdout is rejected.
        #[arg(long)]
        json: bool,
    },
    /// Start WebSocket server for real-time streaming
    Serve {
        #[command(flatten)]
        sim: SimArgs,

        /// WebSocket server port
        #[arg(long, default_value_t = 9001)]
        port: u16,

        /// Wire one declared stream-io stream to stdin/stdout with the
        /// kble-socket protocol, for running as a kble `exec:` plug
        /// (e.g. `--stream-stdio sat0/comlink`). The stream is reserved
        /// (its WS endpoint answers 409); when the stdio peer closes,
        /// the server shuts down (the kble harness owns this process).
        #[arg(long, value_name = "SAT/STREAM")]
        stream_stdio: Option<String>,
    },
    /// Replay a recorded simulation file through the WebSocket viewer
    Replay {
        /// Path to the .rrd file to replay
        input: String,

        /// WebSocket server port
        #[arg(long, default_value_t = 9001)]
        port: u16,
    },
    /// Convert between data formats
    Convert {
        /// Input file path
        input: String,

        /// Output format
        #[arg(long)]
        format: OutputFormat,

        /// Output path (default: stdout)
        #[arg(long)]
        output: Option<String>,
    },
    /// Inspect and validate simulation config files
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Print an example simulation config to stdout
    Example {
        /// Config file format
        #[arg(long, default_value = "toml")]
        format: ConfigFormat,
    },
    /// Validate a simulation config file and report the result
    Validate {
        /// Path to the config file (.toml / .json / .yaml)
        path: String,

        /// Emit a machine-readable JSON verdict on stdout (the human-readable
        /// message goes to stderr otherwise). Exit code is 0 when valid, 2
        /// when invalid, either way.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConfigFormat {
    Toml,
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Rrd,
    Csv,
}

#[derive(Parser, Debug, Clone)]
pub struct SimArgs {
    /// Central body name (e.g. earth, moon, mars)
    #[arg(long, default_value = "earth")]
    pub body: String,

    /// Integration time step in seconds
    #[arg(long, default_value_t = 10.0)]
    pub dt: f64,

    /// Output interval in seconds (defaults to dt if not specified)
    #[arg(long)]
    pub output_interval: Option<f64>,

    /// WebSocket streaming interval in seconds (defaults to output-interval)
    #[arg(long)]
    pub stream_interval: Option<f64>,

    /// Simulation epoch in ISO 8601 format (e.g. "2024-03-20T12:00:00Z")
    #[arg(long)]
    pub epoch: Option<String>,

    /// TLE file path (2-line or 3-line format), use "-" for stdin
    #[arg(long)]
    pub tle: Option<String>,

    /// OMM file path (CCSDS JSON / KVN / XML), use "-" for stdin
    #[arg(long)]
    pub omm: Option<String>,

    /// TLE line 1 (direct input, use with --tle-line2)
    #[arg(long)]
    pub tle_line1: Option<String>,

    /// TLE line 2 (direct input, use with --tle-line1)
    #[arg(long)]
    pub tle_line2: Option<String>,

    /// NORAD catalog number to fetch TLE from CelesTrak
    #[arg(long)]
    pub norad_id: Option<u32>,

    /// Satellite specifications (repeatable).
    /// Format: key=value,key=value (keys: altitude, norad-id, tle-line1, tle-line2, id, name).
    /// Quick shorthand for simple cases; for generated or multi-satellite setups
    /// prefer a config file via --config (see `orts config example`).
    #[arg(long = "sat", num_args = 1)]
    pub sats: Vec<String>,

    /// Integration method
    #[arg(long, default_value = "dp45")]
    pub integrator: IntegratorChoice,

    /// Absolute tolerance for adaptive integrators (dp45, dop853)
    #[arg(long, default_value_t = 1e-10)]
    pub atol: f64,

    /// Relative tolerance for adaptive integrators (dp45, dop853)
    #[arg(long, default_value_t = 1e-8)]
    pub rtol: f64,

    /// Atmospheric density model for drag computation
    #[arg(long, default_value = "exponential")]
    pub atmosphere: AtmosphereChoice,

    /// F10.7 solar radio flux [SFU] for NRLMSISE-00.
    /// Controls solar activity level: ~70 (solar min), ~150 (moderate), ~250 (solar max).
    /// Only used when --atmosphere=nrlmsise00.
    #[arg(long, default_value_t = 150.0)]
    pub f107: f64,

    /// Ap geomagnetic index for NRLMSISE-00.
    /// Controls geomagnetic activity: ~4 (quiet), ~15 (moderate), ~50 (storm).
    /// Only used when --atmosphere=nrlmsise00 and --space-weather is not set.
    #[arg(long, default_value_t = 15.0)]
    pub ap: f64,

    /// Space weather data source for NRLMSISE-00.
    /// "auto": download from CelesTrak (cached for 24h).
    /// File path: load a CSSI-format file (SW-Last5Years.txt).
    /// Omit to use constant --f107/--ap values.
    #[arg(long)]
    pub space_weather: Option<String>,

    /// Total simulation duration in seconds (overrides orbital period)
    #[arg(long)]
    pub duration: Option<f64>,

    /// Path to simulation config file (JSON/TOML/YAML).
    /// When specified, orbit-related args (--sat, --tle, etc.) are ignored.
    #[arg(long)]
    pub config: Option<String>,

    /// WASM plugin backend.
    ///
    /// - `sync`: one OS thread per controlled satellite. Fastest
    ///   dispatch (~3 µs/tick on Pulley) but scales poorly beyond a
    ///   few hundred satellites because of thread stack overhead.
    /// - `async`: one tokio worker thread multiplexes all controller
    ///   tasks via wasmtime fiber suspension. Higher per-tick
    ///   dispatch overhead but scales to thousands of satellites.
    ///   Requires the `plugin-wasm-async` build feature.
    /// - `auto` (default): pick automatically based on satellite
    ///   count. Uses `sync` when `n_sats <= threshold`, `async`
    ///   otherwise (when available). Threshold is derived from the
    ///   machine's thread count; override with `--plugin-backend-threshold`.
    #[arg(long, value_enum, default_value = "auto")]
    pub plugin_backend: PluginBackendChoice,

    /// Satellite-count threshold above which `--plugin-backend=auto`
    /// switches to the async backend.
    ///
    /// If unset, the default is derived from
    /// `std::thread::available_parallelism() * 32` (e.g. 256 on an
    /// 8-core machine), which keeps the sync backend engaged for
    /// small-fleet ergonomics while switching to async before the OS
    /// thread count becomes problematic.
    #[arg(long)]
    pub plugin_backend_threshold: Option<usize>,

    /// Async backend execution mode (`orts run` only).
    ///
    /// - `throughput` (default): multi-worker tokio runtime,
    ///   `orts run` fans the per-satellite `step_controlled` out
    ///   across CPU cores via rayon. Measurably faster on any
    ///   multi-core host. Since each satellite's `step_controlled`
    ///   is independent (no shared mutable state between sats),
    ///   the result is byte-for-byte identical to deterministic
    ///   mode — the speedup comes for free.
    /// - `deterministic`: single tokio worker thread, strictly
    ///   sequential. Pick this if you need a hard scheduling-order
    ///   guarantee (e.g. for future features that introduce
    ///   cross-satellite side effects or shared mutable host state).
    ///
    /// Ignored when `--plugin-backend=sync`. `orts serve` currently
    /// always runs in deterministic mode regardless of this flag.
    #[arg(long, value_enum, default_value = "throughput")]
    pub plugin_backend_async_mode: PluginAsyncModeChoice,
}

impl SimArgs {
    /// Returns true if explicit orbit-specifying arguments were provided.
    pub fn has_orbit_args(&self) -> bool {
        !self.sats.is_empty()
            || self.tle.is_some()
            || self.omm.is_some()
            || self.tle_line1.is_some()
            || self.tle_line2.is_some()
            || self.norad_id.is_some()
    }
}

/// Async WASM backend execution mode.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum PluginAsyncModeChoice {
    /// Bit-for-bit reproducible, single worker thread.
    Deterministic,
    /// Parallel, multi-worker runtime + rayon-driven sim loop.
    Throughput,
}

/// Explicit backend choice from CLI.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum PluginBackendChoice {
    /// Sync backend: one OS thread per satellite.
    Sync,
    /// Async backend: tokio tasks multiplexed on a single worker.
    /// Requires the `plugin-wasm-async` build feature.
    Async,
    /// Automatic selection based on `--plugin-backend-threshold`.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum IntegratorChoice {
    /// Fixed-step 4th-order Runge-Kutta
    Rk4,
    /// Adaptive Dormand-Prince RK5(4)
    Dp45,
    /// Adaptive DOP853 8th-order Dormand-Prince (high accuracy)
    Dop853,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum AtmosphereChoice {
    /// Piecewise exponential (US Standard Atmosphere 1976)
    Exponential,
    /// Harris-Priester (diurnal variation, uses Sun position)
    HarrisPriester,
    /// NRLMSISE-00 empirical model (uses F10.7 and Ap)
    Nrlmsise00,
}
