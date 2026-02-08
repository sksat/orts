use std::sync::Arc;

use clap::Parser;
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
#[derive(Parser, Debug, Clone)]
#[command(name = "orts-cli")]
struct Args {
    /// Start WebSocket server mode
    #[arg(long)]
    serve: bool,

    /// WebSocket server port
    #[arg(long, default_value_t = 9001)]
    port: u16,

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
    fn from_args(args: &Args) -> Self {
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
    let args = Args::parse();

    if args.serve {
        run_server(args);
    } else {
        run_csv(args);
    }
}

fn run_csv(args: Args) {
    let params = SimParams::from_args(&args);
    let system = TwoBodySystem { mu: params.mu };
    let initial = params.initial_state();

    // Build a Recording alongside CSV output.
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

    // Print initial state
    print_state(0.0, &initial);
    record_state(&mut rec, 0.0, step, &initial);
    step += 1;

    // Propagate for one full period, emitting output at the configured interval.
    let mut next_output_t = params.output_interval;
    let mut last_output_t = 0.0_f64;
    let final_state =
        Rk4::integrate(&system, initial, 0.0, params.period, params.dt, |t, state| {
            if t >= next_output_t - 1e-9 {
                print_state(t, state);
                record_state(&mut rec, t, step, state);
                step += 1;
                last_output_t = t;
                next_output_t += params.output_interval;
            }
        });

    // Always emit the final state so the output covers the full period.
    if (params.period - last_output_t) > 1e-9 {
        print_state(params.period, &final_state);
        record_state(&mut rec, params.period, step, &final_state);
    }

    eprintln!(
        "Recording: {} entities, {} data points for satellite",
        rec.entity_paths().count(),
        rec.entity(&sat_path).map_or(0, |s| s.num_rows),
    );
}

fn run_server(args: Args) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async_server(args));
}

async fn async_server(args: Args) {
    let params = Arc::new(SimParams::from_args(&args));
    let addr = format!("0.0.0.0:{}", args.port);
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind to {addr}: {e}"));

    eprintln!(
        "WebSocket server listening on ws://localhost:{}",
        args.port
    );

    // Broadcast channel for simulation state messages.
    // The capacity is generous enough to avoid dropping frames under normal conditions.
    let (tx, _rx) = broadcast::channel::<String>(256);

    // Spawn the simulation loop that produces messages.
    let sim_tx = tx.clone();
    let sim_params = Arc::clone(&params);
    tokio::spawn(async move {
        simulation_loop(sim_params, sim_tx).await;
    });

    // Accept incoming WebSocket connections.
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

/// Run the simulation in an infinite loop, broadcasting each state to all
/// connected clients via the broadcast channel.
async fn simulation_loop(params: Arc<SimParams>, tx: broadcast::Sender<String>) {
    let system = TwoBodySystem { mu: params.mu };
    let dt = params.dt;
    // Compute a sleep duration proportional to the integration step.
    // We use dt / 100 as the real-time factor so a 10s step takes 100ms of wall time.
    let sleep_duration = std::time::Duration::from_secs_f64((dt / 100.0).max(0.01));

    loop {
        let initial = params.initial_state();

        // Send the initial state (t = 0).
        let msg = state_message(0.0, &initial);
        if tx.send(msg).is_err() {
            // No receivers; wait a bit and retry.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }
        tokio::time::sleep(sleep_duration).await;

        // Propagate one full period, collecting only states at the output interval.
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

        // Always include the final state so the output covers the full period.
        if (params.period - last_output_t) > 1e-9 {
            states.push((params.period, final_state));
        }

        for (t, state) in &states {
            let msg = state_message(*t, state);
            // If no receivers are left, just continue; new clients may arrive.
            let _ = tx.send(msg);
            tokio::time::sleep(sleep_duration).await;
        }

        // Brief pause before restarting the orbit.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Handle a single WebSocket client connection.
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

    // Send the info message first.
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

    // Forward broadcast messages to this client, while also listening for
    // incoming messages (to detect disconnection).
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
                            break; // Client disconnected.
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Client lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break; // Broadcast channel closed.
                    }
                }
            }
            ws_msg = ws_receiver.next() => {
                match ws_msg {
                    Some(Ok(_)) => {
                        // Ignore client messages for now (future: control protocol).
                    }
                    Some(Err(_)) | None => {
                        break; // Client disconnected.
                    }
                }
            }
        }
    }

    eprintln!("Client disconnected");
}

/// Build a JSON string for a state message.
fn state_message(t: f64, state: &State) -> String {
    let msg = WsMessage::State {
        t,
        position: [state.position.x, state.position.y, state.position.z],
        velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}

fn print_state(t: f64, state: &State) {
    println!(
        "{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
        t,
        state.position.x,
        state.position.y,
        state.position.z,
        state.velocity.x,
        state.velocity.y,
        state.velocity.z,
    );
}
