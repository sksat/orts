use nalgebra::Vector3;

use crate::epoch::Epoch;
use crate::sun::AU_KM;

/// Heliocentric orbital elements at J2000.0 and their century rates.
///
/// Reference: Standish (1992), Meeus "Astronomical Algorithms" Table 31.A.
/// Valid for ~3000 BC to 3000 AD with ~1 arcminute accuracy for inner planets.
struct OrbitalElements {
    /// Mean longitude at J2000 [deg]
    l0: f64,
    /// Mean longitude rate [deg/century]
    l_rate: f64,
    /// Semi-major axis [AU]
    a: f64,
    /// Eccentricity at J2000
    e0: f64,
    /// Eccentricity rate [per century]
    e_rate: f64,
    /// Inclination at J2000 [deg]
    i0: f64,
    /// Inclination rate [deg/century]
    i_rate: f64,
    /// Longitude of ascending node at J2000 [deg]
    omega0: f64,
    /// Node rate [deg/century]
    omega_rate: f64,
    /// Longitude of perihelion at J2000 [deg]
    pi0: f64,
    /// Perihelion longitude rate [deg/century]
    pi_rate: f64,
}

// Standish (1992) / Meeus Table 31.A elements

const MERCURY: OrbitalElements = OrbitalElements {
    l0: 252.250_84,
    l_rate: 149_472.674_11,
    a: 0.387_10,
    e0: 0.205_63,
    e_rate: 0.000_020_527,
    i0: 7.004_97,
    i_rate: -0.005_90,
    omega0: 48.331_67,
    omega_rate: -0.125_34,
    pi0: 77.456_45,
    pi_rate: 0.159_40,
};

const VENUS: OrbitalElements = OrbitalElements {
    l0: 181.979_73,
    l_rate: 58_517.815_39,
    a: 0.723_33,
    e0: 0.006_77,
    e_rate: -0.000_047_765,
    i0: 3.394_67,
    i_rate: -0.000_78,
    omega0: 76.680_69,
    omega_rate: -0.278_70,
    pi0: 131.532_98,
    pi_rate: 0.004_75,
};

const EARTH: OrbitalElements = OrbitalElements {
    l0: 100.464_57,
    l_rate: 35_999.372_45,
    a: 1.000_00,
    e0: 0.016_71,
    e_rate: -0.000_042_037,
    i0: 0.000_05,
    i_rate: -0.012_94,
    omega0: -11.260_64,
    omega_rate: -0.181_75,
    pi0: 102.937_68,
    pi_rate: 0.323_27,
};

const MARS: OrbitalElements = OrbitalElements {
    l0: 355.453_32,
    l_rate: 19_140.302_68,
    a: 1.523_68,
    e0: 0.093_40,
    e_rate: 0.000_090_484,
    i0: 1.849_69,
    i_rate: -0.008_13,
    omega0: 49.559_54,
    omega_rate: -0.292_58,
    pi0: 336.040_84,
    pi_rate: 0.443_23,
};

const JUPITER: OrbitalElements = OrbitalElements {
    l0: 34.404_38,
    l_rate: 3_034.746_13,
    a: 5.202_60,
    e0: 0.048_49,
    e_rate: 0.000_163_225,
    i0: 1.303_27,
    i_rate: -0.019_53,
    omega0: 100.556_15,
    omega_rate: 0.205_14,
    pi0: 14.753_85,
    pi_rate: 0.212_52,
};

const SATURN: OrbitalElements = OrbitalElements {
    l0: 49.944_32,
    l_rate: 1_222.493_62,
    a: 9.554_91,
    e0: 0.055_51,
    e_rate: -0.000_346_057,
    i0: 2.488_88,
    i_rate: 0.002_58,
    omega0: 113.715_04,
    omega_rate: -0.250_68,
    pi0: 92.431_94,
    pi_rate: 0.546_09,
};

/// Solve Kepler's equation M = E - e*sin(E) for eccentric anomaly E.
///
/// Uses Newton-Raphson iteration. Convergence is rapid for e < 0.9.
/// For the planets (e ≤ 0.21 for Mercury), this converges in 3-5 iterations.
fn solve_kepler(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let m = mean_anomaly;
    let e = eccentricity;

    // Initial guess (good for e < 0.4)
    let mut ea = m + e * m.sin();

    for _ in 0..15 {
        let delta = (ea - e * ea.sin() - m) / (1.0 - e * ea.cos());
        ea -= delta;
        if delta.abs() < 1e-12 {
            break;
        }
    }
    ea
}

/// Mean obliquity of the ecliptic at epoch [radians].
///
/// Reference: Meeus, "Astronomical Algorithms".
pub fn obliquity(epoch: &Epoch) -> f64 {
    let t = epoch.centuries_since_j2000();
    (23.439_291 - 0.013_004_2 * t).to_radians()
}

/// Rotate a vector from ecliptic to J2000 equatorial frame.
///
/// This is a rotation around the X-axis by the obliquity angle ε.
pub fn ecliptic_to_equatorial(v: &Vector3<f64>, epsilon: f64) -> Vector3<f64> {
    let cos_eps = epsilon.cos();
    let sin_eps = epsilon.sin();
    Vector3::new(
        v.x,
        cos_eps * v.y - sin_eps * v.z,
        sin_eps * v.y + cos_eps * v.z,
    )
}

/// Compute heliocentric position of a planet in the ecliptic frame [km].
///
/// Returns `None` if the body is not a recognized planet.
/// Accuracy: ~1 arcminute for inner planets, sufficient for sun-direction lighting.
pub fn heliocentric_position_ecliptic(body: &str, epoch: &Epoch) -> Option<Vector3<f64>> {
    let elements = match body {
        "mercury" => &MERCURY,
        "venus" => &VENUS,
        "earth" => &EARTH,
        "mars" => &MARS,
        "jupiter" => &JUPITER,
        "saturn" => &SATURN,
        _ => return None,
    };

    let t = epoch.centuries_since_j2000();

    // Compute elements at epoch
    let l = (elements.l0 + elements.l_rate * t).to_radians();
    let e = elements.e0 + elements.e_rate * t;
    let i = (elements.i0 + elements.i_rate * t).to_radians();
    let omega = (elements.omega0 + elements.omega_rate * t).to_radians(); // ascending node
    let pi_lon = (elements.pi0 + elements.pi_rate * t).to_radians(); // longitude of perihelion

    // Mean anomaly
    let m = l - pi_lon;

    // Solve Kepler's equation
    let ea = solve_kepler(m, e);

    // True anomaly
    let nu = 2.0 * ((1.0 + e).sqrt() * (ea / 2.0).sin()).atan2((1.0 - e).sqrt() * (ea / 2.0).cos());

    // Heliocentric distance [AU]
    let r = elements.a * (1.0 - e * ea.cos());

    // Argument of perihelion
    let omega_peri = pi_lon - omega;

    // Argument of latitude (angle from ascending node)
    let u = omega_peri + nu;

    // Position in ecliptic coordinates [AU]
    let x = r * (omega.cos() * u.cos() - omega.sin() * u.sin() * i.cos());
    let y = r * (omega.sin() * u.cos() + omega.cos() * u.sin() * i.cos());
    let z = r * (u.sin() * i.sin());

    // Convert AU to km
    Some(Vector3::new(x, y, z) * AU_KM)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Kepler solver tests ---

    #[test]
    fn kepler_circular_orbit() {
        // For e=0, E = M
        let m = 1.5;
        let e = solve_kepler(m, 0.0);
        assert!(
            (e - m).abs() < 1e-12,
            "Circular orbit: E should equal M, got E={e}, M={m}"
        );
    }

    #[test]
    fn kepler_symmetric_solution() {
        // For any e, M=π → E=π (by symmetry: π - e*sin(π) = π)
        let e_vals = [0.0, 0.1, 0.2, 0.5, 0.9];
        for &ecc in &e_vals {
            let ea = solve_kepler(std::f64::consts::PI, ecc);
            assert!(
                (ea - std::f64::consts::PI).abs() < 1e-10,
                "M=π, e={ecc}: E should be π, got {ea}"
            );
        }
    }

    #[test]
    fn kepler_convergence() {
        // Verify M = E - e*sin(E) holds for various (M, e) pairs
        let test_cases = [
            (0.5, 0.1),
            (1.0, 0.2),
            (2.0, 0.05),
            (3.0, 0.2),
            (0.1, 0.2),
            (5.0, 0.1),
        ];
        for &(m, ecc) in &test_cases {
            let ea = solve_kepler(m, ecc);
            let residual = (ea - ecc * ea.sin() - m).abs();
            assert!(residual < 1e-12, "M={m}, e={ecc}: residual={residual:.2e}");
        }
    }

    #[test]
    fn kepler_mercury_eccentricity() {
        // Mercury has the highest eccentricity (~0.2056) among the planets
        let m = 1.234;
        let e = 0.2056;
        let ea = solve_kepler(m, e);
        let residual = (ea - e * ea.sin() - m).abs();
        assert!(
            residual < 1e-12,
            "Mercury-like e={e}: residual={residual:.2e}"
        );
    }

    // --- Heliocentric position tests ---

    #[test]
    fn earth_at_1au() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let pos = heliocentric_position_ecliptic("earth", &epoch).unwrap();
        let dist_au = pos.magnitude() / AU_KM;
        assert!(
            (dist_au - 1.0).abs() < 0.02,
            "Earth should be ~1 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn mars_at_correct_distance() {
        // Mars semi-major axis ~1.524 AU, varies between ~1.38 and ~1.67 AU
        let epoch = Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0);
        let pos = heliocentric_position_ecliptic("mars", &epoch).unwrap();
        let dist_au = pos.magnitude() / AU_KM;
        assert!(
            dist_au > 1.3 && dist_au < 1.7,
            "Mars should be 1.3-1.7 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn venus_inside_earth() {
        let epoch = Epoch::from_gregorian(2024, 9, 1, 12, 0, 0.0);
        let venus = heliocentric_position_ecliptic("venus", &epoch).unwrap();
        let earth = heliocentric_position_ecliptic("earth", &epoch).unwrap();
        assert!(
            venus.magnitude() < earth.magnitude(),
            "Venus ({:.4} AU) should be closer than Earth ({:.4} AU)",
            venus.magnitude() / AU_KM,
            earth.magnitude() / AU_KM
        );
    }

    #[test]
    fn mercury_smallest_orbit() {
        let epoch = Epoch::from_gregorian(2024, 1, 15, 12, 0, 0.0);
        let pos = heliocentric_position_ecliptic("mercury", &epoch).unwrap();
        let dist_au = pos.magnitude() / AU_KM;
        // Mercury: ~0.31-0.47 AU
        assert!(
            dist_au > 0.28 && dist_au < 0.50,
            "Mercury should be 0.28-0.50 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn jupiter_outer_planet() {
        let epoch = Epoch::from_gregorian(2024, 6, 1, 12, 0, 0.0);
        let pos = heliocentric_position_ecliptic("jupiter", &epoch).unwrap();
        let dist_au = pos.magnitude() / AU_KM;
        // Jupiter: ~4.95-5.46 AU
        assert!(
            dist_au > 4.5 && dist_au < 5.8,
            "Jupiter should be 4.5-5.8 AU, got {dist_au:.4} AU"
        );
    }

    #[test]
    fn unknown_body_returns_none() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 12, 0, 0.0);
        assert!(heliocentric_position_ecliptic("pluto", &epoch).is_none());
        assert!(heliocentric_position_ecliptic("foo", &epoch).is_none());
    }

    #[test]
    fn earth_position_anti_aligns_with_sun_eci() {
        // Earth's heliocentric position should be roughly opposite to geocentric sun direction
        use crate::sun;

        let epoch = Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0);
        let earth_helio = heliocentric_position_ecliptic("earth", &epoch).unwrap();
        let sun_dir = sun::sun_direction_eci(&epoch).into_inner();

        // Convert Earth heliocentric to equatorial for comparison
        let epsilon = obliquity(&epoch);
        let earth_eq = ecliptic_to_equatorial(&earth_helio, epsilon).normalize();

        // They should be roughly anti-parallel (dot product ≈ -1)
        let dot = earth_eq.dot(&sun_dir);
        assert!(
            dot < -0.95,
            "Earth heliocentric should anti-align with geocentric sun direction, dot={dot:.3}"
        );
    }

    // --- Coordinate conversion tests ---

    #[test]
    fn ecliptic_to_equatorial_x_unchanged() {
        // X-axis is shared between ecliptic and equatorial
        let v = Vector3::new(1.0, 0.0, 0.0);
        let result = ecliptic_to_equatorial(&v, 0.4); // any obliquity
        assert!((result.x - 1.0).abs() < 1e-10);
        assert!(result.y.abs() < 1e-10);
        assert!(result.z.abs() < 1e-10);
    }

    #[test]
    fn ecliptic_to_equatorial_preserves_magnitude() {
        let v = Vector3::new(1.5, 2.3, -0.8);
        let epsilon = 0.409; // ~23.44 degrees
        let result = ecliptic_to_equatorial(&v, epsilon);
        assert!(
            (result.magnitude() - v.magnitude()).abs() < 1e-10,
            "Rotation should preserve magnitude"
        );
    }

    #[test]
    fn obliquity_near_23_degrees() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let eps = obliquity(&epoch).to_degrees();
        assert!(
            (eps - 23.44).abs() < 0.1,
            "Obliquity should be ~23.44°, got {eps:.2}°"
        );
    }
}
