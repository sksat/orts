use kaname::epoch::Epoch;
use nalgebra::{Matrix3, Vector3};

use super::state::AttitudeState;
use super::TorqueModel;

/// Gravity gradient torque on a rigid body in a gravitational field.
///
/// Torque: τ_gg = (3μ / r⁵) (r_body × (I · r_body))
///
/// where r_body is the spacecraft position expressed in the body frame,
/// I is the inertia tensor, μ is the gravitational parameter, and r = |r|.
pub struct GravityGradientTorque {
    mu: f64,
    inertia: Matrix3<f64>,
    position_fn: Box<dyn Fn(f64) -> Vector3<f64> + Send + Sync>,
}

impl GravityGradientTorque {
    /// Create with a gravitational parameter, inertia tensor, and position function.
    ///
    /// `position_fn` returns the spacecraft position in the inertial frame at time `t`.
    /// This allows decoupled attitude/orbit propagation.
    pub fn new(
        mu: f64,
        inertia: Matrix3<f64>,
        position_fn: impl Fn(f64) -> Vector3<f64> + Send + Sync + 'static,
    ) -> Self {
        Self {
            mu,
            inertia,
            position_fn: Box::new(position_fn),
        }
    }

    /// Create for a circular orbit in the x-y plane.
    ///
    /// The position traces a circle of given radius at the mean motion rate.
    /// Useful for testing gravity gradient libration.
    pub fn circular_orbit(
        mu: f64,
        radius: f64,
        inertia: Matrix3<f64>,
    ) -> Self {
        let n = (mu / radius.powi(3)).sqrt(); // mean motion
        Self::new(mu, inertia, move |t| {
            let theta = n * t;
            Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0)
        })
    }
}

impl TorqueModel for GravityGradientTorque {
    fn name(&self) -> &str {
        "gravity_gradient"
    }

    fn torque(&self, t: f64, state: &AttitudeState, _epoch: Option<&Epoch>) -> Vector3<f64> {
        let r_eci = (self.position_fn)(t);
        let r_mag = r_eci.magnitude();
        if r_mag < 1e-10 {
            return Vector3::zeros();
        }

        // Transform position to body frame: r_body = R_bi * r_eci
        let r_bi = state.inertial_to_body();
        let r_body = r_bi * r_eci;

        // τ_gg = (3μ / r⁵) (r_body × (I · r_body))
        let coeff = 3.0 * self.mu / r_mag.powi(5);
        let i_r = self.inertia * r_body;
        coeff * r_body.cross(&i_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{UnitQuaternion, Vector4};
    use std::f64::consts::PI;

    fn diagonal_inertia(ix: f64, iy: f64, iz: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(ix, iy, iz))
    }

    #[test]
    fn equilibrium_zero_torque() {
        // Body z-axis aligned with radial direction → zero torque for diagonal inertia
        // (because r_body × (I · r_body) = 0 when r_body is along a principal axis)
        let inertia = diagonal_inertia(10.0, 20.0, 30.0);
        let mu = 398600.4418; // Earth
        let r = 6778.0; // LEO

        let gg = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r, 0.0, 0.0));

        // Body x-axis aligned with radial (identity quaternion, r along x)
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        };
        let tau = gg.torque(0.0, &state, None);
        assert!(tau.magnitude() < 1e-15, "Expected zero torque, got {tau:?}");
    }

    #[test]
    fn torque_nonzero_for_tilted_body() {
        // Tilt the body so principal axes don't align with radial → nonzero torque
        let inertia = diagonal_inertia(10.0, 20.0, 30.0);
        let mu = 398600.4418;
        let r = 6778.0;

        let gg = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r, 0.0, 0.0));

        // Rotate 45° about z-axis
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let state = AttitudeState::new(uq, Vector3::zeros());
        let tau = gg.torque(0.0, &state, None);
        assert!(tau.magnitude() > 1e-10, "Expected nonzero torque");
    }

    #[test]
    fn torque_scales_with_mu() {
        let inertia = diagonal_inertia(10.0, 20.0, 30.0);
        let r = 6778.0;

        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let state = AttitudeState::new(uq, Vector3::zeros());

        let gg1 = GravityGradientTorque::new(1.0, inertia, move |_| Vector3::new(r, 0.0, 0.0));
        let gg2 = GravityGradientTorque::new(2.0, inertia, move |_| Vector3::new(r, 0.0, 0.0));

        let tau1 = gg1.torque(0.0, &state, None);
        let tau2 = gg2.torque(0.0, &state, None);

        // Torque should scale linearly with μ
        let ratio = tau2.magnitude() / tau1.magnitude();
        assert!((ratio - 2.0).abs() < 1e-10, "Expected ratio 2, got {ratio}");
    }

    #[test]
    fn torque_scales_with_distance() {
        let inertia = diagonal_inertia(10.0, 20.0, 30.0);
        let mu = 398600.4418;

        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let state = AttitudeState::new(uq, Vector3::zeros());

        let r1 = 7000.0;
        let r2 = 14000.0;
        let gg1 = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r1, 0.0, 0.0));
        let gg2 = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r2, 0.0, 0.0));

        let tau1 = gg1.torque(0.0, &state, None);
        let tau2 = gg2.torque(0.0, &state, None);

        // τ ∝ 1/r³ (r⁵ in denominator, r² from r_body products)
        let expected_ratio = (r1 / r2).powi(3);
        let actual_ratio = tau2.magnitude() / tau1.magnitude();
        assert!(
            (actual_ratio - expected_ratio).abs() < 1e-6,
            "Expected ratio {expected_ratio}, got {actual_ratio}"
        );
    }

    #[test]
    fn symmetric_body_zero_torque_any_orientation() {
        // Spherically symmetric body: I = diag(I, I, I) → r × (I·r) = I(r × r) = 0
        let i = 15.0;
        let inertia = diagonal_inertia(i, i, i);
        let mu = 398600.4418;
        let r = 7000.0;

        let gg = GravityGradientTorque::new(mu, inertia, move |_| Vector3::new(r, 0.0, 0.0));

        // Arbitrary orientation
        let axis = nalgebra::Unit::new_normalize(Vector3::new(1.0, 2.0, 3.0));
        let uq = UnitQuaternion::from_axis_angle(&axis, 1.234);
        let state = AttitudeState::new(uq, Vector3::zeros());

        let tau = gg.torque(0.0, &state, None);
        assert!(
            tau.magnitude() < 1e-10,
            "Symmetric body should have zero GG torque, got {tau:?}"
        );
    }

    #[test]
    fn circular_orbit_helper() {
        let mu = 398600.4418;
        let r = 7000.0;
        let inertia = diagonal_inertia(10.0, 20.0, 30.0);
        let gg = GravityGradientTorque::circular_orbit(mu, r, inertia);

        // At t=0, position should be (r, 0, 0)
        let state = AttitudeState::identity();
        // Just verify it doesn't panic and returns a valid torque
        let tau = gg.torque(0.0, &state, None);
        assert!(tau.iter().all(|v| v.is_finite()));
    }
}
