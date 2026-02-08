use nalgebra::Vector3;

use crate::epoch::Epoch;

/// Approximate sun direction (unit vector) in ECI (J2000) frame.
///
/// Uses a low-precision analytical model based on mean orbital elements.
/// Accuracy is ~1 arcminute, sufficient for visualization purposes.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
pub fn sun_direction_eci(epoch: &Epoch) -> Vector3<f64> {
    let t = epoch.centuries_since_j2000();

    // Mean longitude (degrees)
    let l0 = 280.46646 + 36000.76983 * t;
    // Mean anomaly (degrees)
    let m_deg = 357.52911 + 35999.05029 * t;
    let m = m_deg.to_radians();

    // Equation of center (degrees)
    let c = (1.9146 - 0.004817 * t) * m.sin() + 0.019993 * (2.0 * m).sin();

    // Sun's ecliptic longitude (degrees → radians)
    let lambda = (l0 + c).to_radians();

    // Obliquity of the ecliptic (degrees → radians)
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    // Sun direction in ECI (equatorial coordinates)
    let x = lambda.cos();
    let y = epsilon.cos() * lambda.sin();
    let z = epsilon.sin() * lambda.sin();

    Vector3::new(x, y, z).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_direction_is_unit_vector() {
        // Check at several dates across a year
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 12, 21, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let dir = sun_direction_eci(epoch);
            let norm = dir.norm();
            assert!(
                (norm - 1.0).abs() < 1e-10,
                "Not unit vector at JD {}: norm = {norm}",
                epoch.jd()
            );
        }
    }

    #[test]
    fn march_equinox_sun_near_plus_x() {
        // At March equinox (~2024-03-20), sun is near +X direction (RA ≈ 0°)
        let epoch = Epoch::from_gregorian(2024, 3, 20, 3, 6, 0.0); // ~03:06 UTC is 2024 equinox
        let dir = sun_direction_eci(&epoch);

        // X should be dominant and positive
        assert!(
            dir.x > 0.9,
            "March equinox: x={:.3} should be > 0.9",
            dir.x
        );
        // Y and Z should be small
        assert!(
            dir.y.abs() < 0.2,
            "March equinox: y={:.3} should be near 0",
            dir.y
        );
        assert!(
            dir.z.abs() < 0.1,
            "March equinox: z={:.3} should be near 0",
            dir.z
        );
    }

    #[test]
    fn june_solstice_sun_positive_z() {
        // At June solstice (~2024-06-20), sun has significant +Z (northern declination ~23.4°)
        let epoch = Epoch::from_gregorian(2024, 6, 20, 20, 51, 0.0);
        let dir = sun_direction_eci(&epoch);

        // Z should be positive and near sin(23.44°) ≈ 0.398
        assert!(
            dir.z > 0.35,
            "June solstice: z={:.3} should be > 0.35",
            dir.z
        );
        // X should be near 0 (RA ≈ 90°)
        assert!(
            dir.x.abs() < 0.15,
            "June solstice: x={:.3} should be near 0",
            dir.x
        );
        // Y should be dominant and positive
        assert!(
            dir.y > 0.85,
            "June solstice: y={:.3} should be > 0.85",
            dir.y
        );
    }

    #[test]
    fn september_equinox_sun_near_minus_x() {
        // At September equinox (~2024-09-22), sun is near -X direction (RA ≈ 180°)
        let epoch = Epoch::from_gregorian(2024, 9, 22, 12, 44, 0.0);
        let dir = sun_direction_eci(&epoch);

        // X should be dominant and negative
        assert!(
            dir.x < -0.9,
            "September equinox: x={:.3} should be < -0.9",
            dir.x
        );
        // Y and Z should be small
        assert!(
            dir.y.abs() < 0.2,
            "September equinox: y={:.3} should be near 0",
            dir.y
        );
        assert!(
            dir.z.abs() < 0.1,
            "September equinox: z={:.3} should be near 0",
            dir.z
        );
    }

    #[test]
    fn december_solstice_sun_negative_z() {
        // At December solstice (~2024-12-21), sun has significant -Z (southern declination ~-23.4°)
        let epoch = Epoch::from_gregorian(2024, 12, 21, 9, 21, 0.0);
        let dir = sun_direction_eci(&epoch);

        // Z should be negative and near -sin(23.44°) ≈ -0.398
        assert!(
            dir.z < -0.35,
            "December solstice: z={:.3} should be < -0.35",
            dir.z
        );
        // Y should be negative (RA ≈ 270°)
        assert!(
            dir.y < -0.85,
            "December solstice: y={:.3} should be < -0.85",
            dir.y
        );
    }

    #[test]
    fn sun_direction_varies_over_year() {
        // Verify the sun position actually changes throughout the year
        let epoch1 = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 7, 1, 12, 0, 0.0);
        let dir1 = sun_direction_eci(&epoch1);
        let dir2 = sun_direction_eci(&epoch2);

        // Should be significantly different (roughly opposite)
        let dot = dir1.dot(&dir2);
        assert!(
            dot < 0.0,
            "Jan vs Jul sun directions should be roughly opposite, dot={dot:.3}"
        );
    }
}
