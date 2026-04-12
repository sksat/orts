//! WebAssembly bindings for tobari Earth environment models.
//!
//! Exposes atmospheric density and magnetic field computations to JavaScript
//! via wasm-bindgen. All functions accept simple scalar types and return
//! flat arrays for efficient JS↔WASM data exchange.

use wasm_bindgen::prelude::*;

use arika::SimpleEcef;
use arika::earth::Geodetic;
use arika::epoch::Epoch;
use arika::frame::{self, Rotation};
use nalgebra::Vector3;

use std::sync::OnceLock;

use crate::cssi::{CssiData, CssiSpaceWeather};
use crate::gfz::{self, SpaceWeatherFormat};
use crate::magnetic::{Igrf, MagneticFieldInput, MagneticFieldModel, TiltedDipole};
use crate::nrlmsise00::{Nrlmsise00, Nrlmsise00Input};
use crate::space_weather::SpaceWeatherProvider;
use crate::{AtmosphereInput, AtmosphereModel, ConstantWeather, HarrisPriester};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute ECEF→NED rotation for a magnetic field vector at a geodetic point.
///
/// Returns (B_north, B_east, B_down) in Tesla.
fn ecef_to_ned(b_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    let b_north =
        -sin_lat * cos_lon * b_ecef[0] - sin_lat * sin_lon * b_ecef[1] + cos_lat * b_ecef[2];
    let b_east = -sin_lon * b_ecef[0] + cos_lon * b_ecef[1];
    let b_down =
        -cos_lat * cos_lon * b_ecef[0] - cos_lat * sin_lon * b_ecef[1] - sin_lat * b_ecef[2];

    (b_north, b_east, b_down)
}

/// Compute magnetic field info at a point, returning [Bn, Be, Bd, |B|, inc_deg, dec_deg].
fn field_info(b_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> Vec<f64> {
    let (bn, be, bd) = ecef_to_ned(b_ecef, lat_deg, lon_deg);
    let bh = (bn * bn + be * be).sqrt();
    let b_total = (bn * bn + be * be + bd * bd).sqrt();
    let inc_deg = bd.atan2(bh).to_degrees();
    let dec_deg = be.atan2(bn).to_degrees();
    // Convert T → nT for output
    vec![
        bn * 1e9,
        be * 1e9,
        bd * 1e9,
        b_total * 1e9,
        inc_deg,
        dec_deg,
    ]
}

/// Build a [`MagneticFieldInput`] from geodetic degrees + epoch.
fn make_mag_input(
    lat_deg: f64,
    lon_deg: f64,
    altitude_km: f64,
    epoch: &Epoch,
) -> MagneticFieldInput<'_> {
    MagneticFieldInput {
        geodetic: Geodetic {
            latitude: lat_deg.to_radians(),
            longitude: lon_deg.to_radians(),
            altitude: altitude_km,
        },
        utc: epoch,
    }
}

// ---------------------------------------------------------------------------
// Atmospheric density — single point
// ---------------------------------------------------------------------------

/// Exponential atmosphere density [kg/m³] at the given altitude.
#[wasm_bindgen]
pub fn exponential_density(altitude_km: f64) -> f64 {
    crate::exponential::density(altitude_km)
}

/// Harris-Priester density [kg/m³] at a geodetic point and epoch.
///
/// `epoch_jd`: Julian Date of the epoch.
#[wasm_bindgen]
pub fn harris_priester_density(lat_deg: f64, lon_deg: f64, altitude_km: f64, epoch_jd: f64) -> f64 {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let input = AtmosphereInput {
        geodetic: Geodetic {
            latitude: lat_deg.to_radians(),
            longitude: lon_deg.to_radians(),
            altitude: altitude_km,
        },
        utc: &epoch,
    };
    hp.density(&input)
}

/// NRLMSISE-00 density [kg/m³] at a geodetic point with constant space weather.
///
/// `f107`: F10.7 solar radio flux [SFU].
/// `ap`: daily Ap geomagnetic index.
#[wasm_bindgen]
pub fn nrlmsise00_density(
    lat_deg: f64,
    lon_deg: f64,
    altitude_km: f64,
    epoch_jd: f64,
    f107: f64,
    ap: f64,
) -> f64 {
    let epoch = Epoch::from_jd(epoch_jd);
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon_deg, &epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let input = Nrlmsise00Input {
        day_of_year: doy,
        ut_seconds: ut_sec,
        altitude_km,
        latitude_deg: lat_deg,
        longitude_deg: lon_deg,
        local_solar_time_hours: lst,
        f107_daily: sw.f107_daily,
        f107_avg: sw.f107_avg,
        ap_daily: sw.ap_daily,
        ap_array: sw.ap_3hour_history,
    };

    model.calculate(&input).total_mass_density
}

// ---------------------------------------------------------------------------
// Atmospheric density — batch
// ---------------------------------------------------------------------------

/// Compute altitude profile for all 3 atmosphere models.
///
/// Returns flat `[exp_0, hp_0, msis_0, exp_1, hp_1, msis_1, ...]` (length = N×3).
#[wasm_bindgen]
pub fn atmosphere_altitude_profile(
    altitudes: &[f64],
    lat_deg: f64,
    lon_deg: f64,
    epoch_jd: f64,
    f107: f64,
    ap: f64,
) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon_deg, &epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let mut out = Vec::with_capacity(altitudes.len() * 3);
    for &alt in altitudes {
        let exp_rho = crate::exponential::density(alt);
        let hp_input = AtmosphereInput {
            geodetic: Geodetic {
                latitude: lat_deg.to_radians(),
                longitude: lon_deg.to_radians(),
                altitude: alt,
            },
            utc: &epoch,
        };
        let hp_rho = hp.density(&hp_input);

        let msis_input = Nrlmsise00Input {
            day_of_year: doy,
            ut_seconds: ut_sec,
            altitude_km: alt,
            latitude_deg: lat_deg,
            longitude_deg: lon_deg,
            local_solar_time_hours: lst,
            f107_daily: sw.f107_daily,
            f107_avg: sw.f107_avg,
            ap_daily: sw.ap_daily,
            ap_array: sw.ap_3hour_history,
        };
        let msis_rho = msis.calculate(&msis_input).total_mass_density;

        out.push(exp_rho);
        out.push(hp_rho);
        out.push(msis_rho);
    }
    out
}

/// Compute lat/lon density map for a chosen atmosphere model.
///
/// `model`: `"exponential"`, `"harris-priester"`, or `"nrlmsise00"`.
/// Returns flat row-major `[rho_0, rho_1, ...]` (length = n_lat × n_lon).
/// Latitude ranges from -90 to +90, longitude from -180 to +180.
#[wasm_bindgen]
pub fn atmosphere_latlon_map(
    model: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
    f107: f64,
    ap: f64,
) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let n = (n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(n);

    for i_lat in 0..n_lat {
        let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
        for i_lon in 0..n_lon {
            let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

            let rho = match model {
                "exponential" => crate::exponential::density(altitude_km),
                "harris-priester" => {
                    let atm_input = AtmosphereInput {
                        geodetic: Geodetic {
                            latitude: lat.to_radians(),
                            longitude: lon.to_radians(),
                            altitude: altitude_km,
                        },
                        utc: &epoch,
                    };
                    hp.density(&atm_input)
                }
                _ => {
                    let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
                    let input = Nrlmsise00Input {
                        day_of_year: doy,
                        ut_seconds: ut_sec,
                        altitude_km,
                        latitude_deg: lat,
                        longitude_deg: lon,
                        local_solar_time_hours: lst,
                        f107_daily: sw.f107_daily,
                        f107_avg: sw.f107_avg,
                        ap_daily: sw.ap_daily,
                        ap_array: sw.ap_3hour_history,
                    };
                    msis.calculate(&input).total_mass_density
                }
            };
            out.push(rho);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Magnetic field — single point
// ---------------------------------------------------------------------------

/// IGRF-14 field at a geodetic point.
///
/// Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
#[wasm_bindgen]
pub fn igrf_field_at(lat_deg: f64, lon_deg: f64, altitude_km: f64, epoch_jd: f64) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let input = make_mag_input(lat_deg, lon_deg, altitude_km, &epoch);
    let b_ecef = igrf.field_ecef(&input);
    field_info(&b_ecef, lat_deg, lon_deg)
}

/// Tilted dipole field at a geodetic point.
///
/// Returns `[B_north, B_east, B_down, |B|, inclination_deg, declination_deg]` in nT.
#[wasm_bindgen]
pub fn dipole_field_at(lat_deg: f64, lon_deg: f64, altitude_km: f64, epoch_jd: f64) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let dipole = TiltedDipole::earth();
    let input = make_mag_input(lat_deg, lon_deg, altitude_km, &epoch);
    let b_ecef = dipole.field_ecef(&input);
    field_info(&b_ecef, lat_deg, lon_deg)
}

// ---------------------------------------------------------------------------
// Magnetic field — batch
// ---------------------------------------------------------------------------

/// Compute lat/lon magnetic field map.
///
/// `model`: `"igrf"` or `"dipole"`.
/// `component`: `"total"`, `"inclination"`, `"declination"`, `"north"`, `"east"`, `"down"`.
/// Returns flat row-major values (length = n_lat × n_lon).
/// Values in nT for field components, degrees for angles.
#[wasm_bindgen]
pub fn magnetic_field_latlon_map(
    model: &str,
    component: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();

    let n = (n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(n);

    for i_lat in 0..n_lat {
        let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
        for i_lon in 0..n_lon {
            let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

            let input = make_mag_input(lat, lon, altitude_km, &epoch);

            let b_ecef = match model {
                "dipole" => dipole.field_ecef(&input),
                _ => igrf.field_ecef(&input),
            };

            let info = field_info(&b_ecef, lat, lon);
            // info: [Bn, Be, Bd, |B|, inc, dec]
            let val = match component {
                "north" => info[0],
                "east" => info[1],
                "down" => info[2],
                "total" => info[3],
                "inclination" => info[4],
                "declination" => info[5],
                _ => info[3], // default: total
            };
            out.push(val);
        }
    }
    out
}

/// Compute 3D magnetic field volume as Float32.
///
/// Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
/// Returns values (length = n_alt × n_lat × n_lon + 2, with [min, max] appended).
/// Values in nT for field components, degrees for angles.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn magnetic_field_volume(
    model: &str,
    component: &str,
    alt_min_km: f64,
    alt_max_km: f64,
    n_alt: u32,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();

    let total = (n_alt * n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(total + 2);
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;

    for i_alt in 0..n_alt {
        let alt = if n_alt == 1 {
            alt_min_km
        } else {
            alt_min_km + (alt_max_km - alt_min_km) * i_alt as f64 / (n_alt - 1) as f64
        };

        for i_lat in 0..n_lat {
            let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
            for i_lon in 0..n_lon {
                let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

                let input = make_mag_input(lat, lon, alt, &epoch);

                let b_ecef = match model {
                    "dipole" => dipole.field_ecef(&input),
                    _ => igrf.field_ecef(&input),
                };

                // Inline field_info to avoid per-point Vec allocation
                let (bn, be, bd) = ecef_to_ned(&b_ecef, lat, lon);
                let val = match component {
                    "north" => bn * 1e9,
                    "east" => be * 1e9,
                    "down" => bd * 1e9,
                    "total" => (bn * bn + be * be + bd * bd).sqrt() * 1e9,
                    "inclination" => {
                        let bh = (bn * bn + be * be).sqrt();
                        bd.atan2(bh).to_degrees()
                    }
                    "declination" => be.atan2(bn).to_degrees(),
                    _ => (bn * bn + be * be + bd * bd).sqrt() * 1e9,
                } as f32;

                if val < min_val {
                    min_val = val;
                }
                if val > max_val {
                    max_val = val;
                }
                out.push(val);
            }
        }
    }

    out.push(min_val);
    out.push(max_val);
    out
}

// ---------------------------------------------------------------------------
// Volume data (3D: lat × lon × alt)
// ---------------------------------------------------------------------------

/// Compute 3D atmospheric density volume as Float32.
///
/// Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
/// Returns `[rho_0, rho_1, ...]` (length = n_alt × n_lat × n_lon).
/// Also returns `[min, max]` appended at the end (total length = n_alt*n_lat*n_lon + 2).
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn atmosphere_volume(
    model: &str,
    alt_min_km: f64,
    alt_max_km: f64,
    n_alt: u32,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
    f107: f64,
    ap: f64,
) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let total = (n_alt * n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(total + 2);
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;

    for i_alt in 0..n_alt {
        let alt = if n_alt == 1 {
            alt_min_km
        } else {
            alt_min_km + (alt_max_km - alt_min_km) * i_alt as f64 / (n_alt - 1) as f64
        };

        for i_lat in 0..n_lat {
            let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
            for i_lon in 0..n_lon {
                let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

                let rho = match model {
                    "exponential" => crate::exponential::density(alt),
                    "harris-priester" => {
                        let atm_input = AtmosphereInput {
                            geodetic: Geodetic {
                                latitude: lat.to_radians(),
                                longitude: lon.to_radians(),
                                altitude: alt,
                            },
                            utc: &epoch,
                        };
                        hp.density(&atm_input)
                    }
                    _ => {
                        let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
                        let input = Nrlmsise00Input {
                            day_of_year: doy,
                            ut_seconds: ut_sec,
                            altitude_km: alt,
                            latitude_deg: lat,
                            longitude_deg: lon,
                            local_solar_time_hours: lst,
                            f107_daily: sw.f107_daily,
                            f107_avg: sw.f107_avg,
                            ap_daily: sw.ap_daily,
                            ap_array: sw.ap_3hour_history,
                        };
                        msis.calculate(&input).total_mass_density
                    }
                };
                let v = rho as f32;
                if v < min_val {
                    min_val = v;
                }
                if v > max_val {
                    max_val = v;
                }
                out.push(v);
            }
        }
    }
    out.push(min_val);
    out.push(max_val);
    out
}

// ---------------------------------------------------------------------------
// Magnetic field lines
// ---------------------------------------------------------------------------

/// Integrate magnetic field lines from seed points using RK4.
///
/// `seed_lats`, `seed_lons`: geodetic seed points (degrees).
/// `seed_alt_km`: starting altitude for all seeds.
/// `model`: `"igrf"` or `"dipole"`.
/// `max_steps`: max integration steps per line.
/// `step_km`: step size in km.
///
/// Returns flat `[n_lines, n_pts_0, x0,y0,z0, x1,y1,z1, ..., n_pts_1, ...]`
/// where coordinates are in Earth radii (6371 km).
#[wasm_bindgen]
pub fn magnetic_field_lines(
    seed_lats: &[f64],
    seed_lons: &[f64],
    seed_alt_km: f64,
    epoch_jd: f64,
    model: &str,
    max_steps: u32,
    step_km: f64,
) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();
    let earth_r = 6371.0;

    let n_seeds = seed_lats.len().min(seed_lons.len());
    let mut out: Vec<f32> = Vec::new();
    out.push(n_seeds as f32);

    for i in 0..n_seeds {
        let gmst = epoch.gmst();
        let geod = Geodetic {
            latitude: seed_lats[i].to_radians(),
            longitude: seed_lons[i].to_radians(),
            altitude: seed_alt_km,
        };
        let ecef = SimpleEcef::from(geod);
        let start_eci = Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst)
            .transform(&ecef)
            .into_inner();

        // Integrate both forward and backward
        let mut points: Vec<Vector3<f64>> = Vec::new();

        for direction in [-1.0_f64, 1.0] {
            let mut pos = start_eci;
            let ds = step_km * direction;

            let start_idx = points.len();
            if direction > 0.0 {
                points.push(pos);
            }

            for _ in 0..max_steps {
                // RK4 step
                let b1 = field_at_eci(&pos, &epoch, model, &igrf, &dipole);
                if b1.magnitude() < 1e-15 {
                    break;
                }
                let b1n = b1.normalize();

                let p2 = pos + b1n * (ds * 0.5);
                let b2 = field_at_eci(&p2, &epoch, model, &igrf, &dipole);
                if b2.magnitude() < 1e-15 {
                    break;
                }
                let b2n = b2.normalize();

                let p3 = pos + b2n * (ds * 0.5);
                let b3 = field_at_eci(&p3, &epoch, model, &igrf, &dipole);
                if b3.magnitude() < 1e-15 {
                    break;
                }
                let b3n = b3.normalize();

                let p4 = pos + b3n * ds;
                let b4 = field_at_eci(&p4, &epoch, model, &igrf, &dipole);
                if b4.magnitude() < 1e-15 {
                    break;
                }
                let b4n = b4.normalize();

                pos += (b1n + 2.0 * b2n + 2.0 * b3n + b4n) * (ds / 6.0);

                // Stop if below surface or too far
                let r = pos.magnitude();
                if r < earth_r || r > earth_r + 5000.0 {
                    break;
                }

                if direction > 0.0 {
                    points.push(pos);
                } else {
                    points.insert(start_idx, pos);
                }
            }
        }

        // Write points for this line
        out.push(points.len() as f32);
        for p in &points {
            out.push((p.x / earth_r) as f32);
            out.push((p.y / earth_r) as f32);
            out.push((p.z / earth_r) as f32);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Space weather (CSSI / GFZ)
// ---------------------------------------------------------------------------

/// Global space weather provider, loaded once via `load_space_weather`.
static SPACE_WEATHER: OnceLock<CssiSpaceWeather> = OnceLock::new();

/// Load space weather data from text (CSSI or GFZ format, auto-detected).
///
/// Returns `true` on success. Can only be called once; subsequent calls
/// return `false` without replacing the existing data.
#[wasm_bindgen]
pub fn load_space_weather(text: &str) -> bool {
    let data = match gfz::detect_format(text) {
        SpaceWeatherFormat::Cssi => match CssiData::parse(text) {
            Ok(d) => d,
            Err(_) => return false,
        },
        SpaceWeatherFormat::Gfz => match gfz::parse_gfz(text) {
            Ok(d) => d,
            Err(_) => return false,
        },
    };
    SPACE_WEATHER.set(CssiSpaceWeather::new(data)).is_ok()
}

/// Look up space weather for an epoch from the loaded dataset.
///
/// Returns `[f107_daily, f107_avg, ap_daily, ap_3h_0..6]` (length = 10).
/// Returns empty vec if no data is loaded.
#[wasm_bindgen]
pub fn space_weather_lookup(epoch_jd: f64) -> Vec<f64> {
    let Some(provider) = SPACE_WEATHER.get() else {
        return Vec::new();
    };
    let epoch = Epoch::from_jd(epoch_jd);
    let sw = provider.get(&epoch);
    let mut out = Vec::with_capacity(10);
    out.push(sw.f107_daily);
    out.push(sw.f107_avg);
    out.push(sw.ap_daily);
    out.extend_from_slice(&sw.ap_3hour_history);
    out
}

/// Get date range of the loaded space weather data.
///
/// Returns `[jd_first, jd_last]` or empty vec if no data loaded.
/// `jd_last` includes the full final day (midnight of the day after).
#[wasm_bindgen]
pub fn space_weather_date_range() -> Vec<f64> {
    let Some(provider) = SPACE_WEATHER.get() else {
        return Vec::new();
    };
    match provider.data().date_range() {
        // Add 1.0 to last JD so the full final day is included
        Some((first, last)) => vec![first.jd(), last.jd() + 1.0],
        None => Vec::new(),
    }
}

/// Get all space weather records as a flat array for charting.
///
/// Returns flat `[jd_0, f107_0, ap_0, jd_1, f107_1, ap_1, ...]` (length = N × 3).
/// Returns empty vec if no data loaded.
#[wasm_bindgen]
pub fn space_weather_series() -> Vec<f64> {
    let Some(provider) = SPACE_WEATHER.get() else {
        return Vec::new();
    };
    let records = provider.data().records();
    let mut out = Vec::with_capacity(records.len() * 3);
    for r in records {
        out.push(r.jd_midnight);
        out.push(r.f107_obs);
        out.push(r.ap_daily);
    }
    out
}

/// Compute lat/lon density map using loaded space weather data.
///
/// Like `atmosphere_latlon_map` but uses the loaded CSSI/GFZ data
/// instead of constant F10.7/Ap values.
/// Falls back to solar moderate conditions if no data is loaded.
#[wasm_bindgen]
pub fn atmosphere_latlon_map_sw(
    model: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    // Get space weather if available; non-MSIS models don't need it
    let sw = SPACE_WEATHER
        .get()
        .map(|p| p.get(&epoch))
        .unwrap_or_else(|| ConstantWeather::solar_moderate().get(&epoch));
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(sw.f107_daily, sw.ap_daily)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);

    let n = (n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(n);

    for i_lat in 0..n_lat {
        let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
        for i_lon in 0..n_lon {
            let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

            let rho = match model {
                "exponential" => crate::exponential::density(altitude_km),
                "harris-priester" => {
                    let atm_input = AtmosphereInput {
                        geodetic: Geodetic {
                            latitude: lat.to_radians(),
                            longitude: lon.to_radians(),
                            altitude: altitude_km,
                        },
                        utc: &epoch,
                    };
                    hp.density(&atm_input)
                }
                _ => {
                    let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
                    let input = Nrlmsise00Input {
                        day_of_year: doy,
                        ut_seconds: ut_sec,
                        altitude_km,
                        latitude_deg: lat,
                        longitude_deg: lon,
                        local_solar_time_hours: lst,
                        f107_daily: sw.f107_daily,
                        f107_avg: sw.f107_avg,
                        ap_daily: sw.ap_daily,
                        ap_array: sw.ap_3hour_history,
                    };
                    msis.calculate(&input).total_mass_density
                }
            };
            out.push(rho);
        }
    }
    out
}

/// Compute 3D atmosphere volume using loaded space weather data.
/// Falls back to solar moderate conditions if no data is loaded.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn atmosphere_volume_sw(
    model: &str,
    alt_min_km: f64,
    alt_max_km: f64,
    n_alt: u32,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd);
    let sw = SPACE_WEATHER
        .get()
        .map(|p| p.get(&epoch))
        .unwrap_or_else(|| ConstantWeather::solar_moderate().get(&epoch));
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(sw.f107_daily, sw.ap_daily)));

    let (doy, ut_sec) = crate::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);

    let total = (n_alt * n_lat * n_lon) as usize;
    let mut out = Vec::with_capacity(total + 2);
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;

    for i_alt in 0..n_alt {
        let alt = if n_alt == 1 {
            alt_min_km
        } else {
            alt_min_km + (alt_max_km - alt_min_km) * i_alt as f64 / (n_alt - 1) as f64
        };

        for i_lat in 0..n_lat {
            let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
            for i_lon in 0..n_lon {
                let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

                let rho = match model {
                    "exponential" => crate::exponential::density(alt),
                    "harris-priester" => {
                        let atm_input = AtmosphereInput {
                            geodetic: Geodetic {
                                latitude: lat.to_radians(),
                                longitude: lon.to_radians(),
                                altitude: alt,
                            },
                            utc: &epoch,
                        };
                        hp.density(&atm_input)
                    }
                    _ => {
                        let lst = crate::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
                        let input = Nrlmsise00Input {
                            day_of_year: doy,
                            ut_seconds: ut_sec,
                            altitude_km: alt,
                            latitude_deg: lat,
                            longitude_deg: lon,
                            local_solar_time_hours: lst,
                            f107_daily: sw.f107_daily,
                            f107_avg: sw.f107_avg,
                            ap_daily: sw.ap_daily,
                            ap_array: sw.ap_3hour_history,
                        };
                        msis.calculate(&input).total_mass_density
                    }
                };
                let v = rho as f32;
                if v < min_val {
                    min_val = v;
                }
                if v > max_val {
                    max_val = v;
                }
                out.push(v);
            }
        }
    }
    out.push(min_val);
    out.push(max_val);
    out
}

// ---------------------------------------------------------------------------
// Magnetic field lines
// ---------------------------------------------------------------------------

/// Evaluate magnetic field at an ECI position, returning the field in ECI.
///
/// Internally converts ECI → ECEF → Geodetic, calls `field_ecef`, then
/// rotates the result back to ECI.
fn field_at_eci(
    pos: &Vector3<f64>,
    epoch: &Epoch,
    model: &str,
    igrf: &Igrf,
    dipole: &TiltedDipole,
) -> Vector3<f64> {
    let gmst = epoch.gmst();
    // ECI → ECEF
    let eci = arika::SimpleEci::from_raw(*pos);
    let ecef = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(gmst).transform(&eci);
    let geodetic = ecef.to_geodetic();
    let input = MagneticFieldInput {
        geodetic,
        utc: epoch,
    };
    let b_ecef = match model {
        "dipole" => dipole.field_ecef(&input),
        _ => igrf.field_ecef(&input),
    };
    // ECEF → ECI
    let b_ecef_vec = arika::SimpleEcef::from_raw(Vector3::new(b_ecef[0], b_ecef[1], b_ecef[2]));
    Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst)
        .transform(&b_ecef_vec)
        .into_inner()
}
