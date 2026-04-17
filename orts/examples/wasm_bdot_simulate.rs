//! Run the WASM B-dot finite-difference guest controller across
//! multiple parameter combinations and write the angular-velocity
//! time series to CSV files for plotting.
//!
//! Usage:
//!
//! ```sh
//! # Build the guest first:
//! cd plugins/bdot-finite-diff && cargo +1.91.0 component build --release && cd -
//!
//! # Run the simulation sweep:
//! cargo run --example wasm_bdot_simulate --features plugin-wasm --release
//! ```
//!
//! Outputs CSV files in `plugins/bdot-finite-diff/`:
//!
//! - `sim_gain_<gain>_omega_<omega>.csv`
//!
//! Each file has columns: `t,omega_x,omega_y,omega_z,omega_mag`

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use arika::earth::{MU as MU_EARTH, R as R_EARTH};
use arika::epoch::Epoch;
use nalgebra::{Matrix3, Vector3, Vector4};
use tobari::magnetic::TiltedDipole;
use utsuroi::{Integrator, Rk4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{AttitudeState, CommandedMagnetorquer, DecoupledAttitudeSystem};
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{ActuatorBundle, ActuatorState, MtqCommand, PluginController, TickInput};
use orts::sensor::{Gyroscope, Magnetometer, SensorBundle};

/// Convert per-MTQ moments from ActuatorBundle to a Vector3 for
/// CommandedMagnetorquer (3-axis orthogonal layout assumed).
fn mtq_moment_vec3(bundle: &ActuatorBundle) -> Vector3<f64> {
    match bundle.mtq_command() {
        Some(MtqCommand::Moments(v)) if !v.is_empty() => Vector3::from_row_slice(v),
        _ => Vector3::zeros(),
    }
}

const MASS: f64 = 50.0;
const ALT_KM: f64 = 500.0;
const ODE_DT: f64 = 0.1;
const SAMPLE_PERIOD: f64 = 1.0;
const T_END: f64 = 600.0;

/// Parameter combinations to sweep.
struct SimCase {
    gain: f64,
    initial_omega_mag: f64,
    label: String,
}

fn main() {
    let gains = [1e3, 1e4, 1e5];
    let initial_omegas = [0.05, 0.1, 0.2]; // rad/s magnitude

    let mut cases = Vec::new();
    for &gain in &gains {
        for &omega in &initial_omegas {
            cases.push(SimCase {
                gain,
                initial_omega_mag: omega,
                label: format!("gain_{:.0e}_omega_{:.2}", gain, omega),
            });
        }
    }

    // Set up WASM engine + component (shared across all cases).
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let wasm_path = format!(
        "{}/../plugins/bdot-finite-diff/target/wasm32-wasip1/release/orts_example_plugin_bdot_finite_diff.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    let wasm_bytes = std::fs::read(&wasm_path).unwrap_or_else(|e| {
        panic!(
            "Guest WASM not found at {wasm_path}: {e}\n\
             Build it first:\n  cd plugins/bdot-finite-diff\n  \
             cargo +1.91.0 component build --release"
        )
    });
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component compile failed");
    let pre = WasmController::prepare(&engine, &component).expect("prepare failed");

    let output_dir: PathBuf =
        format!("{}/../plugins/bdot-finite-diff", env!("CARGO_MANIFEST_DIR")).into();

    for case in &cases {
        println!("Running: {} ...", case.label);
        let csv_path = output_dir.join(format!("sim_{}.csv", case.label));
        let trajectory = run_case(&pre, case);
        write_csv(&csv_path, &trajectory);
        let final_omega = trajectory.last().unwrap().1;
        println!(
            "  {} -> |omega| {:.6} -> {:.6} rad/s ({:.1}% reduction)",
            case.label,
            case.initial_omega_mag,
            final_omega,
            (1.0 - final_omega / case.initial_omega_mag) * 100.0,
        );
    }

    println!("\nDone. CSV files written to {}", output_dir.display());
    println!("Plot with: cd plugins/bdot-finite-diff && python plot.py");
}

/// (t, |omega|, omega_x, omega_y, omega_z)
type TrajectoryPoint = (f64, f64, f64, f64, f64);

fn run_case(
    pre: &orts::plugin::wasm::bindings::PluginPre<orts::plugin::wasm::host_state::HostState>,
    case: &SimCase,
) -> Vec<TrajectoryPoint> {
    let config = format!(r#"{{"gain":{}}}"#, case.gain);
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;
    let epoch = Epoch::j2000();
    let inertia = Matrix3::from_diagonal(&Vector3::new(1.0, 1.0, 1.0));

    // Distribute initial angular velocity along all 3 axes.
    let omega0 = case.initial_omega_mag / 3.0_f64.sqrt();
    let initial = AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: Vector3::new(omega0, omega0, -omega0),
    };

    let field_model: Arc<dyn tobari::magnetic::MagneticFieldModel> =
        Arc::new(TiltedDipole::earth());

    let mut ctrl =
        WasmController::new(pre, &case.label, &config).expect("WasmController::new failed");
    let mut bundle = ActuatorBundle::new();

    let mut sensor_bundle = SensorBundle {
        magnetometers: vec![Magnetometer::new(Arc::clone(&field_model))],
        gyroscopes: vec![Gyroscope::new()],
        star_trackers: vec![],
    };
    let mut state = initial;
    let mut t = 0.0;
    let mut trajectory = Vec::new();

    // Record initial state.
    let w = &state.angular_velocity;
    trajectory.push((t, w.magnitude(), w.x, w.y, w.z));

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(mtq_moment_vec3(&bundle), TiltedDipole::earth());
        let system = DecoupledAttitudeSystem::circular_orbit(inertia, mu, radius, MASS)
            .with_model(actuator)
            .with_epoch(epoch);
        state = Rk4.integrate(&system, state, t, t_next, ODE_DT, |_, _| {});
        t = t_next;

        let n = (mu / radius.powi(3)).sqrt();
        let v = (mu / radius).sqrt();
        let theta = n * t;
        let orbit_at_t = OrbitalState::new(
            Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
            Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
        );
        let current_epoch = epoch.add_seconds(t);
        let snapshot = SpacecraftState {
            orbit: orbit_at_t,
            attitude: state.clone(),
            mass: MASS,
        };
        let sensors = sensor_bundle.evaluate(&snapshot, &current_epoch);
        let actuator_state = ActuatorState::default();
        let obs = TickInput {
            t,
            epoch: Some(&current_epoch),
            sensors: &sensors,
            actuators: &actuator_state,
            spacecraft: &snapshot,
        };
        if let Some(cmd) = ctrl.update(&obs).expect("WASM update must succeed") {
            bundle.apply(&cmd).expect("command must be finite");
        }

        let w = &state.angular_velocity;
        trajectory.push((t, w.magnitude(), w.x, w.y, w.z));
    }

    trajectory
}

fn write_csv(path: &std::path::Path, trajectory: &[TrajectoryPoint]) {
    let mut f = std::fs::File::create(path).expect("cannot create CSV");
    writeln!(f, "t,omega_x,omega_y,omega_z,omega_mag").unwrap();
    for &(t, mag, ox, oy, oz) in trajectory {
        writeln!(f, "{t},{ox},{oy},{oz},{mag}").unwrap();
    }
}
