use nalgebra::Vector3;

/// A gravitational field model that computes acceleration from position.
///
/// Implementations range from simple point-mass gravity to full spherical
/// harmonics (geoid) models.
pub trait GravityField: Send + Sync {
    /// Compute gravitational acceleration [km/s²] at the given position.
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64>;
}

/// Point-mass (spherically symmetric) gravity: a = -μ/|r|³ * r
pub struct PointMass;

impl GravityField for PointMass {
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64> {
        let r_mag = position.magnitude();
        -mu / (r_mag * r_mag * r_mag) * position
    }
}

/// Zonal harmonics gravity field: point mass + J2 (+ optional J3, J4).
///
/// Models the oblateness of a celestial body using zonal (axially symmetric)
/// spherical harmonic coefficients. The z-axis is the body's spin axis.
pub struct ZonalHarmonics {
    /// Equatorial radius of the central body [km]
    pub r_body: f64,
    /// J2 coefficient (dimensionless)
    pub j2: f64,
}

impl GravityField for ZonalHarmonics {
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64> {
        let r_mag = position.magnitude();
        let r2 = r_mag * r_mag;
        let r5 = r2 * r2 * r_mag;

        // Point-mass term
        let a_pm = -mu / (r_mag * r2) * position;

        // J2 perturbation
        let x = position.x;
        let y = position.y;
        let z = position.z;
        let re2 = self.r_body * self.r_body;
        let z2_over_r2 = (z * z) / r2;
        let coeff = 1.5 * self.j2 * mu * re2 / r5;

        let a_j2 = Vector3::new(
            coeff * x * (5.0 * z2_over_r2 - 1.0),
            coeff * y * (5.0 * z2_over_r2 - 1.0),
            coeff * z * (5.0 * z2_over_r2 - 3.0),
        );

        a_pm + a_j2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{J2_EARTH, MU_EARTH, R_EARTH};

    #[test]
    fn point_mass_acceleration_direction() {
        let state_pos = Vector3::new(6778.137, 0.0, 0.0);
        let accel = PointMass.acceleration(MU_EARTH, &state_pos);

        // Acceleration should be antiparallel to position
        let dot = accel.dot(&state_pos);
        assert!(dot < 0.0, "acceleration should point toward center (dot={dot})");

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

    // --- ZonalHarmonics tests ---

    fn earth_j2() -> ZonalHarmonics {
        ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
        }
    }

    #[test]
    fn zonal_zero_j2_matches_point_mass() {
        let zonal = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
        };
        let r = Vector3::new(6778.137, 1000.0, 500.0);

        let a_pm = PointMass.acceleration(MU_EARTH, &r);
        let a_zonal = zonal.acceleration(MU_EARTH, &r);

        assert!(
            (a_pm - a_zonal).magnitude() < 1e-15,
            "ZonalHarmonics with j2=0 should match PointMass"
        );
    }

    #[test]
    fn j2_acceleration_magnitude_at_iss() {
        // Hand calculation for ISS at (r, 0, 0) on equator:
        // a_J2 = (3/2) * J2 * μ * R² / r⁵ * [x*(5*0/r²-1), 0, 0]
        //      = (3/2) * J2 * μ * R² / r⁴ * (-1)  (radial component, equatorial)
        let grav = earth_j2();
        let r = R_EARTH + 400.0; // ISS altitude
        let pos = Vector3::new(r, 0.0, 0.0);

        let a_total = grav.acceleration(MU_EARTH, &pos);
        let a_pm = PointMass.acceleration(MU_EARTH, &pos);
        let a_j2 = a_total - a_pm;

        // Expected: |a_J2| ≈ (3/2) * 1.08263e-3 * 398600 * 6378² / 6778⁴
        let expected = 1.5 * J2_EARTH * MU_EARTH * R_EARTH * R_EARTH / r.powi(4);
        let actual = a_j2.magnitude();

        let rel_err = (actual - expected).abs() / expected;
        assert!(
            rel_err < 0.01,
            "J2 magnitude at ISS: expected≈{expected:.6e}, actual={actual:.6e}, rel_err={rel_err:.4e}"
        );
    }

    #[test]
    fn j2_equatorial_plane_z_behavior() {
        // On the equatorial plane (z=0), the J2 z-component should be zero
        let grav = earth_j2();
        let pos = Vector3::new(7000.0, 0.0, 0.0);

        let a_total = grav.acceleration(MU_EARTH, &pos);
        let a_pm = PointMass.acceleration(MU_EARTH, &pos);
        let a_j2 = a_total - a_pm;

        assert!(
            a_j2.z.abs() < 1e-20,
            "J2 z-component should be zero on equatorial plane, got {}",
            a_j2.z
        );
    }

    #[test]
    fn j2_polar_stronger_than_equatorial() {
        // J2 perturbation is stronger at polar positions than equatorial
        let grav = earth_j2();
        let r = 7000.0;

        let pos_eq = Vector3::new(r, 0.0, 0.0);
        let pos_polar = Vector3::new(0.0, 0.0, r);

        let a_eq = grav.acceleration(MU_EARTH, &pos_eq);
        let a_polar = grav.acceleration(MU_EARTH, &pos_polar);

        let a_pm = PointMass.acceleration(MU_EARTH, &pos_eq);

        let j2_eq = (a_eq - a_pm).magnitude();
        let j2_polar = (a_polar - PointMass.acceleration(MU_EARTH, &pos_polar)).magnitude();

        assert!(
            j2_polar > j2_eq,
            "J2 at pole ({j2_polar:.6e}) should be larger than at equator ({j2_eq:.6e})"
        );
    }

    #[test]
    fn j2_r_inverse_fourth_dependence() {
        // J2 perturbation scales as r^(-4) (since it adds to r^(-2) point mass as r^(-4) correction)
        let grav = earth_j2();
        let pos1 = Vector3::new(7000.0, 0.0, 500.0);
        let pos2 = Vector3::new(14000.0, 0.0, 1000.0); // same direction, 2x distance

        let a1_total = grav.acceleration(MU_EARTH, &pos1);
        let a1_pm = PointMass.acceleration(MU_EARTH, &pos1);
        let j2_1 = (a1_total - a1_pm).magnitude();

        let a2_total = grav.acceleration(MU_EARTH, &pos2);
        let a2_pm = PointMass.acceleration(MU_EARTH, &pos2);
        let j2_2 = (a2_total - a2_pm).magnitude();

        // For same direction (same z/r ratio), J2 ~ r^(-4)
        // Ratio should be (2)^4 = 16
        let ratio = j2_1 / j2_2;
        assert!(
            (ratio - 16.0).abs() < 0.5,
            "J2 ratio for 2x distance should be ~16, got {ratio:.2}"
        );
    }
}
