use arika::frame::{self, Rotation};
use nalgebra::{UnitQuaternion, Vector3, Vector4};
use utsuroi::{OdeState, Tolerances};

use crate::model::HasAttitude;

/// Attitude state: unit quaternion (orientation) + angular velocity in body frame.
///
/// The quaternion is stored as `[w, x, y, z]` (Hamilton scalar-first convention).
/// During integration, the quaternion may deviate slightly from unit norm;
/// [`OdeState::project`] renormalizes it after each step.
#[derive(Debug, Clone, PartialEq)]
pub struct AttitudeState {
    /// Orientation quaternion `[w, x, y, z]` (body-to-inertial rotation).
    pub quaternion: Vector4<f64>,
    /// Angular velocity in body frame `[rad/s]`.
    pub angular_velocity: Vector3<f64>,
}

impl AttitudeState {
    /// Create from a nalgebra `UnitQuaternion` and angular velocity.
    pub fn new(orientation: UnitQuaternion<f64>, angular_velocity: Vector3<f64>) -> Self {
        Self {
            quaternion: Vector4::new(orientation.w, orientation.i, orientation.j, orientation.k),
            angular_velocity,
        }
    }

    /// Identity orientation with zero angular velocity.
    pub fn identity() -> Self {
        Self {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        }
    }

    /// Get the orientation as a nalgebra `UnitQuaternion`.
    pub fn orientation(&self) -> UnitQuaternion<f64> {
        let q = nalgebra::Quaternion::new(
            self.quaternion[0],
            self.quaternion[1],
            self.quaternion[2],
            self.quaternion[3],
        );
        UnitQuaternion::from_quaternion(q)
    }

    /// Typed rotation: body frame → ECI (inertial).
    pub fn rotation_to_eci(&self) -> Rotation<frame::Body, frame::SimpleEci> {
        Rotation::from_raw(self.orientation())
    }

    /// Typed rotation: ECI (inertial) → body frame.
    pub fn rotation_to_body(&self) -> Rotation<frame::SimpleEci, frame::Body> {
        self.rotation_to_eci().inverse()
    }

    /// Quaternion kinematic equation: dq/dt = 0.5 * q ⊗ (0, ω).
    ///
    /// Returns the time derivative of the quaternion as a 4-vector.
    pub fn q_dot(&self) -> Vector4<f64> {
        let (w, x, y, z) = (
            self.quaternion[0],
            self.quaternion[1],
            self.quaternion[2],
            self.quaternion[3],
        );
        let (wx, wy, wz) = (
            self.angular_velocity[0],
            self.angular_velocity[1],
            self.angular_velocity[2],
        );
        // dq/dt = 0.5 * q ⊗ (0, ω)
        Vector4::new(
            0.5 * (-x * wx - y * wy - z * wz),
            0.5 * (w * wx + y * wz - z * wy),
            0.5 * (w * wy + z * wx - x * wz),
            0.5 * (w * wz + x * wy - y * wx),
        )
    }

    /// Create an AttitudeState representing a derivative (q_dot, angular_acceleration).
    ///
    /// In the ODE formulation y = (q, ω), the derivative dy/dt = (q_dot, α)
    /// has the same type:
    /// - `quaternion` field holds dq/dt
    /// - `angular_velocity` field holds dω/dt (angular acceleration)
    pub fn from_derivative(q_dot: Vector4<f64>, angular_acceleration: Vector3<f64>) -> Self {
        Self {
            quaternion: q_dot,
            angular_velocity: angular_acceleration,
        }
    }
}

impl HasAttitude for AttitudeState {
    fn attitude(&self) -> &AttitudeState {
        self
    }
}

impl OdeState for AttitudeState {
    fn zero_like(&self) -> Self {
        Self {
            quaternion: Vector4::zeros(),
            angular_velocity: Vector3::zeros(),
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Self {
            quaternion: self.quaternion + scale * other.quaternion,
            angular_velocity: self.angular_velocity + scale * other.angular_velocity,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Self {
            quaternion: factor * self.quaternion,
            angular_velocity: factor * self.angular_velocity,
        }
    }

    fn is_finite(&self) -> bool {
        self.quaternion.iter().all(|v| v.is_finite())
            && self.angular_velocity.iter().all(|v| v.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let mut sum_sq = 0.0;
        let n = 7; // 4 quaternion + 3 angular velocity

        for i in 0..4 {
            let sc = tol.atol + tol.rtol * self.quaternion[i].abs().max(y_next.quaternion[i].abs());
            let e = error.quaternion[i] / sc;
            sum_sq += e * e;
        }
        for i in 0..3 {
            let sc = tol.atol
                + tol.rtol
                    * self.angular_velocity[i]
                        .abs()
                        .max(y_next.angular_velocity[i].abs());
            let e = error.angular_velocity[i] / sc;
            sum_sq += e * e;
        }

        (sum_sq / n as f64).sqrt()
    }

    fn project(&mut self, _t: f64) {
        let norm = self.quaternion.magnitude();
        if norm > 0.0 {
            self.quaternion /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn ode_state_zero_like() {
        let state = AttitudeState::identity();
        let zero = state.zero_like();
        assert_eq!(zero.quaternion, Vector4::zeros());
        assert_eq!(zero.angular_velocity, Vector3::zeros());
    }

    #[test]
    fn ode_state_axpy() {
        let a = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(1.0, 0.0, 0.0),
        };
        let b = AttitudeState {
            quaternion: Vector4::new(0.0, 1.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.0, 2.0, 0.0),
        };
        let result = a.axpy(0.5, &b);
        assert_eq!(result.quaternion, Vector4::new(1.0, 0.5, 0.0, 0.0));
        assert_eq!(result.angular_velocity, Vector3::new(1.0, 1.0, 0.0));
    }

    #[test]
    fn ode_state_scale() {
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 2.0, 3.0, 4.0),
            angular_velocity: Vector3::new(5.0, 6.0, 7.0),
        };
        let scaled = state.scale(2.0);
        assert_eq!(scaled.quaternion, Vector4::new(2.0, 4.0, 6.0, 8.0));
        assert_eq!(scaled.angular_velocity, Vector3::new(10.0, 12.0, 14.0));
    }

    #[test]
    fn ode_state_is_finite() {
        let good = AttitudeState::identity();
        assert!(good.is_finite());

        let bad_q = AttitudeState {
            quaternion: Vector4::new(f64::NAN, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        };
        assert!(!bad_q.is_finite());

        let bad_w = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.0, f64::INFINITY, 0.0),
        };
        assert!(!bad_w.is_finite());
    }

    #[test]
    fn ode_state_project_normalizes() {
        let mut state = AttitudeState {
            quaternion: Vector4::new(2.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(1.0, 2.0, 3.0),
        };
        state.project(0.0);
        let norm = state.quaternion.magnitude();
        assert!((norm - 1.0).abs() < 1e-15);
        // Angular velocity should be unchanged
        assert_eq!(state.angular_velocity, Vector3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn ode_state_project_preserves_unit() {
        let mut state = AttitudeState::identity();
        state.project(0.0);
        assert!((state.quaternion.magnitude() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn ode_state_error_norm() {
        let y_n = AttitudeState::identity();
        let y_next = AttitudeState::identity();
        let error = AttitudeState {
            quaternion: Vector4::new(1e-8, 1e-8, 1e-8, 1e-8),
            angular_velocity: Vector3::new(1e-8, 1e-8, 1e-8),
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);
        assert!(norm > 0.0);
        assert!(norm.is_finite());
    }

    #[test]
    fn q_dot_zero_omega() {
        let state = AttitudeState::identity();
        let dq = state.q_dot();
        assert!(dq.magnitude() < 1e-15);
    }

    #[test]
    fn q_dot_single_axis_x() {
        // Rotation about body x-axis at 1 rad/s, starting from identity
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(1.0, 0.0, 0.0),
        };
        let dq = state.q_dot();
        // dq/dt = 0.5 * (0, ω) for identity quaternion
        assert!((dq[0] - 0.0).abs() < 1e-15); // dw/dt = 0
        assert!((dq[1] - 0.5).abs() < 1e-15); // dx/dt = 0.5 * ωx
        assert!((dq[2] - 0.0).abs() < 1e-15);
        assert!((dq[3] - 0.0).abs() < 1e-15);
    }

    #[test]
    fn new_from_unit_quaternion() {
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let angle = PI / 4.0;
        let uq = UnitQuaternion::from_axis_angle(&axis, angle);
        let state = AttitudeState::new(uq, Vector3::new(0.1, 0.2, 0.3));
        let recovered = state.orientation();
        assert!((recovered.angle() - angle).abs() < 1e-14);
    }

    #[test]
    fn from_derivative_fields() {
        let q_dot = Vector4::new(0.1, 0.2, 0.3, 0.4);
        let alpha = Vector3::new(0.5, 0.6, 0.7);
        let d = AttitudeState::from_derivative(q_dot, alpha);
        assert_eq!(d.quaternion, q_dot);
        assert_eq!(d.angular_velocity, alpha);
    }
}
