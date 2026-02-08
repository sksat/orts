use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use nalgebra::vector;
use orts_datamodel::archetypes::OrbitalState;
use orts_datamodel::components::{BodyRadius, GravitationalParameter};
use orts_datamodel::entity_path::EntityPath;
use orts_datamodel::recording::Recording;
use orts_datamodel::timeline::TimePoint;
use orts_coords::epoch::Epoch;
use orts_integrator::{Rk4, State};
use orts_orbits::{body::KnownBody, two_body::TwoBodySystem};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Orts CLI — orbital mechanics simulation tool
#[derive(Parser, Debug)]
#[command(name = "orts-cli")]
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
}

/// Simulation parameters derived from CLI arguments.
struct SimParams {
    body: KnownBody,
    mu: f64,
    r0: f64,
    v0: f64,
    period: f64,
    dt: f64,
    altitude: f64,
    output_interval: f64,
    stream_interval: f64,
    epoch: Option<Epoch>,
}

impl SimParams {
    fn from_sim_args(args: &SimArgs) -> Self {
        let body = parse_body(&args.body);
        let props = body.properties();
        let mu = props.mu;
        let r0 = props.radius + args.altitude;
        let v0 = (mu / r0).sqrt();
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        let output_interval = args.output_interval.unwrap_or(args.dt);
        let stream_interval = args
            .stream_interval
            .unwrap_or(output_interval)
            .clamp(args.dt, output_interval);
        let epoch = match &args.epoch {
            Some(s) => Some(
                Epoch::from_iso8601(s)
                    .unwrap_or_else(|| panic!("Invalid epoch format: {s}. Expected ISO 8601 (e.g. 2024-03-20T12:00:00Z)"))
            ),
            // Default to current time for known solar-system bodies
            None => Some(Epoch::now()),
        };
        Self {
            body,
            mu,
            r0,
            v0,
            period,
            dt: args.dt,
            altitude: args.altitude,
            output_interval,
            stream_interval,
            epoch,
        }
    }

    fn initial_state(&self) -> State {
        State {
            position: vector![self.r0, 0.0, 0.0],
            velocity: vector![0.0, self.v0, 0.0],
        }
    }
}

/// A single state snapshot used in history messages.
#[derive(Serialize, Clone, Debug)]
struct HistoryState {
    t: f64,
    position: [f64; 3],
    velocity: [f64; 3],
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
}

impl HistoryBuffer {
    fn new(capacity: usize, data_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
        HistoryBuffer {
            states: VecDeque::new(),
            capacity,
            data_dir,
            segment_count: 0,
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
        let sat_path = EntityPath::parse("/world/sat/default");

        for (i, hs) in to_flush.iter().enumerate() {
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
                        all.push(HistoryState {
                            t: row.t,
                            position: [row.x, row.y, row.z],
                            velocity: [row.vx, row.vy, row.vz],
                        });
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to read segment {i}: {e}");
                }
            }
        }

        // Append in-memory buffer
        all.extend(self.states.iter().cloned());

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
        altitude: f64,
        period: f64,
        dt: f64,
        output_interval: f64,
        stream_interval: f64,
        central_body: String,
        central_body_radius: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        epoch_jd: Option<f64>,
    },
    /// A single simulation state snapshot.
    #[serde(rename = "state")]
    State {
        t: f64,
        position: [f64; 3],
        velocity: [f64; 3],
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
    let params = SimParams::from_sim_args(sim);

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

/// Run the simulation and return a Recording.
fn run_simulation(params: &SimParams) -> Recording {
    let system = TwoBodySystem { mu: params.mu };
    let initial = params.initial_state();

    let mut rec = Recording::new();
    let body_path = EntityPath::parse(&format!("/world/{}", params.body.properties().name));
    let sat_path = EntityPath::parse("/world/sat/default");

    rec.log_static(&body_path, &GravitationalParameter(params.mu));
    rec.log_static(&body_path, &BodyRadius(params.body.properties().radius));

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
    let final_state =
        Rk4::integrate(&system, initial, 0.0, params.period, params.dt, |t, state| {
            if t >= next_output_t - 1e-9 {
                record_state(&mut rec, t, step, state);
                step += 1;
                last_output_t = t;
                next_output_t += params.output_interval;
            }
        });

    if (params.period - last_output_t) > 1e-9 {
        record_state(&mut rec, params.period, step, &final_state);
    }

    rec.metadata = orts_datamodel::recording::SimMetadata {
        epoch_jd: params.epoch.map(|e| e.jd()),
        mu: Some(params.mu),
        body_radius: Some(params.body.properties().radius),
        body_name: Some(params.body.properties().name.to_string()),
        altitude: Some(params.altitude),
        period: Some(params.period),
    };

    rec
}

/// Print a Recording as CSV to stdout.
fn print_recording_as_csv(rec: &Recording, params: &SimParams) {
    use orts_datamodel::component::Component;
    use orts_datamodel::components::{Position3D, Velocity3D};
    use orts_datamodel::timeline::TimelineName;

    println!("# Orts 2-body orbit propagation");
    println!("# mu = {} km^3/s^2", params.mu);
    println!(
        "# Initial orbit: circular at {} km altitude (r = {} km)",
        params.altitude, params.r0
    );
    println!(
        "# Period = {:.1} s ({:.1} min)",
        params.period,
        params.period / 60.0
    );
    if let Some(epoch) = params.epoch {
        println!("# epoch_jd = {}", epoch.jd());
        let (y, mo, d, h, mi, s) = epoch.to_gregorian();
        println!("# epoch = {:04}-{:02}-{:02}T{:02}:{:02}:{:02.0}Z", y, mo, d, h, mi, s);
    }
    println!(
        "# central_body = {}",
        params.body.properties().name.to_lowercase()
    );
    println!(
        "# central_body_radius = {} km",
        params.body.properties().radius
    );
    println!("# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]");

    let sat_path = EntityPath::parse("/world/sat/default");
    let store = match rec.entity(&sat_path) {
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

    // Each temporal row was logged twice (once for Position3D, once for Velocity3D),
    // so sim_times has 2x the rows. We take every other entry.
    for i in 0..pos_col.num_rows() {
        let t = match sim_times.get(i * 2) {
            Some(orts_datamodel::timeline::TimeIndex::Seconds(s)) => *s,
            _ => 0.0,
        };
        let pos = pos_col.get_row(i).unwrap();
        let vel = vel_col.get_row(i).unwrap();
        println!(
            "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
            t, pos[0], pos[1], pos[2], vel[0], vel[1], vel[2],
        );
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
    let params = Arc::new(SimParams::from_sim_args(sim));
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    eprintln!("WebSocket server listening on ws://localhost:{port}");

    let data_dir = std::env::temp_dir().join(format!("orts-{}", std::process::id()));
    let history = Arc::new(tokio::sync::RwLock::new(HistoryBuffer::new(5000, data_dir)));

    let (tx, _rx) = broadcast::channel::<String>(256);

    let sim_tx = tx.clone();
    let sim_params = Arc::clone(&params);
    let sim_history = Arc::clone(&history);
    tokio::spawn(async move {
        simulation_loop(sim_params, sim_tx, sim_history).await;
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

        tokio::spawn(async move {
            handle_connection(stream, rx, client_params, client_history).await;
        });
    }
}

async fn simulation_loop(
    params: Arc<SimParams>,
    tx: broadcast::Sender<String>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
) {
    let system = TwoBodySystem { mu: params.mu };
    let dt = params.dt;

    // Batch N stream intervals into a single compute chunk.
    // stream_interval controls WebSocket send cadence (fine, latency-sensitive).
    // output_interval controls history save cadence (coarse, throughput-sensitive).
    const OUTPUTS_PER_CHUNK: usize = 10;
    let chunk_sim_time = params.stream_interval * OUTPUTS_PER_CHUNK as f64;

    // Wall-clock pacing: target sim speed ratio.
    let wall_per_sim_sec = ((dt / 100.0).max(0.01)) / params.stream_interval;
    let chunk_wall_time =
        std::time::Duration::from_secs_f64(chunk_sim_time * wall_per_sim_sec);

    loop {
        let initial = params.initial_state();

        // Emit t=0 state
        let hs = HistoryState {
            t: 0.0,
            position: [initial.position.x, initial.position.y, initial.position.z],
            velocity: [initial.velocity.x, initial.velocity.y, initial.velocity.z],
        };
        history.write().await.push(hs);

        let msg = state_message(0.0, &initial);
        if tx.send(msg).is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }

        let mut state = initial;
        let mut t = 0.0;
        let mut next_stream_t = params.stream_interval;
        let mut next_save_t = params.output_interval;

        while t < params.period {
            let chunk_start = tokio::time::Instant::now();
            let chunk_end = (t + chunk_sim_time).min(params.period);

            // Pure computation: collect outputs at stream_interval cadence
            let (outputs, new_state, new_t) = compute_output_chunk(
                &system,
                state,
                t,
                chunk_end,
                dt,
                params.stream_interval,
                &mut next_stream_t,
            );

            state = new_state;
            t = new_t;

            // Save only output_interval-aligned states to history (coarse).
            // Broadcast all stream outputs to WebSocket clients (fine).
            if !outputs.is_empty() {
                {
                    let mut h = history.write().await;
                    for out in &outputs {
                        if out.t >= next_save_t - 1e-9 {
                            h.push(out.clone());
                            next_save_t += params.output_interval;
                        }
                    }
                }

                let send_interval = chunk_wall_time / outputs.len() as u32;
                for out in &outputs {
                    let send_start = tokio::time::Instant::now();
                    let msg = serde_json::to_string(&WsMessage::State {
                        t: out.t,
                        position: out.position,
                        velocity: out.velocity,
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

        // Emit final state if the last output didn't land on period end
        let last_output_t = if let Some(last) = history.read().await.states.back() {
            last.t
        } else {
            0.0
        };
        if (params.period - last_output_t).abs() > 1e-9 {
            let hs = HistoryState {
                t: params.period,
                position: [state.position.x, state.position.y, state.position.z],
                velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
            };
            history.write().await.push(hs);

            let msg = state_message(params.period, &state);
            let _ = tx.send(msg);
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    params: Arc<SimParams>,
    history: Arc<tokio::sync::RwLock<HistoryBuffer>>,
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
    let info = WsMessage::Info {
        mu: params.mu,
        altitude: params.altitude,
        period: params.period,
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
    };
    let info_json = serde_json::to_string(&info).expect("failed to serialize info message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(info_json.into()))
        .await
        .is_err()
    {
        return;
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
                                ClientMessage::QueryRange { t_min, t_max, max_points } => {
                                    let states = history.read().await.query_range(t_min, t_max, max_points);
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

/// Compute RK4 integration from t_start to chunk_end, collecting output states
/// at output_interval boundaries. Pure computation with no IO.
///
/// Returns (output_states, final_state, final_t).
fn compute_output_chunk(
    system: &TwoBodySystem,
    mut state: State,
    t_start: f64,
    chunk_end: f64,
    dt: f64,
    output_interval: f64,
    next_output_t: &mut f64,
) -> (Vec<HistoryState>, State, f64) {
    let mut outputs = Vec::new();
    let mut t = t_start;

    while t < chunk_end {
        let h = dt.min(chunk_end - t);
        state = Rk4::step(system, t, &state, h);
        t += h;

        if t >= *next_output_t - 1e-9 {
            outputs.push(HistoryState {
                t,
                position: [state.position.x, state.position.y, state.position.z],
                velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
            });
            *next_output_t += output_interval;
        }
    }

    (outputs, state, t)
}

fn state_message(t: f64, state: &State) -> String {
    let msg = WsMessage::State {
        t,
        position: [state.position.x, state.position.y, state.position.z],
        velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(t: f64) -> HistoryState {
        HistoryState {
            t,
            position: [6778.0 + t, t * 0.1, 0.0],
            velocity: [0.0, 7.669, 0.0],
        }
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
        let mut buf = HistoryBuffer::new(100, dir.clone());

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
        let mut buf = HistoryBuffer::new(4, dir.clone());

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
        let mut buf = HistoryBuffer::new(4, dir.clone());

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
        let mut buf = HistoryBuffer::new(10_000, dir.clone());

        for i in 0..5000 {
            buf.states.push_back(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        buf.flush();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 1000,
            "flush took {}ms, expected <1000ms",
            elapsed.as_millis()
        );
        assert_eq!(buf.segment_count, 1);

        cleanup_dir(&dir);
    }

    #[test]
    fn load_all_performance() {
        let dir = temp_data_dir("load-perf");
        let mut buf = HistoryBuffer::new(2000, dir.clone());

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
            states: vec![
                HistoryState {
                    t: 0.0,
                    position: [6778.137, 0.0, 0.0],
                    velocity: [0.0, 7.669, 0.0],
                },
                HistoryState {
                    t: 10.0,
                    position: [6777.0, 76.0, 0.0],
                    velocity: [-0.086, 7.668, 0.0],
                },
            ],
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
            states: vec![HistoryState {
                t: 5.0,
                position: [1.0, 2.0, 3.0],
                velocity: [4.0, 5.0, 6.0],
            }],
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
            } => {
                assert!((t_min - 10.0).abs() < 1e-9);
                assert!((t_max - 50.0).abs() < 1e-9);
                assert_eq!(max_points, Some(100));
            }
        }
    }

    #[test]
    fn client_message_query_range_without_max_points() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { max_points, .. } => {
                assert_eq!(max_points, None);
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
        let mut buf = HistoryBuffer::new(100, dir.clone());

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
        let mut buf = HistoryBuffer::new(200, dir.clone());

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
        let mut buf = HistoryBuffer::new(100, dir.clone());

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
        };
        let params = SimParams::from_sim_args(&args);
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
        };
        let params = SimParams::from_sim_args(&args);
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
        };
        let params = SimParams::from_sim_args(&args);
        assert!((params.stream_interval - 5.0).abs() < 1e-9);

        // stream_interval > output_interval → clamped to output_interval
        let args2 = SimArgs {
            altitude: 400.0,
            body: "earth".to_string(),
            dt: 1.0,
            output_interval: Some(10.0),
            stream_interval: Some(20.0),
            epoch: None,
        };
        let params2 = SimParams::from_sim_args(&args2);
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
        };
        let params = SimParams::from_sim_args(&args);
        assert!(params.epoch.is_some());
        let epoch = params.epoch.unwrap();
        // 2024-03-20 12:00:00 UTC
        assert!((epoch.jd() - 2460390.0).abs() < 0.01);
    }

    #[test]
    fn info_message_with_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            altitude: 400.0,
            period: 5554.0,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        assert_eq!(v["epoch_jd"], 2460390.0);
    }

    #[test]
    fn info_message_without_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            altitude: 400.0,
            period: 5554.0,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        // epoch_jd should be absent (skip_serializing_if)
        assert!(v.get("epoch_jd").is_none());
    }

    // --- compute_output_chunk tests ---

    #[test]
    fn chunk_output_count_matches_interval() {
        // dt=10, output_interval=10, chunk=100s → expect 10 outputs
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = TwoBodySystem { mu };
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _final_state, final_t) =
            compute_output_chunk(&system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output);

        assert_eq!(outputs.len(), 10);
        assert!((final_t - 100.0).abs() < 1e-9);
    }

    #[test]
    fn chunk_fine_dt_batches_steps() {
        // dt=1, output_interval=10, chunk=100s → still 10 outputs but 100 RK4 steps
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = TwoBodySystem { mu };
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _, _) =
            compute_output_chunk(&system, initial, 0.0, 100.0, 1.0, 10.0, &mut next_output);

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
        let system = TwoBodySystem { mu };
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };
        let initial_energy = v0 * v0 / 2.0 - mu / r0;

        let mut next_output = 10.0;
        let (outputs, _, _) =
            compute_output_chunk(&system, initial, 0.0, 500.0, 10.0, 10.0, &mut next_output);

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
        let system = TwoBodySystem { mu };
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        // chunk_end=55 with output_interval=10 → outputs at 10,20,30,40,50 (5 outputs)
        let (outputs, _, final_t) =
            compute_output_chunk(&system, initial, 0.0, 55.0, 10.0, 10.0, &mut next_output);

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
        let system = TwoBodySystem { mu };
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let stream_interval = 2.0;
        let output_interval = 10.0;
        let mut next_stream = stream_interval;

        let (outputs, _, _) =
            compute_output_chunk(&system, initial, 0.0, 20.0, 1.0, stream_interval, &mut next_stream);

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
        let system = TwoBodySystem { mu };
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
            state_ss = Rk4::step(&system, t, &state_ss, dt);
            t += dt;
            step_outputs.push(HistoryState {
                t,
                position: [state_ss.position.x, state_ss.position.y, state_ss.position.z],
                velocity: [state_ss.velocity.x, state_ss.velocity.y, state_ss.velocity.z],
            });
        }

        // Chunked
        let mut next_output = 10.0;
        let (chunk_outputs, _, _) =
            compute_output_chunk(&system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output);

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
}
