//! Geographic and temporal conversions for NRLMSISE-00.
//!
//! Converts between ECI coordinates and the geodetic/time parameters
//! required by the NRLMSISE-00 model.

use kaname::Eci;
use kaname::epoch::Epoch;
use nalgebra::Vector3;

/// Convert ECI position + epoch to WGS-84 geodetic latitude and longitude [degrees].
///
/// Uses GMST to rotate from ECI to ECEF, then computes geodetic latitude
/// via Bowring iteration (delegated to `kaname::Ecef::to_geodetic()`).
pub fn eci_to_geodetic_latlon(position: &Vector3<f64>, epoch: &Epoch) -> (f64, f64) {
    let gmst_rad = epoch.gmst();
    let ecef = Eci::from_raw(*position).to_ecef(gmst_rad);
    let geod = ecef.to_geodetic();
    (geod.latitude.to_degrees(), geod.longitude.to_degrees())
}

/// Convert epoch to (day_of_year, ut_seconds).
pub fn epoch_to_day_of_year_and_ut(epoch: &Epoch) -> (u32, f64) {
    let dt = epoch.to_datetime();

    // Day of year
    let doy = day_of_year(dt.year, dt.month, dt.day);

    // UT seconds since midnight
    let ut_sec = dt.hour as f64 * 3600.0 + dt.min as f64 * 60.0 + dt.sec;

    (doy, ut_sec)
}

/// Compute day of year from (year, month, day).
fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let is_leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut doy = 0u32;
    for m in 1..month {
        doy += days_in_month[m as usize];
        if m == 2 && is_leap {
            doy += 1;
        }
    }
    doy + day
}

/// Compute local apparent solar time [hours].
///
/// Applies the Equation of Time correction to convert from mean to apparent
/// solar time:
///
///   LST_apparent = UT/3600 + lon/15 + EoT(epoch)
///
/// where EoT accounts for Earth's orbital eccentricity and axial tilt
/// (up to ±16 minutes correction).
pub fn local_solar_time(ut_sec: f64, longitude_deg: f64, epoch: &Epoch) -> f64 {
    let eot_hours = kaname::sun::equation_of_time(epoch);
    let lst = ut_sec / 3600.0 + longitude_deg / 15.0 + eot_hours;
    // Normalize to [0, 24)
    ((lst % 24.0) + 24.0) % 24.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_year_jan1() {
        assert_eq!(day_of_year(2024, 1, 1), 1);
    }

    #[test]
    fn day_of_year_dec31_non_leap() {
        assert_eq!(day_of_year(2023, 12, 31), 365);
    }

    #[test]
    fn day_of_year_dec31_leap() {
        assert_eq!(day_of_year(2024, 12, 31), 366);
    }

    #[test]
    fn day_of_year_mar1_leap() {
        // Jan(31) + Feb(29) + 1 = 61
        assert_eq!(day_of_year(2024, 3, 1), 61);
    }

    #[test]
    fn day_of_year_mar1_non_leap() {
        // Jan(31) + Feb(28) + 1 = 60
        assert_eq!(day_of_year(2023, 3, 1), 60);
    }

    #[test]
    fn local_solar_time_greenwich_noon() {
        // UT=12h, lon=0° — EoT shifts by up to ~16 min
        let epoch = Epoch::from_gregorian(2024, 4, 15, 12, 0, 0.0); // EoT ≈ 0
        let lst = local_solar_time(43200.0, 0.0, &epoch);
        assert!((lst - 12.0).abs() < 0.05, "lst={lst}");
    }

    #[test]
    fn local_solar_time_east_90() {
        // UT=0h, lon=90° → mean LST=6h, plus EoT correction
        let epoch = Epoch::from_gregorian(2024, 4, 15, 0, 0, 0.0);
        let lst = local_solar_time(0.0, 90.0, &epoch);
        assert!((lst - 6.0).abs() < 0.05, "lst={lst}");
    }

    #[test]
    fn local_solar_time_west_90() {
        // UT=0h, lon=-90° → mean LST=18h (wraps), plus EoT
        let epoch = Epoch::from_gregorian(2024, 4, 15, 0, 0, 0.0);
        let lst = local_solar_time(0.0, -90.0, &epoch);
        assert!((lst - 18.0).abs() < 0.05, "lst={lst}");
    }

    #[test]
    fn local_solar_time_wraps_24() {
        // UT=23h, lon=30° → mean LST=25h → ~1h, plus EoT
        let epoch = Epoch::from_gregorian(2024, 4, 15, 23, 0, 0.0);
        let lst = local_solar_time(23.0 * 3600.0, 30.0, &epoch);
        assert!((lst - 1.0).abs() < 0.05, "lst={lst}");
    }

    #[test]
    fn local_solar_time_eot_effect_february() {
        // February: EoT ≈ -14 min (sundial slow) → LST shifted ~0.23h behind mean
        let epoch = Epoch::from_gregorian(2024, 2, 12, 12, 0, 0.0);
        let lst = local_solar_time(43200.0, 0.0, &epoch);
        // Without EoT: exactly 12.0; with EoT: ~11.77
        assert!(
            lst > 11.65 && lst < 11.85,
            "Feb EoT should shift LST behind: lst={lst}"
        );
    }

    #[test]
    fn epoch_to_day_of_year_and_ut_vernal_equinox_2024() {
        // 2024-03-20T12:00:00Z
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let (doy, ut_sec) = epoch_to_day_of_year_and_ut(&epoch);
        // Jan(31) + Feb(29) + 20 = 80
        assert_eq!(doy, 80);
        assert!((ut_sec - 43200.0).abs() < 1.0); // 12h = 43200s
    }

    #[test]
    fn eci_to_latlon_on_equator_at_gmst_zero() {
        // At GMST=0, ECI x-axis = ECEF x-axis → lon=0
        // Position on equator along x-axis
        let epoch = Epoch::from_jd(2451545.0); // J2000.0 (GMST ≈ 280.46°)
        let gmst = epoch.gmst();

        // Place the satellite along the ECEF x-axis (lon=0)
        // In ECI: rotate by +GMST
        let r = 6778.0; // LEO
        let pos = Vector3::new(r * gmst.cos(), r * gmst.sin(), 0.0);

        let (lat, lon) = eci_to_geodetic_latlon(&pos, &epoch);
        assert!(lat.abs() < 0.1, "lat={lat}, expected ~0");
        assert!(lon.abs() < 0.1, "lon={lon}, expected ~0");
    }

    #[test]
    fn eci_to_latlon_north_pole() {
        let epoch = Epoch::from_jd(2451545.0);
        let pos = Vector3::new(0.0, 0.0, 6378.0);
        let (lat, _lon) = eci_to_geodetic_latlon(&pos, &epoch);
        assert!((lat - 90.0).abs() < 0.1, "lat={lat}, expected ~90");
    }

    #[test]
    fn eci_to_latlon_matches_kaname_geodetic_at_iss_inclination() {
        // ISS-like: 400 km altitude, 51.6° geodetic latitude
        // Geocentric vs geodetic differs by ~0.17° at this latitude.
        // Round-trip: Geodetic → ECEF → ECI → eci_to_geodetic_latlon
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let expected_lat: f64 = 51.6;
        let expected_lon: f64 = 30.0;
        let geod = kaname::Geodetic {
            latitude: expected_lat.to_radians(),
            longitude: expected_lon.to_radians(),
            altitude: 400.0,
        };
        let eci = geod.to_ecef().to_eci(gmst);

        let (lat_deg, lon_deg) = eci_to_geodetic_latlon(eci.inner(), &epoch);

        assert!(
            (lat_deg - expected_lat).abs() < 0.01,
            "lat={lat_deg}, expected {expected_lat} (geodetic, not geocentric)"
        );
        assert!(
            (lon_deg - expected_lon).abs() < 0.01,
            "lon={lon_deg}, expected {expected_lon}"
        );
    }

    #[test]
    fn eci_to_latlon_matches_kaname_geodetic_at_polar() {
        // Near-polar: 800 km altitude, 80° geodetic latitude
        // Maximum geocentric↔geodetic difference region
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let expected_lat: f64 = 80.0;
        let expected_lon: f64 = -45.0;
        let geod = kaname::Geodetic {
            latitude: expected_lat.to_radians(),
            longitude: expected_lon.to_radians(),
            altitude: 800.0,
        };
        let eci = geod.to_ecef().to_eci(gmst);

        let (lat_deg, lon_deg) = eci_to_geodetic_latlon(eci.inner(), &epoch);

        assert!(
            (lat_deg - expected_lat).abs() < 0.01,
            "lat={lat_deg}, expected {expected_lat} (geodetic, not geocentric)"
        );
        // Longitude wraps to [-180, 180]
        let lon_diff = ((lon_deg - expected_lon + 180.0) % 360.0 - 180.0).abs();
        assert!(lon_diff < 0.01, "lon={lon_deg}, expected {expected_lon}");
    }
}
