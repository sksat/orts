//! WGS-84 geodetic coordinates and Cartesian ↔ geodetic conversions.
//!
//! All conversions assume the WGS-84 reference ellipsoid; see
//! [`super::ellipsoid`] for the underlying constants. Conversions on
//! [`crate::SimpleEcef`] use Bowring iteration.

use nalgebra::Vector3;

use super::ellipsoid::{WGS84_A, WGS84_B, WGS84_E2};
use crate::SimpleEcef;

/// Geodetic coordinates (WGS-84).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodetic {
    /// Geodetic latitude [rad].
    pub latitude: f64,
    /// Longitude [rad].
    pub longitude: f64,
    /// Height above the WGS-84 ellipsoid [km].
    pub altitude: f64,
}

// ─── SimpleEcef ↔ Geodetic type-to-type conversions ──────────────
//
// These are the WGS-84 ellipsoid Cartesian ↔ (lat, lon, height)
// conversions. They are parameter-free (the ellipsoid constants are
// hardcoded WGS-84) so `From` / `Into` is the natural shape.
//
// Scale/ERA conversions between `SimpleEci` and `SimpleEcef` are not
// `From` / `Into` (they require an Epoch / ERA parameter) — use
// `Rotation::<SimpleEci, SimpleEcef>::from_ut1(&epoch)` for those.

impl From<SimpleEcef> for Geodetic {
    /// Convert a WGS-84 Cartesian `SimpleEcef` vector to geodetic
    /// (latitude, longitude, height). Uses iterative Bowring method.
    fn from(ecef: SimpleEcef) -> Self {
        let v = ecef.inner();
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

impl From<Geodetic> for SimpleEcef {
    /// Convert geodetic (latitude, longitude, height) to a WGS-84 Cartesian
    /// `SimpleEcef` vector.
    fn from(geo: Geodetic) -> Self {
        let sin_lat = geo.latitude.sin();
        let cos_lat = geo.latitude.cos();
        let sin_lon = geo.longitude.sin();
        let cos_lon = geo.longitude.cos();

        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

        SimpleEcef::from_raw(Vector3::new(
            (n + geo.altitude) * cos_lat * cos_lon,
            (n + geo.altitude) * cos_lat * sin_lon,
            (n * (1.0 - WGS84_E2) + geo.altitude) * sin_lat,
        ))
    }
}

/// Compute WGS-84 geodetic altitude \[km\] directly from a position vector \[km\].
///
/// Works on any Earth-centered frame ([`crate::SimpleEci`] or [`crate::SimpleEcef`]) — geodetic
/// altitude depends only on `p = sqrt(x² + y²)` and `z`, which are invariant
/// under Z-axis rotation. Uses Bowring iteration (converges in 2-3 iterations
/// to sub-mm accuracy at LEO).
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

#[cfg(test)]
mod tests {
    use super::*;

    // Geodetic <-> SimpleEcef conversion via From / Into

    #[test]
    fn test_equator_prime_meridian() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = SimpleEcef::from(geo);
        let eps = 1e-10;
        assert!((ecef.x() - WGS84_A).abs() < eps);
        assert!(ecef.y().abs() < eps);
        assert!(ecef.z().abs() < eps);
    }

    #[test]
    fn test_equator_90east() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: std::f64::consts::FRAC_PI_2,
            altitude: 0.0,
        };
        let ecef: SimpleEcef = geo.into();
        let eps = 1e-10;
        assert!(ecef.x().abs() < eps);
        assert!((ecef.y() - WGS84_A).abs() < eps);
        assert!(ecef.z().abs() < eps);
    }

    #[test]
    fn test_north_pole() {
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_2,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = SimpleEcef::from(geo);
        let eps = 1e-6;
        assert!(ecef.x().abs() < eps);
        assert!(ecef.y().abs() < eps);
        assert!((ecef.z() - WGS84_B).abs() < eps);
    }

    #[test]
    fn test_roundtrip_geodetic() {
        let original = Geodetic {
            latitude: 0.7,
            longitude: 2.1,
            altitude: 350.0,
        };
        let ecef = SimpleEcef::from(original);
        let roundtrip = Geodetic::from(ecef);
        let eps = 1e-10;
        assert!((roundtrip.latitude - original.latitude).abs() < eps);
        assert!((roundtrip.longitude - original.longitude).abs() < eps);
        assert!((roundtrip.altitude - original.altitude).abs() < eps);
    }

    #[test]
    fn test_with_altitude() {
        let alt = 500.0;
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
        let ecef_surface = SimpleEcef::from(geo_surface);
        let ecef_alt = SimpleEcef::from(geo_alt);
        let eps = 1e-10;
        assert!((ecef_alt.x() - ecef_surface.x() - alt).abs() < eps);
        assert!(ecef_alt.y().abs() < eps);
        assert!(ecef_alt.z().abs() < eps);
    }

    // geodetic_altitude() tests

    #[test]
    fn geodetic_altitude_equator() {
        let pos = Vector3::new(WGS84_A + 400.0, 0.0, 0.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-9);
    }

    #[test]
    fn geodetic_altitude_north_pole() {
        let pos = Vector3::new(0.0, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-6);
    }

    #[test]
    fn geodetic_altitude_south_pole() {
        let pos = Vector3::new(0.0, 0.0, -(WGS84_B + 400.0));
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-6);
    }

    #[test]
    fn geodetic_altitude_matches_to_geodetic() {
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_4,
            longitude: 0.5,
            altitude: 400.0,
        };
        let ecef = SimpleEcef::from(geo);
        let expected = Geodetic::from(ecef).altitude;
        let actual = geodetic_altitude(ecef.inner());
        assert!((actual - expected).abs() < 1e-9);
    }

    #[test]
    fn geodetic_altitude_spherical_difference_at_iss_inclination() {
        let lat = 51.6_f64.to_radians();
        let geo = Geodetic {
            latitude: lat,
            longitude: 0.0,
            altitude: 400.0,
        };
        let ecef = SimpleEcef::from(geo);
        let r = ecef.magnitude();
        let spherical_alt = r - WGS84_A;
        let geodetic_alt = geodetic_altitude(ecef.inner());

        let diff = spherical_alt - geodetic_alt;
        assert!(
            diff.abs() > 5.0 && diff.abs() < 20.0,
            "spherical-geodetic diff at 51.6° should be ~10-15 km, got {diff:.2} km"
        );
    }

    #[test]
    fn geodetic_altitude_near_polar_edge_case() {
        let pos = Vector3::new(1e-12, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-3);
    }

    #[test]
    fn geodetic_altitude_invariant_under_z_rotation() {
        let r = WGS84_A + 400.0;
        let z = 3000.0;
        let p = (r * r - z * z).sqrt();

        let alt1 = geodetic_altitude(&Vector3::new(p, 0.0, z));
        let alt2 = geodetic_altitude(&Vector3::new(p * 0.6, p * 0.8, z));
        let alt3 = geodetic_altitude(&Vector3::new(-p * 0.5, p * (3.0_f64).sqrt() / 2.0, z));

        assert!((alt1 - alt2).abs() < 1e-10);
        assert!((alt1 - alt3).abs() < 1e-10);
    }
}
