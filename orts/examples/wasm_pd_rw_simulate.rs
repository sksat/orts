//! Run the WASM PD+RW attitude controller and write time series to CSV.
//!
//! Usage:
//!
//! ```sh
//! # Build the guest first:
//! cd plugins/pd-rw-control && cargo +1.91.0 component build --release && cd -
//!
//! # Run the simulation:
//! cargo run --example wasm_pd_rw_simulate --features plugin-wasm --release
//! ```
//!
//! Outputs CSV in `plugins/pd-rw-control/`:
//! - `sim_pd_rw.csv`: attitude error, angular velocity, RW momentum

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use kaname::earth::{MU as MU_EARTH, R as R_EARTH};
use kaname::epoch::Epoch;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use utsuroi::{Integrator, Rk4};
use wasmtime::component::Component;

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{AttitudeState, AugmentedAttitudeSystem, GravityGradientTorque};
use orts::effector::AugmentedState;
use orts::plugin::wasm::{WasmController, WasmEngine};
use orts::plugin::{ActuatorBundle, ActuatorState, PluginController, TickInput};
use orts::sensor::{Gyroscope, SensorBundle, StarTracker};
use orts::spacecraft::ReactionWheelAssembly;

const MASS: f64 = 500.0;
const ALT_KM: f64 = 400.0;
const KP: f64 = 1.0;
const KD: f64 = 2.0;
const DT_CTRL: f64 = 0.1;
const DT_ODE: f64 = 0.01;
const T_END: f64 = 120.0;

fn main() {
    let engine = Arc::new(WasmEngine::new().expect("WasmEngine must init"));
    let wasm_path = format!(
        "{}/../plugins/pd-rw-control/target/wasm32-wasip1/release/orts_example_plugin_pd_rw_control.wasm",
        env!("CARGO_MANIFEST_DIR")
    );
    let wasm_bytes = std::fs::read(&wasm_path).unwrap_or_else(|e| {
        panic!(
            "Guest WASM not found at {wasm_path}: {e}\n\
             Build it first:\n  cd plugins/pd-rw-control\n  \
             cargo +1.91.0 component build --release"
        )
    });
    let component = Component::new(engine.inner(), &wasm_bytes).expect("Component compile failed");
    let pre = WasmController::prepare(&engine, &component).expect("prepare failed");

    let config = format!(r#"{{"kp":{},"kd":{},"sample_period":{}}}"#, KP, KD, DT_CTRL);
    let mut ctrl = WasmController::new(&pre, "pd-rw", &config).expect("WasmController::new failed");

    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;
    let epoch = Epoch::j2000();
    let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0));
    let target_q = UnitQuaternion::identity();

    // Initial condition: 30 deg error about Z + some angular velocity.
    let angle0 = 30.0_f64.to_radians();
    let axis = nalgebra::Unit::new_normalize(Vector3::new(1.0, 0.5, 0.3).normalize());
    let initial_q = UnitQuaternion::from_axis_angle(&axis, angle0);
    let initial = AttitudeState::new(initial_q, Vector3::new(0.02, -0.01, 0.015));

    let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
    let field_model: Arc<dyn tobari::magnetic::MagneticFieldModel> =
        Arc::new(tobari::magnetic::TiltedDipole::earth());

    let mut bundle = ActuatorBundle::new();

    let mut sensor_bundle = SensorBundle {
        magnetometer: Some(orts::sensor::Magnetometer::new(Arc::clone(&field_model))),
        gyroscope: Some(Gyroscope::new()),
        star_tracker: Some(StarTracker::new()),
    };

    let mut state = AugmentedState {
        plant: initial,
        aux: vec![0.0, 0.0, 0.0],
        aux_bounds: vec![],
    };
    let mut t = 0.0;

    let mut rows: Vec<CsvRow> = Vec::new();
    rows.push(record(&state, t, &target_q));

    while t < T_END - 1e-12 {
        let t_next = (t + DT_CTRL).min(T_END);

        let mut rw_seg = rw.clone();
        rw_seg.commanded_torque = bundle.rw_torque().into_inner();
        let gg = GravityGradientTorque::circular_orbit(mu, radius, inertia);
        let system = AugmentedAttitudeSystem::circular_orbit(inertia, mu, radius, MASS)
            .with_model(gg)
            .with_effector(rw_seg);
        state = Rk4.integrate(&system, state, t, t_next, DT_ODE, |_, _| {});
        t = t_next;

        let n = (mu / radius.powi(3)).sqrt();
        let v = (mu / radius).sqrt();
        let theta = n * t;
        let orbit = OrbitalState::new(
            Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
            Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
        );
        let current_epoch = epoch.add_seconds(t);
        let snapshot = SpacecraftState {
            orbit,
            attitude: state.plant.clone(),
            mass: MASS,
        };
        let sensors = sensor_bundle.evaluate(&snapshot, &current_epoch);
        let actuator_state = ActuatorState {
            rw_momentum: Some(state.aux.clone()),
        };
        let input = TickInput {
            t,
            epoch: Some(&current_epoch),
            sensors: &sensors,
            actuators: &actuator_state,
            spacecraft: &snapshot,
        };
        if let Some(cmd) = ctrl.update(&input).expect("WASM update must succeed") {
            bundle.apply(&cmd).expect("command must be finite");
        }

        rows.push(record(&state, t, &target_q));
    }

    let output_dir: PathBuf =
        format!("{}/../plugins/pd-rw-control", env!("CARGO_MANIFEST_DIR")).into();
    let csv_path = output_dir.join("sim_pd_rw.csv");
    write_csv(&csv_path, &rows);

    let last = rows.last().unwrap();
    println!(
        "Done. {:.0}s simulated, final attitude error: {:.4} deg, |omega|: {:.6} rad/s",
        T_END, last.angle_error_deg, last.omega_mag
    );
    println!("CSV written to {}", csv_path.display());
    println!("Plot with: cd plugins/pd-rw-control && uv run plot.py");
}

struct CsvRow {
    t: f64,
    angle_error_deg: f64,
    omega_x: f64,
    omega_y: f64,
    omega_z: f64,
    omega_mag: f64,
    h_x: f64,
    h_y: f64,
    h_z: f64,
    h_mag: f64,
}

fn record(state: &AugmentedState<AttitudeState>, t: f64, target_q: &UnitQuaternion<f64>) -> CsvRow {
    let q_err = target_q.inverse() * state.plant.orientation();
    let angle_error_deg = q_err.angle().to_degrees();
    let w = &state.plant.angular_velocity;
    let h = Vector3::new(state.aux[0], state.aux[1], state.aux[2]);
    CsvRow {
        t,
        angle_error_deg,
        omega_x: w.x,
        omega_y: w.y,
        omega_z: w.z,
        omega_mag: w.magnitude(),
        h_x: h.x,
        h_y: h.y,
        h_z: h.z,
        h_mag: h.magnitude(),
    }
}

fn write_csv(path: &std::path::Path, rows: &[CsvRow]) {
    let mut f = std::fs::File::create(path).expect("cannot create CSV");
    writeln!(
        f,
        "t,angle_error_deg,omega_x,omega_y,omega_z,omega_mag,h_x,h_y,h_z,h_mag"
    )
    .unwrap();
    for r in rows {
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{},{}",
            r.t,
            r.angle_error_deg,
            r.omega_x,
            r.omega_y,
            r.omega_z,
            r.omega_mag,
            r.h_x,
            r.h_y,
            r.h_z,
            r.h_mag
        )
        .unwrap();
    }
}
