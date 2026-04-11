//! Earth environment models for orbital mechanics.
//!
//! ## Atmosphere
//!
//! Provides pluggable atmospheric density models behind the [`AtmosphereModel`] trait.
//!
//! - [`Exponential`] — US Standard Atmosphere 1976, altitude-only (simplest, no epoch needed)
//! - [`HarrisPriester`] — diurnal density variation using Sun position
//! - [`Nrlmsise00`] — full empirical model driven by solar and geomagnetic activity indices
//!
//! All models implement [`AtmosphereModel`] and can be swapped at runtime via
//! `Box<dyn AtmosphereModel>`.
//!
//! ## Magnetic field
//!
//! Provides pluggable geomagnetic field models behind the
//! [`magnetic::MagneticFieldModel`] trait.
//!
//! - [`magnetic::TiltedDipole`] — simple tilted dipole approximation (fastest)
//!
//! ## Space weather
//!
//! The [`space_weather`] module defines [`SpaceWeather`] conditions and the
//! [`SpaceWeatherProvider`] trait for supplying time-varying solar/geomagnetic data.
//! [`CssiSpaceWeather`] parses CelesTrak CSSI-format files and (with the `fetch`
//! feature) downloads them automatically with local caching.
//!
//! ## Data attribution
//!
//! Space weather indices are sourced from:
//! - Kp/Ap geomagnetic indices: GFZ Helmholtz Centre for Geosciences (CC BY 4.0)
//! - F10.7 solar radio flux: NOAA SWPC / NRCan DRAO (public domain)
//! - Aggregated and distributed by CelesTrak (<https://celestrak.org/SpaceData/>)

pub mod cssi;
pub mod exponential;
pub mod gfz;
pub mod harris_priester;
pub mod magnetic;
pub mod nrlmsise00;
pub mod space_weather;
#[cfg(feature = "wasm")]
pub mod wasm;

pub use cssi::{CssiData, CssiSpaceWeather, OutOfRangeBehavior};
pub use exponential::Exponential;
pub use harris_priester::HarrisPriester;
pub use nrlmsise00::Nrlmsise00;
pub use space_weather::{ConstantWeather, SpaceWeather, SpaceWeatherProvider};

use kaname::SimpleEci;
use kaname::epoch::{Epoch, Utc};

/// An atmospheric density model.
///
/// Computes density \[kg/m³\] from altitude, satellite position, and epoch.
/// Implementors must be `Send + Sync` for use inside [`ForceModel`](orts::perturbations::ForceModel).
///
/// # Frame and scale discipline (Phase 4)
///
/// `position_eci` is a phantom-typed [`kaname::SimpleEci`] — the simple
/// path of the Phase 1–3 frame redesign. A future `density_precise`
/// entry point that takes `&kaname::frame::Vec3<kaname::frame::Itrs>` +
/// a full EOP provider is planned for Phase 4B; the current trait only
/// covers the simple path so every existing atmosphere model can
/// continue to participate.
///
/// `epoch` is a [`kaname::epoch::Epoch<Utc>`] so implementors that need
/// a time argument (Harris-Priester's diurnal bulge, NRLMSISE-00's
/// local solar time, etc.) receive a scale-tagged epoch rather than a
/// bare JD, matching the rest of the kaname time-scale discipline.
pub trait AtmosphereModel: Send + Sync {
    /// Compute atmospheric density \[kg/m³\].
    ///
    /// # Arguments
    /// - `altitude_km` — altitude above the reference body surface \[km\]
    /// - `position_eci` — satellite position in the simple ECI frame
    ///   \[km\]
    /// - `epoch` — absolute UTC time (`None` if unavailable)
    fn density(
        &self,
        altitude_km: f64,
        position_eci: &SimpleEci,
        epoch: Option<&Epoch<Utc>>,
    ) -> f64;
}
