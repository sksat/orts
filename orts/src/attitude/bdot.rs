use kaname::epoch::Epoch;
use kaname::magnetic::TiltedDipole;
use nalgebra::Vector3;

use crate::model::{HasAttitude, HasOrbit, Model};
use crate::spacecraft::ExternalLoads;

/// B-dot detumbling controller using stateless analytical approximation.
///
/// Estimates the time-derivative of the magnetic field in the body frame as
/// dB_body/dt ≈ −ω × B_body (valid when |ω| >> orbital angular rate),
/// then commands a magnetic moment m = −k · dB/dt to dissipate rotational
/// energy.
///
/// The resulting torque τ = m × B always opposes the component of angular
/// velocity perpendicular to the local magnetic field (provable via
/// Cauchy-Schwarz: ω · τ ≤ 0).
pub struct BdotDetumbler {
    /// Gain k > 0  [A·m²·s/(rad·T)]
    gain: f64,
    /// Per-axis maximum magnetic moment [A·m²]
    max_moment: Vector3<f64>,
    /// Geomagnetic field model
    field: TiltedDipole,
}

impl BdotDetumbler {
    /// Create a new B-dot detumbler with custom field model.
    ///
    /// # Panics
    /// Panics if `gain` is negative or any component of `max_moment` is negative.
    pub fn new(gain: f64, max_moment: Vector3<f64>, field: TiltedDipole) -> Self {
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

impl<S: HasAttitude + HasOrbit> Model<S> for BdotDetumbler {
    fn name(&self) -> &str {
        "bdot"
    }

    fn eval(&self, _t: f64, state: &S, _epoch: Option<&Epoch>) -> ExternalLoads {
        let att = state.attitude();
        let orbit = state.orbit();

        // 1. Compute B in ECI
        let b_eci = self.field.field_eci(orbit.position());

        // 2. Transform to body frame
        let r_bi = att.inertial_to_body();
        let b_body = r_bi * b_eci;

        // 3. Analytical approximation: dB_body/dt ≈ −ω × B_body
        //    (valid when |ω| >> orbital angular rate)
        let omega = &att.angular_velocity;
        let db_body_dt = -omega.cross(&b_body);

        // 4. Commanded magnetic moment: m = −k · dB/dt = k · (ω × B)
        let mut m_cmd = -self.gain * db_body_dt;

        // 5. Clamp per-axis
        for i in 0..3 {
            m_cmd[i] = m_cmd[i].clamp(-self.max_moment[i], self.max_moment[i]);
        }

        // 6. Torque: τ = m × B [N·m]
        let tau = m_cmd.cross(&b_body);

        ExternalLoads::torque(tau)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;

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
        let loads = ctrl.eval(0.0, &state, None);
        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Zero omega should give zero torque, got {:?}",
            loads.torque_body
        );
    }

    #[test]
    fn torque_opposes_omega_component() {
        // By Cauchy-Schwarz: ω · τ ≤ 0 (always)
        let ctrl = BdotDetumbler::new(1e4, Vector3::new(10.0, 10.0, 10.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.05),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let loads = ctrl.eval(0.0, &state, None);
        let dot = state.attitude.angular_velocity.dot(&loads.torque_body);
        assert!(
            dot <= 0.0,
            "ω · τ should be ≤ 0 (Cauchy-Schwarz), got {dot:.6e}"
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
        let loads = ctrl.eval(0.0, &state, None);
        assert!(loads.acceleration_inertial.magnitude() < 1e-15);
        assert!(loads.mass_rate.abs() < 1e-15);
    }

    #[test]
    fn moment_clamping() {
        // Use a very high gain so the unclamped moment would be huge
        let max_m = 0.001; // very small max moment
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
        let loads = ctrl.eval(0.0, &state, None);
        // Torque is bounded because moment is clamped: |τ| = |m × B| ≤ |m| * |B|
        // With clamped m, |m| ≤ sqrt(3) * max_m
        let b = TiltedDipole::earth()
            .field_eci(&Vector3::new(7000.0, 0.0, 0.0))
            .magnitude();
        let max_torque = 3.0_f64.sqrt() * max_m * b;
        assert!(
            loads.torque_body.magnitude() <= max_torque * 1.01,
            "Torque should be bounded by clamped moment: |τ|={:.6e}, bound={max_torque:.6e}",
            loads.torque_body.magnitude()
        );
    }
}
