use nalgebra::Vector3;

/// Tilted dipole geomagnetic field model with ECI-fixed axis.
///
/// Approximates Earth's magnetic field as a tilted dipole, suitable for
/// B-dot detumbling simulations in LEO. Does not account for Earth rotation
/// (adequate when attitude dynamics are much faster than orbital period).
///
/// The dipole field at position **r** is:
///
/// **B** = (dipole_strength / r³) [3(m̂·r̂)r̂ − m̂]
///
/// where m̂ is the dipole axis unit vector and r is in metres.
pub struct TiltedDipole {
    /// Dipole strength [T·m³] = μ₀m/(4π), absorbs μ₀/4π into the constant.
    dipole_strength: f64,
    /// Dipole axis unit vector in ECI.
    axis_eci: Vector3<f64>,
}

impl TiltedDipole {
    /// Create a tilted dipole with the given strength and axis.
    ///
    /// # Panics
    /// Panics if `axis_eci` is zero-length.
    pub fn new(dipole_strength: f64, axis_eci: Vector3<f64>) -> Self {
        let norm = axis_eci.magnitude();
        assert!(norm > 1e-15, "Dipole axis must be non-zero");
        Self {
            dipole_strength,
            axis_eci: axis_eci / norm,
        }
    }

    /// Earth's tilted dipole (IGRF approximate).
    ///
    /// - Dipole strength: ~7.94e15 T·m³ (= μ₀/(4π) × 7.94e22 A·m²)
    /// - Axis tilted ~11.5° from geographic north (simplified: tilt in x-z plane)
    pub fn earth() -> Self {
        let tilt = 11.5_f64.to_radians();
        Self {
            dipole_strength: crate::constants::EARTH_DIPOLE_STRENGTH,
            axis_eci: Vector3::new(tilt.sin(), 0.0, tilt.cos()).normalize(),
        }
    }

    /// Compute magnetic field vector in ECI [T] at position_eci [km].
    ///
    /// Returns the zero vector for positions inside 1 km from Earth's centre
    /// (guard against singularity).
    pub fn field_eci(&self, position_eci: &Vector3<f64>) -> Vector3<f64> {
        let r_km = position_eci.magnitude();
        if r_km < 1.0 {
            return Vector3::zeros();
        }

        // Convert km to m for the formula
        let r_m = r_km * 1000.0;
        let r3 = r_m * r_m * r_m;

        let r_hat = position_eci / r_km; // unit vector (same in km or m)
        let m_hat = &self.axis_eci;

        // B = dipole_strength * [3(m̂·r̂)r̂ − m̂] / r³
        let m_dot_r = m_hat.dot(&r_hat);
        self.dipole_strength * (3.0 * m_dot_r * r_hat - m_hat) / r3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equatorial_field_magnitude_at_leo() {
        // At equatorial LEO (7000 km from centre, on x-axis)
        // Expected |B| ~ 25-35 μT
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let b = dipole.field_eci(&pos);
        let b_mag = b.magnitude();

        // Convert to μT for readability
        let b_micro_t = b_mag * 1e6;
        assert!(
            b_micro_t > 20.0 && b_micro_t < 50.0,
            "Equatorial LEO field should be ~25-35 μT, got {b_micro_t:.2} μT"
        );
    }

    #[test]
    fn inverse_cube_scaling() {
        // B ∝ 1/r³: at double distance, field should be 1/8
        let dipole = TiltedDipole::earth();
        let pos1 = Vector3::new(7000.0, 0.0, 0.0);
        let pos2 = Vector3::new(14000.0, 0.0, 0.0);
        let b1 = dipole.field_eci(&pos1).magnitude();
        let b2 = dipole.field_eci(&pos2).magnitude();

        let ratio = b1 / b2;
        assert!(
            (ratio - 8.0).abs() < 0.01,
            "Expected 1/r³ scaling (ratio 8.0), got {ratio:.4}"
        );
    }

    #[test]
    fn polar_field_stronger_than_equatorial() {
        // Along the dipole axis the field is 2x the equatorial field at the same distance.
        // For a pure dipole with axis along ẑ:
        //   B_pole   = dipole_strength * 2 / r³  (along axis, m̂·r̂ = 1 → 3·1·r̂ - m̂ = 2m̂)
        //   B_equator = dipole_strength * 1 / r³  (perpendicular, m̂·r̂ = 0 → -m̂, magnitude 1)
        //
        // Our axis is tilted, so use a simpler dipole for this test.
        let dipole = TiltedDipole::new(7.94e15, Vector3::new(0.0, 0.0, 1.0));
        let r = 7000.0;

        let b_pole = dipole.field_eci(&Vector3::new(0.0, 0.0, r)).magnitude();
        let b_eq = dipole.field_eci(&Vector3::new(r, 0.0, 0.0)).magnitude();

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
        let b = dipole.field_eci(&pos);
        assert_eq!(b, Vector3::zeros());
    }

    #[test]
    fn zero_at_origin() {
        let dipole = TiltedDipole::earth();
        let b = dipole.field_eci(&Vector3::zeros());
        assert_eq!(b, Vector3::zeros());
    }

    #[test]
    fn field_is_finite() {
        let dipole = TiltedDipole::earth();
        let pos = Vector3::new(6778.0, 0.0, 0.0);
        let b = dipole.field_eci(&pos);
        assert!(b.iter().all(|v| v.is_finite()));
    }
}
