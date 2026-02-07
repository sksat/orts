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
}
