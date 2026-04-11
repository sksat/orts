//! Phase P0.5 oracle: bit-exact agreement between the pre-existing
//! native `BdotFiniteDiff` (used by `oracle_discrete_bdot.rs`) and an
//! independent reimplementation of the same finite-difference B-dot
//! control law written purely against the plugin layer
//! (`PluginController` / `TickInput` / `Command`).
//!
//! Both implementations share the same math, so the oracle is
//! bit-exact: the final spacecraft state after a fixed-horizon
//! simulation must agree to the last bit. A deliberate divergence
//! (different operator order, different clamp strategy, ...) requires
//! regenerating this oracle.
//!
//! Bit-exact agreement validates two things:
//!
//! 1. The plugin layer (`TickInput` / `Command` / `ActuatorBundle`)
//!    carries enough information end-to-end for a real stateful
//!    attitude controller.
//! 2. The plugin-layer reimplementation of `BdotFiniteDiff` is
//!    honest; when Phase P1 lands a WASM guest as a **third**
//!    implementation, this one becomes the trusted reference against
//!    which the guest is compared.

use kaname::earth::{MU as MU_EARTH, R as R_EARTH};
use kaname::epoch::Epoch;
use kaname::frame::{Body, Vec3};
use nalgebra::{Matrix3, Vector3, Vector4};
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};
use utsuroi::{Integrator, Rk4};

use orts::OrbitalState;
use orts::SpacecraftState;
use orts::attitude::{
    AttitudeState, BdotFiniteDiff as NativeBdot, CommandedMagnetorquer, DecoupledAttitudeSystem,
};
use orts::control::DiscreteController;
use orts::plugin::{
    ActuatorBundle, ActuatorState, Command, PluginController, PluginError, Sensors, TickInput,
};

const MASS: f64 = 50.0;
const ALT_KM: f64 = 500.0;
const SAMPLE_PERIOD: f64 = 1.0;
const ODE_DT: f64 = 0.1;
const T_END: f64 = 20.0;
const GAIN: f64 = 1e4;
const MAX_MOMENT: f64 = 10.0;

// =============================================================
// Plugin-layer reimplementation of BdotFiniteDiff.
//
// Not part of the orts library API. This is the independent second
// implementation of `orts::attitude::BdotFiniteDiff` used as the
// oracle target. Operator order matches the native implementation
// line-for-line so that a bit-exact assertion is legitimate.
// =============================================================

struct PluginBdotFiniteDiff<F: MagneticFieldModel = TiltedDipole> {
    gain: f64,
    max_moment: Vector3<f64>,
    field: F,
    sample_period: f64,
    prev_b_body: Option<Vector3<f64>>,
    prev_t: f64,
}

impl<F: MagneticFieldModel> PluginBdotFiniteDiff<F> {
    fn new(gain: f64, max_moment: Vector3<f64>, field: F, sample_period: f64) -> Self {
        assert!(gain >= 0.0);
        assert!(max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0);
        assert!(sample_period > 0.0);
        Self {
            gain,
            max_moment,
            field,
            sample_period,
            prev_b_body: None,
            prev_t: 0.0,
        }
    }
}

impl<F: MagneticFieldModel> PluginController for PluginBdotFiniteDiff<F> {
    fn name(&self) -> &str {
        "plugin::bdot_finite_diff"
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

        let m_cmd = match self.prev_b_body {
            Some(prev_b) => {
                let dt = obs.t - self.prev_t;
                if dt < 1e-15 {
                    return Ok(Some(Command::magnetic_moment(Vec3::zeros())));
                }
                let db_dt = (b_body - prev_b) / dt;
                let mut m = -self.gain * db_dt;
                for i in 0..3 {
                    m[i] = m[i].clamp(-self.max_moment[i], self.max_moment[i]);
                }
                m
            }
            None => Vector3::zeros(),
        };

        self.prev_b_body = Some(b_body);
        self.prev_t = obs.t;

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
        angular_velocity: Vector3::new(0.1, 0.05, -0.03),
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

/// Drive the native `BdotFiniteDiff` through `DiscreteController::update`.
fn run_native_path(initial: AttitudeState, epoch: Epoch) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let mut ctrl = NativeBdot::new(
        GAIN,
        Vector3::new(MAX_MOMENT, MAX_MOMENT, MAX_MOMENT),
        TiltedDipole::earth(),
        SAMPLE_PERIOD,
    );
    let mut state = initial;
    let mut cmd = ctrl.initial_command();
    let mut t = 0.0;

    while t < T_END - 1e-12 {
        let t_next = (t + SAMPLE_PERIOD).min(T_END);
        let actuator = CommandedMagnetorquer::new(cmd, TiltedDipole::earth());
        let system = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
            .with_model(actuator)
            .with_epoch(epoch);
        state = Rk4.integrate(&system, state, t, t_next, ODE_DT, |_, _| {});
        t = t_next;
        let orbit_at_t = circular_orbit_at(t, mu, radius);
        let current_epoch = epoch.add_seconds(t);
        cmd = ctrl.update(t, &state, &orbit_at_t, Some(&current_epoch));
    }

    state
}

/// Drive the plugin-layer `PluginBdotFiniteDiff` through
/// `PluginController::update(&TickInput)` with `ActuatorBundle` in
/// between.
fn run_plugin_path(initial: AttitudeState, epoch: Epoch) -> AttitudeState {
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let mut ctrl = PluginBdotFiniteDiff::new(
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
fn plugin_bdot_finitediff_matches_native_bitwise() {
    let epoch = Epoch::j2000();
    let initial = initial_attitude();

    let native_state = run_native_path(initial.clone(), epoch);
    let plugin_state = run_plugin_path(initial, epoch);

    // Bit-exact comparison. Both paths transcribe the same control-law
    // formula with the same operator order: `b_eci -> b_body`, `dt`
    // guard, `(b_body - prev_b) / dt`, `-gain * db_dt`, clamp loop in
    // axis order 0..3, then the same ZOH + RK4 integrator. No floating
    // point reordering happens between them, so the result must agree
    // to the last f64 bit. If this ever flakes, re-audit both
    // implementations for reordered multiplies or a diverged clamp
    // strategy BEFORE relaxing to `assert_relative_eq!`: the whole
    // point of this oracle is that the plugin layer does not silently
    // drop precision.
    assert_eq!(
        native_state.quaternion, plugin_state.quaternion,
        "quaternion mismatch between native and plugin BdotFiniteDiff"
    );
    assert_eq!(
        native_state.angular_velocity, plugin_state.angular_velocity,
        "angular velocity mismatch between native and plugin BdotFiniteDiff"
    );
}

#[test]
fn plugin_bdot_finitediff_is_not_trivially_zero() {
    // Sanity: the plugin path is actually exercising its
    // finite-difference branch (not trivially returning zero forever).
    let epoch = Epoch::j2000();
    let mu = MU_EARTH;
    let radius = R_EARTH + ALT_KM;

    let mut ctrl = PluginBdotFiniteDiff::new(
        GAIN,
        Vector3::new(MAX_MOMENT, MAX_MOMENT, MAX_MOMENT),
        TiltedDipole::earth(),
        SAMPLE_PERIOD,
    );
    let mut bundle = ActuatorBundle::new();

    let sensors = Sensors::empty();
    let actuator_state = ActuatorState::default();
    let initial = initial_attitude();

    let actuator =
        CommandedMagnetorquer::new(bundle.magnetic_moment().into_inner(), TiltedDipole::earth());
    let system = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
        .with_model(actuator)
        .with_epoch(epoch);
    let state1 = Rk4.integrate(&system, initial, 0.0, SAMPLE_PERIOD, ODE_DT, |_, _| {});
    let orbit1 = circular_orbit_at(SAMPLE_PERIOD, mu, radius);
    let epoch1 = epoch.add_seconds(SAMPLE_PERIOD);
    let snapshot1 = SpacecraftState {
        orbit: orbit1,
        attitude: state1.clone(),
        mass: MASS,
    };
    let obs1 = TickInput {
        t: SAMPLE_PERIOD,
        spacecraft: &snapshot1,
        epoch: Some(&epoch1),
        sensors: &sensors,
        actuators: &actuator_state,
    };
    let cmd1 = ctrl.update(&obs1).unwrap().expect("must return Some");
    bundle.apply(&cmd1).unwrap();
    // First tick: no finite-difference history => zero command.
    assert_eq!(cmd1.magnetic_moment, Some(Vec3::<Body>::zeros()));

    let actuator2 =
        CommandedMagnetorquer::new(bundle.magnetic_moment().into_inner(), TiltedDipole::earth());
    let system2 = DecoupledAttitudeSystem::circular_orbit(inertia(), mu, radius, MASS)
        .with_model(actuator2)
        .with_epoch(epoch);
    let state2 = Rk4.integrate(
        &system2,
        state1,
        SAMPLE_PERIOD,
        2.0 * SAMPLE_PERIOD,
        ODE_DT,
        |_, _| {},
    );
    let orbit2 = circular_orbit_at(2.0 * SAMPLE_PERIOD, mu, radius);
    let epoch2 = epoch.add_seconds(2.0 * SAMPLE_PERIOD);
    let snapshot2 = SpacecraftState {
        orbit: orbit2,
        attitude: state2,
        mass: MASS,
    };
    let obs2 = TickInput {
        t: 2.0 * SAMPLE_PERIOD,
        spacecraft: &snapshot2,
        epoch: Some(&epoch2),
        sensors: &sensors,
        actuators: &actuator_state,
    };
    let cmd2 = ctrl.update(&obs2).unwrap().expect("must return Some");
    let m = cmd2
        .magnetic_moment
        .expect("controller must emit a magnetic moment command");
    assert!(
        m.magnitude() > 0.0,
        "second plugin-path update should produce non-zero moment, got {m:?}"
    );
}
