/// Earth gravitational parameter (km^3/s^2, WGS84)
pub const MU_EARTH: f64 = 398600.4418;

/// Sun gravitational parameter (km^3/s^2)
pub const MU_SUN: f64 = 132712440018.0;

/// Earth equatorial radius (km, WGS84)
pub const R_EARTH: f64 = 6378.137;

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
}
