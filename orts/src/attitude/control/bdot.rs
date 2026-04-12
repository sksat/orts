use arika::epoch::Epoch;
use arika::frame::{self, Vec3};
use nalgebra::Vector3;
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};

use crate::OrbitalState;
use crate::attitude::AttitudeState;
use crate::control::DiscreteController;
use crate::magnetic;
use crate::model::ExternalLoads;
use crate::model::{HasAttitude, HasOrbit, Model};

/// B-dot detumbling controller using stateless analytical approximation.
///
/// Estimates the time-derivative of the magnetic field in the body frame as
/// dB_body/dt = -omega x B_body (valid when |omega| >> orbital angular rate),
/// then commands a magnetic moment m = -k * dB/dt to dissipate rotational
/// energy.
///
/// The resulting torque tau = m x B always opposes the component of angular
/// velocity perpendicular to the local magnetic field (provable via
/// Cauchy-Schwarz: omega . tau <= 0).
///
/// When no epoch is available, returns zero loads (magnetic field models
/// require epoch for ECEF↔ECI rotation and secular variation).
pub struct BdotDetumbler<F: MagneticFieldModel = TiltedDipole> {
    /// Gain k > 0  [A*m^2*s/(rad*T)]
    gain: f64,
    /// Per-axis maximum magnetic moment [A*m^2]
    max_moment: Vector3<f64>,
    /// Geomagnetic field model
    field: F,
}

impl<F: MagneticFieldModel> BdotDetumbler<F> {
    /// Create a new B-dot detumbler with custom field model.
    ///
    /// # Panics
    /// Panics if `gain` is negative or any component of `max_moment` is negative.
    pub fn new(gain: f64, max_moment: Vector3<f64>, field: F) -> Self {
        assert!(gain >= 0.0, "gain must be non-negative, got {gain}");
        assert!(
            max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0,
            "max_moment must be non-negative, got {max_moment:?}"
        );
        Self {
            gain,
            max_moment,
            field,
        }
    }
}

impl<F: MagneticFieldModel, S: HasAttitude + HasOrbit> Model<S> for BdotDetumbler<F> {
    fn name(&self) -> &str {
        "bdot"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };

        let att = state.attitude();
        let orbit = state.orbit();

        // 1. Compute B in ECI (requires epoch for ECEF->ECI rotation)
        let b_eci = magnetic::field_eci(&self.field, &orbit.position_eci(), epoch).into_inner();
        if b_eci.magnitude() < 1e-30 {
            return ExternalLoads::zeros();
        }

        // 2. Transform to body frame
        let b_body = att
            .rotation_to_body()
            .transform(&Vec3::<frame::SimpleEci>::from_raw(b_eci))
            .into_inner();

        // 3. Analytical approximation: dB_body/dt = -omega x B_body
        //    (valid when |omega| >> orbital angular rate)
        let omega = &att.angular_velocity;
        let db_body_dt = -omega.cross(&b_body);

        // 4. Commanded magnetic moment: m = -k * dB/dt = k * (omega x B)
        let mut m_cmd = -self.gain * db_body_dt;

        // 5. Clamp per-axis
        for i in 0..3 {
            m_cmd[i] = m_cmd[i].clamp(-self.max_moment[i], self.max_moment[i]);
        }

        // 6. Torque: tau = m x B [N*m]
        let tau = m_cmd.cross(&b_body);

        ExternalLoads::torque(tau)
    }
}

/// Actuator model that applies a commanded magnetic moment as torque.
///
/// The `commanded_moment` is held constant (set externally between ODE segments).
/// Torque is computed as tau = m x B where B is the local geomagnetic field in the body frame.
///
/// When no epoch is available, returns zero loads.
pub struct CommandedMagnetorquer<F: MagneticFieldModel = TiltedDipole> {
    /// Current commanded magnetic moment \[A*m^2\] in body frame.
    pub commanded_moment: Vector3<f64>,
    /// Geomagnetic field model.
    field: F,
}

impl<F: MagneticFieldModel> CommandedMagnetorquer<F> {
    /// Create a new magnetorquer actuator model.
    pub fn new(commanded_moment: Vector3<f64>, field: F) -> Self {
        Self {
            commanded_moment,
            field,
        }
    }
}

impl<F: MagneticFieldModel, S: HasAttitude + HasOrbit> Model<S> for CommandedMagnetorquer<F> {
    fn name(&self) -> &str {
        "magnetorquer"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };
        let b_eci =
            magnetic::field_eci(&self.field, &state.orbit().position_eci(), epoch).into_inner();
        if b_eci.magnitude() < 1e-30 {
            return ExternalLoads::zeros();
        }
        let b_body = state
            .attitude()
            .rotation_to_body()
            .transform(&Vec3::<frame::SimpleEci>::from_raw(b_eci))
            .into_inner();
        ExternalLoads::torque(self.commanded_moment.cross(&b_body))
    }
}

/// B-dot controller using finite-difference dB/dt estimation.
///
/// Unlike [`BdotDetumbler`] which uses the analytical approximation
/// dB_body/dt = -omega x B_body, this controller measures the actual
/// magnetic field at each sample time and computes dB/dt via backward
/// finite difference. This is more realistic (flight software only sees
/// magnetometer readings) but introduces a one-sample delay and produces
/// zero command on the first call.
///
/// When no epoch is available, returns zero command.
pub struct BdotFiniteDiff<F: MagneticFieldModel = TiltedDipole> {
    gain: f64,
    max_moment: Vector3<f64>,
    field: F,
    sample_period: f64,
    prev_b_body: Option<Vector3<f64>>,
    prev_t: f64,
}

impl<F: MagneticFieldModel> BdotFiniteDiff<F> {
    /// Create a new finite-difference B-dot controller.
    ///
    /// # Panics
    /// Panics if `gain` is negative, any component of `max_moment` is negative,
    /// or `sample_period` is not positive.
    pub fn new(gain: f64, max_moment: Vector3<f64>, field: F, sample_period: f64) -> Self {
        assert!(gain >= 0.0, "gain must be non-negative, got {gain}");
        assert!(
            max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0,
            "max_moment must be non-negative, got {max_moment:?}"
        );
        assert!(
            sample_period > 0.0,
            "sample_period must be positive, got {sample_period}"
        );
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

impl<F: MagneticFieldModel> DiscreteController for BdotFiniteDiff<F> {
    type Command = Vector3<f64>;

    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn initial_command(&self) -> Vector3<f64> {
        Vector3::zeros()
    }

    fn update(
        &mut self,
        t: f64,
        attitude: &AttitudeState,
        orbit: &OrbitalState,
        epoch: Option<&Epoch>,
    ) -> Vector3<f64> {
        let Some(epoch) = epoch else {
            return Vector3::zeros();
        };
        let b_eci = magnetic::field_eci(&self.field, &orbit.position_eci(), epoch).into_inner();
        if b_eci.magnitude() < 1e-30 {
            return Vector3::zeros();
        }
        let b_body = attitude
            .rotation_to_body()
            .transform(&Vec3::<frame::SimpleEci>::from_raw(b_eci))
            .into_inner();

        let m_cmd = match self.prev_b_body {
            Some(prev_b) => {
                let dt = t - self.prev_t;
                if dt < 1e-15 {
                    return Vector3::zeros();
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
        self.prev_t = t;
        m_cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use arika::epoch::Epoch;
    use arika::frame::Vec3 as FrameVec3;
    use nalgebra::Vector4;

    fn test_epoch() -> Epoch {
        Epoch::j2000()
    }

    /// Combined state for testing (provides HasAttitude + HasOrbit).
    struct TestState {
        attitude: AttitudeState,
        orbit: OrbitalState,
    }

    impl HasAttitude for TestState {
        fn attitude(&self) -> &AttitudeState {
            &self.attitude
        }
    }

    impl HasOrbit for TestState {
        fn orbit(&self) -> &OrbitalState {
            &self.orbit
        }
    }

    #[test]
    fn zero_omega_gives_zero_torque() {
        let ctrl = BdotDetumbler::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Zero omega should give zero torque, got {:?}",
            loads.torque_body
        );
    }

    #[test]
    fn torque_opposes_omega_component() {
        let ctrl = BdotDetumbler::new(1e4, Vector3::new(10.0, 10.0, 10.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.05),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        let dot = state
            .attitude
            .angular_velocity
            .dot(&loads.torque_body.into_inner());
        assert!(
            dot <= 0.0,
            "omega . tau should be <= 0 (Cauchy-Schwarz), got {dot:.6e}"
        );
    }

    #[test]
    fn no_acceleration_or_mass_rate() {
        let ctrl = BdotDetumbler::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.0, 0.0),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        assert!(loads.acceleration_inertial.magnitude() < 1e-15);
        assert!(loads.mass_rate.abs() < 1e-15);
    }

    #[test]
    fn moment_clamping() {
        let max_m = 0.001;
        let ctrl = BdotDetumbler::new(
            1e10,
            Vector3::new(max_m, max_m, max_m),
            TiltedDipole::earth(),
        );
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 1.0, 1.0),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        let b = magnetic::field_eci(
            &TiltedDipole::earth(),
            &FrameVec3::<frame::SimpleEci>::new(7000.0, 0.0, 0.0),
            &epoch,
        )
        .magnitude();
        let max_torque = 3.0_f64.sqrt() * max_m * b;
        assert!(
            loads.torque_body.magnitude() <= max_torque * 1.01,
            "Torque should be bounded by clamped moment: |tau|={:.6e}, bound={max_torque:.6e}",
            loads.torque_body.magnitude()
        );
    }

    #[test]
    fn no_epoch_returns_zero_loads() {
        // Without epoch, magnetic field models cannot compute the field,
        // so the controller returns zero loads.
        let ctrl = BdotDetumbler::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.05),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let loads = ctrl.eval(0.0, &state, None);
        assert!(
            loads.torque_body.magnitude() < 1e-30,
            "Without epoch, should return zero loads"
        );
    }
}
