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
}
