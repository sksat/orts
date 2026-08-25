//! Earth: physical constants, reference ellipsoid, geodetic coordinates,
//! Earth rotation models, and Earth Orientation Parameters.
//!
//! # Submodules
//!
//! - [`ellipsoid`] — WGS-84 reference ellipsoid constants
//! - [`geodetic`] — WGS-84 Cartesian ↔ geodetic conversions, [`Geodetic`] type
//! - [`topocentric`] — ground-site look angles ([`TopocentricSite`], [`LookAngles`])
//! - [`rotation`] — IAU 2009 WGCCRE rotation model (`EARTH` const)
//! - [`eop`] — Earth Orientation Parameters provider traits
//!   ([`Ut1Offset`](eop::Ut1Offset), [`PolarMotion`](eop::PolarMotion),
//!   [`NutationCorrections`](eop::NutationCorrections),
//!   [`LengthOfDay`](eop::LengthOfDay)) and [`NullEop`](eop::NullEop) placeholder
//! - [`iau2006`] — full IAU 2006 / 2000A_R06 CIO-based Earth rotation chain:
//!   the supporting math (angular units, fundamental arguments, precession /
//!   nutation polynomials) plus the `Rotation<Gcrs, Cirs>::iau2006` /
//!   `Rotation<Cirs, Tirs>::from_era` / `Rotation<Tirs, Itrs>::polar_motion`
//!   constructors that consume the [`eop`] traits.
//! - [`transform`] — per-frame Earth rotation pole
//!   ([`EarthRotationPole`](transform::EarthRotationPole)) and ECI ↔ ECEF
//!   transform ([`EarthFixedTransform`](transform::EarthFixedTransform), whose
//!   inputs are bundled as [`EarthOrientation`](transform::EarthOrientation))
//!   for `SimpleEci` / `Gcrs`, used by frame-aware force models

pub mod ellipsoid;
pub mod eop;
pub mod fk5;
pub mod geodetic;
pub mod iau2006;
pub mod rotation;
pub mod teme;
pub mod topocentric;
pub mod transform;

pub use ellipsoid::{WGS84_A, WGS84_B, WGS84_E2, WGS84_F};
#[cfg(feature = "alloc")]
pub use eop::GcrsEopStorage;
pub use eop::PositionEop;
pub use geodetic::{Geodetic, geodetic_altitude};
pub use topocentric::{LookAngles, TopocentricSite};
pub use transform::{EarthFixedTransform, EarthOrientation, EarthRotationPole};

// Physical constants

/// Earth gravitational parameter [km³/s²] (WGS-84).
pub const MU: f64 = 398600.4418;

/// Earth equatorial radius [km] (WGS-84).
pub const R: f64 = 6378.137;

/// Earth J2 zonal harmonic coefficient (WGS-84 / EGM96).
pub const J2: f64 = 1.08263e-3;

/// Earth J3 zonal harmonic coefficient (WGS-84 / EGM96).
pub const J3: f64 = -2.5356e-6;

/// Earth J4 zonal harmonic coefficient (WGS-84 / EGM96).
pub const J4: f64 = -1.6199e-6;

/// Earth rotation rate [rad/s] (IERS 2010).
pub const OMEGA: f64 = 7.2921159e-5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mu_is_positive() {
        assert!(MU > 0.0);
    }

    #[test]
    fn r_is_positive() {
        assert!(R > 0.0);
    }

    #[test]
    fn surface_gravity_approximate() {
        // g ≈ μ/R² ≈ 9.798e-3 km/s² ≈ 9.798 m/s²
        let g = MU / (R * R);
        assert!((g - 9.798e-3).abs() < 0.01e-3);
    }

    #[test]
    fn j2_is_positive() {
        assert!(J2 > 0.0);
    }

    #[test]
    fn j3_is_negative() {
        assert!(J3 < 0.0);
    }

    #[test]
    fn j4_is_negative() {
        assert!(J4 < 0.0);
    }

    #[test]
    fn j2_dominates_higher_harmonics() {
        assert!(J2 > J3.abs());
        assert!(J3.abs() > J4.abs());
    }
}
