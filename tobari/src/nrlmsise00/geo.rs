//! Geographic and temporal conversions for NRLMSISE-00.
//!
//! Converts between satellite position / epoch and the geodetic / time
//! parameters required by NRLMSISE-00. Two entry points are provided:
//!
//! - [`simple_eci_to_geodetic_latlon`] — the Phase 1–3 simple path:
//!   `Vec3<SimpleEci>` + `Epoch<Utc>` rotated by GMST (naive ERA, no
//!   dUT1) to `Vec3<SimpleEcef>`, then Bowring to geodetic
//! - [`precise_gcrs_to_geodetic_latlon`] — the Phase 3B precise path:
//!   `Vec3<Gcrs>` + `Epoch<Utc>` + full EOP provider rotated by the
//!   IAU 2006 CIO chain (`Rotation<Gcrs, Itrs>::iau2006_full_from_utc`)
//!   to `Vec3<Itrs>`, then Bowring to geodetic
//!
//! The entry points have distinct names **and distinct input types**
//! so that a caller cannot accidentally wire a simple-path position
//! into the precise function or vice versa — the type system rejects
//! the mix at compile time.

use arika::SimpleEci;
use arika::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};
use arika::epoch::{Epoch, Ut1, Utc};
use arika::frame::{self, Rotation, Vec3};

/// Convert a simple-path ECI position to WGS-84 geodetic latitude and
/// longitude [degrees].
///
/// The input `position_eci` is a `Vec3<SimpleEci>` — the phantom-typed
/// marker produced by the Phase 1–3 simple rotation path. The rotation
/// to `SimpleEcef` uses naive ERA (`epoch.gmst()` on a UTC epoch,
/// equivalent to assuming `dUT1 = 0`). Accuracy is bounded by the
/// ~0.9 s dUT1 drift, producing up to ~24 km longitude error at the
/// equator.
///
/// For the precise IAU 2006 CIO chain with real EOP data, use
/// [`precise_gcrs_to_geodetic_latlon`].
pub fn simple_eci_to_geodetic_latlon(position_eci: &SimpleEci, epoch: &Epoch<Utc>) -> (f64, f64) {
    let ecef = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(epoch.gmst())
        .transform(position_eci);
    let geod = ecef.to_geodetic();
    (geod.latitude.to_degrees(), geod.longitude.to_degrees())
}

/// Convert a GCRS position + UTC epoch + EOP provider to WGS-84
/// geodetic latitude and longitude [degrees] via the full IAU 2006
/// CIO-based rotation chain.
///
/// This is the first downstream consumer of Phase 3B's
/// [`Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc`].
/// The EOP provider must supply `dUT1`, `dX`/`dY`, and `xp`/`yp` so
/// the full GCRS → ITRS transformation can be built; the trait bound
/// is [`Ut1Offset`] + [`NutationCorrections`] + [`PolarMotion`], matching
/// the `iau2006_full_from_utc` signature exactly.
///
/// `arika::earth::eop::NullEop` is rejected at compile time — see
/// `arika/tests/trybuild/null_eop_in_iau2006_full_from_utc.rs`.
pub fn precise_gcrs_to_geodetic_latlon<P>(
    position_gcrs: &Vec3<frame::Gcrs>,
    utc: &Epoch<Utc>,
    eop: &P,
) -> (f64, f64)
where
    P: Ut1Offset + NutationCorrections + PolarMotion + ?Sized,
{
    let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(utc, eop);
    let pos_itrs: Vec3<frame::Itrs> = rot.transform(position_gcrs);
    let geod = pos_itrs.to_geodetic();
    (geod.latitude.to_degrees(), geod.longitude.to_degrees())
}

/// Unused import suppression helper: keeps `Ut1` in scope for Phase
/// 4B follow-up work that will accept an explicit `&Epoch<Ut1>` for
/// users who already performed the UT1 derivation upstream.
#[allow(dead_code)]
fn _touch_ut1_for_phase_4b(_: &Epoch<Ut1>) {}

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
    let eot_hours = arika::sun::equation_of_time(epoch);
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
    fn simple_eci_to_latlon_on_equator_at_gmst_zero() {
        // At GMST=0, ECI x-axis = ECEF x-axis → lon=0
        // Position on equator along x-axis
        let epoch = Epoch::<Utc>::from_jd(2451545.0); // J2000.0 (GMST ≈ 280.46°)
        let gmst = epoch.gmst();

        // Place the satellite along the ECEF x-axis (lon=0)
        // In ECI: rotate by +GMST
        let r = 6778.0; // LEO
        let pos = SimpleEci::new(r * gmst.cos(), r * gmst.sin(), 0.0);

        let (lat, lon) = simple_eci_to_geodetic_latlon(&pos, &epoch);
        assert!(lat.abs() < 0.1, "lat={lat}, expected ~0");
        assert!(lon.abs() < 0.1, "lon={lon}, expected ~0");
    }

    #[test]
    fn simple_eci_to_latlon_north_pole() {
        let epoch = Epoch::<Utc>::from_jd(2451545.0);
        let pos = SimpleEci::new(0.0, 0.0, 6378.0);
        let (lat, _lon) = simple_eci_to_geodetic_latlon(&pos, &epoch);
        assert!((lat - 90.0).abs() < 0.1, "lat={lat}, expected ~90");
    }

    #[test]
    fn simple_eci_to_latlon_matches_arika_geodetic_at_iss_inclination() {
        // ISS-like: 400 km altitude, 51.6° geodetic latitude
        // Geocentric vs geodetic differs by ~0.17° at this latitude.
        // Round-trip: Geodetic → ECEF → ECI → simple_eci_to_geodetic_latlon
        let epoch = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let expected_lat: f64 = 51.6;
        let expected_lon: f64 = 30.0;
        let geod = arika::earth::Geodetic {
            latitude: expected_lat.to_radians(),
            longitude: expected_lon.to_radians(),
            altitude: 400.0,
        };
        let ecef = arika::SimpleEcef::from(geod);
        let eci = Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst).transform(&ecef);

        let (lat_deg, lon_deg) = simple_eci_to_geodetic_latlon(&eci, &epoch);

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
    fn simple_eci_to_latlon_matches_arika_geodetic_at_polar() {
        // Near-polar: 800 km altitude, 80° geodetic latitude
        // Maximum geocentric↔geodetic difference region
        let epoch = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let gmst = epoch.gmst();

        let expected_lat: f64 = 80.0;
        let expected_lon: f64 = -45.0;
        let geod = arika::earth::Geodetic {
            latitude: expected_lat.to_radians(),
            longitude: expected_lon.to_radians(),
            altitude: 800.0,
        };
        let ecef = arika::SimpleEcef::from(geod);
        let eci = Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst).transform(&ecef);

        let (lat_deg, lon_deg) = simple_eci_to_geodetic_latlon(&eci, &epoch);

        assert!(
            (lat_deg - expected_lat).abs() < 0.01,
            "lat={lat_deg}, expected {expected_lat} (geodetic, not geocentric)"
        );
        // Longitude wraps to [-180, 180]
        let lon_diff = ((lon_deg - expected_lon + 180.0) % 360.0 - 180.0).abs();
        assert!(lon_diff < 0.01, "lon={lon_deg}, expected {expected_lon}");
    }

    // ─── Precise path tests ──────────────────────────────────────

    /// Minimal all-zero EOP provider used by tests — `dut1 = xp = yp =
    /// dX = dY = 0`. Lets us build the full IAU 2006 chain without a
    /// real EOP table; the chain still exercises every scalar rotation
    /// (precession / nutation / ERA / polar motion `sp`).
    struct ZeroEop;
    impl Ut1Offset for ZeroEop {
        fn dut1(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    impl PolarMotion for ZeroEop {
        fn x_pole(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
        fn y_pole(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    impl NutationCorrections for ZeroEop {
        fn dx(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
        fn dy(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }

    /// Round-trip: build a `Vec3<Gcrs>` from a known ECEF position by
    /// applying the inverse IAU 2006 full chain, feed it through
    /// `precise_gcrs_to_geodetic_latlon`, and verify we recover the
    /// original geodetic lat/lon to degree tolerance.
    #[test]
    fn precise_gcrs_to_latlon_roundtrips_iss_inclination() {
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let expected_lat: f64 = 51.6;
        let expected_lon: f64 = 30.0;

        // Forward: Geodetic → SimpleEcef → Vec3<Itrs> (reinterpret) →
        // Vec3<Gcrs> (inverse full chain).
        let geod = arika::earth::Geodetic {
            latitude: expected_lat.to_radians(),
            longitude: expected_lon.to_radians(),
            altitude: 400.0,
        };
        let ecef_simple = arika::SimpleEcef::from(geod);
        let pos_itrs: Vec3<frame::Itrs> = Vec3::from_raw(*ecef_simple.inner());
        let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(&utc, &ZeroEop);
        let pos_gcrs: Vec3<frame::Gcrs> = rot.inverse().transform(&pos_itrs);

        // Reverse: precise path back to geodetic.
        let (lat_deg, lon_deg) = precise_gcrs_to_geodetic_latlon(&pos_gcrs, &utc, &ZeroEop);

        // Roundtrip should be near-exact (only floating-point
        // rounding in the 3×3 matrix inversion).
        assert!(
            (lat_deg - expected_lat).abs() < 1e-9,
            "lat={lat_deg}, expected {expected_lat}"
        );
        assert!(
            (lon_deg - expected_lon).abs() < 1e-9,
            "lon={lon_deg}, expected {expected_lon}"
        );
    }

    /// Simple-path vs precise-path divergence: with `ZeroEop`, the two
    /// entry points differ by the IAU 2006 precession + 2000A_R06
    /// nutation accumulated between J2000 and the test epoch. For a
    /// 2024 test date (~24.2 years from J2000), the precession in
    /// longitude `ψ_A` is about `5038.48″ × 0.242 ≈ 1220″ ≈ 0.34°`;
    /// the `(6778, 0, 0)` ECI position therefore lands on a lat/lon
    /// that differs from the simple path's result by about a tenth
    /// of a degree. Pins that the precise path is
    /// **not equal to** the simple path (== wiring catches an
    /// accidental identity) and **not off by more than 1°** (== no
    /// sign flip or wrong axis).
    #[test]
    fn precise_vs_simple_path_shows_expected_precession_magnitude() {
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        // Place the satellite on the ECI x-axis at ISS altitude.
        let pos_eci = SimpleEci::new(6778.0, 0.0, 0.0);
        let (lat_simple, lon_simple) = simple_eci_to_geodetic_latlon(&pos_eci, &utc);

        // Reinterpret the same components as GCRS — the test is
        // structural, so we treat the raw 3-vector as if it were the
        // GCRS representation of the same satellite.
        let pos_gcrs: Vec3<frame::Gcrs> = Vec3::from_raw(*pos_eci.inner());
        let (lat_precise, lon_precise) = precise_gcrs_to_geodetic_latlon(&pos_gcrs, &utc, &ZeroEop);

        let lat_delta = (lat_simple - lat_precise).abs();
        let lon_delta = (lon_simple - lon_precise).abs();

        // Upper bound: precession in longitude over 24 years is
        // < 0.5°, and rotating the (x, 0, 0) point on that manifold
        // can shift geodetic latitude by a comparable amount. 1° is
        // loose enough to absorb both without letting a sign flip slip
        // through (which would be ~180° off).
        assert!(
            lat_delta < 1.0,
            "precise−simple lat = {lat_delta}° exceeds 1° bound (sign flip?)"
        );
        assert!(
            lon_delta < 1.0,
            "precise−simple lon = {lon_delta}° exceeds 1° bound (sign flip?)"
        );
        // Lower bound: not exactly zero — the precise chain must
        // actually apply precession/nutation. At 2024 the combined
        // angle change is about 0.1° at minimum.
        assert!(
            lat_delta > 0.01 || lon_delta > 0.01,
            "precise vs simple differ by <0.01° (lat={lat_delta}, lon={lon_delta}) — \
             precession/nutation not being applied"
        );
    }
}
