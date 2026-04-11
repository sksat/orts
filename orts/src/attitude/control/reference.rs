use crate::OrbitalState;
use arika::epoch::Epoch;
use nalgebra::{Matrix3, UnitQuaternion, Vector3};

/// A target attitude reference that provides desired orientation and angular velocity.
///
/// Implementations define different pointing strategies (inertial hold, nadir pointing, etc.).
pub trait AttitudeReference: Send + Sync {
    /// Compute the target orientation and angular velocity at time `t`.
    ///
    /// Returns `(q_target, omega_target)` where:
    /// - `q_target` is the desired body-to-inertial quaternion
    /// - `omega_target` is the desired angular velocity in the target body frame [rad/s]
    fn target(
        &self,
        t: f64,
        orbit: &OrbitalState,
        epoch: Option<&Epoch>,
    ) -> (UnitQuaternion<f64>, Vector3<f64>);
}

/// Inertial pointing: hold a fixed orientation in the inertial frame.
pub struct InertialPointing {
    pub target_q: UnitQuaternion<f64>,
}

impl AttitudeReference for InertialPointing {
    fn target(
        &self,
        _t: f64,
        _orbit: &OrbitalState,
        _epoch: Option<&Epoch>,
    ) -> (UnitQuaternion<f64>, Vector3<f64>) {
        (self.target_q, Vector3::zeros())
    }
}

/// Nadir pointing: align the body Z-axis with nadir (toward Earth center).
///
/// LVLH (Local Vertical Local Horizontal) frame definition:
/// - Z_lvlh = -r/|r| (nadir direction)
/// - Y_lvlh = -(r × v)/|r × v| (negative orbit normal)
/// - X_lvlh = Y_lvlh × Z_lvlh (approximately along velocity for circular orbits)
///
/// The target angular velocity in the LVLH body frame is `[0, -n, 0]` where
/// `n = |r × v| / r²` is the instantaneous angular rate.
pub struct NadirPointing;

impl AttitudeReference for NadirPointing {
    fn target(
        &self,
        _t: f64,
        orbit: &OrbitalState,
        _epoch: Option<&Epoch>,
    ) -> (UnitQuaternion<f64>, Vector3<f64>) {
        let r = *orbit.position();
        let v = *orbit.velocity();
        let r_mag = r.magnitude();

        // Angular momentum vector
        let h = r.cross(&v);
        let h_mag = h.magnitude();

        // LVLH frame axes in inertial coordinates
        let z_lvlh = -r / r_mag; // nadir
        let y_lvlh = -h / h_mag; // negative orbit normal
        let x_lvlh = y_lvlh.cross(&z_lvlh); // ~along velocity

        // Rotation matrix from LVLH body to inertial: columns are LVLH axes in inertial frame
        let r_lvlh_to_inertial = Matrix3::from_columns(&[x_lvlh, y_lvlh, z_lvlh]);
        let q_target = UnitQuaternion::from_rotation_matrix(
            &nalgebra::Rotation3::from_matrix_unchecked(r_lvlh_to_inertial),
        );

        // Instantaneous angular rate: n = |h| / r²
        let n = h_mag / (r_mag * r_mag);

        // Angular velocity of LVLH frame in LVLH body frame: [0, -n, 0]
        let omega_target = Vector3::new(0.0, -n, 0.0);

        (q_target, omega_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn inertial_pointing_returns_fixed_target() {
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let ref_point = InertialPointing { target_q: q };

        let orbit = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));

        let (q_out, omega_out) = ref_point.target(0.0, &orbit, None);
        assert!((q_out.angle() - PI / 4.0).abs() < 1e-14);
        assert!(omega_out.magnitude() < 1e-15);
    }

    #[test]
    fn nadir_z_axis_points_toward_earth() {
        let orbit = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));

        let nadir = NadirPointing;
        let (q_target, _omega) = nadir.target(0.0, &orbit, None);

        // The body Z-axis in inertial frame should point nadir (toward -r)
        let r_mat = q_target.to_rotation_matrix();
        let z_body_inertial = r_mat * Vector3::new(0.0, 0.0, 1.0);

        let r_hat = orbit.position().normalize();
        // Z_lvlh = -r/|r|, so body Z should be -r_hat
        let expected = -r_hat;
        let error = (z_body_inertial - expected).magnitude();
        assert!(
            error < 1e-14,
            "Body Z should point nadir, error: {error:.2e}"
        );
    }

    #[test]
    fn nadir_omega_target_circular_orbit() {
        let mu: f64 = 398600.4418;
        let r = 7000.0;
        let v_circ = (mu / r).sqrt();

        let orbit = OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v_circ, 0.0));

        let nadir = NadirPointing;
        let (_q_target, omega_target) = nadir.target(0.0, &orbit, None);

        // n = |h| / r² = r*v / r² = v/r for circular orbit
        let n_expected = v_circ / r;

        // omega_target should be [0, -n, 0]
        assert!(
            omega_target[0].abs() < 1e-15,
            "omega_x should be 0, got {}",
            omega_target[0]
        );
        assert!(
            (omega_target[1] + n_expected).abs() < 1e-12,
            "omega_y should be -{n_expected}, got {}",
            omega_target[1]
        );
        assert!(
            omega_target[2].abs() < 1e-15,
            "omega_z should be 0, got {}",
            omega_target[2]
        );
    }

    #[test]
    fn nadir_orthonormal_frame() {
        // Verify the LVLH frame is orthonormal
        let orbit = OrbitalState::new(
            Vector3::new(5000.0, 3000.0, 1000.0),
            Vector3::new(-1.0, 6.0, 2.0),
        );

        let nadir = NadirPointing;
        let (q_target, _) = nadir.target(0.0, &orbit, None);

        // The rotation matrix should be orthonormal
        let r_mat = q_target.to_rotation_matrix();
        let m = r_mat.matrix();
        let identity = m.transpose() * m;

        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (identity[(i, j)] - expected).abs() < 1e-14,
                    "R^T R[{i},{j}] = {}, expected {expected}",
                    identity[(i, j)]
                );
            }
        }
    }
}
