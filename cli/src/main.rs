use nalgebra::vector;
use orts_integrator::{Rk4, State};
use orts_orbits::{constants, two_body::TwoBodySystem};

fn main() {
    let mu = constants::MU_EARTH;
    let r0 = constants::R_EARTH + 400.0; // ISS-like orbit altitude
    let v0 = (mu / r0).sqrt(); // Circular velocity

    let system = TwoBodySystem { mu };
    let initial = State {
        position: vector![r0, 0.0, 0.0],
        velocity: vector![0.0, v0, 0.0],
    };

    let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
    let dt = 10.0; // seconds

    println!("# Orts 2-body orbit propagation");
    println!("# mu = {} km^3/s^2", mu);
    println!(
        "# Initial orbit: circular at {} km altitude (r = {} km)",
        400.0, r0
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
