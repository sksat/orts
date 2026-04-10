use clap::{Parser, Subcommand, ValueEnum};

/// Orts CLI — orbital mechanics simulation tool
#[derive(Parser, Debug)]
#[command(name = "orts")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a simulation and save results
    Run {
        #[command(flatten)]
        sim: SimArgs,

        /// Output path (use "stdout" to write to standard output)
        #[arg(long, default_value = "output.rrd")]
        output: String,

        /// Output format
        #[arg(long, default_value = "rrd")]
        format: OutputFormat,
    },
    /// Start WebSocket server for real-time streaming
    Serve {
        #[command(flatten)]
        sim: SimArgs,

        /// WebSocket server port
        #[arg(long, default_value_t = 9001)]
        port: u16,
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
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Rrd,
    Csv,
}

#[derive(Parser, Debug, Clone)]
pub struct SimArgs {
    /// Orbit altitude in km
    #[arg(long, default_value_t = 400.0)]
    pub altitude: f64,

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
    /// Format: key=value,key=value (keys: altitude, norad-id, tle-line1, tle-line2, id, name)
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
    /// When specified, orbit-related args (--altitude, --sat, --tle, etc.) are ignored.
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
    /// - `deterministic` (default): single tokio worker thread, bit-
    ///   for-bit reproducible. Used by oracle tests.
    /// - `throughput`: multi-worker tokio runtime, `orts run` fans
    ///   the per-satellite `step_controlled` out across CPU cores
    ///   via rayon. Higher wall-clock throughput at the cost of
    ///   bit-for-bit reproducibility.
    ///
    /// Ignored when `--plugin-backend=sync`. `orts serve` currently
    /// always runs in deterministic mode.
    #[arg(long, value_enum, default_value = "deterministic")]
    pub plugin_backend_async_mode: PluginAsyncModeChoice,
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum IntegratorChoice {
    /// Fixed-step 4th-order Runge-Kutta
    Rk4,
    /// Adaptive Dormand-Prince RK5(4)
    Dp45,
    /// Adaptive DOP853 8th-order Dormand-Prince (high accuracy)
    Dop853,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AtmosphereChoice {
    /// Piecewise exponential (US Standard Atmosphere 1976)
    Exponential,
    /// Harris-Priester (diurnal variation, uses Sun position)
    HarrisPriester,
    /// NRLMSISE-00 empirical model (uses F10.7 and Ap)
    Nrlmsise00,
}
