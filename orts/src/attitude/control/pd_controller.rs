use arika::epoch::Epoch;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};

use crate::model::ExternalLoads;
use crate::model::{HasAttitude, HasFrame, HasOrbit, Model};

use super::reference::AttitudeReference;

/// PD controller for inertial pointing (hold a fixed orientation).
///
/// Computes body-frame torque using quaternion error feedback:
/// - Proportional: τ_p = -Kp · θ_err (where θ_err ≈ 2 * q_err.imag for small angles)
/// - Derivative: τ_d = -Kd · ω
///
/// The quaternion error uses left-invariant convention: q_err = q_target⁻¹ * q_current,
/// which gives the error in the body frame. This ensures correct behavior for any
/// target orientation, not just identity.
/// Hemisphere selection (shortest path) is applied by negating q_err when w < 0.
pub struct InertialPdController {
    kp: Matrix3<f64>,
    kd: Matrix3<f64>,
    target_q: UnitQuaternion<f64>,
}

impl InertialPdController {
    /// Create a new inertial PD controller with gain matrices and target orientation.
    pub fn new(kp: Matrix3<f64>, kd: Matrix3<f64>, target_q: UnitQuaternion<f64>) -> Self {
        Self { kp, kd, target_q }
    }

    /// Convenience constructor for diagonal (isotropic) gains.
    pub fn diagonal(kp: f64, kd: f64, target_q: UnitQuaternion<f64>) -> Self {
        Self::new(
            Matrix3::from_diagonal(&Vector3::new(kp, kp, kp)),
            Matrix3::from_diagonal(&Vector3::new(kd, kd, kd)),
            target_q,
        )
    }
}

impl<S: HasFrame<Frame = arika::frame::SimpleEci> + HasAttitude> Model<S> for InertialPdController {
    fn name(&self) -> &str {
        "pd_inertial"
    }

    fn eval(&self, _t: f64, state: &S, _epoch: Option<&Epoch>) -> ExternalLoads {
        let att = state.attitude();

        // Left-invariant error: q_err = q_target^{-1} * q_current
        // This gives the error in the **body frame**: if q_current = q_target * q_perturb,
        // then q_err = q_perturb, and 2*q_err.vec is the body-frame error axis.
        let mut q_err = self.target_q.inverse() * att.orientation();

        // Hemisphere selection (shortest path)
        if q_err.w < 0.0 {
            q_err = UnitQuaternion::new_unchecked(-q_err.into_inner());
        }

        // Body-frame error: θ ≈ 2 * q_err_vec [rad]
        let q_vec = q_err.as_ref().vector();
        let theta_error = 2.0 * Vector3::new(q_vec[0], q_vec[1], q_vec[2]);

        let tau = -self.kp * theta_error - self.kd * att.angular_velocity;
        ExternalLoads::torque(tau)
    }
}

/// PD controller for tracking a time-varying attitude reference.
///
/// Uses the same left-invariant quaternion error as [`InertialPdController`],
/// but additionally compensates for the reference angular velocity:
/// - ω_error = ω_body - q_err⁻¹ · ω_target
///
/// where q_err = q_target⁻¹ * q_current maps current body to target body frame.
///
/// The reference is generic over the inertial frame: the controller is a
/// `Model<S, F>` for every frame `F` its reference supports (see
/// [`AttitudeReference`]), so no frame bound is imposed on the struct itself.
pub struct TrackingPdController<R> {
    kp: Matrix3<f64>,
    kd: Matrix3<f64>,
    reference: R,
}

impl<R> TrackingPdController<R> {
    /// Create a new tracking PD controller with gain matrices and reference.
    pub fn new(kp: Matrix3<f64>, kd: Matrix3<f64>, reference: R) -> Self {
        Self { kp, kd, reference }
    }

    /// Convenience constructor for diagonal (isotropic) gains.
    pub fn diagonal(kp: f64, kd: f64, reference: R) -> Self {
        Self::new(
            Matrix3::from_diagonal(&Vector3::new(kp, kp, kp)),
            Matrix3::from_diagonal(&Vector3::new(kd, kd, kd)),
            reference,
        )
    }
}

// Frame-generic: the reference is asked for its target in the state's own
// inertial frame `F` (see #151). The torque itself is body-frame, so the
// returned `ExternalLoads<F>` carries a zero acceleration in that same frame.
impl<
    F: arika::frame::Eci,
    S: HasFrame<Frame = F> + HasAttitude + HasOrbit,
    R: AttitudeReference<F> + 'static,
> Model<S, F> for TrackingPdController<R>
{
    fn name(&self) -> &str {
        "pd_tracking"
    }

    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        let att = state.attitude();
        let (q_target, omega_target) = self.reference.target(t, state.orbit(), epoch);
        // The target's frame tag has done its job: it had to match the frame
        // of the state's *orbit* (`HasFrame<Frame = F> + HasOrbit`) for this call to
        // type-check. `q_current` below is still an untyped `AttitudeState`
        // quaternion, so that is as far as the check reaches today. Everything
        // below is body-frame error algebra, so the tag is dropped here.
        let q_target = q_target.into_inner();

        let q_current = att.orientation();

        // Left-invariant error: q_err = q_target^{-1} * q_current
        // This gives the attitude error in the **body frame**.
        let mut q_err = q_target.inverse() * q_current;

        // Hemisphere selection (shortest path)
        if q_err.w < 0.0 {
            q_err = UnitQuaternion::new_unchecked(-q_err.into_inner());
        }

        // Body-frame error: θ ≈ 2 * q_err_vec [rad]
        let q_vec = q_err.as_ref().vector();
        let theta_error = 2.0 * Vector3::new(q_vec[0], q_vec[1], q_vec[2]);

        // omega_target is in the target body frame.
        // For the body-frame rate error, we need omega_target in the current body frame.
        // The left-invariant error q_err = q_target^{-1} * q_current represents the rotation
        // from target to current body frame. So:
        //   omega_target_in_body = q_err^{-1} * omega_target
        // Wait: q_err takes vectors from target to current? Let's verify.
        // q_err = q_target^{-1} * q_current. If we apply q_err to a vector v:
        //   q_err * v = (q_target^{-1} * q_current) * v
        //   = q_target^{-1} * (q_current * v)
        //   q_current * v takes v from body_current to inertial
        //   q_target^{-1} takes from inertial to body_target
        //   So q_err * v_body_current = v_body_target
        //
        // Therefore q_err maps current-body → target-body.
        // Its inverse maps target-body → current-body.
        let omega_target_body = q_err.inverse() * omega_target;
        let omega_error = att.angular_velocity - omega_target_body;

        let tau = -self.kp * theta_error - self.kd * omega_error;
        ExternalLoads::torque(tau)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;
    use std::f64::consts::PI;

    #[test]
    fn inertial_pd_zero_torque_at_target() {
        let target_q = UnitQuaternion::identity();
        let ctrl = InertialPdController::diagonal(1.0, 2.0, target_q);

        let state = AttitudeState::identity();
        let loads = ctrl.eval(0.0, &state, None);
        assert!(
            loads.torque_body.magnitude() < 1e-15,
            "Expected zero torque at target, got {:?}",
            loads.torque_body
        );
    }

    #[test]
    fn inertial_pd_restoring_torque() {
        let target_q = UnitQuaternion::identity();
        let kp = 1.0;
        let ctrl = InertialPdController::diagonal(kp, 0.0, target_q);

        // Rotate 10° about Z
        let angle = 10.0_f64.to_radians();
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, angle);
        let state = AttitudeState::new(uq, Vector3::zeros());

        let loads = ctrl.eval(0.0, &state, None);
        // Torque should be negative about Z (restoring)
        assert!(
            loads.torque_body.z() < 0.0,
            "Expected restoring torque, got {:?}",
            loads.torque_body
        );

        // Magnitude should be approximately kp * angle for small angles
        let expected_mag = kp * angle;
        let actual_mag = loads.torque_body.z().abs();
        let rel_err = ((actual_mag - expected_mag) / expected_mag).abs();
        assert!(
            rel_err < 0.01,
            "Expected torque ~{expected_mag:.4}, got {actual_mag:.4} (err {rel_err:.2e})"
        );
    }

    #[test]
    fn inertial_pd_damping_torque() {
        let target_q = UnitQuaternion::identity();
        let kd = 2.0;
        let ctrl = InertialPdController::diagonal(0.0, kd, target_q);

        let omega = Vector3::new(0.1, 0.0, 0.0);
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: omega,
        };

        let loads = ctrl.eval(0.0, &state, None);
        // Damping torque = -kd * omega
        let expected = -kd * omega;
        let err = (loads.torque_body.into_inner() - expected).magnitude();
        assert!(
            err < 1e-14,
            "Expected damping torque {expected:?}, got {:?}",
            loads.torque_body
        );
    }

    #[test]
    fn inertial_pd_hemisphere_selection() {
        // Rotate by 350° about Z (equivalent to -10°, should use short path)
        let angle = 350.0_f64.to_radians();
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, angle);

        let target_q = UnitQuaternion::identity();
        let ctrl = InertialPdController::diagonal(1.0, 0.0, target_q);
        let state = AttitudeState::new(uq, Vector3::zeros());

        let loads = ctrl.eval(0.0, &state, None);
        // Should produce torque for the short path (+10°), not the long path (-350°)
        // Short path would give positive torque about Z (rotating back +10°)
        assert!(
            loads.torque_body.z() > 0.0,
            "Expected positive torque (short path), got {:?}",
            loads.torque_body
        );

        // Magnitude should be small (~10° worth), not large (~350° worth)
        let short_angle = (2.0 * PI - angle).abs();
        assert!(
            loads.torque_body.z().abs() < short_angle * 2.0,
            "Torque magnitude too large for short path"
        );
    }

    // Frame-generalization characterization (#151)

    use crate::OrbitalState;
    use crate::attitude::control::NadirPointing;

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
        type Frame = arika::frame::SimpleEci;
    }

    impl HasOrbit for TestState {
        fn orbit(&self) -> &OrbitalState {
            &self.orbit
        }
    }

    fn snapshot_state() -> TestState {
        TestState {
            attitude: AttitudeState::new(
                UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
        }
    }

    /// Characterization: pinned `SimpleEci` tracking torque, so parameterizing
    /// [`AttitudeReference`] by the inertial frame cannot change it.
    #[test]
    fn tracking_pd_simple_eci_torque_snapshot() {
        let ctrl = TrackingPdController::diagonal(1.0, 2.0, NadirPointing);
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let got = ctrl
            .eval(0.0, &snapshot_state(), Some(&epoch))
            .torque_body
            .into_inner();
        let expected = Vector3::new(
            -1.4373171017803277,
            -0.8407481028427832,
            -0.37237905200853855,
        );
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "SimpleEci tracking PD torque changed: {got:?}"
        );
    }

    // Characterization for the frame-typed attitude target (#332)

    use crate::attitude::control::InertialPointing;
    use arika::frame::{Body, Rotation};

    /// A target orientation with all four components distinct and non-zero.
    fn nontrivial_target() -> UnitQuaternion<f64> {
        UnitQuaternion::from_axis_angle(
            &nalgebra::Unit::new_normalize(Vector3::new(-0.2, 0.7, 0.4)),
            1.1,
        )
    }

    fn state_with(q: UnitQuaternion<f64>, omega: Vector3<f64>) -> TestState {
        TestState {
            attitude: AttitudeState::new(q, omega),
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
        }
    }

    /// Characterization: pinned `SimpleEci` tracking torque against an
    /// `InertialPointing` reference, so typing the target by its inertial frame
    /// cannot change it.
    #[test]
    fn tracking_pd_inertial_pointing_torque_snapshot() {
        let ctrl = TrackingPdController::diagonal(
            1.0,
            2.0,
            InertialPointing {
                target_q: Rotation::<Body, arika::frame::SimpleEci>::from_raw(nontrivial_target()),
            },
        );
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let state = state_with(
            UnitQuaternion::from_axis_angle(
                &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                0.7,
            ),
            Vector3::new(0.01, -0.02, 0.03),
        );
        let got = ctrl
            .eval(0.0, &state, Some(&epoch))
            .torque_body
            .into_inner();
        let expected = Vector3::new(-0.10232165152129191, 1.2848812683428932, -0.107551192649679);
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude(),
            "InertialPointing tracking PD torque changed: {got:?}"
        );
    }

    /// Characterization of the hemisphere-selection predicate (`q_err.w < 0`)
    /// on the tracking path: the current attitude is nearly the negation of the
    /// target, so the branch fires and the torque must follow the short path.
    #[test]
    fn tracking_pd_hemisphere_selection_snapshot() {
        let target_q = nontrivial_target();
        // 1.1 rad + 1.9π about the same axis: same axis, opposite hemisphere.
        let current = UnitQuaternion::from_axis_angle(
            &nalgebra::Unit::new_normalize(Vector3::new(-0.2, 0.7, 0.4)),
            1.1 + PI * 1.9,
        );
        let state = state_with(current, Vector3::new(0.01, -0.02, 0.03));

        // The branch under test: q_err.w is genuinely negative here.
        let q_err = target_q.inverse() * state.attitude.orientation();
        assert!(
            q_err.w < -0.5,
            "fixture must exercise the hemisphere flip, got q_err.w = {}",
            q_err.w
        );

        let ctrl = TrackingPdController::diagonal(
            1.0,
            2.0,
            InertialPointing {
                target_q: Rotation::<Body, arika::frame::SimpleEci>::from_raw(target_q),
            },
        );
        let got = ctrl.eval(0.0, &state, None).torque_body.into_inner();
        let expected = Vector3::new(
            -0.09532998610353678,
            0.30365495136237847,
            0.09065997220707345,
        );
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude(),
            "hemisphere-selected tracking PD torque changed: {got:?}"
        );
        // Short path: without the flip the proportional term would point the
        // other way (θ_err ≈ 2·q_err_vec with q_err.w < 0).
        let unflipped_theta = 2.0 * Vector3::new(q_err.i, q_err.j, q_err.k);
        assert!(
            got.dot(&unflipped_theta) > 0.0,
            "expected the flipped (short-path) sign, got {got:?}"
        );
    }

    /// Characterization: a non-finite attitude reaches the hemisphere predicate
    /// (`NaN < 0.0` is `false`, so no flip) and yields a non-finite torque
    /// instead of panicking. Pins `NaN` and `±∞` for both references.
    #[test]
    fn tracking_pd_non_finite_attitude_yields_non_finite_torque() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let state = TestState {
                attitude: AttitudeState {
                    quaternion: Vector4::new(bad, 0.0, 0.0, 1.0),
                    angular_velocity: Vector3::new(0.01, 0.0, 0.0),
                },
                orbit: OrbitalState::new(
                    Vector3::new(4000.0, -5000.0, 2500.0),
                    Vector3::new(1.0, 2.0, 7.0),
                ),
            };

            let inertial = TrackingPdController::diagonal(
                1.0,
                2.0,
                InertialPointing {
                    target_q: Rotation::<Body, arika::frame::SimpleEci>::from_raw(
                        nontrivial_target(),
                    ),
                },
            );
            // The claim is that a non-finite state produces a non-finite
            // command rather than a plausible-looking one. Whether a given
            // component lands on NaN or ±inf depends on the quaternion
            // normalization inside, which is not this controller's contract.
            let tau = inertial.eval(0.0, &state, None).torque_body.into_inner();
            assert!(
                tau.iter().any(|c| !c.is_finite()),
                "expected a non-finite torque for {bad}, got {tau:?}"
            );

            let nadir = TrackingPdController::diagonal(1.0, 2.0, NadirPointing);
            let tau = nadir.eval(0.0, &state, None).torque_body.into_inner();
            assert!(
                tau.iter().any(|c| !c.is_finite()),
                "expected a non-finite nadir torque for {bad}, got {tau:?}"
            );
        }
    }

    #[test]
    fn inertial_pd_no_acceleration_or_mass_rate() {
        let ctrl = InertialPdController::diagonal(1.0, 1.0, UnitQuaternion::identity());
        let state = AttitudeState::new(
            UnitQuaternion::from_axis_angle(&nalgebra::Unit::new_normalize(Vector3::x()), 0.1),
            Vector3::new(0.01, 0.02, 0.03),
        );
        let loads = ctrl.eval(0.0, &state, None);
        assert!(loads.acceleration_inertial.magnitude() < 1e-15);
        assert!(loads.mass_rate.abs() < 1e-15);
    }
}
