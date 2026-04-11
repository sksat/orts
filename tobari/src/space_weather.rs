//! Space weather data types and providers.
//!
//! Defines the [`SpaceWeather`] conditions struct and the [`SpaceWeatherProvider`] trait
//! used by atmosphere models that depend on solar and geomagnetic activity indices
//! (e.g., NRLMSISE-00).

use arika::epoch::Epoch;

/// Space weather conditions at a given epoch.
///
/// These inputs drive the solar and geomagnetic activity variation
/// terms in atmosphere models such as NRLMSISE-00.
#[derive(Debug, Clone)]
pub struct SpaceWeather {
    /// Previous day's 10.7 cm solar radio flux [SFU].
    pub f107_daily: f64,
    /// 81-day centered average of F10.7 [SFU].
    pub f107_avg: f64,
    /// Daily Ap geomagnetic index.
    pub ap_daily: f64,
    /// 3-hour Ap history array (7 elements):
    ///   [0] = daily Ap
    ///   [1] = 3-hr Ap for current time
    ///   [2] = 3-hr Ap for 3 hours before
    ///   [3] = 3-hr Ap for 6 hours before
    ///   [4] = 3-hr Ap for 9 hours before
    ///   [5] = average of 3-hr Ap for 12-33 hours before
    ///   [6] = average of 3-hr Ap for 36-57 hours before
    pub ap_3hour_history: [f64; 7],
}

/// Provider of space weather data for a given epoch.
///
/// Implementations supply time-varying solar activity data to atmosphere models.
/// Must be `Send + Sync` for use inside `ForceModel` trait objects.
pub trait SpaceWeatherProvider: Send + Sync {
    /// Returns space weather conditions at the given epoch.
    fn get(&self, epoch: &Epoch) -> SpaceWeather;
}

/// Constant space weather — returns the same F10.7 and Ap for all epochs.
///
/// Useful for testing or when historical space weather data is not available.
#[derive(Debug, Clone)]
pub struct ConstantWeather {
    weather: SpaceWeather,
}

impl ConstantWeather {
    /// Create a constant weather provider with the given F10.7 and Ap values.
    ///
    /// - `f107`: 10.7 cm solar radio flux \[SFU\]. Higher values indicate stronger
    ///   solar activity and higher thermospheric density. Typical range: 70–250.
    /// - `ap`: daily geomagnetic index. Higher values indicate geomagnetic storms
    ///   which expand the upper atmosphere. Typical range: 0–400, quiet ≤ 15.
    ///
    /// Both `f107_daily` and `f107_avg` are set to `f107`; all Ap history
    /// entries are set to `ap` (constant approximation).
    pub fn new(f107: f64, ap: f64) -> Self {
        Self {
            weather: SpaceWeather {
                f107_daily: f107,
                f107_avg: f107,
                ap_daily: ap,
                ap_3hour_history: [ap; 7],
            },
        }
    }

    /// Solar minimum conditions (F10.7 = 70, Ap = 4).
    ///
    /// Represents quiet solar conditions near the bottom of the 11-year solar cycle.
    /// Thermospheric density is at its lowest, resulting in minimal atmospheric drag.
    pub fn solar_min() -> Self {
        Self::new(70.0, 4.0)
    }

    /// Solar moderate conditions (F10.7 = 150, Ap = 15).
    ///
    /// Represents typical mid-cycle solar conditions. This is a reasonable default
    /// for general-purpose simulations when specific space weather data is unavailable.
    pub fn solar_moderate() -> Self {
        Self::new(150.0, 15.0)
    }

    /// Solar maximum conditions (F10.7 = 250, Ap = 50).
    ///
    /// Represents active solar conditions near the peak of the 11-year cycle.
    /// Thermospheric density is significantly elevated, causing much stronger drag
    /// on LEO satellites.
    pub fn solar_max() -> Self {
        Self::new(250.0, 50.0)
    }
}

impl SpaceWeatherProvider for ConstantWeather {
    fn get(&self, _epoch: &Epoch) -> SpaceWeather {
        self.weather.clone()
    }
}
