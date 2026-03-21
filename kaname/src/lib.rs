pub mod body;
pub mod constants;
pub mod epoch;
pub mod moon;
pub mod planets;
pub mod sun;

#[cfg(feature = "wasm")]
pub mod wasm;

use nalgebra::Vector3;

/// Earth-Centered Inertial (ECI/J2000) frame position
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eci(pub Vector3<f64>);

/// Earth-Centered Earth-Fixed (ECEF) frame position
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ecef(pub Vector3<f64>);

/// Geodetic coordinates (WGS84)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodetic {
    pub latitude: f64,  // rad
    pub longitude: f64, // rad
    pub altitude: f64,  // km
}

impl Eci {
    /// Convert ECI position to ECEF by rotating around the Z-axis by GMST angle (radians).
    pub fn to_ecef(&self, gmst: f64) -> Ecef {
        let cos_g = gmst.cos();
        let sin_g = gmst.sin();
        let v = &self.0;
        Ecef(Vector3::new(
            cos_g * v.x + sin_g * v.y,
            -sin_g * v.x + cos_g * v.y,
            v.z,
        ))
    }
}

impl Ecef {
    /// Convert ECEF position to ECI by rotating around the Z-axis by -GMST angle (radians).
    pub fn to_eci(&self, gmst: f64) -> Eci {
        let cos_g = gmst.cos();
        let sin_g = gmst.sin();
        let v = &self.0;
        Eci(Vector3::new(
            cos_g * v.x - sin_g * v.y,
            sin_g * v.x + cos_g * v.y,
            v.z,
        ))
    }

    /// Convert ECEF position to geodetic coordinates using iterative Bowring method (WGS84).
    pub fn to_geodetic(&self) -> Geodetic {
        let v = &self.0;
        let p = (v.x * v.x + v.y * v.y).sqrt();
        let longitude = v.y.atan2(v.x);

        // Near-polar special case
        if p < 1e-10 {
            return Geodetic {
                latitude: v.z.signum() * std::f64::consts::FRAC_PI_2,
                longitude,
                altitude: v.z.abs() - WGS84_B,
            };
        }

        // Bowring iteration with convergence check
        let mut lat = v.z.atan2(p * (1.0 - WGS84_E2));
        let mut alt = 0.0_f64;

        for _ in 0..5 {
            let sin_lat = lat.sin();
            let cos_lat = lat.cos();
            let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
            let new_alt = p / cos_lat - n;
            lat = (v.z / p / (1.0 - WGS84_E2 * n / (n + new_alt))).atan();
            if (new_alt - alt).abs() < 1e-12 {
                alt = new_alt;
                break;
            }
            alt = new_alt;
        }

        Geodetic {
            latitude: lat,
            longitude,
            altitude: alt,
        }
    }
}

/// WGS84 semi-major axis (km)
pub const WGS84_A: f64 = 6378.137;

/// WGS84 flattening
pub const WGS84_F: f64 = 1.0 / 298.257223563;

/// WGS84 semi-minor axis (km)
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);

/// WGS84 first eccentricity squared
pub const WGS84_E2: f64 = 1.0 - (1.0 - WGS84_F) * (1.0 - WGS84_F);

/// Compute WGS-84 geodetic altitude \[km\] directly from a position vector \[km\].
///
/// Works on ECI (or ECEF) coordinates — geodetic altitude depends only on
/// `p = sqrt(x² + y²)` and `z`, which are invariant under Z-axis rotation.
/// Uses Bowring iteration (converges in 2-3 iterations to sub-mm accuracy at LEO).
pub fn geodetic_altitude(position: &Vector3<f64>) -> f64 {
    let p = (position.x * position.x + position.y * position.y).sqrt();
    let z = position.z;

    // Near-polar special case: avoid p/cos(lat) singularity
    if p < 1e-10 {
        return z.abs() - WGS84_B;
    }

    // Bowring iteration for geodetic latitude
    let mut lat = z.atan2(p * (1.0 - WGS84_E2));
    let mut alt = 0.0_f64;

    for _ in 0..5 {
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        let new_alt = p / cos_lat - n;
        let new_lat = (z / p / (1.0 - WGS84_E2 * n / (n + new_alt))).atan();
        if (new_alt - alt).abs() < 1e-9 {
            return new_alt;
        }
        alt = new_alt;
        lat = new_lat;
    }

    alt
}

impl Geodetic {
    /// Convert geodetic coordinates to ECEF position (WGS84).
    pub fn to_ecef(&self) -> Ecef {
        let sin_lat = self.latitude.sin();
        let cos_lat = self.latitude.cos();
        let sin_lon = self.longitude.sin();
        let cos_lon = self.longitude.cos();

        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

        Ecef(Vector3::new(
            (n + self.altitude) * cos_lat * cos_lon,
            (n + self.altitude) * cos_lat * sin_lon,
            (n * (1.0 - WGS84_E2) + self.altitude) * sin_lat,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eci_construction() {
        let eci = Eci(Vector3::new(1.0, 2.0, 3.0));
        assert_eq!(eci.0.x, 1.0);
        assert_eq!(eci.0.y, 2.0);
        assert_eq!(eci.0.z, 3.0);
    }

    #[test]
    fn test_eci_clone() {
        let eci = Eci(Vector3::new(1.0, 2.0, 3.0));
        let eci2 = eci.clone();
        assert_eq!(eci, eci2);
    }

    #[test]
    fn test_eci_debug() {
        let eci = Eci(Vector3::new(1.0, 2.0, 3.0));
        let debug_str = format!("{:?}", eci);
        assert!(debug_str.contains("Eci"));
    }

    #[test]
    fn test_ecef_construction() {
        let ecef = Ecef(Vector3::new(4.0, 5.0, 6.0));
        assert_eq!(ecef.0.x, 4.0);
        assert_eq!(ecef.0.y, 5.0);
        assert_eq!(ecef.0.z, 6.0);
    }

    #[test]
    fn test_ecef_clone() {
        let ecef = Ecef(Vector3::new(4.0, 5.0, 6.0));
        let ecef2 = ecef.clone();
        assert_eq!(ecef, ecef2);
    }

    #[test]
    fn test_ecef_debug() {
        let ecef = Ecef(Vector3::new(4.0, 5.0, 6.0));
        let debug_str = format!("{:?}", ecef);
        assert!(debug_str.contains("Ecef"));
    }

    #[test]
    fn test_geodetic_construction() {
        let geo = Geodetic {
            latitude: 0.5,
            longitude: 1.0,
            altitude: 100.0,
        };
        assert_eq!(geo.latitude, 0.5);
        assert_eq!(geo.longitude, 1.0);
        assert_eq!(geo.altitude, 100.0);
    }

    #[test]
    fn test_geodetic_clone() {
        let geo = Geodetic {
            latitude: 0.5,
            longitude: 1.0,
            altitude: 100.0,
        };
        let geo2 = geo.clone();
        assert_eq!(geo, geo2);
    }

    #[test]
    fn test_geodetic_debug() {
        let geo = Geodetic {
            latitude: 0.5,
            longitude: 1.0,
            altitude: 100.0,
        };
        let debug_str = format!("{:?}", geo);
        assert!(debug_str.contains("Geodetic"));
    }

    // ECI <-> ECEF conversion tests

    #[test]
    fn test_eci_ecef_zero_gmst() {
        let eci = Eci(Vector3::new(7000.0, 1000.0, 500.0));
        let ecef = eci.to_ecef(0.0);
        let eps = 1e-10;
        assert!((ecef.0.x - eci.0.x).abs() < eps);
        assert!((ecef.0.y - eci.0.y).abs() < eps);
        assert!((ecef.0.z - eci.0.z).abs() < eps);
    }

    #[test]
    fn test_eci_ecef_90deg() {
        let gmst = std::f64::consts::FRAC_PI_2;
        let eci = Eci(Vector3::new(1.0, 0.0, 0.0));
        let ecef = eci.to_ecef(gmst);
        let eps = 1e-10;
        // x_eci -> y_ecef (with sign flip: -sin(pi/2)*x = -1 for y? No.)
        // At GMST=pi/2: ecef_x = cos(pi/2)*x + sin(pi/2)*y = 0*1 + 1*0 = 0
        //                ecef_y = -sin(pi/2)*x + cos(pi/2)*y = -1*1 + 0*0 = -1
        // But the expected behavior: x_eci -> y_ecef (not -y).
        // Actually: a point along x_eci, when Earth has rotated pi/2,
        // should appear at -y_ecef direction.
        // ecef_x = 0, ecef_y = -1 for unit vector along x_eci
        assert!(ecef.0.x.abs() < eps);
        assert!((ecef.0.y - (-1.0)).abs() < eps);
        assert!(ecef.0.z.abs() < eps);

        // y_eci -> x_ecef at GMST=pi/2
        let eci2 = Eci(Vector3::new(0.0, 1.0, 0.0));
        let ecef2 = eci2.to_ecef(gmst);
        // ecef_x = cos(pi/2)*0 + sin(pi/2)*1 = 1
        // ecef_y = -sin(pi/2)*0 + cos(pi/2)*1 = 0
        assert!((ecef2.0.x - 1.0).abs() < eps);
        assert!(ecef2.0.y.abs() < eps);
        assert!(ecef2.0.z.abs() < eps);
    }

    #[test]
    fn test_eci_ecef_roundtrip() {
        let original = Eci(Vector3::new(6700.0, 1500.0, 3200.0));
        let gmst = 1.234;
        let roundtrip = original.to_ecef(gmst).to_eci(gmst);
        let eps = 1e-10;
        assert!((roundtrip.0.x - original.0.x).abs() < eps);
        assert!((roundtrip.0.y - original.0.y).abs() < eps);
        assert!((roundtrip.0.z - original.0.z).abs() < eps);
    }

    #[test]
    fn test_eci_ecef_magnitude_preserved() {
        let eci = Eci(Vector3::new(6700.0, 1500.0, 3200.0));
        let gmst = 2.5;
        let ecef = eci.to_ecef(gmst);
        let eps = 1e-10;
        assert!((eci.0.norm() - ecef.0.norm()).abs() < eps);
    }

    // Geodetic <-> ECEF conversion tests

    #[test]
    fn test_equator_prime_meridian() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = geo.to_ecef();
        let eps = 1e-10;
        assert!((ecef.0.x - WGS84_A).abs() < eps);
        assert!(ecef.0.y.abs() < eps);
        assert!(ecef.0.z.abs() < eps);
    }

    #[test]
    fn test_equator_90east() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: std::f64::consts::FRAC_PI_2,
            altitude: 0.0,
        };
        let ecef = geo.to_ecef();
        let eps = 1e-10;
        assert!(ecef.0.x.abs() < eps);
        assert!((ecef.0.y - WGS84_A).abs() < eps);
        assert!(ecef.0.z.abs() < eps);
    }

    #[test]
    fn test_north_pole() {
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_2,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = geo.to_ecef();
        let eps = 1e-6;
        assert!(ecef.0.x.abs() < eps);
        assert!(ecef.0.y.abs() < eps);
        assert!((ecef.0.z - WGS84_B).abs() < eps);
    }

    #[test]
    fn test_roundtrip_geodetic() {
        let original = Geodetic {
            latitude: 0.7,  // ~40 degrees
            longitude: 2.1, // ~120 degrees
            altitude: 350.0,
        };
        let roundtrip = original.to_ecef().to_geodetic();
        let eps = 1e-10;
        assert!(
            (roundtrip.latitude - original.latitude).abs() < eps,
            "latitude: expected {}, got {}",
            original.latitude,
            roundtrip.latitude,
        );
        assert!(
            (roundtrip.longitude - original.longitude).abs() < eps,
            "longitude: expected {}, got {}",
            original.longitude,
            roundtrip.longitude,
        );
        assert!(
            (roundtrip.altitude - original.altitude).abs() < eps,
            "altitude: expected {}, got {}",
            original.altitude,
            roundtrip.altitude,
        );
    }

    // geodetic_altitude() tests

    #[test]
    fn geodetic_altitude_equator() {
        // At equator, geodetic altitude = r - WGS84_A (exact)
        let pos = Vector3::new(WGS84_A + 400.0, 0.0, 0.0);
        let alt = geodetic_altitude(&pos);
        assert!(
            (alt - 400.0).abs() < 1e-9,
            "equator: expected 400.0, got {alt}"
        );
    }

    #[test]
    fn geodetic_altitude_north_pole() {
        // At north pole, geodetic altitude = |z| - WGS84_B
        let pos = Vector3::new(0.0, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!(
            (alt - 400.0).abs() < 1e-6,
            "north pole: expected 400.0, got {alt}"
        );
    }

    #[test]
    fn geodetic_altitude_south_pole() {
        let pos = Vector3::new(0.0, 0.0, -(WGS84_B + 400.0));
        let alt = geodetic_altitude(&pos);
        assert!(
            (alt - 400.0).abs() < 1e-6,
            "south pole: expected 400.0, got {alt}"
        );
    }

    #[test]
    fn geodetic_altitude_matches_to_geodetic() {
        // At 45° latitude, compare geodetic_altitude with Ecef::to_geodetic()
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_4, // 45°
            longitude: 0.5,
            altitude: 400.0,
        };
        let ecef = geo.to_ecef();
        let expected = ecef.to_geodetic().altitude;
        let actual = geodetic_altitude(&ecef.0);
        assert!(
            (actual - expected).abs() < 1e-9,
            "45° lat: to_geodetic={expected}, geodetic_altitude={actual}"
        );
    }

    #[test]
    fn geodetic_altitude_spherical_difference_at_iss_inclination() {
        // ISS-like position at high latitude (~51.6°)
        // Geodetic altitude should differ from spherical by ~10-15 km
        let lat = 51.6_f64.to_radians();
        let geo = Geodetic {
            latitude: lat,
            longitude: 0.0,
            altitude: 400.0,
        };
        let ecef = geo.to_ecef();
        let r = ecef.0.magnitude();
        let spherical_alt = r - WGS84_A;
        let geodetic_alt = geodetic_altitude(&ecef.0);

        let diff = spherical_alt - geodetic_alt;
        // Spherical altitude is lower than geodetic at high latitudes
        // because r is smaller (oblate) but we subtract equatorial radius
        assert!(
            diff.abs() > 5.0 && diff.abs() < 20.0,
            "spherical-geodetic diff at 51.6° should be ~10-15 km, got {diff:.2} km"
        );
    }

    #[test]
    fn geodetic_altitude_near_polar_edge_case() {
        // Very small p (near pole but not exactly)
        let pos = Vector3::new(1e-12, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!(
            (alt - 400.0).abs() < 1e-3,
            "near-polar: expected ~400.0, got {alt}"
        );
    }

    #[test]
    fn geodetic_altitude_invariant_under_z_rotation() {
        // Geodetic altitude should be the same regardless of XY angle (Z-rotation invariant)
        let r = WGS84_A + 400.0;
        let z = 3000.0; // some z component
        let p = (r * r - z * z).sqrt();

        let alt1 = geodetic_altitude(&Vector3::new(p, 0.0, z));
        let alt2 = geodetic_altitude(&Vector3::new(p * 0.6, p * 0.8, z));
        let alt3 = geodetic_altitude(&Vector3::new(-p * 0.5, p * (3.0_f64).sqrt() / 2.0, z));

        assert!(
            (alt1 - alt2).abs() < 1e-10,
            "Z-rotation invariance: {alt1} vs {alt2}"
        );
        assert!(
            (alt1 - alt3).abs() < 1e-10,
            "Z-rotation invariance: {alt1} vs {alt3}"
        );
    }

    #[test]
    fn test_with_altitude() {
        let alt = 500.0; // km
        let geo_surface = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let geo_alt = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: alt,
        };
        let ecef_surface = geo_surface.to_ecef();
        let ecef_alt = geo_alt.to_ecef();
        let eps = 1e-10;
        // At equator/prime meridian, altitude adds directly to x
        assert!((ecef_alt.0.x - ecef_surface.0.x - alt).abs() < eps);
        assert!(ecef_alt.0.y.abs() < eps);
        assert!(ecef_alt.0.z.abs() < eps);
    }
}
