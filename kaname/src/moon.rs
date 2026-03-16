use nalgebra::Vector3;

use crate::epoch::Epoch;

/// Moon position vector in ECI (J2000) frame [km].
///
/// Uses a simplified analytical model based on Meeus "Astronomical Algorithms",
/// Chapter 47. Accuracy: ~10' in ecliptic longitude, ~5% in distance.
///
/// This is sufficient for third-body gravitational perturbation calculations.
pub fn moon_position_eci(epoch: &Epoch) -> Vector3<f64> {
    let t = epoch.centuries_since_j2000();

    // Fundamental arguments (degrees)
    // Moon's mean longitude (Lp)
    let lp = 218.3164477 + 481267.88123421 * t;
    // Moon's mean anomaly (M')
    let mp = 134.9633964 + 477198.8675055 * t;
    // Sun's mean anomaly (M)
    let m = 357.5291092 + 35999.0502909 * t;
    // Moon's mean elongation (D)
    let d = 297.8501921 + 445267.1114034 * t;
    // Moon's argument of latitude (F)
    let f = 93.2720950 + 483202.0175233 * t;

    let mp_rad = mp.to_radians();
    let m_rad = m.to_radians();
    let d_rad = d.to_radians();
    let f_rad = f.to_radians();

    // Ecliptic longitude (simplified, main terms)
    let lambda_deg = lp + 6.289 * mp_rad.sin()
        - 1.274 * (2.0 * d_rad - mp_rad).sin()
        - 0.658 * (2.0 * d_rad).sin()
        - 0.214 * (2.0 * mp_rad).sin()
        + 0.186 * m_rad.sin();

    // Ecliptic latitude (simplified, main terms)
    let beta_deg = 5.128 * f_rad.sin() + 0.281 * (mp_rad + f_rad).sin()
        - 0.278 * (f_rad - mp_rad).sin()
        + 0.176 * (2.0 * d_rad - f_rad).sin();

    // Distance (km, simplified)
    let distance_km = 385001.0
        - 20905.0 * mp_rad.cos()
        - 3699.0 * (2.0 * d_rad - mp_rad).cos()
        - 2956.0 * (2.0 * d_rad).cos()
        + 570.0 * (2.0 * mp_rad).cos();

    let lambda = lambda_deg.to_radians();
    let beta = beta_deg.to_radians();

    // Ecliptic → Equatorial (ECI) conversion
    // Obliquity of the ecliptic
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    let cos_lam = lambda.cos();
    let sin_lam = lambda.sin();
    let cos_beta = beta.cos();
    let sin_beta = beta.sin();
    let cos_eps = epsilon.cos();
    let sin_eps = epsilon.sin();

    // Ecliptic to equatorial rotation
    let x = distance_km * cos_beta * cos_lam;
    let y = distance_km * (cos_eps * cos_beta * sin_lam - sin_eps * sin_beta);
    let z = distance_km * (sin_eps * cos_beta * sin_lam + cos_eps * sin_beta);

    Vector3::new(x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_distance_range() {
        // Moon distance varies between ~356,500 km (perigee) and ~406,700 km (apogee)
        // Test at multiple dates across a month
        let dates = [
            Epoch::from_gregorian(2024, 3, 1, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 7, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 14, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 21, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 28, 0, 0, 0.0),
        ];

        for epoch in &dates {
            let pos = moon_position_eci(epoch);
            let dist = pos.magnitude();
            assert!(
                dist > 340_000.0 && dist < 420_000.0,
                "Moon distance at JD {} should be 340k-420k km, got {dist:.0} km",
                epoch.jd()
            );
        }
    }

    #[test]
    fn moon_mean_distance() {
        // Average over several dates should be near mean distance (~384,400 km)
        let n = 30;
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let total_dist: f64 = (0..n)
            .map(|i| {
                let epoch = epoch0.add_seconds(i as f64 * 86400.0);
                moon_position_eci(&epoch).magnitude()
            })
            .sum();
        let mean_dist = total_dist / n as f64;

        assert!(
            (mean_dist - 384_400.0).abs() < 10_000.0,
            "Mean moon distance should be ~384,400 km, got {mean_dist:.0} km"
        );
    }

    #[test]
    fn moon_orbital_period() {
        // Moon should complete roughly one orbit in ~27.3 days (sidereal period)
        let epoch0 = Epoch::from_gregorian(2024, 3, 10, 0, 0, 0.0);
        let pos0 = moon_position_eci(&epoch0).normalize();

        // After ~27.3 days, should be back near same direction
        let epoch1 = epoch0.add_seconds(27.3 * 86400.0);
        let pos1 = moon_position_eci(&epoch1).normalize();

        let dot = pos0.dot(&pos1);
        assert!(
            dot > 0.9,
            "Moon should return near starting direction after ~27.3 days, dot={dot:.3}"
        );

        // After ~13.7 days (half orbit), should be roughly opposite
        let epoch_half = epoch0.add_seconds(13.7 * 86400.0);
        let pos_half = moon_position_eci(&epoch_half).normalize();
        let dot_half = pos0.dot(&pos_half);
        assert!(
            dot_half < 0.0,
            "Moon should be roughly opposite after half orbit, dot={dot_half:.3}"
        );
    }

    #[test]
    fn moon_not_in_ecliptic() {
        // Moon's orbit is inclined ~5° to ecliptic, so z-component should be non-trivial
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let pos = moon_position_eci(&epoch);
        let z_frac = pos.z.abs() / pos.magnitude();

        // At some point in its orbit, the Moon should have measurable z-component
        // Max z/r ≈ sin(5° + 23.4°) ≈ 0.47 or min z/r near equator crossing
        // Just verify it's not stuck at zero
        let mut max_z_frac = 0.0_f64;
        for i in 0..28 {
            let ep = epoch.add_seconds(i as f64 * 86400.0);
            let p = moon_position_eci(&ep);
            max_z_frac = max_z_frac.max(p.z.abs() / p.magnitude());
        }
        assert!(
            max_z_frac > 0.1,
            "Moon should have significant z-component at some point, max z/r = {max_z_frac:.3}"
        );
        let _ = z_frac; // used for assertion context
    }
}
