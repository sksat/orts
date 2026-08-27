use arika::frame::{self, Rotation};
use nalgebra::{UnitQuaternion, Vector3, Vector4};
use utsuroi::{OdeState, Projection, Tolerances};

use crate::model::HasAttitude;

/// Tolerance on `|q| - 1` below which [`OdeState::project`] leaves the
/// quaternion alone.
///
/// A few ulp of norm drift changes no orientation — `orientation()`
/// normalizes on read — so correcting it would only cost the adaptive solvers
/// their FSAL derivative for nothing.
const QUATERNION_NORM_TOLERANCE: f64 = 8.0 * f64::EPSILON;

/// Attitude state: unit quaternion (orientation) + angular velocity in body frame.
///
/// The quaternion is stored as `[w, x, y, z]` (Hamilton scalar-first convention).
/// During integration, the quaternion may deviate slightly from unit norm;
/// [`OdeState::project`] renormalizes it after each accepted step once the
/// drift exceeds a few ulp.
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
        self.rotation_to_inertial()
    }

    /// Typed rotation: body frame → inertial frame `F`.
    ///
    /// The quaternion data is frame-independent — only the phantom type
    /// tag changes. This allows models to produce `ExternalLoads<F>` in
    /// any propagation frame without `AttitudeState` needing a type
    /// parameter.
    pub fn rotation_to_inertial<F: frame::Eci>(&self) -> Rotation<frame::Body, F> {
        Rotation::from_raw(self.orientation())
    }

    /// Typed rotation: ECI (inertial) → body frame.
    pub fn rotation_to_body(&self) -> Rotation<frame::SimpleEci, frame::Body> {
        self.rotation_from_inertial()
    }

    /// Typed rotation: inertial frame `F` → body frame.
    pub fn rotation_from_inertial<F: frame::Eci>(&self) -> Rotation<F, frame::Body> {
        self.rotation_to_inertial::<F>().inverse()
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
        // Halve ω before multiplying rather than the sum afterwards. Each term
        // is then bounded by `|q_i| · ½|ω|`, so a unit quaternion keeps the sum
        // within `½ √3 f64::MAX` by Cauchy-Schwarz. Scaling last lets the sum
        // overflow at a rate the result is nowhere near: with
        // `q = [0, 1/√2, 1/√2, 0]` and `ω = [1.4e308, 1.4e308, 0]` the true
        // `q̇.w` is about -9.9e307, while `-x·wx - y·wy` reaches -1.98e308 and
        // becomes `-inf` that the later `0.5` cannot bring back.
        let (wx, wy, wz) = (
            0.5 * self.angular_velocity[0],
            0.5 * self.angular_velocity[1],
            0.5 * self.angular_velocity[2],
        );
        // dq/dt = 0.5 * q ⊗ (0, ω)
        Vector4::new(
            -x * wx - y * wy - z * wz,
            w * wx + y * wz - z * wy,
            w * wy + z * wx - x * wz,
            w * wz + x * wy - y * wx,
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

    /// Finite components are not enough for the quaternion: its norm has to be
    /// usable too.
    ///
    /// Components around 1e157 are each finite while their squares sum to
    /// infinity, and such a quaternion names no orientation — `orientation()`
    /// divides by that norm. `project` deliberately leaves it alone rather than
    /// turning it into a plausible-looking zero, which makes this the check that
    /// has to catch it. Reported here, the integrators stop at the step that
    /// produced it instead of handing it to sensors and a controller first.
    fn is_finite(&self) -> bool {
        let norm_sq = self.quaternion.norm_squared();
        norm_sq.is_finite()
            && norm_sq > 0.0
            && self.quaternion.iter().all(|v| v.is_finite())
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

    /// Renormalize the quaternion when it has drifted off the unit sphere.
    ///
    /// The tolerance keeps the FSAL cache of the adaptive solvers alive: a
    /// correction of a few ulp buys no accuracy (`orientation()` normalizes on
    /// read anyway) but reporting it as a change costs one extra derivative
    /// evaluation per step.
    ///
    /// A zero norm is left alone: it carries no orientation to rescale, and
    /// snapping to identity would invent one and hide the failure instead. A
    /// non-finite norm is left alone for the opposite reason — dividing by it
    /// would turn an infinite quaternion into a plausible-looking zero one and
    /// defeat the integrators' finiteness checks.
    fn project(&mut self, _t: f64) -> Projection {
        let norm = self.quaternion.magnitude();
        if norm.is_finite() && norm > 0.0 && (norm - 1.0).abs() > QUATERNION_NORM_TOLERANCE {
            self.quaternion /= norm;
            Projection::Changed
        } else {
            Projection::Unchanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Scaling ω before the products, not the sum after them: with these
    /// values the mathematical result is comfortably finite while the
    /// intermediate sum is not, so multiplying by 0.5 last returned `-inf`.
    #[test]
    fn q_dot_does_not_overflow_where_its_result_is_finite() {
        let state = AttitudeState {
            quaternion: Vector4::new(0.0, 1.0 / 2.0_f64.sqrt(), 1.0 / 2.0_f64.sqrt(), 0.0),
            angular_velocity: Vector3::new(1.4e308, 1.4e308, 0.0),
        };
        let q_dot = state.q_dot();
        assert!(
            q_dot.iter().all(|v| v.is_finite()),
            "q_dot overflowed: {q_dot:?}"
        );
        // -(x·wx + y·wy)/2 with x = y = 1/√2 and wx = wy = 1.4e308.
        let expected = -1.4e308 / 2.0_f64.sqrt();
        assert!(
            (q_dot[0] - expected).abs() < expected.abs() * 1e-12,
            "expected about {expected}, got {}",
            q_dot[0]
        );
    }

    /// A quaternion whose components are finite but whose norm is not names no
    /// orientation, and `project` leaves it alone rather than collapsing it to
    /// zero — so this is the check that has to reject it.
    #[test]
    fn a_quaternion_whose_norm_overflows_is_not_finite() {
        let state = AttitudeState {
            quaternion: Vector4::new(1e157, 1e157, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        };
        assert!(
            state.quaternion.iter().all(|v| v.is_finite()),
            "the components are meant to be finite"
        );
        assert!(
            !state.quaternion.norm_squared().is_finite(),
            "the sum of squares is meant to overflow"
        );
        assert!(!state.is_finite(), "should be reported as non-finite");
    }

    #[test]
    fn an_all_zero_quaternion_is_not_finite() {
        let state = AttitudeState {
            quaternion: Vector4::zeros(),
            angular_velocity: Vector3::zeros(),
        };
        assert!(!state.is_finite(), "a zero quaternion names no orientation");
    }

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
        assert_eq!(state.project(0.0), Projection::Changed);
        let norm = state.quaternion.magnitude();
        assert!((norm - 1.0).abs() < 1e-15);
        // Angular velocity should be unchanged
        assert_eq!(state.angular_velocity, Vector3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn ode_state_project_preserves_unit() {
        let mut state = AttitudeState::identity();
        assert_eq!(state.project(0.0), Projection::Unchanged);
        assert!((state.quaternion.magnitude() - 1.0).abs() < 1e-15);
    }

    /// Drift of a few ulp is left alone: `orientation()` normalizes on read,
    /// so correcting it buys nothing and would cost the adaptive solvers their
    /// FSAL derivative.
    #[test]
    fn ode_state_project_ignores_ulp_scale_drift() {
        let drift = 1.0 + 2.0 * f64::EPSILON;
        let mut state = AttitudeState {
            quaternion: Vector4::new(drift, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        };
        assert_eq!(state.project(0.0), Projection::Unchanged);
        assert_eq!(state.quaternion[0], drift);
    }

    /// Drift large enough to matter is corrected, and reported as a change so
    /// the solvers drop the derivative taken at the unprojected state.
    #[test]
    fn ode_state_project_reports_correction() {
        let mut state = AttitudeState {
            quaternion: Vector4::new(1.0 + 1e-9, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        };
        assert_eq!(state.project(0.0), Projection::Changed);
        assert!((state.quaternion.magnitude() - 1.0).abs() < f64::EPSILON);
    }

    /// A degenerate quaternion is passed through untouched. Rescaling a zero
    /// norm is impossible, and rescaling an infinite one would produce a
    /// finite-looking zero quaternion that the integrators' `is_finite`
    /// checks would then wave through.
    #[test]
    fn ode_state_project_leaves_degenerate_quaternions_alone() {
        for q in [
            Vector4::zeros(),
            Vector4::new(f64::INFINITY, 0.0, 0.0, 0.0),
            Vector4::new(f64::NAN, 0.0, 0.0, 0.0),
        ] {
            let mut state = AttitudeState {
                quaternion: q,
                angular_velocity: Vector3::zeros(),
            };
            assert_eq!(state.project(0.0), Projection::Unchanged);
            assert!(
                state
                    .quaternion
                    .iter()
                    .zip(q.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "degenerate quaternion {q:?} must be passed through, got {:?}",
                state.quaternion
            );
        }
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
