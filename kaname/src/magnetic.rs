use nalgebra::Vector3;

use crate::epoch::Epoch;

/// Tilted dipole geomagnetic field model with ECEF-fixed axis.
///
/// Approximates Earth's magnetic field as a tilted dipole, suitable for
/// B-dot detumbling simulations in LEO. The dipole axis is stored in ECEF
/// coordinates and rotated to ECI using the epoch's GMST, correctly
/// accounting for Earth rotation.
///
/// The dipole field at position **r** is:
///
/// **B** = (dipole_strength / r^3) [3(m_hat . r_hat) r_hat - m_hat]
///
/// where m_hat is the dipole axis unit vector and r is in metres.
pub struct TiltedDipole {
    /// Dipole strength [T*m^3] = mu_0 * m / (4*pi), absorbs mu_0/(4*pi) into the constant.
    dipole_strength: f64,
    /// Dipole axis unit vector in ECEF.
    axis_ecef: Vector3<f64>,
}

impl TiltedDipole {
    /// Create a tilted dipole with the given strength and axis in ECEF coordinates.
    ///
    /// # Panics
    /// Panics if `axis_ecef` is zero-length.
    pub fn new(dipole_strength: f64, axis_ecef: Vector3<f64>) -> Self {
        let norm = axis_ecef.magnitude();
        assert!(norm > 1e-15, "Dipole axis must be non-zero");
        Self {
            dipole_strength,
            axis_ecef: axis_ecef / norm,
        }
    }

    /// Earth's tilted dipole (IGRF approximate).
    ///
    /// - Dipole strength: ~7.94e15 T*m^3 (= mu_0/(4*pi) * 7.94e22 A*m^2)
    /// - Axis tilted ~11.5 deg from geographic north (simplified: tilt in x-z plane in ECEF)
    ///
    /// The axis is stored in ECEF coordinates and will be rotated to ECI
    /// using the epoch's GMST when computing the field.
    pub fn earth() -> Self {
        let tilt = 11.5_f64.to_radians();
        Self {
            dipole_strength: crate::constants::EARTH_DIPOLE_STRENGTH,
            axis_ecef: Vector3::new(tilt.sin(), 0.0, tilt.cos()).normalize(),
        }
    }

    /// Rotate the ECEF axis to ECI using GMST.
    ///
    /// ECEF->ECI rotation about Z by gmst:
    ///   x_eci = cos(gmst) * x_ecef - sin(gmst) * y_ecef
    ///   y_eci = sin(gmst) * x_ecef + cos(gmst) * y_ecef
    ///   z_eci = z_ecef
    fn axis_to_eci(&self, gmst: f64) -> Vector3<f64> {
        let cos_g = gmst.cos();
        let sin_g = gmst.sin();
        Vector3::new(
            cos_g * self.axis_ecef.x - sin_g * self.axis_ecef.y,
            sin_g * self.axis_ecef.x + cos_g * self.axis_ecef.y,
            self.axis_ecef.z,
        )
    }

    /// Compute magnetic field vector in ECI [T] at position_eci [km].
    ///
    /// When `epoch` is provided, the dipole axis is rotated from ECEF to ECI
    /// using GMST (correctly accounting for Earth rotation).
    /// When `epoch` is `None`, the ECEF axis is used as-is (ECI-fixed approximation).
    ///
    /// Returns the zero vector for positions inside 1 km from Earth's centre.
    pub fn field_eci(&self, position_eci: &Vector3<f64>, epoch: Option<&Epoch>) -> Vector3<f64> {
        let r_km = position_eci.magnitude();
        if r_km < 1.0 {
            return Vector3::zeros();
        }

        // Convert km to m for the formula
        let r_m = r_km * 1000.0;
        let r3 = r_m * r_m * r_m;

        let r_hat = position_eci / r_km;

        // Rotate dipole axis from ECEF to ECI if epoch is available,
        // otherwise use ECEF axis as-is (ECI-fixed fallback)
        let m_hat = match epoch {
            Some(e) => self.axis_to_eci(e.gmst()),
            None => self.axis_ecef,
        };

        // B = dipole_strength * [3(m_hat . r_hat) r_hat - m_hat] / r^3
        let m_dot_r = m_hat.dot(&r_hat);
        self.dipole_strength * (3.0 * m_dot_r * r_hat - m_hat) / r3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j2000_epoch() -> Epoch {
        Epoch::j2000()
    }

    #[test]
    fn equatorial_field_magnitude_at_leo() {
        // At equatorial LEO (7000 km from centre, on x-axis)
        // Expected |B| ~ 20-50 uT
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let epoch = j2000_epoch();
        let b = dipole.field_eci(&pos, Some(&epoch));
        let b_mag = b.magnitude();

        // Convert to uT for readability
        let b_micro_t = b_mag * 1e6;
        assert!(
            b_micro_t > 20.0 && b_micro_t < 50.0,
            "Equatorial LEO field should be ~25-35 uT, got {b_micro_t:.2} uT"
        );
    }

    #[test]
    fn inverse_cube_scaling() {
        // B is proportional to 1/r^3: at double distance, field should be 1/8
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        let pos1 = Vector3::new(7000.0, 0.0, 0.0);
        let pos2 = Vector3::new(14000.0, 0.0, 0.0);
        let b1 = dipole.field_eci(&pos1, Some(&epoch)).magnitude();
        let b2 = dipole.field_eci(&pos2, Some(&epoch)).magnitude();

        let ratio = b1 / b2;
        assert!(
            (ratio - 8.0).abs() < 0.01,
            "Expected 1/r^3 scaling (ratio 8.0), got {ratio:.4}"
        );
    }

    #[test]
    fn polar_field_stronger_than_equatorial() {
        // Along the dipole axis the field is 2x the equatorial field at the same distance.
        // Use a Z-axis dipole for this test (axis in ECEF z = geographic north)
        let dipole = TiltedDipole::new(7.94e15, Vector3::new(0.0, 0.0, 1.0));
        let r = 7000.0;
        let epoch = j2000_epoch();

        // Pole position in ECI depends on GMST, but for a z-axis dipole,
        // the z-component is always z regardless of GMST
        let b_pole = dipole
            .field_eci(&Vector3::new(0.0, 0.0, r), Some(&epoch))
            .magnitude();

        // For equatorial: at J2000 the ECEF x-axis is rotated by GMST from ECI x-axis.
        // The equatorial field perpendicular to the dipole axis has magnitude 1x.
        // We need a position perpendicular to the rotated z-axis (which is still z in ECI).
        // Any position in the x-y plane works.
        let b_eq = dipole
            .field_eci(&Vector3::new(r, 0.0, 0.0), Some(&epoch))
            .magnitude();

        let ratio = b_pole / b_eq;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Polar/equatorial ratio should be 2.0, got {ratio:.4}"
        );
    }

    #[test]
    fn zero_inside_earth_guard() {
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(0.5, 0.0, 0.0); // 0.5 km from centre
        let epoch = j2000_epoch();
        let b = dipole.field_eci(&pos, Some(&epoch));
        assert_eq!(b, Vector3::zeros());
    }

    #[test]
    fn zero_at_origin() {
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        let b = dipole.field_eci(&Vector3::zeros(), Some(&epoch));
        assert_eq!(b, Vector3::zeros());
    }

    #[test]
    fn field_is_finite() {
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(6778.0, 0.0, 0.0);
        let epoch = j2000_epoch();
        let b = dipole.field_eci(&pos, Some(&epoch));
        assert!(b.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn none_epoch_uses_ecef_axis_as_fallback() {
        // Without epoch, the ECEF axis is used as-is (ECI-fixed approximation)
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let b = dipole.field_eci(&pos, None);
        // Should produce a non-zero field (not silently disabled)
        assert!(b.magnitude() > 1e-10, "Expected non-zero field, got {b:?}");
    }

    #[test]
    fn field_rotates_with_epoch() {
        // The field at the same ECI position should change as the epoch changes
        // (because the dipole axis rotates with Earth)
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(7000.0, 0.0, 0.0);

        let epoch1 = Epoch::j2000();
        let epoch2 = Epoch::j2000().add_seconds(6.0 * 3600.0); // 6 hours later

        let b1 = dipole.field_eci(&pos, Some(&epoch1));
        let b2 = dipole.field_eci(&pos, Some(&epoch2));

        // The fields should be different (Earth has rotated ~90 deg in 6 hours)
        let diff = (b1 - b2).magnitude();
        assert!(
            diff > 1e-10,
            "Field should differ at different epochs, diff={diff:.3e}"
        );

        // But magnitudes should be similar (same distance from centre)
        let mag_ratio = b1.magnitude() / b2.magnitude();
        assert!(
            (mag_ratio - 1.0).abs() < 0.5,
            "Magnitudes should be similar, ratio={mag_ratio:.3}"
        );
    }
}
