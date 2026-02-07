use clap::Parser;
use nalgebra::vector;
use orts_integrator::{Rk4, State};
use orts_orbits::{constants, two_body::TwoBodySystem};

/// Orts CLI — orbital mechanics simulation tool
#[derive(Parser, Debug)]
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

    /// Integration time step in seconds
    #[arg(long, default_value_t = 10.0)]
    dt: f64,
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
    let mu = constants::MU_EARTH;
    let r0 = constants::R_EARTH + args.altitude;
    let v0 = (mu / r0).sqrt(); // Circular velocity

    let system = TwoBodySystem { mu };
    let initial = State {
        position: vector![r0, 0.0, 0.0],
        velocity: vector![0.0, v0, 0.0],
    };

    let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
    let dt = args.dt;

    println!("# Orts 2-body orbit propagation");
    println!("# mu = {} km^3/s^2", mu);
    println!(
        "# Initial orbit: circular at {} km altitude (r = {} km)",
        args.altitude, r0
    );
    println!("# Period = {:.1} s ({:.1} min)", period, period / 60.0);
    println!("# t[s],x[km],y[km],z[km],vx[km/s],vy[km/s],vz[km/s]");

    // Print initial state
    print_state(0.0, &initial);

    // Propagate for one full period
    Rk4::integrate(&system, initial, 0.0, period, dt, |t, state| {
        print_state(t, state);
    });
}

fn run_server(_args: Args) {
    eprintln!("WebSocket server mode not yet implemented");
    std::process::exit(1);
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
