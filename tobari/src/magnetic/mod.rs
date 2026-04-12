//! Geomagnetic field models.
//!
//! Provides pluggable magnetic field models behind the [`MagneticFieldModel`] trait.
//!
//! - [`TiltedDipole`] — simple tilted dipole approximation (fastest)
//! - [`Igrf`] — IGRF-13/14 spherical harmonic model up to degree 13
//!
//! All models implement [`MagneticFieldModel`] and can be used generically via
//! `F: MagneticFieldModel` bounds.

pub mod dipole;
pub mod igrf;

pub use dipole::TiltedDipole;
pub use igrf::Igrf;

use arika::earth::geodetic::Geodetic;
use arika::epoch::{Epoch, Utc};

/// Pre-computed input for magnetic field evaluation.
///
/// The caller computes `geodetic` from the propagator's frame-typed
/// position — the model itself is frame-agnostic.
pub struct MagneticFieldInput<'a> {
    /// Satellite geodetic coordinates (latitude/longitude in rad, altitude in km).
    pub geodetic: Geodetic,
    /// Absolute UTC epoch (required for secular variation and ECEF orientation).
    pub utc: &'a Epoch<Utc>,
}

/// A geomagnetic field model.
///
/// Computes the magnetic field vector in ECEF Cartesian coordinates.
/// The model is **frame-agnostic**: it receives pre-computed geodetic
/// coordinates and returns the field in the Earth-fixed frame.
/// The caller is responsible for rotating to their inertial frame.
pub trait MagneticFieldModel: Send + Sync {
    /// Compute the magnetic field vector in ECEF Cartesian \[T\].
    fn field_ecef(&self, input: &MagneticFieldInput<'_>) -> [f64; 3];
}
