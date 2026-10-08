//! Torque from the spacecraft's residual magnetic dipole.

use std::sync::Arc;

use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::Epoch;
use arika::frame;
use nalgebra::Vector3;
use tobari::magnetic::MagneticFieldModel;

use crate::magnetic;
use crate::model::{ExternalLoads, HasAttitude, HasFrame, HasOrbit, Model};

/// Torque the geomagnetic field exerts on the spacecraft's residual dipole.
///
/// ```text
/// τ = m_res × B_body
/// ```
///
/// `m_res` [A·m², body frame] is the equivalent dipole of the spacecraft's own
/// magnetization (permanent magnets, magnetized structure, current loops in
/// the harness), as a magnetic-cleanliness test measures it. It is constant:
/// a dipole that drifts with temperature or follows the field (hysteresis) is
/// not modelled.
///
/// `B` is the geomagnetic field alone. The fields of the spacecraft's own
/// sources (the MTQs at a magnetometer, a residual field there) are internal
/// and exert no net torque on the spacecraft.
///
/// `Fr` is the inertial frame the field is evaluated in, as for
/// [`MtqAssembly`](crate::spacecraft::MtqAssembly).
pub struct ResidualDipoleTorque<Fr: EarthFixedTransform = frame::SimpleEci> {
    dipole_body: Vector3<f64>,
    field: Arc<dyn MagneticFieldModel>,
    eop: Fr::EopStorage,
}

impl ResidualDipoleTorque<frame::SimpleEci> {
    /// A residual dipole [A·m², body frame] in the default `SimpleEci` frame.
    ///
    /// # Panics
    /// Panics if a component of `dipole_body` is non-finite.
    pub fn new(dipole_body: Vector3<f64>, field: Arc<dyn MagneticFieldModel>) -> Self {
        Self::new_in_frame(dipole_body, field, ())
    }
}

impl<Fr: EarthFixedTransform> ResidualDipoleTorque<Fr> {
    /// A residual dipole evaluated in an arbitrary inertial frame `Fr`, with
    /// that frame's EOP storage (`()` for `SimpleEci`).
    ///
    /// # Panics
    /// Panics if a component of `dipole_body` is non-finite.
    pub fn new_in_frame(
        dipole_body: Vector3<f64>,
        field: Arc<dyn MagneticFieldModel>,
        eop: Fr::EopStorage,
    ) -> Self {
        assert!(
            dipole_body.iter().all(|v| v.is_finite()),
            "residual dipole must be finite, got {dipole_body:?}"
        );
        Self {
            dipole_body,
            field,
            eop,
        }
    }

    /// The residual dipole [A·m², body frame].
    pub fn dipole_body(&self) -> &Vector3<f64> {
        &self.dipole_body
    }
}

impl<Fr: EarthFixedTransform, S: HasFrame<Frame = Fr> + HasAttitude + HasOrbit> Model<S>
    for ResidualDipoleTorque<Fr>
{
    fn name(&self) -> &str {
        "residual_dipole"
    }

    /// Zero without an epoch: the field model needs the instant, the same
    /// contract as the MTQ assembly's.
    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<Fr> {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };
        let b_inertial = magnetic::field_inertial::<Fr>(
            self.field.as_ref(),
            &state.orbit().position_vec(),
            &EarthOrientation::new(*epoch, &self.eop),
        );
        let b_body = state
            .attitude_from_inertial()
            .transform(&b_inertial)
            .into_inner();
        ExternalLoads::torque(self.dipole_body.cross(&b_body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use crate::spacecraft::{MtqAssembly, MtqCommand};
    use tobari::magnetic::{NoField, TiltedDipole};

    struct TestState {
        attitude: AttitudeState,
        orbit: OrbitalState,
    }

    impl HasAttitude for TestState {
        fn attitude(&self) -> &AttitudeState {
            &self.attitude
        }
    }

    impl HasFrame for TestState {
        type Frame = frame::SimpleEci;
    }

    impl HasOrbit for TestState {
        fn orbit(&self) -> &OrbitalState<frame::SimpleEci> {
            &self.orbit
        }
    }

    fn state() -> TestState {
        TestState {
            attitude: AttitudeState::new(
                nalgebra::UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::zeros(),
            ),
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
        }
    }

    fn torque(model: &dyn Model<TestState>, epoch: Option<&Epoch>) -> Vector3<f64> {
        model.eval(0.0, &state(), epoch).torque_body.into_inner()
    }

    /// A residual dipole is a dipole that is always on: its torque is the one
    /// an MTQ assembly holding the same moment makes, bit for bit.
    #[test]
    fn torque_matches_an_mtq_holding_the_same_moment() {
        let m = Vector3::new(0.05, -0.02, 0.1);
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let residual = ResidualDipoleTorque::new(m, Arc::new(TiltedDipole::earth()));
        let mut mtq = MtqAssembly::three_axis(1.0, TiltedDipole::earth());
        mtq.command = MtqCommand::Moments(vec![m.x, m.y, m.z]);

        let got = torque(&residual, Some(&epoch));
        assert_eq!(got, torque(&mtq, Some(&epoch)));
        assert!(got.norm() > 0.0, "a LEO field acts on the dipole");
    }

    #[test]
    fn torque_is_zero_without_an_epoch() {
        let residual =
            ResidualDipoleTorque::new(Vector3::new(1.0, 0.0, 0.0), Arc::new(TiltedDipole::earth()));
        assert_eq!(torque(&residual, None), Vector3::zeros());
    }

    #[test]
    fn torque_is_zero_without_a_field() {
        let residual = ResidualDipoleTorque::new(Vector3::new(1.0, 0.0, 0.0), Arc::new(NoField));
        let epoch = Epoch::j2000();
        assert_eq!(torque(&residual, Some(&epoch)), Vector3::zeros());
    }

    #[test]
    fn the_model_adds_no_force_or_mass_rate() {
        let residual =
            ResidualDipoleTorque::new(Vector3::new(1.0, 0.0, 0.0), Arc::new(TiltedDipole::earth()));
        let loads = residual.eval(0.0, &state(), Some(&Epoch::j2000()));
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
        assert_eq!(loads.mass_rate, 0.0);
    }

    #[test]
    #[should_panic(expected = "residual dipole must be finite")]
    fn a_non_finite_dipole_is_rejected() {
        let _ = ResidualDipoleTorque::new(
            Vector3::new(f64::NAN, 0.0, 0.0),
            Arc::new(TiltedDipole::earth()),
        );
    }
}
