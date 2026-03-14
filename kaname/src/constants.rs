/// Earth gravitational parameter (km^3/s^2, WGS84)
pub const MU_EARTH: f64 = 398600.4418;

/// Sun gravitational parameter (km^3/s^2)
pub const MU_SUN: f64 = 132712440018.0;

/// Moon gravitational parameter (km^3/s^2)
pub const MU_MOON: f64 = 4902.8;

/// Earth equatorial radius (km, WGS84)
pub const R_EARTH: f64 = 6378.137;

/// Earth J2 zonal harmonic coefficient (WGS84/EGM96)
pub const J2_EARTH: f64 = 1.08263e-3;

/// Earth J3 zonal harmonic coefficient (WGS84/EGM96)
pub const J3_EARTH: f64 = -2.5356e-6;

/// Earth J4 zonal harmonic coefficient (WGS84/EGM96)
pub const J4_EARTH: f64 = -1.6199e-6;

/// Earth rotation rate (rad/s, IERS 2010)
pub const OMEGA_EARTH: f64 = 7.2921159e-5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mu_earth_is_positive() {
        assert!(MU_EARTH > 0.0);
    }

    #[test]
    fn mu_sun_is_positive() {
        assert!(MU_SUN > 0.0);
    }

    #[test]
    fn mu_sun_greater_than_mu_earth() {
        assert!(MU_SUN > MU_EARTH);
    }

    #[test]
    fn r_earth_is_positive() {
        assert!(R_EARTH > 0.0);
    }

    #[test]
    fn mu_earth_wgs84_value() {
        assert!((MU_EARTH - 398600.4418).abs() < 1e-4);
    }

    #[test]
    fn mu_sun_value() {
        assert!((MU_SUN - 132712440018.0).abs() < 1.0);
    }

    #[test]
    fn r_earth_wgs84_value() {
        assert!((R_EARTH - 6378.137).abs() < 1e-3);
    }

    #[test]
    fn surface_gravity_approximate() {
        // g ≈ μ/R² ≈ 9.798e-3 km/s² ≈ 9.798 m/s²
        let g = MU_EARTH / (R_EARTH * R_EARTH);
        assert!((g - 9.798e-3).abs() < 0.01e-3);
    }

    #[test]
    fn j2_earth_value() {
        assert!((J2_EARTH - 1.08263e-3).abs() < 1e-8);
    }

    #[test]
    fn j2_earth_is_positive() {
        assert!(J2_EARTH > 0.0);
    }

    #[test]
    fn j3_earth_value() {
        assert!((J3_EARTH - (-2.5356e-6)).abs() < 1e-11);
    }

    #[test]
    fn j3_earth_is_negative() {
        assert!(J3_EARTH < 0.0);
    }

    #[test]
    fn j4_earth_value() {
        assert!((J4_EARTH - (-1.6199e-6)).abs() < 1e-11);
    }

    #[test]
    fn j4_earth_is_negative() {
        assert!(J4_EARTH < 0.0);
    }

    #[test]
    fn j2_dominates_higher_harmonics() {
        assert!(J2_EARTH > J3_EARTH.abs());
        assert!(J3_EARTH.abs() > J4_EARTH.abs());
    }
}
