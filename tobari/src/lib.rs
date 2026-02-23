//! Atmospheric density models for orbital mechanics.
//!
//! Provides pluggable atmospheric density models behind the [`AtmosphereModel`] trait.
//! Each model is implemented in its own module:
//!
//! - [`Exponential`] — US Standard Atmosphere 1976, altitude-only (simplest, no epoch needed)
//! - [`HarrisPriester`] — diurnal density variation using Sun position
//! - [`Nrlmsise00`] — full empirical model driven by solar and geomagnetic activity indices
//!
//! All models implement [`AtmosphereModel`] and can be swapped at runtime via
//! `Box<dyn AtmosphereModel>`.
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
pub mod harris_priester;
pub mod nrlmsise00;
pub mod space_weather;

pub use cssi::{CssiData, CssiSpaceWeather};
pub use exponential::Exponential;
pub use harris_priester::HarrisPriester;
pub use nrlmsise00::Nrlmsise00;
pub use space_weather::{ConstantWeather, SpaceWeather, SpaceWeatherProvider};

use kaname::epoch::Epoch;
use nalgebra::Vector3;

/// An atmospheric density model.
///
/// Computes density \[kg/m³\] from altitude, satellite position, and epoch.
/// Implementors must be `Send + Sync` for use inside [`ForceModel`](orts_orbits::perturbations::ForceModel).
pub trait AtmosphereModel: Send + Sync {
    /// Compute atmospheric density \[kg/m³\].
    ///
    /// # Arguments
    /// - `altitude_km` — altitude above the reference body surface \[km\]
    /// - `position` — satellite position in ECI frame \[km\]
    /// - `epoch` — absolute time (`None` if unavailable)
    fn density(&self, altitude_km: f64, position: &Vector3<f64>, epoch: Option<&Epoch>) -> f64;
}
