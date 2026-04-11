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
mod model;

use crate::AtmosphereModel;
use crate::space_weather::SpaceWeatherProvider;
use kaname::epoch::Epoch;
use nalgebra::Vector3;

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
    pub fn calculate(&self, input: &Nrlmsise00Input) -> Nrlmsise00Output {
        let (d, temp_exo, temp_alt) = model::compute(input);
        // d[0..8]: He, O, N2, O2, Ar, total_mass(g/cm³), H, N, anomO
        // Total mass density: d[5] is in g/cm³, convert to kg/m³ (* 1000)
        Nrlmsise00Output {
            temp_exo,
            temp_alt,
            density_he: d[0],
            density_o: d[1],
            density_n2: d[2],
            density_o2: d[3],
            density_ar: d[4],
            density_h: d[6],
            density_n: d[7],
            density_anomalous_o: d[8],
            total_mass_density: d[5] * 1000.0,
        }
    }

    /// Compute full atmospheric composition at the given position and epoch.
    ///
    /// Returns the complete NRLMSISE-00 output including:
    /// - Total mass density \[kg/m³\]
    /// - Number densities \[cm⁻³\] for 9 species: He, O, N₂, O₂, Ar, H, N, anomalous O
    /// - Exospheric and local temperatures \[K\]
    ///
    /// This is the high-level API that handles ECI-to-geodetic coordinate conversions
    /// internally. For direct low-level access with pre-computed geodetic coordinates,
    /// use [`Nrlmsise00::calculate()`].
    ///
    /// Unlike [`AtmosphereModel::density()`] which only returns total mass density,
    /// this method provides the full species breakdown for diagnostics and analysis.
    pub fn density_with_composition(
        &self,
        altitude_km: f64,
        position: &Vector3<f64>,
        epoch: &Epoch,
    ) -> Nrlmsise00Output {
        let (lat_deg, lon_deg) = geo::eci_to_geodetic_latlon(position, epoch);
        let (doy, ut_seconds) = geo::epoch_to_day_of_year_and_ut(epoch);
        let lst = geo::local_solar_time(ut_seconds, lon_deg, epoch);
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

        self.calculate(&input)
    }
}

impl AtmosphereModel for Nrlmsise00 {
    fn density(&self, altitude_km: f64, position: &Vector3<f64>, epoch: Option<&Epoch>) -> f64 {
        match epoch {
            Some(e) => {
                self.density_with_composition(altitude_km, position, e)
                    .total_mass_density
            }
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConstantWeather;

    /// Verify that density computed via ECI position (going through geo.rs conversion)
    /// matches density computed via direct geodetic input to within 0.1%.
    ///
    /// This catches bugs where eci_to_geodetic_latlon returns wrong coordinates.
    #[test]
    fn density_via_eci_matches_direct_input_at_high_latitude() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let f107 = 150.0;
        let ap = 15.0;
        let model = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

        let lat_deg: f64 = 51.6; // ISS inclination
        let lon_deg: f64 = 30.0;
        let alt_km: f64 = 400.0;

        // Path 1: direct input (known-correct geodetic coords)
        let (doy, ut_sec) = geo::epoch_to_day_of_year_and_ut(&epoch);
        let lst = geo::local_solar_time(ut_sec, lon_deg, &epoch);
        let sw = model.weather.get(&epoch);
        let direct_input = Nrlmsise00Input {
            day_of_year: doy,
            ut_seconds: ut_sec,
            altitude_km: alt_km,
            latitude_deg: lat_deg,
            longitude_deg: lon_deg,
            local_solar_time_hours: lst,
            f107_daily: sw.f107_daily,
            f107_avg: sw.f107_avg,
            ap_daily: sw.ap_daily,
            ap_array: sw.ap_3hour_history,
        };
        let direct_density = model.calculate(&direct_input).total_mass_density;

        // Path 2: via ECI position (goes through geo.rs eci_to_geodetic_latlon)
        let gmst = epoch.gmst();
        let geod = kaname::earth::Geodetic {
            latitude: lat_deg.to_radians(),
            longitude: lon_deg.to_radians(),
            altitude: alt_km,
        };
        let ecef = kaname::SimpleEcef::from(geod);
        let eci = kaname::frame::Rotation::<
            kaname::frame::SimpleEcef,
            kaname::frame::SimpleEci,
        >::from_era(gmst)
            .transform(&ecef);
        let eci_density = model
            .density_with_composition(alt_km, eci.inner(), &epoch)
            .total_mass_density;

        let rel_err = (eci_density - direct_density).abs() / direct_density;
        assert!(
            rel_err < 0.001,
            "density mismatch: direct={direct_density:.6e}, eci={eci_density:.6e}, \
             rel_err={rel_err:.4e} (>0.1%)"
        );
    }
}
