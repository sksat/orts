use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use nalgebra::vector;
use orts_datamodel::archetypes::OrbitalState;
use orts_datamodel::components::{BodyRadius, GravitationalParameter};
use orts_datamodel::entity_path::EntityPath;
use orts_datamodel::recording::Recording;
use orts_datamodel::timeline::TimePoint;
use orts_integrator::{Rk4, State};
use orts_orbits::{body::KnownBody, two_body::TwoBodySystem};
use serde::Serialize;
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
}

impl SimParams {
    fn from_sim_args(args: &SimArgs) -> Self {
        let body = parse_body(&args.body);
        let props = body.properties();
        let mu = props.mu;
        let r0 = props.radius + args.altitude;
        let v0 = (mu / r0).sqrt();
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        Self {
            body,
            mu,
            r0,
            v0,
            period,
            dt: args.dt,
            altitude: args.altitude,
            output_interval: args.output_interval.unwrap_or(args.dt),
        }
    }

    fn initial_state(&self) -> State {
        State {
            position: vector![self.r0, 0.0, 0.0],
            velocity: vector![0.0, self.v0, 0.0],
        }
    }
}

/// Server-to-client message: simulation info (sent once on connect).
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
        central_body: String,
        central_body_radius: f64,
    },
    /// A single simulation state snapshot.
    #[serde(rename = "state")]
    State {
        t: f64,
        position: [f64; 3],
        velocity: [f64; 3],
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
            let rows = orts_datamodel::rerun_export::load_from_rrd(input)
                .unwrap_or_else(|e| {
                    eprintln!("Error reading {input}: {e}");
                    std::process::exit(1);
                });

            let write_csv = |w: &mut dyn std::io::Write| -> std::io::Result<()> {
                writeln!(w, "# Converted from {input}")?;
                writeln!(w, "# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]")?;
                for row in &rows {
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

    let (tx, _rx) = broadcast::channel::<String>(256);

    let sim_tx = tx.clone();
    let sim_params = Arc::clone(&params);
    tokio::spawn(async move {
        simulation_loop(sim_params, sim_tx).await;
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

        let rx = tx.subscribe();
        let client_params = Arc::clone(&params);

        tokio::spawn(async move {
            handle_connection(stream, rx, client_params).await;
        });
    }
}

async fn simulation_loop(params: Arc<SimParams>, tx: broadcast::Sender<String>) {
    let system = TwoBodySystem { mu: params.mu };
    let dt = params.dt;
    let sleep_duration = std::time::Duration::from_secs_f64((dt / 100.0).max(0.01));

    loop {
        let initial = params.initial_state();

        let msg = state_message(0.0, &initial);
        if tx.send(msg).is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }
        tokio::time::sleep(sleep_duration).await;

        let mut states: Vec<(f64, State)> = Vec::new();
        let mut next_output_t = params.output_interval;
        let mut last_output_t = 0.0_f64;
        let output_interval = params.output_interval;
        let final_state =
            Rk4::integrate(&system, initial, 0.0, params.period, dt, |t, state| {
                if t >= next_output_t - 1e-9 {
                    states.push((t, state.clone()));
                    last_output_t = t;
                    next_output_t += output_interval;
                }
            });

        if (params.period - last_output_t) > 1e-9 {
            states.push((params.period, final_state));
        }

        for (t, state) in &states {
            let msg = state_message(*t, state);
            let _ = tx.send(msg);
            tokio::time::sleep(sleep_duration).await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<String>,
    params: Arc<SimParams>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {e}");
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let info = WsMessage::Info {
        mu: params.mu,
        altitude: params.altitude,
        period: params.period,
        dt: params.dt,
        output_interval: params.output_interval,
        central_body: serde_json::to_value(&params.body)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string(),
        central_body_radius: params.body.properties().radius,
    };
    let info_json = serde_json::to_string(&info).expect("failed to serialize info message");
    if ws_sender
        .send(tokio_tungstenite::tungstenite::Message::Text(info_json.into()))
        .await
        .is_err()
    {
        return;
    }

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
            ws_msg = ws_receiver.next() => {
                match ws_msg {
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

fn state_message(t: f64, state: &State) -> String {
    let msg = WsMessage::State {
        t,
        position: [state.position.x, state.position.y, state.position.z],
        velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}
