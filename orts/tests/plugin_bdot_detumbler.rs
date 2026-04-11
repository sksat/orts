//! Phase P0.5 sanity: plugin-layer B-dot **detumbling** implementation
//! that uses the rate-gyro (`angular_velocity`) reading from
//! `TickInput::spacecraft.attitude`.
//!
//! Unlike the finite-difference variant tested in
//! `plugin_bdot_finitediff.rs`, this controller is **stateless** and
//! computes the commanded magnetic moment analytically:
//!
//!     m = -k * (omega x B_body)
//!
//! clamped component-wise to `+-max_moment`. This exercises a
//! different TickInput path -- the plugin controller reads
//! `obs.spacecraft.attitude.angular_velocity` directly, validating
//! that the plugin layer can deliver rate-gyro information to guest
//! controllers. Phase P1 WASM guests will use the same field.
//!
//! The pre-existing native equivalent `orts::attitude::BdotDetumbler`
//! is a `Model<S>` evaluated inside the ODE RHS at every integrator
//! step, while this plugin variant is a sample-tick + ZOH
//! `PluginController`. The two are structurally different (discrete
//! vs continuous control), so a bit-exact oracle between them is not
//! meaningful. Instead this test checks that:
//!
//! 1. Every command returned by the plugin controller is finite.
//! 2. The simulation does not diverge numerically.
//! 3. Detumbling actually happens -- the final angular-velocity
//!    magnitude is strictly smaller than the initial one, by a
//!    margin that would not be reached without a working controller.

use kaname::earth::{MU as MU_EARTH, R as R_EARTH};
use kaname::epoch::Epoch;
use kaname::frame::{Body, Vec3};
use nalgebra::{Matrix3, Vector3, Vector4};
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};
use utsuroi::{Integrator, Rk4};

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{AttitudeState, CommandedMagnetorquer, DecoupledAttitudeSystem};
use orts::plugin::{
    ActuatorBundle, ActuatorState, Command, PluginController, PluginError, Sensors, TickInput,
};

const MASS: f64 = 50.0;
const ALT_KM: f64 = 500.0;
const SAMPLE_PERIOD: f64 = 0.5;
const ODE_DT: f64 = 0.1;
const T_END: f64 = 600.0;
// B-dot detumbling time constant is `tau ~ 4*I / (k * |B|^2)`. With a
// 1 kg*m^2 inertia and a TiltedDipole-level field (`|B| ~ 2e-5 T` in
// LEO), a `k = 5e4` gain gives `tau ~ 2*10^5 s` (~55 h), which only
// damps a few percent over 600 s of wall-clock simulation. We bump
// `k` by two orders of magnitude to get a ~2000 s time constant so
// that the 600 s test window sees a measurable reduction -- this is
// not a physically realistic magnetorquer gain, just a test knob.
const GAIN: f64 = 5e6;
// Max commanded moment per axis. With `GAIN = 5e6`, `|omega| ~ 0.1`, and
// `|B| ~ 2e-5 T`, the unclamped command magnitude is roughly
// `k * |omega| * |B| ~ 10 A*m^2`. A real CubeSat magnetorquer tops out at
// 1-3 A*m^2, but for the test we pick 50 to keep clamping rare so
// that the observed damping reflects the control law rather than
// saturation behaviour.
const MAX_MOMENT: f64 = 50.0;
const INITIAL_OMEGA: [f64; 3] = [0.08, 0.05, -0.04];
// Expect at least 10% damping over the 600 s run. A broken
// controller (always zero command) leaves |omega| essentially unchanged
// because gravity gradient on a near-spherical inertia dissipates
// far less than 1% over this horizon.
const DAMPING_FLOOR: f64 = 0.10;

// =============================================================
// Plugin-layer Detumbler (analytic, stateless)
//
// m = -k * (omega x B_body), clamped per-axis.
// =============================================================

struct PluginBdotDetumbler<F: MagneticFieldModel = TiltedDipole> {
    gain: f64,
    max_moment: Vector3<f64>,
    field: F,
    sample_period: f64,
}

impl<F: MagneticFieldModel> PluginBdotDetumbler<F> {
    fn new(gain: f64, max_moment: Vector3<f64>, field: F, sample_period: f64) -> Self {
        assert!(gain >= 0.0);
        assert!(max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0);
        assert!(sample_period > 0.0);
        Self {
            gain,
            max_moment,
            field,
            sample_period,
        }
    }
}

impl<F: MagneticFieldModel> PluginController for PluginBdotDetumbler<F> {
    fn name(&self) -> &str {
        "plugin::bdot_detumbler"
    }
    fn sample_period(&self) -> f64 {
        self.sample_period
    }
    fn update(&mut self, obs: &TickInput<'_>) -> Result<Option<Command>, PluginError> {
        let Some(epoch) = obs.epoch else {
            return Ok(Some(Command::magnetic_moment(Vec3::zeros())));
        };
        let b_eci = self
            .field
            .field_eci(&obs.spacecraft.orbit.position_eci(), epoch)
            .into_inner();
        if b_eci.magnitude() < 1e-30 {
            return Ok(Some(Command::magnetic_moment(Vec3::zeros())));
        }
        let b_body = obs
            .spacecraft
            .attitude
            .rotation_to_body()
            .transform(&kaname::frame::Vec3::from_raw(b_eci))
            .into_inner();
        // Read the rate-gyro measurement directly from the observation.
        // This is the entire point of the detumbler variant: it
        // exercises the TickInput -> plugin path for angular velocity.
        let omega = &obs.spacecraft.attitude.angular_velocity;
        let db_body_dt = -omega.cross(&b_body);
        let mut m_cmd = -self.gain * db_body_dt;
        for i in 0..3 {
            m_cmd[i] = m_cmd[i].clamp(-self.max_moment[i], self.max_moment[i]);
        }
        let cmd = Command::magnetic_moment(Vec3::from_raw(m_cmd));
        if !cmd.is_finite() {
            return Err(PluginError::BadCommand(format!("{cmd:?}")));
        }
        Ok(Some(cmd))
    }
}

// =============================================================
// Simulation harness
// =============================================================

fn inertia() -> Matrix3<f64> {
    Matrix3::from_diagonal(&Vector3::new(1.0, 1.0, 1.0))
}

fn initial_attitude() -> AttitudeState {
    AttitudeState {
        quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
        angular_velocity: Vector3::new(INITIAL_OMEGA[0], INITIAL_OMEGA[1], INITIAL_OMEGA[2]),
    }
}

fn circular_orbit_at(t: f64, mu: f64, radius: f64) -> OrbitalState {
    let n = (mu / radius.powi(3)).sqrt();
    let v = (mu / radius).sqrt();
    let theta = n * t;
    OrbitalState::new(
        Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
        Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
    )
}

fn run(initial: AttitudeState, epoch: Epoch) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let mut ctrl = PluginBdotDetumbler::new(
        GAIN,
        Vector3::new(MAX_MOMENT, MAX_MOMENT, MAX_MOMENT),
        TiltedDipole::earth(),
        SAMPLE_PERIOD,
    );
    let mut bundle = ActuatorBundle::new();

    let sensors = Sensors::empty();
    let actuator_state = ActuatorState::default();
    let mut state = initial;
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(
            bundle.magnetic_moment().into_inner(),
            TiltedDipole::earth(),
        );
        let system = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(actuator)
            .with_epoch(epoch);
        state = Rk4.integrate(&system, state, t, t_next, ODE_DT, |_, _| {});

        // Guard against numerical divergence mid-run.
        assert!(
            state.angular_velocity.iter().all(|x| x.is_finite())
                && state.quaternion.iter().all(|x| x.is_finite()),
            "simulation diverged at t = {t_next}"
        );

        t = t_next;
        let orbit_at_t = circular_orbit_at(t, mu, radius);
        let current_epoch = epoch.add_seconds(t);
        let snapshot = SpacecraftState {
            orbit: orbit_at_t,
            attitude: state.clone(),
            mass: MASS,
        };
        let obs = TickInput {
            t,
            spacecraft: &snapshot,
            epoch: Some(&current_epoch),
            sensors: &sensors,
            actuators: &actuator_state,
        };
        let cmd = ctrl
            .update(&obs)
            .expect("plugin controller must return a valid command")
            .expect("plugin controller must return Some command");
        bundle.apply(&cmd).expect("plugin command must be finite");
    }

    state
}

#[test]
fn plugin_bdot_detumbler_reduces_angular_velocity() {
    let epoch = Epoch::j2000();
    let initial = initial_attitude();
    let initial_magnitude = initial.angular_velocity.magnitude();

    let final_state = run(initial, epoch);
    let final_magnitude = final_state.angular_velocity.magnitude();

    // Basic finiteness of the output state.
    assert!(final_state.quaternion.iter().all(|x| x.is_finite()));
    assert!(final_state.angular_velocity.iter().all(|x| x.is_finite()));

    // Detumbling must actually happen: the final angular velocity
    // magnitude must be noticeably smaller than the initial one.
    let reduction = (initial_magnitude - final_magnitude) / initial_magnitude;
    assert!(
        reduction > DAMPING_FLOOR,
        "expected >{:.0}% damping, got {:.2}% \
         (|omega_initial|={initial_magnitude:.6}, |omega_final|={final_magnitude:.6})",
        DAMPING_FLOOR * 100.0,
        reduction * 100.0,
    );
}

#[test]
fn plugin_bdot_detumbler_uses_angular_velocity_from_observation() {
    // Sanity check: when we pass in zero angular velocity, the
    // stateless detumbler must return a zero command. This is what
    // distinguishes it from the finite-difference variant -- the
    // detumbler depends *only* on the rate-gyro reading.
    let mut ctrl = PluginBdotDetumbler::new(
        GAIN,
        Vector3::new(MAX_MOMENT, MAX_MOMENT, MAX_MOMENT),
        TiltedDipole::earth(),
        SAMPLE_PERIOD,
    );
    let epoch = Epoch::j2000();
    let sensors = Sensors::empty();
    let actuator_state = ActuatorState::default();
    let spacecraft = SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(), // <-- key: no tumbling
        },
        mass: MASS,
    };
    let obs = TickInput {
        t: 0.0,
        spacecraft: &spacecraft,
        epoch: Some(&epoch),
        sensors: &sensors,
        actuators: &actuator_state,
    };
    let cmd = ctrl.update(&obs).unwrap().expect("must return Some");
    assert_eq!(cmd.magnetic_moment, Some(Vec3::<Body>::zeros()));
}
