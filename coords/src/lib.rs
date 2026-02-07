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

        // Initial estimate using Bowring's method
        let mut lat = v.z.atan2(p * (1.0 - WGS84_E2));
        let mut alt;

        // Iterative refinement
        for _ in 0..10 {
            let sin_lat = lat.sin();
            let cos_lat = lat.cos();
            let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
            alt = p / cos_lat - n;
            lat = (v.z / p * (1.0 - WGS84_E2 * n / (n + alt)).powi(-1)).atan();
        }

        let sin_lat = lat.sin();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        alt = p / lat.cos() - n;

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
