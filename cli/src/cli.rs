use clap::{Parser, Subcommand, ValueEnum};

// The propagation frame is a runtime choice here and a type parameter in
// orts, so the enum lives with the frame abstraction it selects.
pub use crate::sim::frame::FrameChoice;

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

/// Where a `run` takes its simulation from, shown at the end of `run --help`.
pub(crate) const RUN_INPUT_HELP: &str = "\
Simulation input:
  A run takes its simulation either from a config (--config, or an orts.toml
  found in the current directory) or from an orbit on the command line (--sat,
  --tle, --omm, --tle-line1/--tle-line2, --norad-id). The tuning flags (--dt,
  --atol, --integrator, --duration, ...) apply to the second: with a config, set
  the same values in it instead. A tuning flag does not override an orts.toml
  found in the current directory; give an orbit, or write the value there.
  The --plugin-backend flags apply either way.";

/// Where a `serve` takes its simulation from, shown at the end of
/// `serve --help`.
pub(crate) const SERVE_INPUT_HELP: &str = "\
Simulation input:
  A server takes its simulation either from a config (--config) or from an
  orbit on the command line (--sat, --tle, --omm, --tle-line1/--tle-line2,
  --norad-id). The tuning flags (--dt, --atol, --integrator, --duration, ...)
  apply to the second: with a config, set the same values in it instead. With
  neither, the server starts idle, and a client sets the simulation, tuning
  values included, through start_simulation. The --plugin-backend flags apply
  to every simulation the server runs, including ones a client starts.";

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a simulation and save results
    #[command(after_long_help = RUN_INPUT_HELP)]
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
    #[command(after_long_help = SERVE_INPUT_HELP)]
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

/// The simulation a command runs, described on the command line.
///
/// A command takes its simulation from one of two inputs: a whole config
/// (`--config`), or an orbit (the `orbit` group) with the tuning flags that
/// adjust it (the `tuning` group). Every flag in either group conflicts with
/// `--config`, and the tuning flags need an orbit, so clap refuses a command
/// line that would have a flag dropped — a `--config` builds its simulation
/// from the config alone, and without an orbit there is nothing to tune. clap
/// checks these relations against the flags written on the command line, so
/// a flag left at its default never counts and one written with its default
/// value does. The plugin-backend flags belong to neither group: both commands
/// apply them whichever input they have.
///
/// The orbit flags each name a whole orbit, so they conflict with one another,
/// except `--tle-line1` and `--tle-line2`, which name one orbit together.
///
/// A subcommand that flattens this inherits the rule. One that lets the
/// command line adjust a config would need the inputs split first.
#[derive(Parser, Debug, Clone)]
#[group(skip)]
#[command(group = clap::ArgGroup::new("orbit").multiple(true))]
#[command(group = clap::ArgGroup::new("tuning").multiple(true).requires("orbit"))]
pub struct SimArgs {
    /// Central body name (e.g. earth, moon, mars)
    #[arg(
        long,
        default_value = "earth",
        group = "tuning",
        conflicts_with = "config"
    )]
    pub body: String,

    /// Integration time step in seconds
    #[arg(
        long,
        default_value_t = 10.0,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub dt: f64,

    /// Output interval in seconds (defaults to dt if not specified)
    #[arg(long, group = "tuning", conflicts_with = "config")]
    pub output_interval: Option<f64>,

    /// WebSocket streaming interval in seconds (defaults to output-interval)
    #[arg(long, group = "tuning", conflicts_with = "config")]
    pub stream_interval: Option<f64>,

    /// Simulation epoch in ISO 8601 format (e.g. "2024-03-20T12:00:00Z")
    #[arg(long, group = "tuning", conflicts_with = "config")]
    pub epoch: Option<String>,

    /// TLE file path (2-line or 3-line format), use "-" for stdin
    #[arg(
        long,
        group = "orbit",
        conflicts_with = "config",
        conflicts_with_all = ["omm", "tle_line1", "tle_line2", "norad_id", "sats"]
    )]
    pub tle: Option<String>,

    /// OMM file path (CCSDS JSON / KVN / XML), use "-" for stdin
    #[arg(
        long,
        group = "orbit",
        conflicts_with = "config",
        conflicts_with_all = ["tle", "tle_line1", "tle_line2", "norad_id", "sats"]
    )]
    pub omm: Option<String>,

    /// TLE line 1 (direct input, use with --tle-line2)
    #[arg(
        long,
        group = "orbit",
        conflicts_with = "config",
        requires = "tle_line2",
        conflicts_with_all = ["tle", "omm", "norad_id", "sats"]
    )]
    pub tle_line1: Option<String>,

    /// TLE line 2 (direct input, use with --tle-line1)
    #[arg(
        long,
        group = "orbit",
        conflicts_with = "config",
        requires = "tle_line1",
        conflicts_with_all = ["tle", "omm", "norad_id", "sats"]
    )]
    pub tle_line2: Option<String>,

    /// NORAD catalog number to fetch TLE from CelesTrak
    #[arg(
        long,
        group = "orbit",
        conflicts_with = "config",
        conflicts_with_all = ["tle", "omm", "tle_line1", "tle_line2", "sats"]
    )]
    pub norad_id: Option<u32>,

    /// Satellite specifications (repeatable).
    /// Format: key=value,key=value. One orbit per satellite: altitude /
    /// inclination / raan (circular; altitude defaults to 400 km, angles to 0),
    /// tle-line1 + tle-line2, or norad-id. Also: id, name, ballistic-coeff,
    /// srp-area-to-mass, srp-cr.
    /// Quick shorthand for simple cases; for generated or multi-satellite setups
    /// prefer a config file via --config (see `orts config example`).
    #[arg(
        long = "sat",
        num_args = 1,
        group = "orbit",
        conflicts_with = "config",
        conflicts_with_all = ["tle", "omm", "tle_line1", "tle_line2", "norad_id"]
    )]
    pub sats: Vec<String>,

    /// Integration method
    #[arg(
        long,
        default_value = "dp45",
        group = "tuning",
        conflicts_with = "config"
    )]
    pub integrator: IntegratorChoice,

    /// Absolute tolerance for adaptive integrators (dp45, dop853)
    #[arg(
        long,
        default_value_t = 1e-10,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub atol: f64,

    /// Relative tolerance for adaptive integrators (dp45, dop853)
    #[arg(
        long,
        default_value_t = 1e-8,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub rtol: f64,

    /// How closely the time a state reaches a limit is located [s].
    ///
    /// A reaction wheel filling up happens mid-step, and the propagation
    /// halves the step until it has the time this narrow. The tolerance is how
    /// far the time it settles on can be from the sign change it found: a wheel
    /// driven at a constant 0.1 N·m is held up to 1e-4 N·m·s past its limit at
    /// the default. It bounds that localization and not the whole error — the
    /// sign change belongs to the computed trajectory, so the state's own
    /// integration error is in there too. Every halving costs one more
    /// evaluation of the step being narrowed, and the spacing of f64 at the
    /// time in question is the floor under the whole thing.
    #[arg(
        long,
        default_value_t = 1e-3,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub root_t_tolerance: f64,

    /// Atmospheric density model for drag computation
    #[arg(
        long,
        default_value = "exponential",
        group = "tuning",
        conflicts_with = "config"
    )]
    pub atmosphere: AtmosphereChoice,

    /// F10.7 solar radio flux [SFU] for NRLMSISE-00.
    /// Controls solar activity level: ~70 (solar min), ~150 (moderate), ~250 (solar max).
    /// Only used when --atmosphere=nrlmsise00.
    #[arg(
        long,
        default_value_t = 150.0,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub f107: f64,

    /// Ap geomagnetic index for NRLMSISE-00.
    /// Controls geomagnetic activity: ~4 (quiet), ~15 (moderate), ~50 (storm).
    /// Only used when --atmosphere=nrlmsise00 and --space-weather is not set.
    #[arg(
        long,
        default_value_t = 15.0,
        group = "tuning",
        conflicts_with = "config"
    )]
    pub ap: f64,

    /// Space weather data source for NRLMSISE-00.
    /// "auto": download from CelesTrak (cached for 24h).
    /// File path: load a CSSI-format file (SW-Last5Years.txt).
    /// Omit to use constant --f107/--ap values.
    #[arg(long, group = "tuning", conflicts_with = "config")]
    pub space_weather: Option<String>,

    /// Spherical-harmonic gravity field: path to an ICGEM .gfc file
    /// (EGM96 / EGM2008 / EIGEN-6C4). Replaces the J2/J3/J4 zonal model and
    /// sets mu to the file's GM. Earth only.
    #[arg(long, value_name = "PATH", group = "tuning", conflicts_with = "config")]
    pub gravity_field: Option<String>,

    /// Truncate the gravity field to this degree (default: the file's maximum).
    /// Only used with --gravity-field.
    #[arg(
        long,
        value_name = "N",
        group = "tuning",
        conflicts_with = "config",
        requires = "gravity_field"
    )]
    pub gravity_degree: Option<usize>,

    /// Truncate the gravity field to this order (default: = degree).
    /// Only used with --gravity-field.
    #[arg(
        long,
        value_name = "M",
        group = "tuning",
        conflicts_with = "config",
        requires = "gravity_field"
    )]
    pub gravity_order: Option<usize>,

    /// Inertial frame to propagate in.
    /// "simple-eci": ERA-only Earth rotation, no EOP (default).
    /// "gcrs": IAU 2006/2000A CIO chain with observed EOP (needs --eop).
    /// `gcrs` covers orbit-only `run`; attitude, controllers and `serve` are
    /// SimpleEci-only.
    ///
    /// `Option` rather than a defaulted value because presence matters:
    /// `--frame simple-eci` next to a `frame = "gcrs"` config is an explicit
    /// disagreement, and a defaulted value could not tell it from no flag at
    /// all. Absent means [`FrameChoice::SimpleEci`], via
    /// [`SimArgs::frame`](Self::frame).
    #[arg(
        long = "frame",
        value_name = "FRAME",
        group = "tuning",
        conflicts_with = "config"
    )]
    pub frame_arg: Option<FrameChoice>,

    /// Earth Orientation Parameters for --frame gcrs.
    /// "auto": download finals2000A.all from IERS (cached for 24h).
    /// File path: load an IERS finals2000A file.
    /// "zero": no observed EOP, IAU 2006 model CIP only (reproducible, not
    /// accurate — ERA is off by up to ~0.4 arcsecond).
    #[arg(
        long,
        value_name = "SOURCE",
        group = "tuning",
        conflicts_with = "config"
    )]
    pub eop: Option<String>,

    /// Total simulation duration in seconds. Omit to cover one orbit
    /// (`orts run`): each satellite's own, or — with `mode = "controlled"`,
    /// where the fleet shares one clock — the longest in the fleet.
    /// `orts serve` streams without end either way.
    #[arg(long, group = "tuning", conflicts_with = "config")]
    pub duration: Option<f64>,

    /// Path to simulation config file (JSON/TOML/YAML).
    /// The simulation comes from the config alone, so the orbit and tuning
    /// flags cannot be given with it; only the --plugin-backend flags apply on
    /// top.
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

    /// Async backend execution mode.
    ///
    /// Left out, `orts run` uses `throughput` and `orts serve` uses
    /// `deterministic`.
    ///
    /// - `throughput`: multi-worker tokio runtime,
    ///   `orts run` fans the per-satellite control step out
    ///   across CPU cores via rayon. Measurably faster on any
    ///   multi-core host. Since each satellite's control step
    ///   is independent (no shared mutable state between sats),
    ///   the result is byte-for-byte identical to deterministic
    ///   mode — the speedup comes for free.
    /// - `deterministic`: single tokio worker thread, strictly
    ///   sequential. Pick this if you need a hard scheduling-order
    ///   guarantee (e.g. for future features that introduce
    ///   cross-satellite side effects or shared mutable host state).
    ///
    /// Ignored when `--plugin-backend=sync`.
    ///
    /// `orts serve` builds its plugin runtime in the mode this flag asks
    /// for, including for fleets a client starts later — the server
    /// operator picks how its plugins run. Leaving the flag out keeps
    /// `serve` in `deterministic`, which is what a server that was never
    /// asked has always done.
    ///
    /// The speedup above is `run`'s. `serve` steps its satellites in turn
    /// and waits for each controller, so `throughput` buys it a
    /// multi-worker runtime rather than control steps that overlap.
    #[arg(long, value_enum)]
    pub plugin_backend_async_mode: Option<PluginAsyncModeChoice>,
}

impl SimArgs {
    /// The selected frame, defaulting to `SimpleEci` when `--frame` is absent.
    pub fn frame(&self) -> FrameChoice {
        self.frame_arg.unwrap_or(FrameChoice::SimpleEci)
    }

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

impl PluginAsyncModeChoice {
    /// The mode a command runs in when `--plugin-backend-async-mode` is left
    /// out.
    ///
    /// The two commands differ, which is why the flag carries no clap default
    /// of its own: `run` fans its control steps out across cores and runs
    /// `Throughput`, while a `serve` nobody asked runs `Deterministic`, the
    /// mode it ran in before the flag could reach its plugins at all.
    pub fn unspecified(is_serve: bool) -> Self {
        if is_serve {
            Self::Deterministic
        } else {
            Self::Throughput
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// `SimArgs` as clap parses it.
    ///
    /// Built, because a flag's `group = ...` reaches the group only when clap
    /// builds the command: before that, a group lists none of its members.
    fn built() -> clap::Command {
        let mut command = SimArgs::command();
        command.build();
        command
    }

    /// The flags `SimArgs` declares, by the id clap gives each.
    fn sim_arg_ids() -> Vec<String> {
        built()
            .get_arguments()
            .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
            .map(|arg| arg.get_id().to_string())
            .collect()
    }

    fn group_members(name: &str) -> Vec<String> {
        built()
            .get_groups()
            .find(|group| group.get_id().as_str() == name)
            .unwrap_or_else(|| panic!("SimArgs declares a `{name}` group"))
            .get_args()
            .map(|id| id.to_string())
            .collect()
    }

    /// Every flag `SimArgs` declares has a place in the input rule.
    ///
    /// A flag added without one would slip past both relations and be dropped
    /// in silence again, on whichever input does not read it: this is the
    /// declaration the rule depends on, checked rather than trusted.
    #[test]
    fn every_sim_arg_belongs_to_one_input() {
        let carried = [
            "config",
            "plugin_backend",
            "plugin_backend_threshold",
            "plugin_backend_async_mode",
        ];
        let orbit = group_members("orbit");
        let tuning = group_members("tuning");
        for id in sim_arg_ids() {
            let places = [
                carried.contains(&id.as_str()),
                orbit.contains(&id),
                tuning.contains(&id),
            ];
            assert_eq!(
                places.iter().filter(|&&in_it| in_it).count(),
                1,
                "`{id}` is in exactly one of carried / orbit / tuning: {places:?}"
            );
        }
    }

    /// Every orbit and tuning flag conflicts with `--config`, from its own
    /// side, so clap names the flag written rather than the whole group.
    #[test]
    fn every_orbit_and_tuning_flag_conflicts_with_the_config() {
        let command = built();
        for id in group_members("orbit")
            .into_iter()
            .chain(group_members("tuning"))
        {
            let arg = command
                .get_arguments()
                .find(|arg| arg.get_id().as_str() == id)
                .expect("a group member is an argument");
            let conflicts: Vec<String> = command
                .get_arg_conflicts_with(arg)
                .into_iter()
                .map(|other| other.get_id().to_string())
                .collect();
            assert!(
                conflicts.iter().any(|other| other == "config"),
                "`{id}` conflicts with --config: {conflicts:?}"
            );
        }
    }

    /// The orbit flags conflict with one another, except the two TLE lines,
    /// which name one orbit together.
    ///
    /// An orbit flag added without these conflicts would let two orbits
    /// through to `SimParams::from_sim_args`, which panics on the pair (#551).
    /// clap refuses a pair when either flag declares the conflict, so either
    /// side counts here.
    #[test]
    fn every_two_orbit_flags_conflict_except_the_tle_lines() {
        let command = built();
        let conflicts_of = |id: &str| -> Vec<String> {
            let arg = command
                .get_arguments()
                .find(|arg| arg.get_id().as_str() == id)
                .expect("a group member is an argument");
            command
                .get_arg_conflicts_with(arg)
                .into_iter()
                .map(|other| other.get_id().to_string())
                .collect()
        };
        let orbit = group_members("orbit");
        for (i, a) in orbit.iter().enumerate() {
            for b in &orbit[i + 1..] {
                let tle_lines = matches!(
                    (a.as_str(), b.as_str()),
                    ("tle_line1", "tle_line2") | ("tle_line2", "tle_line1")
                );
                let conflict = conflicts_of(a).contains(b) || conflicts_of(b).contains(a);
                assert_eq!(
                    conflict, !tle_lines,
                    "`{a}` and `{b}` conflict unless they are the TLE lines"
                );
            }
        }
    }

    /// The orbit group is what `has_orbit_args` looks at, so the command a
    /// flag needs and the input it gets agree.
    #[test]
    fn the_orbit_group_is_what_has_orbit_args_reads() {
        let mut orbit = group_members("orbit");
        orbit.sort();
        let mut read = vec!["norad_id", "omm", "sats", "tle", "tle_line1", "tle_line2"];
        read.sort();
        assert_eq!(orbit, read);
        for (flag, value) in [
            ("--sat", "altitude=400"),
            ("--tle", "x.tle"),
            ("--omm", "x.json"),
            ("--norad-id", "25544"),
        ] {
            let sim = SimArgs::try_parse_from(["orts", flag, value]).expect("valid args");
            assert!(sim.has_orbit_args(), "{flag} is an orbit");
        }
    }
}
