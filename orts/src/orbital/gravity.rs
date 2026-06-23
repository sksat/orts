use nalgebra::Vector3;

/// A gravitational field model that computes acceleration from position.
///
/// Returns raw `Vector3<f64>` because gravitational acceleration is
/// frame-independent (the formula `-mu/|r|^3 * r` gives the same raw
/// vector regardless of the inertial frame). The dynamical system wraps
/// the result in the appropriate `Vec3<F>`.
pub trait GravityField: Send + Sync {
    /// Compute gravitational acceleration [km/s²] at the given position.
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64>;
}

// Blanket impl so Box<dyn GravityField> can be used as G in SpacecraftDynamics<G>.
// This is a builder convenience; performance-critical paths should use concrete types.
impl GravityField for Box<dyn GravityField> {
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64> {
        (**self).acceleration(mu, position)
    }
}

/// Point-mass (spherically symmetric) gravity: a = -μ/|r|³ * r
pub struct PointMass;

impl GravityField for PointMass {
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64> {
        let r_mag = position.magnitude();
        -mu / (r_mag * r_mag * r_mag) * position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::{MU as MU_EARTH, R as R_EARTH};

    #[test]
    fn point_mass_acceleration_direction() {
        let state_pos = Vector3::new(6778.137, 0.0, 0.0);
        let accel = PointMass.acceleration(MU_EARTH, &state_pos);

        // Acceleration should be antiparallel to position
        let dot = accel.dot(&state_pos);
        assert!(
            dot < 0.0,
            "acceleration should point toward center (dot={dot})"
        );

        // Should be collinear
        let cross = accel.cross(&state_pos);
        assert!(
            cross.magnitude() < 1e-10,
            "acceleration should be collinear with position (cross mag={})",
            cross.magnitude()
        );
    }

    #[test]
    fn point_mass_acceleration_magnitude() {
        let r = Vector3::new(6778.137, 0.0, 0.0);
        let accel = PointMass.acceleration(MU_EARTH, &r);

        let r_mag = r.magnitude();
        let expected_mag = MU_EARTH / (r_mag * r_mag);
        let actual_mag = accel.magnitude();

        let rel_err = (actual_mag - expected_mag).abs() / expected_mag;
        assert!(
            rel_err < 1e-12,
            "magnitude mismatch: expected={expected_mag}, actual={actual_mag}, rel_err={rel_err}"
        );
    }

    #[test]
    fn point_mass_surface_gravity() {
        let r = Vector3::new(R_EARTH, 0.0, 0.0);
        let accel = PointMass.acceleration(MU_EARTH, &r);

        let g = accel.magnitude();
        let expected_g = 9.798e-3; // km/s²
        assert!(
            (g - expected_g).abs() < 0.01e-3,
            "surface gravity mismatch: expected≈{expected_g}, actual={g}"
        );
    }

    #[test]
    fn point_mass_off_axis() {
        // Acceleration magnitude depends only on distance, not direction
        let r1 = Vector3::new(7000.0, 0.0, 0.0);
        let r2 = Vector3::new(0.0, 7000.0, 0.0);
        let r3 = Vector3::new(
            7000.0 / 3.0_f64.sqrt(),
            7000.0 / 3.0_f64.sqrt(),
            7000.0 / 3.0_f64.sqrt(),
        );

        let a1 = PointMass.acceleration(MU_EARTH, &r1).magnitude();
        let a2 = PointMass.acceleration(MU_EARTH, &r2).magnitude();
        let a3 = PointMass.acceleration(MU_EARTH, &r3).magnitude();

        assert!((a1 - a2).abs() / a1 < 1e-12);
        assert!((a1 - a3).abs() / a1 < 1e-12);
    }
}
