//! Topocentric look angles (azimuth / elevation / range) from a ground site.
//!
//! [`TopocentricSite`] precomputes the site position and local ENU basis
//! from WGS-84 geodetic coordinates so that per-sample look-angle
//! computation is just three dot products. Works on any Earth-fixed frame
//! marker (`F: Ecef`), mirroring [`super::geodetic`]'s generic conversions.

use super::geodetic::Geodetic;
use crate::frame::{self, Vec3};
#[allow(unused_imports)]
use crate::math::F64Ext;

/// Look angles from a ground site to a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookAngles {
    /// Azimuth [rad]: 0 = north, increasing clockwise (east = π/2), in [0, 2π).
    pub azimuth: f64,
    /// Elevation above the local geodetic horizon [rad].
    pub elevation: f64,
    /// Slant range [km].
    pub range: f64,
}

/// A ground site with precomputed position and ENU basis vectors.
///
/// The "up" direction is the geodetic normal (perpendicular to the WGS-84
/// ellipsoid), not the geocentric direction — elevation is measured from
/// the local geodetic horizon.
#[derive(Debug, Clone, Copy)]
pub struct TopocentricSite<F: frame::Ecef> {
    geodetic: Geodetic,
    position: Vec3<F>,
    east: Vec3<F>,
    north: Vec3<F>,
    up: Vec3<F>,
}

impl<F: frame::Ecef> TopocentricSite<F> {
    /// Construct a site from WGS-84 geodetic coordinates.
    pub fn new(geodetic: Geodetic) -> Self {
        let sin_lat = geodetic.latitude.sin();
        let cos_lat = geodetic.latitude.cos();
        let sin_lon = geodetic.longitude.sin();
        let cos_lon = geodetic.longitude.cos();
        Self {
            geodetic,
            position: geodetic.to_ecef(),
            east: Vec3::new(-sin_lon, cos_lon, 0.0),
            north: Vec3::new(-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat),
            up: Vec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat),
        }
    }

    /// The site's geodetic coordinates.
    pub fn geodetic(&self) -> &Geodetic {
        &self.geodetic
    }

    /// The site's Earth-fixed Cartesian position [km].
    pub fn position(&self) -> &Vec3<F> {
        &self.position
    }

    /// Look angles from this site to `target` (Earth-fixed position [km]).
    ///
    /// `target` must differ from the site position (zero range yields NaN
    /// angles). At zenith the azimuth is ill-defined and returns 0 by the
    /// `atan2(0, 0)` convention.
    pub fn look_angles(&self, target: &Vec3<F>) -> LookAngles {
        let rho = Vec3::<F>::from_raw(target.inner() - self.position.inner());
        let e = rho.dot(&self.east);
        let n = rho.dot(&self.north);
        let u = rho.dot(&self.up);

        let mut azimuth = e.atan2(n);
        if azimuth < 0.0 {
            azimuth += 2.0 * core::f64::consts::PI;
        }
        LookAngles {
            azimuth,
            // atan2 form is well-conditioned near the horizon (vs. asin(u/range)).
            elevation: u.atan2((e * e + n * n).sqrt()),
            range: rho.magnitude(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::ellipsoid::WGS84_A;
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    use nalgebra::Vector3;

    /// Site on the equator at the prime meridian: ENU = (+Y, +Z, +X) in ECEF.
    fn equator_site() -> TopocentricSite<frame::SimpleEcef> {
        TopocentricSite::new(Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        })
    }

    #[test]
    fn zenith_target_has_90deg_elevation() {
        let site = equator_site();
        let la = site.look_angles(&Vec3::new(WGS84_A + 400.0, 0.0, 0.0));
        assert!(
            (la.elevation - FRAC_PI_2).abs() < 1e-12,
            "elevation: {}",
            la.elevation
        );
        assert!((la.range - 400.0).abs() < 1e-9, "range: {}", la.range);
    }

    #[test]
    fn horizon_targets_at_cardinal_azimuths() {
        let site = equator_site();
        // At the equator / prime meridian: east = +Y, north = +Z, up = +X.
        let cases = [
            (Vector3::new(0.0, 0.0, 100.0), 0.0, "north"),
            (Vector3::new(0.0, 100.0, 0.0), FRAC_PI_2, "east"),
            (Vector3::new(0.0, 0.0, -100.0), PI, "south"),
            (Vector3::new(0.0, -100.0, 0.0), 3.0 * FRAC_PI_2, "west"),
        ];
        for (offset, expected_az, label) in cases {
            let target = Vec3::from_raw(Vector3::new(WGS84_A, 0.0, 0.0) + offset);
            let la = site.look_angles(&target);
            assert!(
                (la.azimuth - expected_az).abs() < 1e-12,
                "{label}: azimuth {} != {expected_az}",
                la.azimuth
            );
            assert!(
                la.elevation.abs() < 1e-12,
                "{label}: elevation {}",
                la.elevation
            );
            assert!(
                (la.range - 100.0).abs() < 1e-9,
                "{label}: range {}",
                la.range
            );
        }
    }

    #[test]
    fn elevation_45deg_to_the_north() {
        let site = equator_site();
        let la = site.look_angles(&Vec3::new(WGS84_A + 100.0, 0.0, 100.0));
        assert!(la.azimuth.abs() < 1e-12, "azimuth: {}", la.azimuth);
        assert!(
            (la.elevation - FRAC_PI_4).abs() < 1e-12,
            "elevation: {}",
            la.elevation
        );
        assert!(
            (la.range - 100.0 * 2.0_f64.sqrt()).abs() < 1e-9,
            "range: {}",
            la.range
        );
    }

    #[test]
    fn below_horizon_target_has_negative_elevation() {
        let site = equator_site();
        // Inward of the horizon plane → negative elevation (drives AOS/LOS
        // sign-change detection downstream).
        let la = site.look_angles(&Vec3::new(WGS84_A - 50.0, 0.0, 100.0));
        assert!(la.elevation < 0.0, "elevation: {}", la.elevation);
    }

    #[test]
    fn enu_basis_is_orthonormal_right_handed_at_mid_latitude() {
        let site: TopocentricSite<frame::SimpleEcef> = TopocentricSite::new(Geodetic {
            latitude: 35.68_f64.to_radians(),
            longitude: 139.77_f64.to_radians(),
            altitude: 0.04,
        });
        for (v, label) in [
            (&site.east, "east"),
            (&site.north, "north"),
            (&site.up, "up"),
        ] {
            assert!(
                (v.magnitude() - 1.0).abs() < 1e-12,
                "{label} not unit: {}",
                v.magnitude()
            );
        }
        assert!(site.east.dot(&site.north).abs() < 1e-12);
        assert!(site.east.dot(&site.up).abs() < 1e-12);
        assert!(site.north.dot(&site.up).abs() < 1e-12);
        // Right-handed: east × north = up
        let cross = site.east.cross(&site.north);
        assert!((cross - site.up).magnitude() < 1e-12);
    }

    #[test]
    fn up_is_geodetic_normal_not_geocentric() {
        let site: TopocentricSite<frame::SimpleEcef> = TopocentricSite::new(Geodetic {
            latitude: FRAC_PI_4,
            longitude: 0.0,
            altitude: 0.0,
        });
        // Geodetic normal at 45°N, 0°E.
        let expected = Vector3::new(FRAC_PI_4.cos(), 0.0, FRAC_PI_4.sin());
        assert!((site.up.inner() - expected).norm() < 1e-12);
        // The geocentric direction differs (~0.19° at 45° latitude on WGS-84).
        let geocentric = site.position.normalize();
        let cos_angle = site.up.dot(&geocentric);
        assert!(
            cos_angle < 1.0 - 1e-6,
            "up should differ from geocentric direction: cos = {cos_angle}"
        );
    }

    #[test]
    fn generic_dispatch_works_on_itrs() {
        let site: TopocentricSite<frame::Itrs> = TopocentricSite::new(Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        });
        let la = site.look_angles(&Vec3::new(WGS84_A + 400.0, 0.0, 0.0));
        assert!((la.elevation - FRAC_PI_2).abs() < 1e-12);
    }
}
