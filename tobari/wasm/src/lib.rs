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

use tobari::cssi::{CssiData, CssiSpaceWeather};
use tobari::gfz::{self, SpaceWeatherFormat};
use tobari::magnetic::{Igrf, MagneticFieldInput, MagneticFieldModel, TiltedDipole};
use tobari::nrlmsise00::{Nrlmsise00, Nrlmsise00Input};
use tobari::space_weather::SpaceWeatherProvider;
use tobari::{AtmosphereInput, AtmosphereModel, ConstantWeather, HarrisPriester};

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

/// Largest number of grid points any batch entry point will compute.
///
/// A 4096 x 4096 map or a 128 x 180 x 360 volume fits comfortably. Past this
/// the result buffer alone is tens of megabytes and the evaluation is minutes
/// of work, so the call is rejected instead of hanging the caller's tab.
const MAX_GRID_POINTS: usize = 1 << 24;

/// Validate grid dimensions and return the number of points they describe.
///
/// Widens each dimension to `usize` before multiplying and uses `checked_mul`:
/// multiplying in `u32` first overflows for large grids, which traps in a debug
/// wasm build and, in release, wraps to a small `Vec::with_capacity` while the
/// loops still run the full untruncated count. Zero dimensions are rejected
/// too — they describe no points, but the volume entry points would still
/// return their appended `[min, max]` pair as if they had.
fn grid_points(dims: &[(&str, u32)]) -> Result<usize, String> {
    let mut total: usize = 1;
    for &(name, n) in dims {
        if n == 0 {
            return Err(format!("{name} must be at least 1"));
        }
        total = total
            .checked_mul(n as usize)
            .filter(|t| *t <= MAX_GRID_POINTS)
            .ok_or_else(|| {
                format!("grid exceeds the {MAX_GRID_POINTS} point limit ({name} = {n})")
            })?;
    }
    Ok(total)
}

// Atmospheric density — single point

/// Exponential atmosphere density [kg/m³] at the given altitude.
#[wasm_bindgen]
pub fn exponential_density(altitude_km: f64) -> f64 {
    tobari::exponential::density(altitude_km)
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

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon_deg, &epoch);
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

// Atmospheric density — batch

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

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon_deg, &epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let mut out = Vec::with_capacity(altitudes.len() * 3);
    for &alt in altitudes {
        let exp_rho = tobari::exponential::density(alt);
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
///
/// Throws if either dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
#[wasm_bindgen]
pub fn atmosphere_latlon_map(
    model: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
    f107: f64,
    ap: f64,
) -> Result<Vec<f64>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let n = grid_points(&[("n_lat", n_lat), ("n_lon", n_lon)])?;
    let mut out = Vec::with_capacity(n);

    for i_lat in 0..n_lat {
        let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
        for i_lon in 0..n_lon {
            let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

            let rho = match model {
                "exponential" => tobari::exponential::density(altitude_km),
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
                    let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
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
    Ok(out)
}

// Magnetic field — single point

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

// Magnetic field — batch

/// Compute lat/lon magnetic field map.
///
/// `model`: `"igrf"` or `"dipole"`.
/// `component`: `"total"`, `"inclination"`, `"declination"`, `"north"`, `"east"`, `"down"`.
/// Returns flat row-major values (length = n_lat × n_lon).
/// Values in nT for field components, degrees for angles.
///
/// Throws if either dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
#[wasm_bindgen]
pub fn magnetic_field_latlon_map(
    model: &str,
    component: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Result<Vec<f64>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();

    let n = grid_points(&[("n_lat", n_lat), ("n_lon", n_lon)])?;
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
    Ok(out)
}

/// Compute 3D magnetic field volume as Float32.
///
/// Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
/// Returns values (length = n_alt × n_lat × n_lon + 2, with [min, max] appended).
/// Values in nT for field components, degrees for angles.
///
/// Throws if any dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
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
) -> Result<Vec<f32>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();

    let total = grid_points(&[("n_alt", n_alt), ("n_lat", n_lat), ("n_lon", n_lon)])?;
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
    Ok(out)
}

// Volume data (3D: lat × lon × alt)

/// Compute 3D atmospheric density volume as Float32.
///
/// Layout: alt-major `index = iAlt * nLat * nLon + iLat * nLon + iLon`
/// Returns `[rho_0, rho_1, ...]` (length = n_alt × n_lat × n_lon).
/// Also returns `[min, max]` appended at the end (total length = n_alt*n_lat*n_lon + 2).
///
/// Throws if any dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
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
) -> Result<Vec<f32>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(f107, ap)));

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);
    let sw = ConstantWeather::new(f107, ap).get(&epoch);

    let total = grid_points(&[("n_alt", n_alt), ("n_lat", n_lat), ("n_lon", n_lon)])?;
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
                    "exponential" => tobari::exponential::density(alt),
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
                        let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
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
    Ok(out)
}

// Magnetic field lines

/// Largest number of field-line points `magnetic_field_lines` will produce.
///
/// Each point costs three `f32` in the result and four field evaluations to
/// reach, so this is already well past what a renderer can use.
const MAX_FIELD_LINE_POINTS: usize = 1 << 20;

/// Integrate magnetic field lines from seed points using RK4.
///
/// `seed_lats`, `seed_lons`: geodetic seed points (degrees).
/// `seed_alt_km`: starting altitude for all seeds.
/// `model`: `"igrf"` or `"dipole"`.
/// `max_steps`: max integration steps per line, in each direction.
/// `step_km`: step size in km; must be finite and positive.
///
/// Returns flat `[n_lines, n_pts_0, x0,y0,z0, x1,y1,z1, ..., n_pts_1, ...]`
/// where coordinates are in Earth radii (6371 km).
///
/// Throws if `step_km` is not finite and positive, or if
/// `n_seeds * (2 * max_steps + 1)` exceeds `MAX_FIELD_LINE_POINTS`.
#[wasm_bindgen]
pub fn magnetic_field_lines(
    seed_lats: &[f64],
    seed_lons: &[f64],
    seed_alt_km: f64,
    epoch_jd: f64,
    model: &str,
    max_steps: u32,
    step_km: f64,
) -> Result<Vec<f32>, String> {
    // A zero step never moves the position, so no termination condition can
    // fire and the walk runs the full `max_steps` — up to 2^32 iterations per
    // seed, each with four field evaluations.
    if !step_km.is_finite() || step_km <= 0.0 {
        return Err(format!(
            "step_km must be finite and positive, got {step_km}"
        ));
    }

    let n_seeds = seed_lats.len().min(seed_lons.len());
    let per_seed = (max_steps as usize)
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| format!("max_steps = {max_steps} is too large"))?;
    per_seed
        .checked_mul(n_seeds)
        .filter(|total| *total <= MAX_FIELD_LINE_POINTS)
        .ok_or_else(|| {
            format!(
                "{n_seeds} seeds x {max_steps} steps exceeds the \
                 {MAX_FIELD_LINE_POINTS} field-line point limit"
            )
        })?;

    let epoch = Epoch::from_jd(epoch_jd);
    let igrf = Igrf::earth();
    let dipole = TiltedDipole::earth();
    let earth_r = 6371.0;

    // Walk one leg of a field line away from `start`. RK4 on the unit field
    // direction, stopping at the surface, at 5000 km, or where the field
    // vanishes.
    let walk = |start: Vector3<f64>, direction: f64| -> Vec<Vector3<f64>> {
        let mut leg: Vec<Vector3<f64>> = Vec::new();
        let mut pos = start;
        let ds = step_km * direction;

        for _ in 0..max_steps {
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

            let r = pos.magnitude();
            if r < earth_r || r > earth_r + 5000.0 {
                break;
            }

            leg.push(pos);
        }
        leg
    };

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

        // The line reads continuously from the far end of the backward leg,
        // through the seed, to the far end of the forward leg. Reversing the
        // backward leg once beats inserting each of its points at the front.
        let mut points = walk(start_eci, -1.0);
        points.reverse();
        points.push(start_eci);
        points.extend(walk(start_eci, 1.0));

        out.push(points.len() as f32);
        for p in &points {
            out.push((p.x / earth_r) as f32);
            out.push((p.y / earth_r) as f32);
            out.push((p.z / earth_r) as f32);
        }
    }

    Ok(out)
}

// Space weather (CSSI / GFZ)

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
///
/// Throws if either dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
#[wasm_bindgen]
pub fn atmosphere_latlon_map_sw(
    model: &str,
    altitude_km: f64,
    epoch_jd: f64,
    n_lat: u32,
    n_lon: u32,
) -> Result<Vec<f64>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    // Get space weather if available; non-MSIS models don't need it
    let sw = SPACE_WEATHER
        .get()
        .map(|p| p.get(&epoch))
        .unwrap_or_else(|| ConstantWeather::solar_moderate().get(&epoch));
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(sw.f107_daily, sw.ap_daily)));

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);

    let n = grid_points(&[("n_lat", n_lat), ("n_lon", n_lon)])?;
    let mut out = Vec::with_capacity(n);

    for i_lat in 0..n_lat {
        let lat = -90.0 + (i_lat as f64 + 0.5) * 180.0 / n_lat as f64;
        for i_lon in 0..n_lon {
            let lon = -180.0 + (i_lon as f64 + 0.5) * 360.0 / n_lon as f64;

            let rho = match model {
                "exponential" => tobari::exponential::density(altitude_km),
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
                    let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
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
    Ok(out)
}

/// Compute 3D atmosphere volume using loaded space weather data.
/// Falls back to solar moderate conditions if no data is loaded.
///
/// Throws if any dimension is 0 or the grid exceeds `MAX_GRID_POINTS`.
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
) -> Result<Vec<f32>, String> {
    let epoch = Epoch::from_jd(epoch_jd);
    let sw = SPACE_WEATHER
        .get()
        .map(|p| p.get(&epoch))
        .unwrap_or_else(|| ConstantWeather::solar_moderate().get(&epoch));
    let hp = HarrisPriester::new();
    let msis = Nrlmsise00::new(Box::new(ConstantWeather::new(sw.f107_daily, sw.ap_daily)));

    let (doy, ut_sec) = tobari::nrlmsise00::geo::epoch_to_day_of_year_and_ut(&epoch);

    let total = grid_points(&[("n_alt", n_alt), ("n_lat", n_lat), ("n_lon", n_lon)])?;
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
                    "exponential" => tobari::exponential::density(alt),
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
                        let lst = tobari::nrlmsise00::geo::local_solar_time(ut_sec, lon, &epoch);
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
    Ok(out)
}

// Magnetic field lines

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 2024-03-20T12:00:00Z
    const EPOCH_JD: f64 = 2460390.0;

    #[test]
    fn grid_points_widens_before_multiplying() {
        // 65536 * 65536 = 2^32, which wraps to 0 in u32 arithmetic.
        assert!(grid_points(&[("n_lat", 65536), ("n_lon", 65536)]).is_err());
        // 2^32 - 1 also wraps to a small value in u32 when a third dimension is
        // folded in.
        assert!(grid_points(&[("n_alt", 4), ("n_lat", 65536), ("n_lon", 65536)]).is_err());
        // A grid one point over the ceiling is rejected; the ceiling itself is not.
        assert!(grid_points(&[("n", MAX_GRID_POINTS as u32)]).is_ok());
        assert!(grid_points(&[("n", MAX_GRID_POINTS as u32), ("m", 2)]).is_err());
    }

    #[test]
    fn grid_points_rejects_zero_dimensions() {
        for dims in [
            vec![("n_lat", 0u32), ("n_lon", 8)],
            vec![("n_lat", 8u32), ("n_lon", 0)],
            vec![("n_alt", 0u32), ("n_lat", 4), ("n_lon", 8)],
        ] {
            let err = grid_points(&dims).expect_err("zero dimension must be rejected");
            assert!(err.contains("at least 1"), "unexpected message: {err}");
        }
    }

    #[test]
    fn grid_points_counts_small_grids() {
        assert_eq!(grid_points(&[("n_lat", 4), ("n_lon", 8)]).unwrap(), 32);
        assert_eq!(
            grid_points(&[("n_alt", 3), ("n_lat", 4), ("n_lon", 8)]).unwrap(),
            96
        );
    }

    #[test]
    fn latlon_maps_return_one_value_per_point() {
        let atmo = atmosphere_latlon_map("nrlmsise00", 400.0, EPOCH_JD, 4, 8, 150.0, 15.0).unwrap();
        assert_eq!(atmo.len(), 32);
        assert!(atmo.iter().all(|v| v.is_finite() && *v > 0.0));

        let mag = magnetic_field_latlon_map("igrf", "total", 400.0, EPOCH_JD, 4, 8).unwrap();
        assert_eq!(mag.len(), 32);
        assert!(mag.iter().all(|v| v.is_finite() && *v > 0.0));
    }

    #[test]
    fn volumes_return_one_value_per_point_plus_min_max() {
        let atmo =
            atmosphere_volume("nrlmsise00", 200.0, 600.0, 3, EPOCH_JD, 4, 8, 150.0, 15.0).unwrap();
        assert_eq!(atmo.len(), 3 * 4 * 8 + 2);
        let (values, bounds) = atmo.split_at(3 * 4 * 8);
        assert_eq!(
            bounds[0],
            values.iter().copied().fold(f32::INFINITY, f32::min)
        );
        assert_eq!(
            bounds[1],
            values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        );

        let mag = magnetic_field_volume("igrf", "total", 200.0, 600.0, 3, EPOCH_JD, 4, 8).unwrap();
        assert_eq!(mag.len(), 3 * 4 * 8 + 2);
        assert!(mag.iter().all(|v| v.is_finite()));
    }

    /// Every batch entry point rejects a zero dimension.
    ///
    /// A zero dimension used to produce a two-element `[+Inf, -Inf]` result from
    /// the volume functions — a value the documented layout says cannot occur.
    #[test]
    fn batch_entry_points_reject_zero_dimensions() {
        assert!(atmosphere_latlon_map("nrlmsise00", 400.0, EPOCH_JD, 0, 8, 150.0, 15.0).is_err());
        assert!(atmosphere_latlon_map("nrlmsise00", 400.0, EPOCH_JD, 8, 0, 150.0, 15.0).is_err());
        assert!(magnetic_field_latlon_map("igrf", "total", 400.0, EPOCH_JD, 0, 8).is_err());
        assert!(
            atmosphere_volume("nrlmsise00", 200.0, 600.0, 0, EPOCH_JD, 4, 8, 150.0, 15.0).is_err()
        );
        assert!(magnetic_field_volume("igrf", "total", 200.0, 600.0, 0, EPOCH_JD, 4, 8).is_err());
        assert!(atmosphere_latlon_map_sw("nrlmsise00", 400.0, EPOCH_JD, 0, 8).is_err());
        assert!(atmosphere_volume_sw("nrlmsise00", 200.0, 600.0, 0, EPOCH_JD, 4, 8).is_err());
    }

    /// Every batch entry point rejects a grid whose point count overflows `u32`.
    ///
    /// These calls used to wrap the capacity computation and then run the full
    /// untruncated loop count — 2^32 model evaluations.
    #[test]
    fn batch_entry_points_reject_overflowing_grids() {
        assert!(
            atmosphere_latlon_map("nrlmsise00", 400.0, EPOCH_JD, 65536, 65536, 150.0, 15.0)
                .is_err()
        );
        assert!(magnetic_field_latlon_map("igrf", "total", 400.0, EPOCH_JD, 65536, 65536).is_err());
        assert!(
            atmosphere_volume(
                "nrlmsise00",
                200.0,
                600.0,
                4,
                EPOCH_JD,
                65536,
                65536,
                150.0,
                15.0
            )
            .is_err()
        );
        assert!(
            magnetic_field_volume("igrf", "total", 200.0, 600.0, 4, EPOCH_JD, 65536, 65536)
                .is_err()
        );
        assert!(atmosphere_latlon_map_sw("nrlmsise00", 400.0, EPOCH_JD, 65536, 65536).is_err());
        assert!(
            atmosphere_volume_sw("nrlmsise00", 200.0, 600.0, 4, EPOCH_JD, 65536, 65536).is_err()
        );
    }

    /// Field-line integration rejects a step size that cannot terminate.
    ///
    /// With `step_km = 0` the position never moves, so neither the surface nor
    /// the 5000 km bound nor the vanishing-field check can fire: the walk runs
    /// the full `max_steps` in each direction, four field evaluations each.
    #[test]
    fn field_lines_reject_a_non_advancing_step() {
        let seeds = [40.0f64];
        let lons = [10.0f64];
        for step_km in [0.0, -50.0, f64::NAN, f64::INFINITY] {
            assert!(
                magnetic_field_lines(&seeds, &lons, 400.0, EPOCH_JD, "dipole", 10, step_km)
                    .is_err(),
                "step_km = {step_km} must be rejected"
            );
        }
    }

    #[test]
    fn field_lines_reject_an_unbounded_step_count() {
        let seeds = [40.0f64];
        let lons = [10.0f64];
        assert!(
            magnetic_field_lines(&seeds, &lons, 400.0, EPOCH_JD, "dipole", u32::MAX, 50.0).is_err()
        );
        // Many seeds with a modest step count also add up.
        let many: Vec<f64> = vec![0.0; 4096];
        assert!(
            magnetic_field_lines(&many, &many, 400.0, EPOCH_JD, "dipole", 1_000_000, 50.0).is_err()
        );
    }

    /// The header counts match the payload, and the seed point is included once.
    #[test]
    fn field_lines_report_their_own_layout() {
        let seeds = [40.0f64, -20.0];
        let lons = [10.0f64, 150.0];
        let out = magnetic_field_lines(&seeds, &lons, 400.0, EPOCH_JD, "dipole", 20, 50.0).unwrap();

        assert_eq!(out[0], 2.0, "line count");
        let mut offset = 1usize;
        for _ in 0..2 {
            let n_points = out[offset] as usize;
            assert!(n_points >= 1, "a line must contain at least its seed");
            offset += 1 + 3 * n_points;
        }
        assert_eq!(offset, out.len(), "payload length must match the counts");
        assert!(out.iter().all(|v| v.is_finite()));
    }
}
