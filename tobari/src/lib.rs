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

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod math;

#[cfg(feature = "alloc")]
pub mod cssi;
pub mod exponential;
#[cfg(feature = "alloc")]
pub mod gfz;
pub mod harris_priester;
pub mod magnetic;
pub mod nrlmsise00;
pub mod space_weather;

#[cfg(feature = "alloc")]
pub use cssi::{CssiData, CssiSpaceWeather, OutOfRangeBehavior};
pub use exponential::Exponential;
pub use harris_priester::HarrisPriester;
pub use nrlmsise00::Nrlmsise00;
pub use space_weather::{ConstantWeather, SpaceWeather, SpaceWeatherProvider};

use arika::earth::geodetic::Geodetic;
use arika::epoch::{Epoch, Utc};

/// Pre-computed input for atmospheric density evaluation.
///
/// Contains the geodetic coordinates and UTC epoch needed by all
/// atmosphere models. The caller (e.g., drag force model) is
/// responsible for computing `geodetic` from the propagator's
/// frame-typed position vector — the atmosphere model itself is
/// frame-agnostic.
pub struct AtmosphereInput<'a> {
    /// Satellite geodetic coordinates (latitude/longitude in rad, altitude in km).
    pub geodetic: Geodetic,
    /// Absolute UTC epoch.
    pub utc: &'a Epoch<Utc>,
}

/// An atmospheric density model.
///
/// Computes density \[kg/m³\] from geodetic position and epoch.
/// The model is **frame-agnostic**: it receives pre-computed geodetic
/// coordinates rather than frame-typed ECI vectors. The frame-to-geodetic
/// conversion is the caller's responsibility.
pub trait AtmosphereModel: Send + Sync {
    /// Compute atmospheric density \[kg/m³\].
    fn density(&self, input: &AtmosphereInput<'_>) -> f64;
}
