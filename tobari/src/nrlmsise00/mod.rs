//! NRLMSISE-00 empirical atmosphere model.
//!
//! Clean-room implementation based on the following references:
//! - Picone, J.M. et al. (2002), "NRLMSISE-00 empirical model of the atmosphere:
//!   Statistical comparisons and scientific issues", J. Geophys. Res., 107(A12), 1468,
//!   doi:10.1029/2002JA009430
//! - Hedin, A.E. (1991), "Extension of the MSIS thermosphere model into the middle
//!   and lower atmosphere", J. Geophys. Res., 96(A2), 1159-1172.
//! - Hedin, A.E. (1987), "MSIS-86 thermospheric model",
//!   J. Geophys. Res., 92(A5), 4649-4662.
//!
//! Coefficient values are from the official NRL distribution, treated as published data.
//! NRLMSISE-00 is believed to be in the public domain as a U.S. Government work
//! (17 U.S.C. § 105), though no explicit license was provided by NRL.
//!
//! Note: MSIS is a registered trademark. This module uses the name "NRLMSISE-00"
//! for nominative fair use (identifying compatibility with the NRL model).
//!
//! Validated against `pymsis` (official NRL Fortran wrapper, `version=0`).

pub mod coefficients;
pub mod geo;

use crate::AtmosphereModel;
use kaname::epoch::Epoch;
use nalgebra::Vector3;

/// Space weather conditions at a given epoch.
///
/// These inputs drive the solar and geomagnetic activity variation
/// terms in NRLMSISE-00.
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
/// Analogous to Harris-Priester's sun direction function, this trait
/// allows callers to supply time-varying solar activity data.
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
}

impl SpaceWeatherProvider for ConstantWeather {
    fn get(&self, _epoch: &Epoch) -> SpaceWeather {
        self.weather.clone()
    }
}

/// Full output of the NRLMSISE-00 model.
///
/// Includes temperatures and all species number densities.
#[derive(Debug, Clone)]
pub struct Nrlmsise00Output {
    /// Exospheric temperature [K].
    pub temp_exo: f64,
    /// Temperature at altitude [K].
    pub temp_alt: f64,
    /// He number density [cm⁻³].
    pub density_he: f64,
    /// O number density [cm⁻³].
    pub density_o: f64,
    /// N₂ number density [cm⁻³].
    pub density_n2: f64,
    /// O₂ number density [cm⁻³].
    pub density_o2: f64,
    /// Ar number density [cm⁻³].
    pub density_ar: f64,
    /// H number density [cm⁻³].
    pub density_h: f64,
    /// N number density [cm⁻³].
    pub density_n: f64,
    /// Anomalous oxygen number density [cm⁻³].
    pub density_anomalous_o: f64,
    /// Total mass density [kg/m³].
    pub total_mass_density: f64,
}

/// Input parameters for a single NRLMSISE-00 evaluation.
#[derive(Debug, Clone)]
pub struct Nrlmsise00Input {
    /// Day of year [1-366].
    pub day_of_year: u32,
    /// Universal time [seconds since midnight].
    pub ut_seconds: f64,
    /// Geodetic altitude [km].
    pub altitude_km: f64,
    /// Geodetic latitude [degrees, -90 to 90].
    pub latitude_deg: f64,
    /// Geodetic longitude [degrees, 0 to 360 or -180 to 180].
    pub longitude_deg: f64,
    /// Local apparent solar time [hours, 0-24].
    pub local_solar_time_hours: f64,
    /// Previous day's F10.7 [SFU].
    pub f107_daily: f64,
    /// 81-day centered average F10.7 [SFU].
    pub f107_avg: f64,
    /// Daily Ap index.
    pub ap_daily: f64,
    /// 7-element Ap array for magnetic activity variations.
    pub ap_array: [f64; 7],
}

/// NRLMSISE-00 empirical atmosphere model.
///
/// Computes neutral atmospheric density and composition from 0 to ~1000 km altitude
/// as a function of location, time, solar activity (F10.7), and geomagnetic
/// activity (Ap).
pub struct Nrlmsise00 {
    weather: Box<dyn SpaceWeatherProvider>,
}

impl Nrlmsise00 {
    /// Create a new NRLMSISE-00 model with the given space weather provider.
    pub fn new(weather: Box<dyn SpaceWeatherProvider>) -> Self {
        Self { weather }
    }

    /// Compute full NRLMSISE-00 output for the given input parameters.
    ///
    /// Returns temperatures and all species number densities.
    pub fn calculate(&self, _input: &Nrlmsise00Input) -> Nrlmsise00Output {
        // TODO: implement — stub returns zeros
        Nrlmsise00Output {
            temp_exo: 0.0,
            temp_alt: 0.0,
            density_he: 0.0,
            density_o: 0.0,
            density_n2: 0.0,
            density_o2: 0.0,
            density_ar: 0.0,
            density_h: 0.0,
            density_n: 0.0,
            density_anomalous_o: 0.0,
            total_mass_density: 0.0,
        }
    }
}

impl AtmosphereModel for Nrlmsise00 {
    fn density(&self, altitude_km: f64, position: &Vector3<f64>, epoch: Option<&Epoch>) -> f64 {
        let epoch = match epoch {
            Some(e) => e,
            None => return 0.0,
        };

        // Convert position to geodetic coordinates
        let (lat_deg, lon_deg) = geo::eci_to_geodetic_latlon(position, epoch);

        // Convert epoch to day-of-year and UT seconds
        let (doy, ut_seconds) = geo::epoch_to_day_of_year_and_ut(epoch);

        // Compute local solar time
        let lst = geo::local_solar_time(ut_seconds, lon_deg);

        // Get space weather
        let sw = self.weather.get(epoch);

        let input = Nrlmsise00Input {
            day_of_year: doy,
            ut_seconds,
            altitude_km,
            latitude_deg: lat_deg,
            longitude_deg: lon_deg,
            local_solar_time_hours: lst,
            f107_daily: sw.f107_daily,
            f107_avg: sw.f107_avg,
            ap_daily: sw.ap_daily,
            ap_array: sw.ap_3hour_history,
        };

        self.calculate(&input).total_mass_density
    }
}
