//! WGS-84 reference ellipsoid constants.
//!
//! Used by [`super::geodetic`] for Cartesian ↔ geodetic conversions and by
//! downstream crates that need to reason about the Earth's shape directly
//! (e.g. atmosphere models evaluating altitude).
//!
//! Future work: introduce a `ReferenceEllipsoid` trait so that `Geodetic<E>`
//! can be parameterized over WGS84 / GRS80 / IERS2010 ellipsoids, and so
//! that body-specific ellipsoids (MoonSphere, MarsSpheroid) can live
//! alongside as structural data without being stringly-typed.

/// WGS-84 semi-major axis [km].
pub const WGS84_A: f64 = 6378.137;

/// WGS-84 flattening.
pub const WGS84_F: f64 = 1.0 / 298.257223563;

/// WGS-84 semi-minor axis [km].
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);

/// WGS-84 first eccentricity squared.
pub const WGS84_E2: f64 = 1.0 - (1.0 - WGS84_F) * (1.0 - WGS84_F);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgs84_b_less_than_a() {
        assert!(WGS84_B < WGS84_A);
    }

    #[test]
    fn wgs84_e2_small_and_positive() {
        assert!(WGS84_E2 > 0.0);
        assert!(WGS84_E2 < 0.01); // Earth is very nearly a sphere
    }
}
