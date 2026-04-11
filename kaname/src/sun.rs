use nalgebra::Vector3;

use crate::epoch::Epoch;
use crate::frame::{self, Vec3};
use crate::planets;

/// Solar orbital elements at epoch.
///
/// Intermediate values used by both `sun_direction_eci` and `equation_of_time`.
struct SolarElements {
    /// Mean longitude [degrees] (not normalized to 0-360)
    l0_deg: f64,
    /// Ecliptic longitude [radians]
    lambda_rad: f64,
    /// Obliquity of the ecliptic [radians]
    epsilon_rad: f64,
}

/// Compute solar orbital elements at the given epoch.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
///
/// # Time scale
///
/// Meeus ephemerides take a dynamical time argument (TDB). The public signature
/// accepts `&Epoch<Utc>` (the default alias) for backward compatibility; the
/// UTC epoch is converted to TDB internally via leap seconds + TT offset +
/// Fairhead-Bretagnon periodic correction.
fn solar_elements(epoch: &Epoch) -> SolarElements {
    let t = epoch.to_tdb().centuries_since_j2000();

    // Mean longitude (degrees)
    let l0 = 280.46646 + 36000.76983 * t;
    // Mean anomaly (degrees)
    let m_deg = 357.52911 + 35999.05029 * t;
    let m = m_deg.to_radians();

    // Equation of center (degrees)
    let c = (1.9146 - 0.004817 * t) * m.sin() + 0.019993 * (2.0 * m).sin();

    // Sun's ecliptic longitude (radians)
    let lambda = (l0 + c).to_radians();

    // Obliquity of the ecliptic
    let epsilon = planets::obliquity(epoch);

    SolarElements {
        l0_deg: l0,
        lambda_rad: lambda,
        epsilon_rad: epsilon,
    }
}

/// Approximate sun direction (unit vector) in ECI (J2000) frame.
///
/// Uses a low-precision analytical model based on mean orbital elements.
/// Accuracy is ~1 arcminute, sufficient for visualization purposes.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
pub fn sun_direction_eci(epoch: &Epoch) -> Vec3<frame::Eci> {
    let el = solar_elements(epoch);

    // Sun direction in ECI (equatorial coordinates)
    let x = el.lambda_rad.cos();
    let y = el.epsilon_rad.cos() * el.lambda_rad.sin();
    let z = el.epsilon_rad.sin() * el.lambda_rad.sin();

    Vec3::from_raw(Vector3::new(x, y, z).normalize())
}

/// Equation of Time [hours].
///
/// Returns `apparent_solar_time - mean_solar_time`.
/// Positive means the apparent Sun is ahead of the mean Sun.
///
/// Range: approximately -0.27 to +0.27 hours (-16 to +16 minutes).
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 28.
pub fn equation_of_time(epoch: &Epoch) -> f64 {
    let el = solar_elements(epoch);

    // Right ascension from ecliptic longitude
    let alpha_rad = f64::atan2(
        el.epsilon_rad.cos() * el.lambda_rad.sin(),
        el.lambda_rad.cos(),
    );

    // EoT = L₀ - α (apparent - mean), then convert to hours
    // Positive in November (sundial fast), negative in February (sundial slow).
    let l0_rad = el.l0_deg.to_radians();
    let mut eot_rad = l0_rad - alpha_rad;

    // Normalize to [-π, π]
    eot_rad = ((eot_rad + std::f64::consts::PI) % std::f64::consts::TAU + std::f64::consts::TAU)
        % std::f64::consts::TAU
        - std::f64::consts::PI;

    // Convert radians to hours: 2π rad = 24 hours
    eot_rad * 24.0 / std::f64::consts::TAU
}

/// 1 Astronomical Unit in km.
pub const AU_KM: f64 = 149_597_870.7;

/// Sun-Earth distance [km] at the given epoch.
///
/// Uses simplified Meeus model with eccentricity correction.
/// Accuracy: ~0.01 AU (~1.5 million km), sufficient for perturbation calculations.
///
/// Reference: Meeus, "Astronomical Algorithms", Chapter 25.
pub fn sun_distance_km(epoch: &Epoch) -> f64 {
    // Meeus ephemeris uses TDB dynamical time; convert UTC → TDB internally.
    let t = epoch.to_tdb().centuries_since_j2000();

    let m_deg = 357.52911 + 35999.05029 * t;
    let m = m_deg.to_radians();

    // Distance in AU (Meeus Eq. 25.5)
    let r_au = 1.000_140_12 - 0.016_708_17 * m.cos() - 0.000_139_89 * (2.0 * m).cos();

    r_au * AU_KM
}

/// Sun position vector in ECI (J2000) frame [km].
///
/// Returns the geocentric position of the Sun. Combines direction and distance.
pub fn sun_position_eci(epoch: &Epoch) -> Vec3<frame::Eci> {
    let direction = sun_direction_eci(epoch);
    let distance = sun_distance_km(epoch);
    direction * distance
}

/// Sun distance [km] from a given central body.
///
/// - `"earth"` / `"moon"`: delegates to [`sun_distance_km`]
/// - Other known planets: computed from heliocentric orbital elements
/// - Unknown bodies: fallback to Earth-Sun distance
pub fn sun_distance_from_body(body: &str, epoch: &Epoch) -> f64 {
    match body {
        "earth" | "moon" => sun_distance_km(epoch),
        _ => planets::heliocentric_position_ecliptic(body, epoch)
            .map(|p| p.magnitude())
            .unwrap_or_else(|| sun_distance_km(epoch)),
    }
}

/// Sun direction (unit vector) as seen from a given central body, in J2000 equatorial frame.
///
/// - `"earth"` / `"moon"`: delegates to [`sun_direction_eci`] (Moon parallax < 0.15°, negligible)
/// - Other known planets: computed from heliocentric orbital elements
/// - Unknown bodies: fallback to +X direction (vernal equinox)
///
/// The returned vector points FROM the body TOWARD the Sun.
pub fn sun_direction_from_body(body: &str, epoch: &Epoch) -> Vec3<frame::Eci> {
    match body {
        "earth" | "moon" => sun_direction_eci(epoch),
        _ => {
            if let Some(body_pos_ecl) = planets::heliocentric_position_ecliptic(body, epoch) {
                // Sun is at origin in heliocentric frame, so direction to sun = -body_pos
                let sun_dir_ecl = -body_pos_ecl;
                let epsilon = planets::obliquity(epoch);
                Vec3::from_raw(planets::ecliptic_to_equatorial(&sun_dir_ecl, epsilon).normalize())
            } else {
                // Unknown body: fallback to +X (vernal equinox direction)
                Vec3::new(1.0, 0.0, 0.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Equation of Time tests ---

    #[test]
    fn eot_february_negative() {
        // Mid-February: EoT ≈ -14 minutes (sundial slow, apparent sun behind mean sun)
        let epoch = Epoch::from_gregorian(2024, 2, 12, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch) * 60.0;
        assert!(
            (eot_min - (-14.0)).abs() < 2.0,
            "Feb 12: EoT={eot_min:.1} min, expected ~-14"
        );
    }

    #[test]
    fn eot_november_positive() {
        // Early November: EoT ≈ +16 minutes (sundial fast, apparent sun ahead of mean sun)
        let epoch = Epoch::from_gregorian(2024, 11, 3, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch) * 60.0;
        assert!(
            (eot_min - 16.0).abs() < 2.0,
            "Nov 3: EoT={eot_min:.1} min, expected ~+16"
        );
    }

    #[test]
    fn eot_april_near_zero() {
        // Mid-April: EoT ≈ 0 minutes (one of the four zero-crossings)
        let epoch = Epoch::from_gregorian(2024, 4, 15, 12, 0, 0.0);
        let eot_min = equation_of_time(&epoch) * 60.0;
        assert!(
            eot_min.abs() < 2.0,
            "Apr 15: EoT={eot_min:.1} min, expected ~0"
        );
    }

    #[test]
    fn eot_annual_range() {
        // EoT should stay within ±17 minutes throughout the year
        for month in 1..=12 {
            let epoch = Epoch::from_gregorian(2024, month, 15, 12, 0, 0.0);
            let eot_min = equation_of_time(&epoch) * 60.0;
            assert!(
                eot_min.abs() < 17.0,
                "Month {month}: EoT={eot_min:.1} min, out of range"
            );
        }
    }

    // --- Sun direction tests ---

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
            let norm = dir.magnitude();
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
            dir.x() > 0.9,
            "March equinox: x={:.3} should be > 0.9",
            dir.x()
        );
        // Y and Z should be small
        assert!(
            dir.y().abs() < 0.2,
            "March equinox: y={:.3} should be near 0",
            dir.y()
        );
        assert!(
            dir.z().abs() < 0.1,
            "March equinox: z={:.3} should be near 0",
            dir.z()
        );
    }

    #[test]
    fn june_solstice_sun_positive_z() {
        // At June solstice (~2024-06-20), sun has significant +Z (northern declination ~23.4°)
        let epoch = Epoch::from_gregorian(2024, 6, 20, 20, 51, 0.0);
        let dir = sun_direction_eci(&epoch);

        // Z should be positive and near sin(23.44°) ≈ 0.398
        assert!(
            dir.z() > 0.35,
            "June solstice: z={:.3} should be > 0.35",
            dir.z()
        );
        // X should be near 0 (RA ≈ 90°)
        assert!(
            dir.x().abs() < 0.15,
            "June solstice: x={:.3} should be near 0",
            dir.x()
        );
        // Y should be dominant and positive
        assert!(
            dir.y() > 0.85,
            "June solstice: y={:.3} should be > 0.85",
            dir.y()
        );
    }

    #[test]
    fn september_equinox_sun_near_minus_x() {
        // At September equinox (~2024-09-22), sun is near -X direction (RA ≈ 180°)
        let epoch = Epoch::from_gregorian(2024, 9, 22, 12, 44, 0.0);
        let dir = sun_direction_eci(&epoch);

        // X should be dominant and negative
        assert!(
            dir.x() < -0.9,
            "September equinox: x={:.3} should be < -0.9",
            dir.x()
        );
        // Y and Z should be small
        assert!(
            dir.y().abs() < 0.2,
            "September equinox: y={:.3} should be near 0",
            dir.y()
        );
        assert!(
            dir.z().abs() < 0.1,
            "September equinox: z={:.3} should be near 0",
            dir.z()
        );
    }

    #[test]
    fn december_solstice_sun_negative_z() {
        // At December solstice (~2024-12-21), sun has significant -Z (southern declination ~-23.4°)
        let epoch = Epoch::from_gregorian(2024, 12, 21, 9, 21, 0.0);
        let dir = sun_direction_eci(&epoch);

        // Z should be negative and near -sin(23.44°) ≈ -0.398
        assert!(
            dir.z() < -0.35,
            "December solstice: z={:.3} should be < -0.35",
            dir.z()
        );
        // Y should be negative (RA ≈ 270°)
        assert!(
            dir.y() < -0.85,
            "December solstice: y={:.3} should be < -0.85",
            dir.y()
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

    // --- Sun distance tests ---

    #[test]
    fn sun_distance_approximately_1au() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let d = sun_distance_km(&epoch);
        let d_au = d / AU_KM;
        assert!(
            (d_au - 1.0).abs() < 0.02,
            "Sun distance should be ~1 AU, got {d_au:.4} AU"
        );
    }

    #[test]
    fn perihelion_closer_than_aphelion() {
        // Perihelion ~Jan 3, Aphelion ~Jul 4
        let perihelion = Epoch::from_gregorian(2024, 1, 3, 12, 0, 0.0);
        let aphelion = Epoch::from_gregorian(2024, 7, 5, 12, 0, 0.0);

        let d_peri = sun_distance_km(&perihelion);
        let d_aph = sun_distance_km(&aphelion);

        assert!(
            d_peri < d_aph,
            "Perihelion ({d_peri:.0} km) should be closer than aphelion ({d_aph:.0} km)"
        );
        // Eccentricity ~0.0167, so difference should be ~3.3%
        let ratio = d_aph / d_peri;
        assert!(
            (ratio - 1.034).abs() < 0.01,
            "Aphelion/perihelion ratio should be ~1.034, got {ratio:.4}"
        );
    }

    #[test]
    fn sun_position_magnitude_matches_distance() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let pos = sun_position_eci(&epoch);
        let dist = sun_distance_km(&epoch);

        let rel_err = (pos.magnitude() - dist).abs() / dist;
        assert!(
            rel_err < 1e-10,
            "Position magnitude should match distance, rel_err={rel_err:.6e}"
        );
    }

    // --- sun_direction_from_body tests ---

    #[test]
    fn sun_direction_from_body_earth_matches_eci() {
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let from_body = sun_direction_from_body("earth", epoch);
            let eci = sun_direction_eci(epoch);
            let diff = (from_body - eci).magnitude();
            assert!(
                diff < 1e-10,
                "earth should match sun_direction_eci, diff={diff:.2e}"
            );
        }
    }

    #[test]
    fn sun_direction_from_body_moon_matches_eci() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let from_body = sun_direction_from_body("moon", &epoch);
        let eci = sun_direction_eci(&epoch);
        let diff = (from_body - eci).magnitude();
        assert!(
            diff < 1e-10,
            "moon should match sun_direction_eci, diff={diff:.2e}"
        );
    }

    #[test]
    fn sun_direction_from_body_mars_is_unit_vector() {
        let dates = [
            Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0),
            Epoch::from_gregorian(2024, 12, 1, 12, 0, 0.0),
        ];
        for epoch in &dates {
            let dir = sun_direction_from_body("mars", epoch);
            let norm = dir.magnitude();
            assert!(
                (norm - 1.0).abs() < 1e-10,
                "Mars sun direction should be unit vector, norm={norm}"
            );
        }
    }

    #[test]
    fn sun_direction_from_body_mars_varies() {
        let epoch1 = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 7, 1, 12, 0, 0.0);
        let dir1 = sun_direction_from_body("mars", &epoch1);
        let dir2 = sun_direction_from_body("mars", &epoch2);
        let dot = dir1.dot(&dir2);
        assert!(
            dot < 0.9,
            "Mars sun direction should change significantly over 6 months, dot={dot:.3}"
        );
    }

    #[test]
    fn sun_direction_from_body_unknown_fallback() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let dir = sun_direction_from_body("pluto", &epoch);
        assert!(
            (dir.x() - 1.0).abs() < 1e-10 && dir.y().abs() < 1e-10 && dir.z().abs() < 1e-10,
            "Unknown body should return +X fallback, got ({}, {}, {})",
            dir.x(),
            dir.y(),
            dir.z()
        );
    }

    // --- sun_distance_from_body tests ---

    #[test]
    fn sun_distance_from_body_earth_matches() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let from_body = sun_distance_from_body("earth", &epoch);
        let direct = sun_distance_km(&epoch);
        assert!(
            (from_body - direct).abs() < 1.0,
            "earth distance should match sun_distance_km: {from_body} vs {direct}"
        );
    }

    #[test]
    fn sun_distance_from_body_mars() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let dist = sun_distance_from_body("mars", &epoch);
        let dist_au = dist / AU_KM;
        assert!(
            dist_au > 1.3 && dist_au < 1.7,
            "Mars-Sun distance should be 1.3-1.7 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn sun_distance_from_body_jupiter() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let dist = sun_distance_from_body("jupiter", &epoch);
        let dist_au = dist / AU_KM;
        assert!(
            dist_au > 4.5 && dist_au < 5.8,
            "Jupiter-Sun distance should be 4.5-5.8 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn sun_distance_from_body_unknown_fallback() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        let dist = sun_distance_from_body("pluto", &epoch);
        let earth_dist = sun_distance_km(&epoch);
        assert!(
            (dist - earth_dist).abs() < 1.0,
            "Unknown body should fall back to Earth distance"
        );
    }

    #[test]
    fn sun_position_direction_matches() {
        let epoch = Epoch::from_gregorian(2024, 9, 22, 12, 0, 0.0);
        let pos = sun_position_eci(&epoch);
        let dir = sun_direction_eci(&epoch);

        let pos_dir = pos.normalize();
        let diff = (pos_dir - dir).magnitude();
        assert!(
            diff < 1e-10,
            "Position direction should match unit direction, diff={diff:.6e}"
        );
    }
}
