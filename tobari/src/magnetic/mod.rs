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

use kaname::Eci;
use kaname::epoch::Epoch;
use nalgebra::Vector3;

/// A geomagnetic field model.
///
/// Computes the magnetic field vector at a given position and time.
/// Implementors must be `Send + Sync` for use inside dynamics models.
pub trait MagneticFieldModel: Send + Sync {
    /// Compute the magnetic field vector in the ECI (J2000) frame \[T\].
    ///
    /// # Arguments
    /// - `position_eci` — satellite position in ECI frame \[km\]
    /// - `epoch` — absolute time (required for ECEF↔ECI rotation and secular variation)
    ///
    /// # Returns
    /// Magnetic field vector in the ECI (J2000) frame, in Tesla.
    /// The caller is responsible for frame transformations (e.g., ECI → body).
    fn field_eci(&self, position_eci: &Eci, epoch: &Epoch) -> Vector3<f64>;
}
