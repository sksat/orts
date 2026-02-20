use std::collections::{HashMap, VecDeque};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use orts_datamodel::archetypes::OrbitalState;
use orts_datamodel::components::{BodyRadius, GravitationalParameter};
use orts_datamodel::entity_path::EntityPath;
use orts_datamodel::recording::Recording;
use orts_datamodel::timeline::TimePoint;
use kaname::epoch::Epoch;
use orts_integrator::{AdvanceOutcome, DormandPrince, IntegrationOutcome, Integrator, Rk4, State, Tolerances};
use orts_orbits::{body::KnownBody, drag::AtmosphericDrag, events, events::SimulationEvent, gravity, kepler::KeplerianElements, orbital_system::OrbitalSystem, srp::SolarRadiationPressure, third_body::ThirdBodyGravity, tle::Tle};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Orts CLI — orbital mechanics simulation tool
#[derive(Parser, Debug)]
#[command(name = "orts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
enum OutputFormat {
    Rrd,
    Csv,
}

#[derive(Parser, Debug, Clone)]
struct SimArgs {
    /// Orbit altitude in km
    #[arg(long, default_value_t = 400.0)]
    altitude: f64,

    /// Central body name (e.g. earth, moon, mars)
    #[arg(long, default_value = "earth")]
    body: String,

    /// Integration time step in seconds
    #[arg(long, default_value_t = 10.0)]
    dt: f64,

    /// Output interval in seconds (defaults to dt if not specified)
    #[arg(long)]
    output_interval: Option<f64>,

    /// WebSocket streaming interval in seconds (defaults to output-interval)
    #[arg(long)]
    stream_interval: Option<f64>,

    /// Simulation epoch in ISO 8601 format (e.g. "2024-03-20T12:00:00Z")
    #[arg(long)]
    epoch: Option<String>,

    /// TLE file path (2-line or 3-line format), use "-" for stdin
    #[arg(long)]
    tle: Option<String>,

    /// TLE line 1 (direct input, use with --tle-line2)
    #[arg(long)]
    tle_line1: Option<String>,

    /// TLE line 2 (direct input, use with --tle-line1)
    #[arg(long)]
    tle_line2: Option<String>,

    /// NORAD catalog number to fetch TLE from CelesTrak
    #[arg(long)]
    norad_id: Option<u32>,

    /// Satellite specifications (repeatable).
    /// Format: key=value,key=value (keys: altitude, norad-id, tle-line1, tle-line2, id, name)
    #[arg(long = "sat", num_args = 1)]
    sats: Vec<String>,

    /// Integration method
    #[arg(long, default_value = "dp45")]
    integrator: IntegratorChoice,

    /// Absolute tolerance for adaptive integrator (dp45)
    #[arg(long, default_value_t = 1e-10)]
    atol: f64,

    /// Relative tolerance for adaptive integrator (dp45)
    #[arg(long, default_value_t = 1e-8)]
    rtol: f64,

    /// Atmospheric density model for drag computation
    #[arg(long, default_value = "exponential")]
    atmosphere: AtmosphereChoice,

    /// Total simulation duration in seconds (overrides orbital period)
    #[arg(long)]
    duration: Option<f64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum IntegratorChoice {
    /// Fixed-step 4th-order Runge-Kutta
    Rk4,
    /// Adaptive Dormand-Prince RK5(4) (recommended)
    Dp45,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AtmosphereChoice {
    /// Piecewise exponential (US Standard Atmosphere 1976)
    Exponential,
    /// Harris-Priester (diurnal variation, uses Sun position)
    HarrisPriester,
}

/// How the orbit was specified on the command line.
#[derive(Clone)]
enum OrbitSpec {
    /// Circular orbit from --altitude, with optional inclination and RAAN.
    Circular {
        altitude: f64,
        r0: f64,
        /// Orbital inclination in radians (0 = equatorial).
        inclination: f64,
        /// Right Ascension of Ascending Node in radians.
        raan: f64,
    },
    /// From a TLE (parsed into Keplerian elements).
    Tle { tle_data: Tle, elements: KeplerianElements },
}

/// Per-satellite specification.
#[derive(Clone)]
struct SatelliteSpec {
    /// Unique identifier used in entity paths and WebSocket messages.
    id: String,
    /// Display name (from TLE or user-provided).
    name: Option<String>,
    /// Orbit specification.
    orbit: OrbitSpec,
    /// Orbital period for this satellite.
    period: f64,
    /// Explicit ballistic coefficient Cd*A/(2m) [m²/kg] for drag.
    ballistic_coeff: Option<f64>,
    /// SRP cross-sectional area to mass ratio [m²/kg].
    srp_area_to_mass: Option<f64>,
    /// SRP radiation pressure coefficient (default: 1.5).
    srp_cr: Option<f64>,
}

impl SatelliteSpec {
    fn initial_state(&self, mu: f64) -> State {
        match &self.orbit {
            OrbitSpec::Circular { r0, inclination, raan, .. } => {
                let elements = KeplerianElements {
                    semi_major_axis: *r0,
                    eccentricity: 0.0,
                    inclination: *inclination,
                    raan: *raan,
                    argument_of_periapsis: 0.0,
                    true_anomaly: 0.0,
                };
                let (pos, vel) = elements.to_state_vector(mu);
                State { position: pos, velocity: vel }
            }
            OrbitSpec::Tle { elements, .. } => {
                let (pos, vel) = elements.to_state_vector(mu);
                State { position: pos, velocity: vel }
            }
        }
    }

    /// Altitude for display purposes.
    fn altitude(&self, body: &KnownBody) -> f64 {
        match &self.orbit {
            OrbitSpec::Circular { altitude, .. } => *altitude,
            OrbitSpec::Tle { elements, .. } => {
                let perigee_r = elements.semi_major_axis * (1.0 - elements.eccentricity);
                perigee_r - body.properties().radius
            }
        }
    }

    fn entity_path(&self) -> EntityPath {
        EntityPath::parse(&format!("/world/sat/{}", self.id))
    }
}

/// Satellite info sent in the WebSocket info message.
#[derive(Serialize, Clone, Debug)]
struct SatelliteInfo {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    altitude: f64,
    period: f64,
    /// Names of active perturbation force models (e.g. "drag", "srp", "third_body_sun").
    perturbations: Vec<String>,
}

/// Simulation parameters derived from CLI arguments.
struct SimParams {
    body: KnownBody,
    mu: f64,
    dt: f64,
    output_interval: f64,
    stream_interval: f64,
    epoch: Option<Epoch>,
    satellites: Vec<SatelliteSpec>,
    integrator: IntegratorChoice,
    tolerances: Tolerances,
    atmosphere: AtmosphereChoice,
}

impl SimParams {
    /// Build SimParams from CLI arguments.
    /// `is_serve`: when true and no orbit args are given, defaults to SSO+ISS.
    fn from_sim_args(args: &SimArgs, is_serve: bool) -> Self {
        let body = parse_body(&args.body);
        let mu = body.properties().mu;

        let epoch = match &args.epoch {
            Some(s) => Some(
                Epoch::from_iso8601(s)
                    .unwrap_or_else(|| panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)"))
            ),
            None => Some(Epoch::now()),
        };

        let satellites = if !args.sats.is_empty() {
            // --sat flags provided: parse each spec
            if args.tle.is_some() || args.tle_line1.is_some() || args.norad_id.is_some() {
                panic!("Cannot specify both --sat and --tle/--tle-line1/--tle-line2/--norad-id");
            }
            args.sats.iter().enumerate().map(|(i, s)| {
                let mut spec = parse_sat_spec(s, body);
                if spec.id.is_empty() || spec.id == "auto" {
                    spec.id = format!("sat-{i}");
                }
                spec
            }).collect()
        } else {
            // No --sat flags: use legacy single-satellite args
            let tle_opt = Self::parse_tle_from_args(args);

            if let Some(tle) = tle_opt {
                let elements = tle.to_keplerian_elements(mu);
                let period = elements.period(mu);
                let sat_name = tle.name.clone();
                vec![SatelliteSpec {
                    id: "default".to_string(),
                    name: sat_name,
                    orbit: OrbitSpec::Tle { tle_data: tle, elements },
                    period,
                    ballistic_coeff: None,
                    srp_area_to_mass: None,
                    srp_cr: None,
                }]
            } else if is_serve && args.altitude == 400.0 && args.tle.is_none() && args.tle_line1.is_none() && args.norad_id.is_none() {
                // serve with no explicit orbit → SSO + ISS default
                Self::default_serve_satellites(body, mu)
            } else {
                // Single circular orbit
                let r0 = body.properties().radius + args.altitude;
                let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
                vec![SatelliteSpec {
                    id: "default".to_string(),
                    name: None,
                    orbit: OrbitSpec::Circular { altitude: args.altitude, r0, inclination: 0.0, raan: 0.0 },
                    period,
                    ballistic_coeff: None,
                    srp_area_to_mass: None,
                    srp_cr: None,
                }]
            }
        };

        let output_interval = args.output_interval.unwrap_or(args.dt);
        let stream_interval = args
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(args.dt, output_interval);

        // Apply --duration override: replace each satellite's period with the user-specified duration
        let satellites = if let Some(dur) = args.duration {
            satellites.into_iter().map(|mut s| { s.period = dur; s }).collect()
        } else {
            satellites
        };

        Self {
            body,
            mu,
            dt: args.dt,
            output_interval,
            stream_interval,
            epoch,
            satellites,
            integrator: args.integrator,
            tolerances: Tolerances { atol: args.atol, rtol: args.rtol },
            atmosphere: args.atmosphere,
        }
    }

    /// Default satellites for `serve` with no orbit args: SSO 800km + ISS.
    fn default_serve_satellites(body: KnownBody, mu: f64) -> Vec<SatelliteSpec> {
        let mut sats = Vec::new();

        // SSO at 800 km (always available, no network needed)
        let r0 = body.properties().radius + 800.0;
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        sats.push(SatelliteSpec {
            id: "sso".to_string(),
            name: Some("SSO 800km".to_string()),
            orbit: OrbitSpec::Circular {
                altitude: 800.0, r0,
                inclination: 98.6_f64.to_radians(),
                raan: 0.0,
            },
            period,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
        });

        // ISS: try online sources, fall back to embedded TLE
        let iss_tle = try_fetch_tle_by_norad_id(25544).unwrap_or_else(|| {
            eprintln!("Online TLE sources unavailable. Using embedded ISS TLE.");
            // Embedded ISS TLE (updated 2026-02-13)
            Tle::parse(
                "0 ISS (ZARYA)\n\
                 1 25544U 98067A   26044.11739808  .00007930  00000-0  15398-3 0  9991\n\
                 2 25544  51.6313 193.8240 0011114  93.1734 267.0526 15.48574923552528",
            )
            .expect("embedded ISS TLE must be valid")
        });
        let elements = iss_tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let sat_name = iss_tle.name.clone();
        sats.push(SatelliteSpec {
            id: "iss".to_string(),
            name: sat_name,
            orbit: OrbitSpec::Tle { tle_data: iss_tle, elements },
            period,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
        });

        sats
    }

    fn parse_tle_from_args(args: &SimArgs) -> Option<Tle> {
        // --norad-id: fetch from CelesTrak
        if let Some(norad_id) = args.norad_id {
            if args.tle.is_some() || args.tle_line1.is_some() {
                panic!("Cannot specify both --norad-id and --tle/--tle-line1/--tle-line2");
            }
            return Some(fetch_tle_by_norad_id(norad_id));
        }

        if let Some(path) = &args.tle {
            let text = if path == "-" {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .unwrap_or_else(|e| panic!("Failed to read TLE from stdin: {e}"));
                buf
            } else {
                std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("Failed to read TLE file '{path}': {e}"))
            };
            Some(Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if let (Some(line1), Some(line2)) = (&args.tle_line1, &args.tle_line2) {
            let text = format!("{line1}\n{line2}");
            Some(Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE: {e}")))
        } else if args.tle_line1.is_some() || args.tle_line2.is_some() {
            panic!("Both --tle-line1 and --tle-line2 must be specified together");
        } else {
            None
        }
    }
}

/// Parse a satellite specification string (key=value,key=value).
fn parse_sat_spec(s: &str, body: KnownBody) -> SatelliteSpec {
    let mu = body.properties().mu;
    let mut id = String::new();
    let mut name: Option<String> = None;
    let mut altitude: Option<f64> = None;
    let mut inclination: Option<f64> = None;
    let mut raan: Option<f64> = None;
    let mut norad_id: Option<u32> = None;
    let mut tle_line1: Option<String> = None;
    let mut tle_line2: Option<String> = None;
    let mut ballistic_coeff: Option<f64> = None;
    let mut srp_area_to_mass: Option<f64> = None;
    let mut srp_cr: Option<f64> = None;

    for part in s.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            match key.trim() {
                "id" => id = value.trim().to_string(),
                "name" => name = Some(value.trim().to_string()),
                "altitude" => altitude = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid altitude: {value}"))),
                "inclination" => inclination = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid inclination: {value}"))),
                "raan" => raan = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid raan: {value}"))),
                "norad-id" => norad_id = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid norad-id: {value}"))),
                "tle-line1" => tle_line1 = Some(value.trim().to_string()),
                "tle-line2" => tle_line2 = Some(value.trim().to_string()),
                "ballistic-coeff" => ballistic_coeff = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid ballistic-coeff: {value}"))),
                "srp-area-to-mass" => srp_area_to_mass = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid srp-area-to-mass: {value}"))),
                "srp-cr" => srp_cr = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid srp-cr: {value}"))),
                k => panic!("Unknown satellite spec key: {k}"),
            }
        }
    }

    // Determine orbit
    let (orbit, period, derived_name) = if let Some(norad) = norad_id {
        let tle = fetch_tle_by_norad_id(norad);
        let elements = tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let tle_name = tle.name.clone();
        (OrbitSpec::Tle { tle_data: tle, elements }, period, tle_name)
    } else if let (Some(l1), Some(l2)) = (tle_line1, tle_line2) {
        let text = format!("{l1}\n{l2}");
        let tle = Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE in --sat: {e}"));
        let elements = tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let tle_name = tle.name.clone();
        (OrbitSpec::Tle { tle_data: tle, elements }, period, tle_name)
    } else {
        let alt = altitude.unwrap_or(400.0);
        let r0 = body.properties().radius + alt;
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        let inc = inclination.unwrap_or(0.0).to_radians();
        let ra = raan.unwrap_or(0.0).to_radians();
        (OrbitSpec::Circular { altitude: alt, r0, inclination: inc, raan: ra }, period, None)
    };

    if id.is_empty() {
        id = "auto".to_string();
    }

    SatelliteSpec {
        id,
        name: name.or(derived_name),
        orbit,
        period,
        ballistic_coeff,
        srp_area_to_mass,
        srp_cr,
    }
}

/// A single state snapshot used in history messages.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct HistoryState {
    satellite_id: String,
    t: f64,
    position: [f64; 3],
    velocity: [f64; 3],
    semi_major_axis: f64,
    eccentricity: f64,
    inclination: f64,
    raan: f64,
    argument_of_periapsis: f64,
    true_anomaly: f64,
    /// Per-force acceleration magnitudes [km/s²]: "gravity", "drag", "srp", etc.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    accelerations: HashMap<String, f64>,
}

/// Create a HistoryState from position/velocity, computing Keplerian elements.
fn make_history_state(
    satellite_id: &str,
    t: f64,
    pos: &nalgebra::Vector3<f64>,
    vel: &nalgebra::Vector3<f64>,
    mu: f64,
    accelerations: HashMap<String, f64>,
) -> HistoryState {
    let elements = KeplerianElements::from_state_vector(pos, vel, mu);
    HistoryState {
        satellite_id: satellite_id.to_string(),
        t,
        position: [pos.x, pos.y, pos.z],
        velocity: [vel.x, vel.y, vel.z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        accelerations,
    }
}

/// Compute acceleration breakdown as a HashMap from an OrbitalSystem.
fn accel_breakdown(system: &OrbitalSystem, t: f64, state: &State) -> HashMap<String, f64> {
    system
        .acceleration_breakdown(t, state)
        .into_iter()
        .map(|(name, mag)| (name.to_string(), mag))
        .collect()
}

/// Bounded buffer that accumulates history states and periodically flushes to .rrd segments.
struct HistoryBuffer {
    /// Recent states kept in memory.
    states: VecDeque<HistoryState>,
    /// Maximum number of states to keep in memory before flushing.
    capacity: usize,
    /// Directory for .rrd segment files.
    data_dir: PathBuf,
    /// Number of segment files written so far.
    segment_count: u32,
    /// Gravitational parameter (for computing Keplerian elements from loaded data).
    mu: f64,
}

impl HistoryBuffer {
    fn new(capacity: usize, data_dir: PathBuf, mu: f64) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
        HistoryBuffer {
            states: VecDeque::new(),
            capacity,
            data_dir,
            segment_count: 0,
            mu,
        }
    }

    /// Push a state into the buffer. Flushes to .rrd if capacity is exceeded.
    fn push(&mut self, state: HistoryState) {
        self.states.push_back(state);
        if self.states.len() > self.capacity {
            self.flush();
        }
    }

    /// Flush the oldest half of the buffer to a .rrd segment file.
    fn flush(&mut self) {
        let flush_count = self.states.len() / 2;
        if flush_count == 0 {
            return;
        }

        let to_flush: Vec<HistoryState> = self.states.drain(..flush_count).collect();

        let mut rec = Recording::new();

        for (i, hs) in to_flush.iter().enumerate() {
            let sat_path = EntityPath::parse(&format!("/world/sat/{}", hs.satellite_id));
            let tp = TimePoint::new()
                .with_sim_time(hs.t)
                .with_step(i as u64);
            let os = OrbitalState::new(
                nalgebra::Vector3::new(hs.position[0], hs.position[1], hs.position[2]),
                nalgebra::Vector3::new(hs.velocity[0], hs.velocity[1], hs.velocity[2]),
            );
            rec.log_orbital_state(&sat_path, &tp, &os);
        }

        let seg_path = self
            .data_dir
            .join(format!("seg_{:04}.rrd", self.segment_count));
        if let Err(e) =
            orts_datamodel::rerun_export::save_as_rrd(&rec, "orts", seg_path.to_str().unwrap())
        {
            eprintln!("Warning: failed to flush segment: {e}");
            return;
        }

        self.segment_count += 1;
    }

    /// Load all data: .rrd segments + in-memory buffer, sorted by time.
    fn load_all(&self) -> Vec<HistoryState> {
        let mut all = Vec::new();

        // Read .rrd segment files in order
        for i in 0..self.segment_count {
            let seg_path = self.data_dir.join(format!("seg_{i:04}.rrd"));
            match orts_datamodel::rerun_export::load_from_rrd(seg_path.to_str().unwrap()) {
                Ok(rows) => {
                    for row in rows {
                        let pos = nalgebra::Vector3::new(row.x, row.y, row.z);
                        let vel = nalgebra::Vector3::new(row.vx, row.vy, row.vz);
                        // Entity path info is not preserved in RrdRow; extract from entity_path if available
                        let sid = row.entity_path.as_deref()
                            .and_then(|p| p.rsplit('/').next())
                            .unwrap_or("default");
                        all.push(make_history_state(sid, row.t, &pos, &vel, self.mu, HashMap::new()));
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to read segment {i}: {e}");
                }
            }
        }

        // Append in-memory buffer
        all.extend(self.states.iter().cloned());

        // Sort by time for multi-satellite interleaving
        all.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        all
    }

    /// Query states within a time range, optionally downsampled.
    fn query_range(&self, t_min: f64, t_max: f64, max_points: Option<usize>) -> Vec<HistoryState> {
        let all = self.load_all();
        let filtered: Vec<HistoryState> = all
            .into_iter()
            .filter(|s| s.t >= t_min && s.t <= t_max)
            .collect();
        match max_points {
            Some(mp) => Self::downsample(&filtered, mp),
            None => filtered,
        }
    }

    /// Downsample a list of states to at most `max_points`, always preserving first and last.
    fn downsample(states: &[HistoryState], max_points: usize) -> Vec<HistoryState> {
        let n = states.len();
        if n <= max_points || max_points < 2 {
            return states.to_vec();
        }

        let mut result = Vec::with_capacity(max_points);
        result.push(states[0].clone());

        // Distribute remaining (max_points - 2) samples evenly across the interior
        let interior = max_points - 2;
        for i in 1..=interior {
            let idx = i * (n - 1) / (interior + 1);
            result.push(states[idx].clone());
        }

        result.push(states[n - 1].clone());
        result
    }
}

/// Client-to-server WebSocket message.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "query_range")]
    QueryRange {
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        satellite_id: Option<String>,
    },
}

/// Server-to-client WebSocket message.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type")]
enum WsMessage {
    /// Simulation metadata sent once when a client connects.
    #[serde(rename = "info")]
    Info {
        mu: f64,
        dt: f64,
        output_interval: f64,
        stream_interval: f64,
        central_body: String,
        central_body_radius: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        epoch_jd: Option<f64>,
        satellites: Vec<SatelliteInfo>,
    },
    /// A single simulation state snapshot.
    #[serde(rename = "state")]
    State {
        satellite_id: String,
        t: f64,
        position: [f64; 3],
        velocity: [f64; 3],
        semi_major_axis: f64,
        eccentricity: f64,
        inclination: f64,
        raan: f64,
        argument_of_periapsis: f64,
        true_anomaly: f64,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        accelerations: HashMap<String, f64>,
    },
    /// Downsampled history overview sent on connect.
    #[serde(rename = "history")]
    History { states: Vec<HistoryState> },
    /// Full-resolution history chunk sent in background.
    #[serde(rename = "history_detail")]
    HistoryDetail { states: Vec<HistoryState> },
    /// Marker indicating all detail chunks have been sent.
    #[serde(rename = "history_detail_complete")]
    HistoryDetailComplete,
    /// Response to a client query_range request.
    #[serde(rename = "query_range_response")]
    QueryRangeResponse {
        t_min: f64,
        t_max: f64,
        states: Vec<HistoryState>,
    },
    /// Notification that a satellite's simulation has terminated.
    #[serde(rename = "simulation_terminated")]
    SimulationTerminated {
        satellite_id: String,
        t: f64,
        reason: String,
    },
}

/// Try fetching a TLE by NORAD catalog number. Tries CelesTrak first, falls back to SatNOGS.
fn try_fetch_tle_by_norad_id(norad_id: u32) -> Option<Tle> {
    if let Some(tle) = fetch_tle_celestrak(norad_id) {
        return Some(tle);
    }
    eprintln!("CelesTrak failed, trying SatNOGS...");
    fetch_tle_satnogs(norad_id)
}

/// Fetch a TLE by NORAD catalog number, panicking on failure.
fn fetch_tle_by_norad_id(norad_id: u32) -> Tle {
    try_fetch_tle_by_norad_id(norad_id)
        .unwrap_or_else(|| panic!("Failed to fetch TLE for NORAD ID {norad_id} from any source"))
}

/// Try fetching TLE from CelesTrak (3LE format).
fn fetch_tle_celestrak(norad_id: u32) -> Option<Tle> {
    let url = format!(
        "https://celestrak.org/NORAD/elements/gp.php?CATNR={norad_id}&FORMAT=3LE"
    );
    eprintln!("Fetching TLE for NORAD ID {norad_id} from CelesTrak...");
    let body = match ureq::get(&url).call() {
        Ok(mut resp) => match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read CelesTrak response: {e}");
                return None;
            }
        },
        Err(e) => {
            eprintln!("Failed to fetch TLE from CelesTrak: {e}");
            return None;
        }
    };
    if body.trim().is_empty() {
        eprintln!("No TLE data found on CelesTrak for NORAD ID {norad_id}");
        return None;
    }
    match Tle::parse(&body) {
        Ok(tle) => Some(tle),
        Err(e) => {
            eprintln!("Failed to parse CelesTrak TLE: {e}");
            None
        }
    }
}

/// Try fetching TLE from SatNOGS DB (JSON API).
fn fetch_tle_satnogs(norad_id: u32) -> Option<Tle> {
    let url = format!(
        "https://db.satnogs.org/api/tle/?norad_cat_id={norad_id}&format=json"
    );
    eprintln!("Fetching TLE for NORAD ID {norad_id} from SatNOGS...");
    let body = match ureq::get(&url).call() {
        Ok(mut resp) => match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read SatNOGS response: {e}");
                return None;
            }
        },
        Err(e) => {
            eprintln!("Failed to fetch TLE from SatNOGS: {e}");
            return None;
        }
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to parse SatNOGS JSON: {e}");
            return None;
        }
    };
    let entry = entries.first()?;
    let tle0 = entry["tle0"].as_str().unwrap_or("");
    let tle1 = entry["tle1"].as_str()?;
    let tle2 = entry["tle2"].as_str()?;
    let tle_text = format!("{tle0}\n{tle1}\n{tle2}");
    match Tle::parse(&tle_text) {
        Ok(tle) => Some(tle),
        Err(e) => {
            eprintln!("Failed to parse SatNOGS TLE: {e}");
            None
        }
    }
}

fn parse_body(s: &str) -> KnownBody {
    match s {
        "sun" => KnownBody::Sun,
        "mercury" => KnownBody::Mercury,
        "venus" => KnownBody::Venus,
        "earth" => KnownBody::Earth,
        "moon" => KnownBody::Moon,
        "mars" => KnownBody::Mars,
        "jupiter" => KnownBody::Jupiter,
        "saturn" => KnownBody::Saturn,
        "uranus" => KnownBody::Uranus,
        "neptune" => KnownBody::Neptune,
        _ => panic!("Unknown body: {s}"),
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            sim,
            output,
            format,
        } => run_simulation_cmd(&sim, &output, format),
        Commands::Serve { sim, port } => run_server(&sim, port),
        Commands::Convert {
            input,
            format,
            output,
        } => run_convert(&input, format, output.as_deref()),
    }
}

fn run_simulation_cmd(sim: &SimArgs, output: &str, format: OutputFormat) {
    let params = SimParams::from_sim_args(sim, false);

    // Determine effective format: stdout defaults to csv if format not explicitly set.
    let rec = run_simulation(&params);

    match (output, format) {
        ("stdout", OutputFormat::Csv) | (_, OutputFormat::Csv) => {
            print_recording_as_csv(&rec, &params);
        }
        ("stdout", OutputFormat::Rrd) => {
            eprintln!("Error: cannot write .rrd format to stdout. Use --format csv or specify a file path.");
            std::process::exit(1);
        }
        (path, OutputFormat::Rrd) => {
            orts_datamodel::rerun_export::save_as_rrd(&rec, "orts", path)
                .unwrap_or_else(|e| {
                    eprintln!("Error saving .rrd: {e}");
                    std::process::exit(1);
                });
            eprintln!("Saved to {path}");
        }
    }
}

/// Build an OrbitalSystem for the given body, using ZonalHarmonics if available.
///
/// When `epoch` is provided, epoch-dependent perturbations (third-body gravity)
/// are automatically enabled. When the satellite has a TLE with non-zero B*,
/// atmospheric drag is added (Earth only).
fn build_orbital_system(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteSpec,
    atmosphere: AtmosphereChoice,
) -> OrbitalSystem {
    let props = body.properties();
    let gravity_field: Box<dyn gravity::GravityField> = match props.j2 {
        Some(j2) => Box::new(gravity::ZonalHarmonics {
            r_body: props.radius,
            j2,
            j3: props.j3,
            j4: props.j4,
        }),
        None => Box::new(gravity::PointMass),
    };
    let mut system = OrbitalSystem::new(mu, gravity_field)
        .with_body_radius(props.radius);

    // Set epoch for time-dependent perturbations
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        // Third-body gravity: Sun (always), Moon (Earth only)
        system = system.with_perturbation(Box::new(ThirdBodyGravity::sun()));
        if *body == KnownBody::Earth {
            system = system.with_perturbation(Box::new(ThirdBodyGravity::moon()));
        }
    }

    // Atmospheric drag (Earth only)
    // Enable when: TLE has non-zero B* (implies drag-relevant orbit), or user provides ballistic-coeff
    if *body == KnownBody::Earth {
        let has_tle_drag = matches!(&sat.orbit, OrbitSpec::Tle { tle_data, .. } if tle_data.bstar.abs() > 1e-15);
        if has_tle_drag || sat.ballistic_coeff.is_some() {
            let drag = match atmosphere {
                AtmosphereChoice::Exponential => {
                    AtmosphericDrag::for_earth(sat.ballistic_coeff)
                }
                AtmosphereChoice::HarrisPriester => {
                    AtmosphericDrag::for_earth(sat.ballistic_coeff)
                        .with_atmosphere(Box::new(
                            tobari::harris_priester::HarrisPriester::new(),
                        ))
                }
            };
            system = system.with_perturbation(Box::new(drag));
        }
    }

    // Solar Radiation Pressure (requires epoch for Sun position)
    if epoch.is_some()
        && let Some(am) = sat.srp_area_to_mass
    {
        let mut srp = SolarRadiationPressure::for_earth(Some(am));
        if let Some(cr) = sat.srp_cr {
            srp = srp.with_cr(cr);
        }
        system = system.with_perturbation(Box::new(srp));
    }

    system
}

/// Run the simulation and return a Recording.
fn run_simulation(params: &SimParams) -> Recording {
    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));

    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

    for sat in &params.satellites {
        let system = build_orbital_system(&params.body, params.mu, params.epoch, sat, params.atmosphere);
        let initial = sat.initial_state(params.mu);
        let sat_path = sat.entity_path();

        let mut step: u64 = 0;
        let record_state = |rec: &mut Recording, t: f64, step: u64, state: &State| {
            let tp = TimePoint::new().with_sim_time(t).with_step(step);
            let os = OrbitalState::new(state.position, state.velocity);
            rec.log_orbital_state(&sat_path, &tp, &os);
        };

        record_state(&mut rec, 0.0, step, &initial);
        step += 1;

        let mut next_output_t = params.output_interval;
        let mut last_output_t = 0.0_f64;
        let props = params.body.properties();
        let body_radius = props.radius;
        let event_checker = events::collision_check(body_radius, props.atmosphere_altitude);

        let outcome: IntegrationOutcome<SimulationEvent> = match params.integrator {
            IntegratorChoice::Rk4 => {
                let callback = |t: f64, state: &State| {
                    if t >= next_output_t - 1e-9 {
                        record_state(&mut rec, t, step, state);
                        step += 1;
                        last_output_t = t;
                        next_output_t += params.output_interval;
                    }
                };
                Rk4.integrate_with_events(
                    &system,
                    initial,
                    0.0,
                    sat.period,
                    params.dt,
                    callback,
                    &event_checker,
                )
            }
            IntegratorChoice::Dp45 => {
                let t_end = sat.period;
                let mut stepper = DormandPrince.stepper(
                    &system,
                    initial,
                    0.0,
                    params.dt.min(t_end),
                    params.tolerances.clone(),
                );
                stepper.dt_min = 1e-12 * t_end.abs().max(1.0);

                let mut final_outcome: IntegrationOutcome<SimulationEvent> =
                    IntegrationOutcome::Completed(stepper.state().clone());

                while stepper.t() < t_end {
                    let t_target = next_output_t.min(t_end);
                    match stepper.advance_to(t_target, |_, _| {}, &event_checker) {
                        Ok(AdvanceOutcome::Reached) => {
                            if stepper.t() >= next_output_t - 1e-9 {
                                record_state(&mut rec, stepper.t(), step, stepper.state());
                                step += 1;
                                last_output_t = stepper.t();
                                next_output_t += params.output_interval;
                            }
                            final_outcome =
                                IntegrationOutcome::Completed(stepper.state().clone());
                        }
                        Ok(AdvanceOutcome::Event { reason }) => {
                            let t = stepper.t();
                            final_outcome = IntegrationOutcome::Terminated {
                                state: stepper.into_state(),
                                t,
                                reason,
                            };
                            break;
                        }
                        Err(e) => {
                            final_outcome = IntegrationOutcome::Error(e);
                            break;
                        }
                    }
                }

                final_outcome
            }
        };

        match &outcome {
            IntegrationOutcome::Completed(final_state) => {
                if (sat.period - last_output_t) > 1e-9 {
                    record_state(&mut rec, sat.period, step, final_state);
                }
            }
            IntegrationOutcome::Terminated { state, t, reason } => {
                eprintln!(
                    "Simulation terminated at t={t:.2}s for {}: {reason:?}",
                    sat.id
                );
                record_state(&mut rec, *t, step, state);
            }
            IntegrationOutcome::Error(err) => {
                eprintln!(
                    "Simulation error for {}: {err:?}",
                    sat.id
                );
            }
        }
    }

    // Use first satellite for metadata (backward compatibility)
    let first_sat = params.satellites.first();
    rec.metadata = orts_datamodel::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: first_sat.map(|s| s.altitude(&params.body)),
        period: first_sat.map(|s| s.period),
    };

    rec
}

/// Print a Recording as CSV to stdout.
fn print_recording_as_csv(rec: &Recording, params: &SimParams) {
    println!("# Orts 2-body orbit propagation");
    println!("# mu = {} km^3/s^2", params.mu);
    if let Some(epoch) = params.epoch {
        println!("# epoch_jd = {}", epoch.jd());
        println!("# epoch = {}", epoch.to_datetime());
    }
    println!(
        "# central_body = {}",
        params.body.properties().name.to_lowercase()
    );
    println!(
        "# central_body_radius = {} km",
        params.body.properties().radius
    );

    if params.satellites.len() == 1 {
        // Single satellite: backward-compatible CSV format (no satellite_id column)
        let sat = &params.satellites[0];
        match &sat.orbit {
            OrbitSpec::Circular { altitude, r0, .. } => {
                println!("# Initial orbit: circular at {} km altitude (r = {} km)", altitude, r0);
            }
            OrbitSpec::Tle { tle_data, elements } => {
                println!(
                    "# Initial orbit: from TLE (a = {:.1} km, e = {:.6}, i = {:.2}°)",
                    elements.semi_major_axis, elements.eccentricity, elements.inclination.to_degrees()
                );
                if let Some(name) = &tle_data.name {
                    println!("# satellite = {name}");
                }
            }
        }
        println!("# Period = {:.1} s ({:.1} min)", sat.period, sat.period / 60.0);
        println!("# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]");

        let sat_path = sat.entity_path();
        print_satellite_csv(rec, &sat_path, params.mu, false);
    } else {
        // Multi-satellite: add satellite_id as first column
        println!("# satellites = {}", params.satellites.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", "));
        println!("# satellite_id,t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s],a[km],e[-],i[rad],raan[rad],omega[rad],nu[rad]");

        for sat in &params.satellites {
            println!("# --- {} (period = {:.1} s) ---", sat.name.as_deref().unwrap_or(&sat.id), sat.period);
            let sat_path = sat.entity_path();
            print_satellite_csv(rec, &sat_path, params.mu, true);
        }
    }
}

fn print_satellite_csv(rec: &Recording, sat_path: &EntityPath, mu: f64, with_id: bool) {
    use orts_datamodel::component::Component;
    use orts_datamodel::components::{Position3D, Velocity3D};
    use orts_datamodel::timeline::TimelineName;

    let store = match rec.entity(sat_path) {
        Some(s) => s,
        None => return,
    };
    let pos_col = match store.columns.get(&Position3D::component_name()) {
        Some(c) => c,
        None => return,
    };
    let vel_col = match store.columns.get(&Velocity3D::component_name()) {
        Some(c) => c,
        None => return,
    };
    let sim_times = match store.timelines.get(&TimelineName::SimTime) {
        Some(t) => t,
        None => return,
    };

    // Extract satellite id from path (last segment)
    let id = sat_path.to_string();
    let id = id.rsplit('/').next().unwrap_or("default");

    for i in 0..pos_col.num_rows() {
        let t = match sim_times.get(i * 2) {
            Some(orts_datamodel::timeline::TimeIndex::Seconds(s)) => *s,
            _ => 0.0,
        };
        let pos = pos_col.get_row(i).unwrap();
        let vel = vel_col.get_row(i).unwrap();
        let pos_vec = nalgebra::Vector3::new(pos[0], pos[1], pos[2]);
        let vel_vec = nalgebra::Vector3::new(vel[0], vel[1], vel[2]);
        let elements = KeplerianElements::from_state_vector(&pos_vec, &vel_vec, mu);
        if with_id {
            println!(
                "{},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.10},{:.10},{:.10},{:.10},{:.10}",
                id, t, pos[0], pos[1], pos[2], vel[0], vel[1], vel[2],
                elements.semi_major_axis, elements.eccentricity,
                elements.inclination, elements.raan,
                elements.argument_of_periapsis, elements.true_anomaly,
            );
        } else {
            println!(
                "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.10},{:.10},{:.10},{:.10},{:.10}",
                t, pos[0], pos[1], pos[2], vel[0], vel[1], vel[2],
                elements.semi_major_axis, elements.eccentricity,
                elements.inclination, elements.raan,
                elements.argument_of_periapsis, elements.true_anomaly,
            );
        }
    }
}

fn run_convert(input: &str, format: OutputFormat, output: Option<&str>) {
    match format {
        OutputFormat::Csv => {
            let data = orts_datamodel::rerun_export::load_rrd_data(input)
                .unwrap_or_else(|e| {
                    eprintln!("Error reading {input}: {e}");
                    std::process::exit(1);
                });

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;
                let meta = &data.metadata;
                if let Some(mu) = meta.mu {
                    writeln!(w, "# mu = {} km^3/s^2", mu)?;
                }
                if let Some(epoch_jd) = meta.epoch_jd {
                    writeln!(w, "# epoch_jd = {}", epoch_jd)?;
                }
                if let Some(ref name) = meta.body_name {
                    writeln!(w, "# central_body = {}", name.to_lowercase())?;
                }
                if let Some(radius) = meta.body_radius {
                    writeln!(w, "# central_body_radius = {} km", radius)?;
                }
                writeln!(w, "# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]")?;
                for row in &data.rows {
                    writeln!(
                        w,
                        "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
                        row.t, row.x, row.y, row.z, row.vx, row.vy, row.vz,
                    )?;
                }
                Ok(())
            };

            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(path).unwrap_or_else(|e| {
                        eprintln!("Error creating {path}: {e}");
                        std::process::exit(1);
                    });
                    write_csv(&mut file).unwrap();
                    eprintln!("Converted {input} -> {path}");
                }
                None => {
                    let mut stdout = std::io::stdout().lock();
                    write_csv(&mut stdout).unwrap();
                }
            }
        }
        OutputFormat::Rrd => {
            eprintln!("Error: cannot convert to .rrd format (input is already .rrd)");
            std::process::exit(1);
        }
    }
}

fn run_server(sim: &SimArgs, port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(sim, port));
}

async fn async_server(sim: &SimArgs, port: u16) {
    let params = Arc::new(SimParams::from_sim_args(sim, true));
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    eprintln!("WebSocket server listening on ws://localhost:{port}");

    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let history = Arc::new(tokio::sync::RwLock::new(HistoryBuffer::new(5000, data_dir, params.mu)));

    let (tx, _rx) = broadcast::channel::<String>(256);

    // Shared list of serialized simulation_terminated JSON messages.
    // The simulation loop appends here when a satellite terminates;
    // handle_connection replays them to late-connecting clients.
    let terminated_events: Arc<tokio::sync::RwLock<Vec<String>>> =
        Arc::new(tokio::sync::RwLock::new(Vec::new()));

    let sim_tx = tx.clone();
    let sim_params = Arc::clone(&params);
    let sim_history = Arc::clone(&history);
    let sim_terminated = Arc::clone(&terminated_events);
    tokio::spawn(async move {
        simulation_loop(sim_params, sim_tx, sim_history, sim_terminated).await;
    });

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        eprintln!("New connection from {peer}");

        // Subscribe before spawning handler (no lost messages)
        let rx = tx.subscribe();
        let client_params = Arc::clone(&params);
        let client_history = Arc::clone(&history);
        let client_terminated = Arc::clone(&terminated_events);

        tokio::spawn(async move {
            handle_connection(stream, rx, client_params, client_history, client_terminated).await;
        });
    }
}

/// Per-satellite simulation state tracked across loop iterations.
struct SatSimState {
    spec: SatelliteSpec,
    system: OrbitalSystem,
    state: State,
    t: f64,
    orbit_end_t: f64,
    next_stream_t: f64,
    next_save_t: f64,
    terminated: bool,
}

async fn simulation_loop(
    params: Arc<SimParams>,
    tx: broadcast::Sender<String>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
    terminated_events: Arc<tokio::sync::RwLock<Vec<String>>>,
) {
    let dt = params.dt;

    // Batch N stream intervals into a single compute chunk.
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;

    // Wall-clock pacing: target sim speed ratio.
    let wall_per_sim_sec = ((dt / 100.0).max(0.01)) / params.stream_interval;
    let chunk_wall_time =
        std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    // Initialize per-satellite state
    let mut sat_states: Vec<SatSimState> = params.satellites.iter().map(|spec| {
        let initial = spec.initial_state(params.mu);
        SatSimState {
            spec: spec.clone(),
            system: build_orbital_system(&params.body, params.mu, params.epoch, spec, params.atmosphere),
            state: initial,
            t: 0.0,
            orbit_end_t: spec.period,
            next_stream_t: params.stream_interval,
            next_save_t: params.output_interval,
            terminated: false,
        }
    }).collect();

    // Emit initial states for all satellites
    {
        let mut h = history.write().await;
        for ss in &sat_states {
            let accels = accel_breakdown(&ss.system, 0.0, &ss.state);
            let hs = make_history_state(&ss.spec.id, 0.0, &ss.state.position, &ss.state.velocity, params.mu, accels.clone());
            h.push(hs);
            let msg = state_message(&ss.spec.id, 0.0, &ss.state, params.mu, accels);
            let _ = tx.send(msg);
        }
    }

    loop {
        let chunk_start = tokio::time::Instant::now();
        let mut all_outputs: Vec<HistoryState> = Vec::new();

        for ss in &mut sat_states {
            if ss.terminated {
                continue;
            }

            // Each satellite advances by exactly chunk_sim_time, handling
            // orbit boundaries within the loop so all satellites stay in sync.
            let target_t = ss.t + chunk_sim_time;

            // Skip orbit boundary reset when perturbations are active
            // (orbit is no longer periodic with J2)
            let has_perturbations = params.body.properties().j2.is_some();

            while ss.t < target_t - 1e-9 {
                // Check orbit boundary → reset (only for unperturbed 2-body)
                if !has_perturbations && ss.t >= ss.orbit_end_t - 1e-9 {
                    ss.state = ss.spec.initial_state(params.mu);
                    ss.orbit_end_t = ss.t + ss.spec.period;
                }

                // With perturbations, orbit is not periodic; propagate continuously
                let sub_end = if has_perturbations {
                    target_t
                } else {
                    target_t.min(ss.orbit_end_t)
                };

                let atm_alt = params.body.properties().atmosphere_altitude;
                let (outputs, new_state, new_t, termination) = match params.integrator {
                    IntegratorChoice::Rk4 => compute_output_chunk(
                        &ss.spec.id,
                        &ss.system,
                        ss.state.clone(),
                        ss.t,
                        sub_end,
                        dt,
                        params.stream_interval,
                        &mut ss.next_stream_t,
                        atm_alt,
                    ),
                    IntegratorChoice::Dp45 => compute_output_chunk_adaptive(
                        &ss.spec.id,
                        &ss.system,
                        ss.state.clone(),
                        ss.t,
                        sub_end,
                        dt,
                        &params.tolerances,
                        params.stream_interval,
                        &mut ss.next_stream_t,
                        atm_alt,
                    ),
                };

                ss.state = new_state;
                ss.t = new_t;

                // Save output_interval-aligned states to history
                {
                    let mut h = history.write().await;
                    for out in &outputs {
                        if out.t >= ss.next_save_t - 1e-9 {
                            h.push(out.clone());
                            ss.next_save_t += params.output_interval;
                        }
                    }
                }

                all_outputs.extend(outputs);

                // Handle termination
                if let Some(reason) = termination {
                    eprintln!(
                        "Simulation terminated for {} at t={:.2}s: {}",
                        ss.spec.id, ss.t, reason
                    );
                    let msg = serde_json::to_string(&WsMessage::SimulationTerminated {
                        satellite_id: ss.spec.id.clone(),
                        t: ss.t,
                        reason: reason.to_string(),
                    })
                    .expect("failed to serialize termination message");
                    let _ = tx.send(msg.clone());
                    terminated_events.write().await.push(msg);
                    ss.terminated = true;
                    break;
                }
            }
        }

        // Sort all outputs by time for interleaved sending
        all_outputs.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        if !all_outputs.is_empty() {
            let send_interval = chunk_wall_time / all_outputs.len() as u32;
            for out in &all_outputs {
                let send_start = tokio::time::Instant::now();
                let msg = serde_json::to_string(&WsMessage::State {
                    satellite_id: out.satellite_id.clone(),
                    t: out.t,
                    position: out.position,
                    velocity: out.velocity,
                    semi_major_axis: out.semi_major_axis,
                    eccentricity: out.eccentricity,
                    inclination: out.inclination,
                    raan: out.raan,
                    argument_of_periapsis: out.argument_of_periapsis,
                    true_anomaly: out.true_anomaly,
                    accelerations: out.accelerations.clone(),
                })
                .expect("failed to serialize state");
                let _ = tx.send(msg);

                let send_elapsed = send_start.elapsed();
                if send_elapsed < send_interval {
                    tokio::time::sleep(send_interval - send_elapsed).await;
                }
            }
        } else {
            let elapsed = chunk_start.elapsed();
            if elapsed < chunk_wall_time {
                tokio::time::sleep(chunk_wall_time - elapsed).await;
            }
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    params: Arc<SimParams>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
    terminated_events: Arc<tokio::sync::RwLock<Vec<String>>>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // 1. Send info message
    let satellites_info: Vec<SatelliteInfo> = params.satellites.iter().map(|s| {
        let system = build_orbital_system(&params.body, params.mu, params.epoch, s, params.atmosphere);
        SatelliteInfo {
            id: s.id.clone(),
            name: s.name.clone(),
            altitude: s.altitude(&params.body),
            period: s.period,
            perturbations: system.perturbation_names().into_iter().map(String::from).collect(),
        }
    }).collect();
    let info = WsMessage::Info {
        mu: params.mu,
        dt: params.dt,
        output_interval: params.output_interval,
        stream_interval: params.stream_interval,
        central_body: serde_json::to_value(&params.body)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string(),
        central_body_radius: params.body.properties().radius,
        epoch_jd: params.epoch.map(|e| e.jd()),
        satellites: satellites_info,
    };
    let info_json = serde_json::to_string(&info).expect("failed to serialize info message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(info_json.into()))
        .await
        .is_err()
    {
        return;
    }

    // 1b. Replay termination events for satellites that terminated before this client connected
    {
        let terminated = terminated_events.read().await;
        for event_json in terminated.iter() {
            if ws_sender
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    event_json.clone().into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // 2. Send overview history (downsampled)
    let all_states = history.read().await.load_all();
    let overview = HistoryBuffer::downsample(&all_states, 1000);
    let history_msg = WsMessage::History { states: overview };
    let history_json =
        serde_json::to_string(&history_msg).expect("failed to serialize history message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(
            history_json.into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // 3. Spawn background detail sender
    let (detail_tx, mut detail_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(async move {
        let chunk_size = 1000;
        for chunk in all_states.chunks(chunk_size) {
            let msg = WsMessage::HistoryDetail {
                states: chunk.to_vec(),
            };
            let json = serde_json::to_string(&msg).expect("failed to serialize detail chunk");
            if detail_tx.send(json).await.is_err() {
                return; // Client disconnected
            }
        }
        let complete = serde_json::to_string(&WsMessage::HistoryDetailComplete)
            .expect("failed to serialize detail complete");
        let _ = detail_tx.send(complete).await;
    });

    // 4. Main loop: multiplex broadcast (real-time) + detail (background) + client messages
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_sender
                            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Client lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            detail = detail_rx.recv() => {
                if let Some(json) = detail
                    && ws_sender
                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                        .await
                        .is_err()
                {
                    break;
                }
                // None means detail sender finished — just continue with broadcast only
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            match client_msg {
                                ClientMessage::QueryRange { t_min, t_max, max_points, satellite_id } => {
                                    let mut states = history.read().await.query_range(t_min, t_max, max_points);
                                    if let Some(ref sid) = satellite_id {
                                        states.retain(|s| s.satellite_id == *sid);
                                    }
                                    let resp = WsMessage::QueryRangeResponse { t_min, t_max, states };
                                    let json = serde_json::to_string(&resp)
                                        .expect("failed to serialize query_range_response");
                                    if ws_sender
                                        .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        break;
                    }
                }
            }
        }
    }

    eprintln!("Client disconnected");
}

/// Reason a satellite simulation was terminated in serve mode.
#[derive(Debug)]
enum TerminationReason {
    Collision { altitude_km: f64 },
    AtmosphericEntry { altitude_km: f64 },
    NonFiniteState,
}

impl std::fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Collision { altitude_km } => {
                write!(f, "collision at altitude {altitude_km:.1} km")
            }
            Self::AtmosphericEntry { altitude_km } => {
                write!(f, "atmospheric entry at altitude {altitude_km:.1} km")
            }
            Self::NonFiniteState => write!(f, "numerical divergence (NaN/Inf)"),
        }
    }
}

/// Compute RK4 integration from t_start to chunk_end, collecting output states
/// at output_interval boundaries. Pure computation with no IO.
///
/// Returns (output_states, final_state, final_t, optional termination reason).
#[allow(clippy::too_many_arguments)]
fn compute_output_chunk(
    satellite_id: &str,
    system: &OrbitalSystem,
    mut state: State,
    t_start: f64,
    chunk_end: f64,
    dt: f64,
    output_interval: f64,
    next_output_t: &mut f64,
    atmosphere_altitude: Option<f64>,
) -> (Vec<HistoryState>, State, f64, Option<TerminationReason>) {
    let mu = system.mu;
    let body_radius = system.body_radius;
    let mut outputs = Vec::new();
    let mut t = t_start;

    while t < chunk_end {
        let h = dt.min(chunk_end - t);
        state = Rk4.step(system, t, &state, h);
        t += h;

        // Check for NaN/Inf
        if !state
            .position
            .iter()
            .chain(state.velocity.iter())
            .all(|v| v.is_finite())
        {
            return (outputs, state, t, Some(TerminationReason::NonFiniteState));
        }

        // Check for collision and atmospheric entry
        if let Some(r_body) = body_radius {
            let r = state.position.magnitude();
            if r < r_body {
                return (
                    outputs,
                    state,
                    t,
                    Some(TerminationReason::Collision {
                        altitude_km: r - r_body,
                    }),
                );
            }
            if let Some(atm_alt) = atmosphere_altitude
                && r < r_body + atm_alt
            {
                return (
                    outputs,
                    state,
                    t,
                    Some(TerminationReason::AtmosphericEntry {
                        altitude_km: r - r_body,
                    }),
                );
            }
        }

        if t >= *next_output_t - 1e-9 {
            let accels = accel_breakdown(system, t, &state);
            outputs.push(make_history_state(satellite_id, t, &state.position, &state.velocity, mu, accels));
            *next_output_t += output_interval;
        }
    }

    (outputs, state, t, None)
}

/// Create an event checker for the serve mode adaptive loop.
///
/// Like `events::collision_check` but handles `body_radius: Option<f64>`.
fn make_serve_event_checker(
    body_radius: Option<f64>,
    atmosphere_altitude: Option<f64>,
) -> impl Fn(f64, &State) -> ControlFlow<SimulationEvent> {
    move |_t: f64, state: &State| {
        if let Some(r_body) = body_radius {
            let r = state.position.magnitude();
            if r < r_body {
                return ControlFlow::Break(SimulationEvent::Collision {
                    altitude_km: r - r_body,
                });
            }
            if let Some(atm_alt) = atmosphere_altitude
                && r < r_body + atm_alt
            {
                return ControlFlow::Break(SimulationEvent::AtmosphericEntry {
                    altitude_km: r - r_body,
                });
            }
        }
        ControlFlow::Continue(())
    }
}

/// Adaptive Dormand-Prince version of compute_output_chunk.
/// Step size adapts automatically; outputs are produced at output_interval boundaries
/// by clamping the step to not overshoot the next output time.
#[allow(clippy::too_many_arguments)]
fn compute_output_chunk_adaptive(
    satellite_id: &str,
    system: &OrbitalSystem,
    state: State,
    t_start: f64,
    chunk_end: f64,
    dt_hint: f64,
    tol: &Tolerances,
    output_interval: f64,
    next_output_t: &mut f64,
    atmosphere_altitude: Option<f64>,
) -> (Vec<HistoryState>, State, f64, Option<TerminationReason>) {
    let mu = system.mu;
    let body_radius = system.body_radius;
    let mut outputs = Vec::new();

    let event_checker = make_serve_event_checker(body_radius, atmosphere_altitude);

    let mut stepper = DormandPrince.stepper(system, state, t_start, dt_hint, tol.clone());
    stepper.dt_min = 1e-12 * (chunk_end - t_start).abs().max(1.0);

    while stepper.t() < chunk_end - 1e-12 {
        let t_target = (*next_output_t).min(chunk_end);
        if t_target - stepper.t() < 1e-14 {
            break;
        }

        match stepper.advance_to(t_target, |_, _| {}, &event_checker) {
            Ok(AdvanceOutcome::Reached) => {
                if stepper.t() >= *next_output_t - 1e-9 {
                    let accels = accel_breakdown(system, stepper.t(), stepper.state());
                    outputs.push(make_history_state(
                        satellite_id,
                        stepper.t(),
                        &stepper.state().position,
                        &stepper.state().velocity,
                        mu,
                        accels,
                    ));
                    *next_output_t += output_interval;
                }
            }
            Ok(AdvanceOutcome::Event { reason }) => {
                let t = stepper.t();
                let termination = match reason {
                    SimulationEvent::Collision { altitude_km } => {
                        TerminationReason::Collision { altitude_km }
                    }
                    SimulationEvent::AtmosphericEntry { altitude_km } => {
                        TerminationReason::AtmosphericEntry { altitude_km }
                    }
                };
                return (outputs, stepper.into_state(), t, Some(termination));
            }
            Err(_) => {
                let t = stepper.t();
                return (
                    outputs,
                    stepper.into_state(),
                    t,
                    Some(TerminationReason::NonFiniteState),
                );
            }
        }
    }

    let t = stepper.t();
    (outputs, stepper.into_state(), t, None)
}

fn state_message(
    satellite_id: &str,
    t: f64,
    state: &State,
    mu: f64,
    accelerations: HashMap<String, f64>,
) -> String {
    let elements = KeplerianElements::from_state_vector(&state.position, &state.velocity, mu);
    let msg = WsMessage::State {
        satellite_id: satellite_id.to_string(),
        t,
        position: [state.position.x, state.position.y, state.position.z],
        velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        accelerations,
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    const TEST_MU: f64 = 398600.4418;

    fn make_state(t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        make_history_state("default", t, &pos, &vel, TEST_MU, HashMap::new())
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orts-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    fn cleanup_dir(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // --- HistoryBuffer tests ---

    #[test]
    fn buffer_push_and_read() {
        let dir = temp_data_dir("push-read");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        buf.push(make_state(0.0));
        buf.push(make_state(10.0));
        buf.push(make_state(20.0));

        let all = buf.load_all();
        assert_eq!(all.len(), 3);
        assert!((all[0].t - 0.0).abs() < 1e-9);
        assert!((all[1].t - 10.0).abs() < 1e-9);
        assert!((all[2].t - 20.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_flush_creates_segment() {
        let dir = temp_data_dir("flush-seg");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU);

        // Push 5 states → exceeds capacity of 4 → triggers flush
        for i in 0..5 {
            buf.push(make_state(i as f64 * 10.0));
        }

        assert_eq!(buf.segment_count, 1);
        assert!(dir.join("seg_0000.rrd").exists());
        // After flushing half (2), buffer should have 3 states
        assert_eq!(buf.states.len(), 3);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_load_all_includes_flushed_and_buffered() {
        let dir = temp_data_dir("load-all");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU);

        for i in 0..8 {
            buf.push(make_state(i as f64 * 10.0));
        }

        // Should have flushed some segments
        assert!(buf.segment_count > 0);

        let all = buf.load_all();
        assert_eq!(all.len(), 8);

        // Verify all times are present (order may differ slightly due to .rrd roundtrip)
        let mut times: Vec<f64> = all.iter().map(|s| s.t).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for i in 0..8 {
            assert!(
                (times[i] - i as f64 * 10.0).abs() < 0.01,
                "times[{i}] = {}, expected {}",
                times[i],
                i as f64 * 10.0
            );
        }

        cleanup_dir(&dir);
    }

    // --- Downsample tests ---

    #[test]
    fn downsample_correctness() {
        let states: Vec<HistoryState> = (0..100).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);

        assert_eq!(ds.len(), 10);
        // First and last are preserved
        assert!((ds[0].t - 0.0).abs() < 1e-9);
        assert!((ds[9].t - 99.0).abs() < 1e-9);
    }

    #[test]
    fn downsample_preserves_all_when_small() {
        let states: Vec<HistoryState> = (0..5).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);
        assert_eq!(ds.len(), 5);
    }

    #[test]
    fn downsample_performance() {
        let states: Vec<HistoryState> = (0..100_000).map(|i| make_state(i as f64)).collect();
        let start = std::time::Instant::now();
        let ds = HistoryBuffer::downsample(&states, 1000);
        let elapsed = start.elapsed();

        assert_eq!(ds.len(), 1000);
        assert!(
            elapsed.as_millis() < 10,
            "downsample took {}ms, expected <10ms",
            elapsed.as_millis()
        );
    }

    // --- Performance tests ---

    #[test]
    fn flush_performance() {
        let dir = temp_data_dir("flush-perf");
        let mut buf = HistoryBuffer::new(10_000, dir.clone(), TEST_MU);

        for i in 0..5000 {
            buf.states.push_back(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        buf.flush();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 2000,
            "flush took {}ms, expected <2000ms",
            elapsed.as_millis()
        );
        assert_eq!(buf.segment_count, 1);

        cleanup_dir(&dir);
    }

    #[test]
    fn load_all_performance() {
        let dir = temp_data_dir("load-perf");
        let mut buf = HistoryBuffer::new(2000, dir.clone(), TEST_MU);

        // Insert 10000 points, which will create multiple segments
        for i in 0..10_000 {
            buf.push(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        let all = buf.load_all();
        let elapsed = start.elapsed();

        assert_eq!(all.len(), 10_000);
        assert!(
            elapsed.as_millis() < 2000,
            "load_all took {}ms, expected <2000ms",
            elapsed.as_millis()
        );

        cleanup_dir(&dir);
    }

    // --- WsMessage serialization tests ---

    #[test]
    fn history_message_serialization() {
        let msg = WsMessage::History {
            states: vec![make_state(0.0), make_state(10.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history");
        let states = v["states"].as_array().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0]["t"], 0.0);
        assert_eq!(states[0]["position"].as_array().unwrap().len(), 3);
        assert_eq!(states[0]["velocity"].as_array().unwrap().len(), 3);
        assert_eq!(states[1]["t"], 10.0);
    }

    #[test]
    fn history_message_empty_states() {
        let msg = WsMessage::History { states: vec![] };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history");
        assert_eq!(v["states"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn history_detail_message_serialization() {
        let msg = WsMessage::HistoryDetail {
            states: vec![make_state(5.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history_detail");
        assert_eq!(v["states"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn history_detail_complete_serialization() {
        let msg = WsMessage::HistoryDetailComplete;
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history_detail_complete");
        // Should not have a "states" field
        assert!(v.get("states").is_none());
    }

    // --- ClientMessage tests ---

    #[test]
    fn client_message_query_range_deserialize() {
        let json = r#"{"type":"query_range","t_min":10.0,"t_max":50.0,"max_points":100}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange {
                t_min,
                t_max,
                max_points,
                satellite_id,
            } => {
                assert!((t_min - 10.0).abs() < 1e-9);
                assert!((t_max - 50.0).abs() < 1e-9);
                assert_eq!(max_points, Some(100));
                assert_eq!(satellite_id, None);
            }
        }
    }

    #[test]
    fn client_message_query_range_without_max_points() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { max_points, satellite_id, .. } => {
                assert_eq!(max_points, None);
                assert_eq!(satellite_id, None);
            }
        }
    }

    #[test]
    fn client_message_query_range_with_satellite_id() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0,"satellite_id":"iss"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { satellite_id, .. } => {
                assert_eq!(satellite_id, Some("iss".to_string()));
            }
        }
    }

    // --- QueryRangeResponse serialization ---

    #[test]
    fn query_range_response_serialization() {
        let msg = WsMessage::QueryRangeResponse {
            t_min: 10.0,
            t_max: 50.0,
            states: vec![make_state(20.0), make_state(30.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "query_range_response");
        assert_eq!(v["t_min"], 10.0);
        assert_eq!(v["t_max"], 50.0);
        assert_eq!(v["states"].as_array().unwrap().len(), 2);
    }

    // --- query_range tests ---

    #[test]
    fn query_range_filters_by_time() {
        let dir = temp_data_dir("qr-filter");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(20.0, 60.0, None);
        assert!(result.len() >= 4, "should include t=20,30,40,50,60");
        for s in &result {
            assert!(s.t >= 20.0 && s.t <= 60.0, "t={} out of range", s.t);
        }

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_with_downsample() {
        let dir = temp_data_dir("qr-ds");
        let mut buf = HistoryBuffer::new(200, dir.clone(), TEST_MU);

        for i in 0..100 {
            buf.push(make_state(i as f64));
        }

        let result = buf.query_range(0.0, 99.0, Some(10));
        assert_eq!(result.len(), 10);
        // First and last preserved
        assert!((result[0].t - 0.0).abs() < 1e-9);
        assert!((result[9].t - 99.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_empty_range() {
        let dir = temp_data_dir("qr-empty");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(200.0, 300.0, None);
        assert!(result.is_empty());

        cleanup_dir(&dir);
    }

    // --- SimParams tests ---

    #[test]
    fn sim_params_stream_interval_defaults_to_output_interval() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 10.0).abs() < 1e-9);
        // Defaults to Epoch::now() for known bodies
        assert!(params.epoch.is_some());
    }

    #[test]
    fn sim_params_explicit_stream_interval() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(2.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.dt - 1.0).abs() < 1e-9);
        assert!((params.output_interval - 10.0).abs() < 1e-9);
        assert!((params.stream_interval - 2.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_stream_interval_clamped() {
        // stream_interval < dt → clamped to dt
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 5.0,
            output_interval: Some(10.0),
            stream_interval: Some(1.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!((params.stream_interval - 5.0).abs() < 1e-9);

        // stream_interval > output_interval → clamped to output_interval
        let args2 = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(20.0),
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params2 = SimParams::from_sim_args(&args2, false);
        assert!((params2.stream_interval - 10.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_with_epoch() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2024-03-20T12:00:00Z".to_string()),
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert!(params.epoch.is_some());
        let epoch = params.epoch.unwrap();
        // 2024-03-20 12:00:00 UTC
        assert!((epoch.jd() - 2460390.0).abs() < 0.01);
    }

    #[test]
    fn info_message_with_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![SatelliteInfo {
                id: "default".to_string(),
                name: None,
                altitude: 400.0,
                period: 5554.0,
                perturbations: vec![],
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        assert_eq!(v["epoch_jd"], 2460390.0);
        assert!(v["satellites"].is_array());
    }

    #[test]
    fn info_message_without_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: None,
            satellites: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        // epoch_jd should be absent (skip_serializing_if)
        assert!(v.get("epoch_jd").is_none());
    }

    #[test]
    fn info_message_with_satellite_info() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![SatelliteInfo {
                id: "iss".to_string(),
                name: Some("ISS (ZARYA)".to_string()),
                altitude: 420.0,
                period: 5560.0,
                perturbations: vec![],
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellites"][0]["name"], "ISS (ZARYA)");
        assert_eq!(v["satellites"][0]["id"], "iss");
    }

    #[test]
    #[should_panic(expected = "Cannot specify both")]
    fn sim_params_norad_id_conflicts_with_tle() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some("1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string()),
            tle_line2: Some("2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string()),
            norad_id: Some(25544),
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        SimParams::from_sim_args(&args, false);
    }

    #[test]
    #[ignore] // Requires network access
    fn fetch_iss_tle_from_celestrak() {
        let tle = fetch_tle_by_norad_id(25544);
        assert!(tle.name.is_some(), "ISS TLE should have a name");
        assert_eq!(tle.satellite_number, 25544);
        // Sanity: ISS inclination ~51.6 degrees
        assert!((tle.inclination.to_degrees() - 51.6).abs() < 1.0);
    }

    #[test]
    #[ignore] // Requires network access
    fn fetch_iss_tle_satnogs_fallback() {
        let tle = fetch_tle_satnogs(25544).expect("SatNOGS should return ISS TLE");
        assert!(tle.name.is_some(), "ISS TLE should have a name");
        assert_eq!(tle.satellite_number, 25544);
        assert!((tle.inclination.to_degrees() - 51.6).abs() < 1.0);
    }

    // --- compute_output_chunk tests ---

    #[test]
    fn chunk_output_count_matches_interval() {
        // dt=10, output_interval=10, chunk=100s → expect 10 outputs
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _final_state, final_t, _term) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 10);
        assert!((final_t - 100.0).abs() < 1e-9);
    }

    #[test]
    fn chunk_fine_dt_batches_steps() {
        // dt=1, output_interval=10, chunk=100s → still 10 outputs but 100 RK4 steps
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 1.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 10);
        // Verify output times are at 10s intervals
        for (i, out) in outputs.iter().enumerate() {
            let expected_t = (i + 1) as f64 * 10.0;
            assert!(
                (out.t - expected_t).abs() < 0.1,
                "output[{i}].t = {}, expected {expected_t}",
                out.t
            );
        }
    }

    #[test]
    fn chunk_energy_conservation() {
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };
        let initial_energy = v0 * v0 / 2.0 - mu / r0;

        let mut next_output = 10.0;
        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 500.0, 10.0, 10.0, &mut next_output, None);

        for out in &outputs {
            let r = (out.position[0].powi(2) + out.position[1].powi(2) + out.position[2].powi(2))
                .sqrt();
            let v = (out.velocity[0].powi(2) + out.velocity[1].powi(2) + out.velocity[2].powi(2))
                .sqrt();
            let energy = v * v / 2.0 - mu / r;
            assert!(
                (energy - initial_energy).abs() < 1e-6,
                "energy drift at t={}: {:.2e}",
                out.t,
                (energy - initial_energy).abs()
            );
        }
    }

    #[test]
    fn chunk_partial_end() {
        // chunk_end doesn't align perfectly with output_interval
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        // chunk_end=55 with output_interval=10 → outputs at 10,20,30,40,50 (5 outputs)
        let (outputs, _, final_t, _) =
            compute_output_chunk("test", &system, initial, 0.0, 55.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 5);
        assert!((final_t - 55.0).abs() < 1e-9);
        // next_output should be 60.0 now
        assert!((next_output - 60.0).abs() < 1e-9);
    }

    #[test]
    fn chunk_dual_intervals() {
        // stream_interval=2, output_interval=10, dt=1, chunk=20s
        // → 10 stream outputs, of which 2 are at save boundaries (t=10, t=20)
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let stream_interval = 2.0;
        let output_interval = 10.0;
        let mut next_stream = stream_interval;

        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 20.0, 1.0, stream_interval, &mut next_stream, None);

        assert_eq!(outputs.len(), 10); // 20s / 2s = 10 stream outputs

        // Filter for save boundaries (same logic as simulation_loop will use)
        let mut next_save = output_interval;
        let mut save_count = 0;
        for out in &outputs {
            if out.t >= next_save - 1e-9 {
                save_count += 1;
                next_save += output_interval;
            }
        }
        assert_eq!(save_count, 2); // t=10 and t=20
    }

    #[test]
    fn chunk_matches_step_by_step() {
        // Verify that chunked computation gives identical results to step-by-step
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        // Step-by-step (original approach)
        let mut state_ss = initial.clone();
        let mut t = 0.0;
        let dt = 10.0;
        let mut step_outputs = Vec::new();
        for _ in 0..10 {
            state_ss = Rk4.step(&system, t, &state_ss, dt);
            t += dt;
            step_outputs.push(make_history_state("test", t, &state_ss.position, &state_ss.velocity, mu, HashMap::new()));
        }

        // Chunked
        let mut next_output = 10.0;
        let (chunk_outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(chunk_outputs.len(), step_outputs.len());
        for (c, s) in chunk_outputs.iter().zip(step_outputs.iter()) {
            assert!((c.t - s.t).abs() < 1e-12, "t mismatch: {} vs {}", c.t, s.t);
            for i in 0..3 {
                assert!(
                    (c.position[i] - s.position[i]).abs() < 1e-12,
                    "position[{i}] mismatch at t={}: {} vs {}",
                    c.t,
                    c.position[i],
                    s.position[i]
                );
            }
        }
    }

    // --- TLE input tests ---

    #[test]
    fn sim_params_from_tle_lines() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some("1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string()),
            tle_line2: Some("2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string()),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        // Should have one satellite in TLE mode
        assert_eq!(params.satellites.len(), 1);
        let sat = &params.satellites[0];
        assert!(matches!(sat.orbit, OrbitSpec::Tle { .. }));

        // Altitude should be ~400 km
        let alt = sat.altitude(&params.body);
        assert!(
            (alt - 400.0).abs() < 30.0,
            "ISS altitude: {:.1} km", alt
        );

        // Period should be ~92 minutes
        assert!(
            (sat.period / 60.0 - 92.0).abs() < 2.0,
            "ISS period: {:.1} min",
            sat.period / 60.0
        );
    }

    #[test]
    fn sim_params_tle_initial_state_plausible() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: Some("1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string()),
            tle_line2: Some("2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string()),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        let state = params.satellites[0].initial_state(params.mu);

        let r = state.position.magnitude();
        let v = state.velocity.magnitude();
        let altitude = r - 6378.137;

        // ISS altitude ~400 km
        assert!(
            (altitude - 400.0).abs() < 30.0,
            "ISS altitude from state: {altitude:.1} km"
        );
        // ISS velocity ~7.66 km/s
        assert!(
            (v - 7.66).abs() < 0.2,
            "ISS velocity: {v:.3} km/s"
        );
    }

    #[test]
    fn sim_params_circular_mode_still_works() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        assert_eq!(params.satellites.len(), 1);
        assert!(matches!(params.satellites[0].orbit, OrbitSpec::Circular { .. }));
        assert!((params.satellites[0].altitude(&params.body) - 400.0).abs() < 1e-9);
    }

    #[test]
    fn sim_params_tle_epoch_overridable() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: Some("2025-01-01T00:00:00Z".to_string()),
            tle: None,
            tle_line1: Some("1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993".to_string()),
            tle_line2: Some("2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000".to_string()),
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);

        // Epoch should be overridden to 2025-01-01
        let epoch = params.epoch.unwrap();
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2025);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
    }

    // ===== Multi-satellite tests (TDD — written before implementation) =====

    // --- parse_sat_spec tests ---

    #[test]
    fn parse_sat_spec_circular_altitude() {
        let spec = parse_sat_spec("altitude=800,id=sso", KnownBody::Earth);
        assert_eq!(spec.id, "sso");
        assert!(matches!(spec.orbit, OrbitSpec::Circular { altitude, .. } if (altitude - 800.0).abs() < 1e-9));
        assert!(spec.period > 0.0);
    }

    #[test]
    fn parse_sat_spec_default_id() {
        let spec = parse_sat_spec("altitude=600", KnownBody::Earth);
        // When no id is specified, should auto-generate one
        assert!(!spec.id.is_empty());
    }

    #[test]
    fn parse_sat_spec_with_name() {
        let spec = parse_sat_spec("altitude=800,id=sso,name=SSO 800km", KnownBody::Earth);
        assert_eq!(spec.id, "sso");
        assert_eq!(spec.name.as_deref(), Some("SSO 800km"));
    }

    #[test]
    fn parse_sat_spec_tle_lines() {
        let spec = parse_sat_spec(
            "tle-line1=1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993,tle-line2=2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000,id=iss",
            KnownBody::Earth,
        );
        assert_eq!(spec.id, "iss");
        assert!(matches!(spec.orbit, OrbitSpec::Tle { .. }));
    }

    // --- HistoryState satellite_id tests ---

    #[test]
    fn history_state_has_satellite_id() {
        let hs = make_history_state("test-sat", 10.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU, HashMap::new());
        assert_eq!(hs.satellite_id, "test-sat");
        assert!((hs.t - 10.0).abs() < 1e-9);
    }

    #[test]
    fn history_state_satellite_id_serialized() {
        let hs = make_history_state("my-sat", 5.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU, HashMap::new());
        let json = serde_json::to_string(&hs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellite_id"], "my-sat");
    }

    // --- WsMessage protocol tests ---

    #[test]
    fn info_message_has_satellites_array() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![
                SatelliteInfo {
                    id: "sso".to_string(),
                    name: Some("SSO 800km".to_string()),
                    altitude: 800.0,
                    period: 6052.5,
                    perturbations: vec![],
                },
                SatelliteInfo {
                    id: "iss".to_string(),
                    name: Some("ISS (ZARYA)".to_string()),
                    altitude: 420.0,
                    period: 5560.0,
                    perturbations: vec![],
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        let sats = v["satellites"].as_array().unwrap();
        assert_eq!(sats.len(), 2);
        assert_eq!(sats[0]["id"], "sso");
        assert_eq!(sats[0]["name"], "SSO 800km");
        assert_eq!(sats[1]["id"], "iss");
        // Should NOT have flat altitude/period/satellite_name fields
        assert!(v.get("altitude").is_none());
        assert!(v.get("period").is_none());
        assert!(v.get("satellite_name").is_none());
    }

    #[test]
    fn state_message_has_satellite_id() {
        let msg = WsMessage::State {
            satellite_id: "iss".to_string(),
            t: 100.0,
            position: [6778.0, 0.0, 0.0],
            velocity: [0.0, 7.669, 0.0],
            semi_major_axis: 6778.0,
            eccentricity: 0.001,
            inclination: 0.9,
            raan: 1.2,
            argument_of_periapsis: 0.5,
            true_anomaly: 2.1,
            accelerations: HashMap::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "state");
        assert_eq!(v["satellite_id"], "iss");
    }

    // --- Multi-satellite SimParams tests ---

    #[test]
    fn sim_params_with_sat_flags() {
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec!["altitude=800,id=sso".to_string(), "altitude=600,id=leo".to_string()],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert_eq!(params.satellites.len(), 2);
        assert_eq!(params.satellites[0].id, "sso");
        assert_eq!(params.satellites[1].id, "leo");
    }

    #[test]
    fn sim_params_single_sat_shorthand() {
        // When no --sat flag but --altitude is used, create single satellite
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, false);
        assert_eq!(params.satellites.len(), 1);
        assert_eq!(params.satellites[0].id, "default");
    }

    #[test]
    fn sim_params_serve_default_sso() {
        // serve with no orbit args → at least SSO (ISS requires network)
        let args = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            tle: None,
            tle_line1: None,
            tle_line2: None,
            norad_id: None,
            sats: vec![],
            integrator: IntegratorChoice::Dp45,
            atol: 1e-10,
            rtol: 1e-8,
            atmosphere: AtmosphereChoice::Exponential,
            duration: None,
        };
        let params = SimParams::from_sim_args(&args, true);
        // Should have at least SSO satellite
        assert!(!params.satellites.is_empty());
        assert!(params.satellites.iter().any(|s| s.id == "sso"));
    }

    #[test]
    fn satellite_spec_initial_state_circular() {
        let spec = parse_sat_spec("altitude=400,id=test", KnownBody::Earth);
        let mu = KnownBody::Earth.properties().mu;
        let state = spec.initial_state(mu);
        let r = state.position.magnitude();
        let expected_r = 6378.137 + 400.0;
        assert!((r - expected_r).abs() < 1e-6, "r = {r}, expected {expected_r}");
    }

    #[test]
    fn satellite_spec_initial_state_inclined() {
        let mu = KnownBody::Earth.properties().mu;
        // SSO-like orbit: 800km altitude, ~98.6° inclination
        let spec = parse_sat_spec("altitude=800,inclination=98.6,id=sso-test", KnownBody::Earth);
        let state = spec.initial_state(mu);

        // Radius should match altitude
        let r = state.position.magnitude();
        let expected_r = 6378.137 + 800.0;
        assert!((r - expected_r).abs() < 1e-6, "r = {r}, expected {expected_r}");

        // Velocity magnitude should be circular velocity
        let v = state.velocity.magnitude();
        let expected_v = (mu / expected_r).sqrt();
        assert!((v - expected_v).abs() < 1e-6, "v = {v}, expected {expected_v}");

        // Verify inclination via angular momentum
        let h = state.position.cross(&state.velocity);
        let i = (h[2] / h.magnitude()).acos();
        let expected_i = 98.6_f64.to_radians();
        assert!(
            (i - expected_i).abs() < 1e-10,
            "inclination = {:.4}°, expected {:.4}°",
            i.to_degrees(),
            expected_i.to_degrees()
        );
    }

    #[test]
    fn satellite_spec_initial_state_inclined_with_raan() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec("altitude=400,inclination=51.6,raan=90,id=iss-like", KnownBody::Earth);
        let state = spec.initial_state(mu);

        // Verify inclination
        let h = state.position.cross(&state.velocity);
        let i = (h[2] / h.magnitude()).acos();
        assert!(
            (i - 51.6_f64.to_radians()).abs() < 1e-10,
            "inclination = {:.4}°, expected 51.6°",
            i.to_degrees()
        );

        // Verify RAAN: ascending node should be at 90° from X-axis
        let k = nalgebra::Vector3::new(0.0, 0.0, 1.0);
        let n = k.cross(&h);
        let raan = n[1].atan2(n[0]);
        let raan = if raan < 0.0 { raan + 2.0 * std::f64::consts::PI } else { raan };
        assert!(
            (raan - 90.0_f64.to_radians()).abs() < 1e-10,
            "RAAN = {:.4}°, expected 90°",
            raan.to_degrees()
        );
    }

    #[test]
    fn satellite_spec_initial_state_equatorial_default() {
        // Without inclination, should remain equatorial (z ≈ 0)
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec("altitude=400,id=test", KnownBody::Earth);
        let state = spec.initial_state(mu);
        assert!(
            state.position[2].abs() < 1e-10,
            "equatorial orbit should have z ≈ 0, got {}",
            state.position[2]
        );
    }

    #[test]
    fn satellite_spec_entity_path() {
        let spec = parse_sat_spec("altitude=400,id=my-sat", KnownBody::Earth);
        let path = spec.entity_path();
        assert_eq!(path.to_string(), "/world/sat/my-sat");
    }

    #[test]
    fn build_orbital_system_sets_body_radius() {
        let body = KnownBody::Earth;
        let spec = parse_sat_spec("altitude=400", body);
        let system = build_orbital_system(&body, body.properties().mu, None, &spec, AtmosphereChoice::Exponential);
        assert_eq!(system.body_radius, Some(body.properties().radius));
    }
}
