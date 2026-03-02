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

    /// Absolute tolerance for adaptive integrator (dp45)
    #[arg(long, default_value_t = 1e-10)]
    pub atol: f64,

    /// Relative tolerance for adaptive integrator (dp45)
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
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum IntegratorChoice {
    /// Fixed-step 4th-order Runge-Kutta
    Rk4,
    /// Adaptive Dormand-Prince RK5(4) (recommended)
    Dp45,
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
