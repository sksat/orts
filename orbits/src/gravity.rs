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
    /// J3 coefficient (dimensionless, optional)
    pub j3: Option<f64>,
    /// J4 coefficient (dimensionless, optional)
    pub j4: Option<f64>,
}

impl GravityField for ZonalHarmonics {
    fn acceleration(&self, mu: f64, position: &Vector3<f64>) -> Vector3<f64> {
        let r_mag = position.magnitude();
        let r2 = r_mag * r_mag;
        let r5 = r2 * r2 * r_mag;

        // Point-mass term
        let a_pm = -mu / (r_mag * r2) * position;

        let x = position.x;
        let y = position.y;
        let z = position.z;
        let re = self.r_body;
        let re2 = re * re;
        let s2 = (z * z) / r2; // (z/r)²

        // J2 perturbation
        let coeff2 = 1.5 * self.j2 * mu * re2 / r5;
        let a_j2 = Vector3::new(
            coeff2 * x * (5.0 * s2 - 1.0),
            coeff2 * y * (5.0 * s2 - 1.0),
            coeff2 * z * (5.0 * s2 - 3.0),
        );

        let mut accel = a_pm + a_j2;

        // J3 perturbation (pear-shaped asymmetry)
        // From U_J3 = -(μ J3 Re³/2)(5z³/r⁷ - 3z/r⁵), a = ∇U:
        //   a_x = -(μ J3 Re³/2) × 5xz/r⁷ × (3 - 7s²)
        //   a_z = +(μ J3 Re³/2) × (1/r⁵) × (3 - 30s² + 35s⁴)
        if let Some(j3) = self.j3 {
            let re3 = re2 * re;
            let r7 = r5 * r2;
            let coeff3 = 0.5 * j3 * mu * re3;
            accel += Vector3::new(
                -coeff3 * 5.0 * x * z / r7 * (3.0 - 7.0 * s2),
                -coeff3 * 5.0 * y * z / r7 * (3.0 - 7.0 * s2),
                coeff3 / r5 * (3.0 - 30.0 * s2 + 35.0 * s2 * s2),
            );
        }

        // J4 perturbation
        if let Some(j4) = self.j4 {
            let re4 = re2 * re2;
            let r7 = r5 * r2;
            let s4 = s2 * s2;
            accel += Vector3::new(
                (15.0 / 8.0) * j4 * mu * re4 * x / r7 * (1.0 - 14.0 * s2 + 21.0 * s4),
                (15.0 / 8.0) * j4 * mu * re4 * y / r7 * (1.0 - 14.0 * s2 + 21.0 * s4),
                (5.0 / 8.0) * j4 * mu * re4 * z / r7 * (15.0 - 70.0 * s2 + 63.0 * s4),
            );
        }

        accel
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
            j3: None,
            j4: None,
        }
    }

    #[test]
    fn zonal_zero_j2_matches_point_mass() {
        let zonal = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: None,
            j4: None,
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

    // --- J3 tests ---

    use crate::constants::{J3_EARTH, J4_EARTH};

    fn earth_j2_j3_j4() -> ZonalHarmonics {
        ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: Some(J3_EARTH),
            j4: Some(J4_EARTH),
        }
    }

    #[test]
    fn j3_j4_zero_matches_j2_only() {
        let j2_only = earth_j2();
        let j2_j3_j4 = ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: Some(0.0),
            j4: Some(0.0),
        };
        let r = Vector3::new(6778.137, 1000.0, 500.0);

        let a1 = j2_only.acceleration(MU_EARTH, &r);
        let a2 = j2_j3_j4.acceleration(MU_EARTH, &r);

        assert!(
            (a1 - a2).magnitude() < 1e-15,
            "J3=0, J4=0 should match J2-only"
        );
    }

    #[test]
    fn j3_equatorial_z_force() {
        // J3 creates a z-force even on the equatorial plane (pear-shape effect)
        let grav = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: Some(J3_EARTH),
            j4: None,
        };
        let pos = Vector3::new(7000.0, 0.0, 0.0);

        let a_total = grav.acceleration(MU_EARTH, &pos);
        let a_pm = PointMass.acceleration(MU_EARTH, &pos);
        let a_j3 = a_total - a_pm;

        // On equator (z=0), J3 z-component should be non-zero
        // a_J3_z = (1/2)*J3*μ*R³/r⁵ * (3 - 0 + 0) = (3/2)*J3*μ*R³/r⁵
        let expected_z = 1.5 * J3_EARTH * MU_EARTH * R_EARTH.powi(3) / 7000.0_f64.powi(5);
        assert!(
            (a_j3.z - expected_z).abs() / expected_z.abs() < 1e-10,
            "J3 z-force at equator: expected={expected_z:.6e}, got={:.6e}",
            a_j3.z
        );

        // J3 x,y components should be zero on equator (since z=0)
        assert!(
            a_j3.x.abs() < 1e-20,
            "J3 x-component should be zero on equator, got {}",
            a_j3.x
        );
        assert!(
            a_j3.y.abs() < 1e-20,
            "J3 y-component should be zero on equator, got {}",
            a_j3.y
        );
    }

    #[test]
    fn j3_r_inverse_fifth_dependence() {
        // J3 perturbation scales as r^(-5) for same direction
        let grav = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: Some(J3_EARTH),
            j4: None,
        };
        let pos1 = Vector3::new(7000.0, 0.0, 500.0);
        let pos2 = Vector3::new(14000.0, 0.0, 1000.0);

        let j3_1 = (grav.acceleration(MU_EARTH, &pos1) - PointMass.acceleration(MU_EARTH, &pos1)).magnitude();
        let j3_2 = (grav.acceleration(MU_EARTH, &pos2) - PointMass.acceleration(MU_EARTH, &pos2)).magnitude();

        // For same direction, J3 ~ r^(-5), so ratio should be 2^5 = 32
        let ratio = j3_1 / j3_2;
        assert!(
            (ratio - 32.0).abs() < 1.0,
            "J3 ratio for 2x distance should be ~32, got {ratio:.2}"
        );
    }

    #[test]
    fn j3_is_much_smaller_than_j2() {
        // |J3 perturbation| << |J2 perturbation| at ISS altitude
        let grav_j2 = earth_j2();
        let grav_j3 = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: Some(J3_EARTH),
            j4: None,
        };
        let pos = Vector3::new(6778.0, 0.0, 3000.0); // off-equator for nonzero J3

        let a_j2 = (grav_j2.acceleration(MU_EARTH, &pos) - PointMass.acceleration(MU_EARTH, &pos)).magnitude();
        let a_j3 = (grav_j3.acceleration(MU_EARTH, &pos) - PointMass.acceleration(MU_EARTH, &pos)).magnitude();

        // J3 should be ~1000x smaller than J2 (J3/J2 ~ 2.3e-3)
        assert!(
            a_j3 < a_j2 * 0.01,
            "J3 ({a_j3:.6e}) should be much smaller than J2 ({a_j2:.6e})"
        );
    }

    // --- J4 tests ---

    #[test]
    fn j4_equatorial_symmetric() {
        // J4 is even zonal harmonic: z-component should be zero on equator
        let grav = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: None,
            j4: Some(J4_EARTH),
        };
        let pos = Vector3::new(7000.0, 0.0, 0.0);

        let a_total = grav.acceleration(MU_EARTH, &pos);
        let a_pm = PointMass.acceleration(MU_EARTH, &pos);
        let a_j4 = a_total - a_pm;

        assert!(
            a_j4.z.abs() < 1e-20,
            "J4 z-component should be zero on equatorial plane, got {}",
            a_j4.z
        );
    }

    #[test]
    fn j4_r_inverse_sixth_dependence() {
        // J4 perturbation scales as r^(-6) for same direction
        let grav = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: None,
            j4: Some(J4_EARTH),
        };
        let pos1 = Vector3::new(7000.0, 0.0, 500.0);
        let pos2 = Vector3::new(14000.0, 0.0, 1000.0);

        let j4_1 = (grav.acceleration(MU_EARTH, &pos1) - PointMass.acceleration(MU_EARTH, &pos1)).magnitude();
        let j4_2 = (grav.acceleration(MU_EARTH, &pos2) - PointMass.acceleration(MU_EARTH, &pos2)).magnitude();

        // For same direction, J4 ~ r^(-6), so ratio should be 2^6 = 64
        let ratio = j4_1 / j4_2;
        assert!(
            (ratio - 64.0).abs() < 2.0,
            "J4 ratio for 2x distance should be ~64, got {ratio:.2}"
        );
    }

    #[test]
    fn j4_magnitude_at_iss() {
        // J4 equatorial acceleration magnitude at ISS altitude
        // a_J4_x = (15/8)*J4*μ*R⁴*x/r⁷ * (1 - 0 + 0) = (15/8)*J4*μ*R⁴/r⁶
        let grav = ZonalHarmonics {
            r_body: R_EARTH,
            j2: 0.0,
            j3: None,
            j4: Some(J4_EARTH),
        };
        let r = R_EARTH + 400.0;
        let pos = Vector3::new(r, 0.0, 0.0);

        let a_j4 = grav.acceleration(MU_EARTH, &pos) - PointMass.acceleration(MU_EARTH, &pos);
        let expected_mag = (15.0 / 8.0) * J4_EARTH.abs() * MU_EARTH * R_EARTH.powi(4) / r.powi(6);
        let actual_mag = a_j4.magnitude();

        let rel_err = (actual_mag - expected_mag).abs() / expected_mag;
        assert!(
            rel_err < 0.01,
            "J4 magnitude at ISS: expected≈{expected_mag:.6e}, actual={actual_mag:.6e}, rel_err={rel_err:.4e}"
        );
    }

    #[test]
    fn j2_j3_j4_combined_at_iss() {
        // Full J2+J3+J4 model should differ from J2-only but not dramatically
        let grav_j2 = earth_j2();
        let grav_full = earth_j2_j3_j4();
        let pos = Vector3::new(6778.0, 0.0, 3000.0);

        let a_j2_only = grav_j2.acceleration(MU_EARTH, &pos);
        let a_full = grav_full.acceleration(MU_EARTH, &pos);
        let diff = (a_full - a_j2_only).magnitude();
        let a_j2_mag = (a_j2_only - PointMass.acceleration(MU_EARTH, &pos)).magnitude();

        // J3+J4 correction should be small relative to J2
        assert!(
            diff < a_j2_mag * 0.01,
            "J3+J4 correction ({diff:.6e}) should be <1% of J2 ({a_j2_mag:.6e})"
        );
        // But should be non-zero
        assert!(
            diff > 0.0,
            "J3+J4 correction should be non-zero"
        );
    }
}
