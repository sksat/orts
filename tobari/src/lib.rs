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

pub mod exponential;
pub mod harris_priester;
pub mod nrlmsise00;

pub use exponential::Exponential;
pub use harris_priester::HarrisPriester;
pub use nrlmsise00::{ConstantWeather, Nrlmsise00, SpaceWeather, SpaceWeatherProvider};

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
