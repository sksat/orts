//! Geomagnetic field models.
//!
//! Provides pluggable magnetic field models behind the [`MagneticFieldModel`] trait.
//!
//! - [`TiltedDipole`] — simple tilted dipole approximation (fastest)
//! - [`Igrf`] — IGRF-13/14 spherical harmonic model up to degree 13
//! - [`NoField`] — zero everywhere, for a body whose field is not modelled
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

/// Zero field everywhere.
///
/// [`TiltedDipole`] and [`Igrf`] are both Earth's, so a spacecraft around
/// another body has no field to read. This says so in the value rather than in
/// the caller: a magnetometer reads zero, and a magnetorquer's `m × B` is zero,
/// so a run with a magnetic device on such a body propagates without that
/// device doing anything. Reading Earth's field there instead would report a
/// field the body does not have and make torque from it.
pub struct NoField;

impl MagneticFieldModel for NoField {
    fn field_ecef(&self, _input: &MagneticFieldInput<'_>) -> [f64; 3] {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::geodetic::Geodetic;

    /// `NoField` reads zero wherever it is asked.
    ///
    /// A magnetorquer's torque is `m × B`, so this is what makes the device
    /// inert on a body whose field is not modelled.
    #[test]
    fn no_field_is_zero_everywhere() {
        let epoch = Epoch::<Utc>::j2000();
        let cases: [(f64, f64, f64); 3] =
            [(0.0, 0.0, 0.0), (45.0, -120.0, 400.0), (-80.0, 30.0, 800.0)];
        for (lat_deg, lon_deg, alt) in cases {
            let input = MagneticFieldInput {
                geodetic: Geodetic {
                    latitude: lat_deg.to_radians(),
                    longitude: lon_deg.to_radians(),
                    altitude: alt,
                },
                utc: &epoch,
            };
            assert_eq!(NoField.field_ecef(&input), [0.0, 0.0, 0.0]);
        }
    }
}
