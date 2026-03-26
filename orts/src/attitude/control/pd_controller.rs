use kaname::epoch::Epoch;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};

use crate::model::ExternalLoads;
use crate::model::{HasAttitude, HasOrbit, Model};

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

impl<S: HasAttitude> Model<S> for InertialPdController {
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
pub struct TrackingPdController<R: AttitudeReference> {
    kp: Matrix3<f64>,
    kd: Matrix3<f64>,
    reference: R,
}

impl<R: AttitudeReference> TrackingPdController<R> {
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

impl<S: HasAttitude + HasOrbit, R: AttitudeReference + 'static> Model<S>
    for TrackingPdController<R>
{
    fn name(&self) -> &str {
        "pd_tracking"
    }

    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        let att = state.attitude();
        let (q_target, omega_target) = self.reference.target(t, state.orbit(), epoch);

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
            loads.torque_body[2] < 0.0,
            "Expected restoring torque, got {:?}",
            loads.torque_body
        );

        // Magnitude should be approximately kp * angle for small angles
        let expected_mag = kp * angle;
        let actual_mag = loads.torque_body[2].abs();
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
        let err = (loads.torque_body - expected).magnitude();
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
            loads.torque_body[2] > 0.0,
            "Expected positive torque (short path), got {:?}",
            loads.torque_body
        );

        // Magnitude should be small (~10° worth), not large (~350° worth)
        let short_angle = (2.0 * PI - angle).abs();
        assert!(
            loads.torque_body[2].abs() < short_angle * 2.0,
            "Torque magnitude too large for short path"
        );
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
