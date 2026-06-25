use super::leap::{LEAP_SECONDS, tai_minus_utc_at_mjd};
use super::*;
use core::f64::consts::TAU;

// Epoch construction and accessors

#[test]
fn mjd_jd_relationship() {
    let epoch = Epoch::from_jd(2451545.0);
    assert!((epoch.mjd() - 51544.5).abs() < 1e-12);
}

#[test]
fn scale_name_via_type() {
    assert_eq!(Epoch::<Utc>::scale_name(), "UTC");
    assert_eq!(Epoch::<Tai>::scale_name(), "TAI");
    assert_eq!(Epoch::<Tt>::scale_name(), "TT");
    assert_eq!(Epoch::<Tdb>::scale_name(), "TDB");
}

// Gregorian conversions

#[test]
fn j2000_gregorian() {
    // J2000.0 = 2000-01-01 12:00:00
    let epoch = Epoch::from_gregorian(2000, 1, 1, 12, 0, 0.0);
    assert!(
        (epoch.jd() - J2000_JD).abs() < 1e-6,
        "J2000 JD: expected {}, got {}",
        J2000_JD,
        epoch.jd()
    );
}

#[test]
fn known_date_2024_march_equinox() {
    // 2024-03-20 12:00:00 UTC
    let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let expected_jd = 2460390.0;
    assert!(
        (epoch.jd() - expected_jd).abs() < 0.01,
        "2024-03-20 JD: expected ~{}, got {}",
        expected_jd,
        epoch.jd()
    );
}

#[test]
fn gregorian_roundtrip() {
    let original = Epoch::from_gregorian(2024, 6, 21, 15, 30, 45.0);
    let dt = original.to_datetime();
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 6);
    assert_eq!(dt.day, 21);
    assert_eq!(dt.hour, 15);
    assert_eq!(dt.min, 30);
    assert!(
        (dt.sec - 45.0).abs() < 0.01,
        "sec: expected 45.0, got {}",
        dt.sec
    );
}

#[test]
fn gregorian_roundtrip_january() {
    // Jan and Feb use different month adjustment in JD algorithm
    let original = Epoch::from_gregorian(2024, 1, 15, 0, 0, 0.0);
    let dt = original.to_datetime();
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 1);
    assert_eq!(dt.day, 15);
    assert_eq!(dt.hour, 0);
    assert_eq!(dt.min, 0);
}

#[test]
fn gregorian_roundtrip_february() {
    let original = Epoch::from_gregorian(2024, 2, 29, 6, 0, 0.0);
    let dt = original.to_datetime();
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 2);
    assert_eq!(dt.day, 29);
    assert_eq!(dt.hour, 6);
}

#[test]
fn datetime_display() {
    let dt = DateTime::new(2024, 3, 20, 12, 0, 0.0);
    assert_eq!(dt.to_string(), "2024-03-20T12:00:00Z");
}

#[test]
fn from_datetime_roundtrip() {
    let dt = DateTime::new(2024, 6, 21, 15, 30, 45.0);
    let epoch = Epoch::from_datetime(&dt);
    let rt = epoch.to_datetime();
    assert_eq!(rt.year, dt.year);
    assert_eq!(rt.month, dt.month);
    assert_eq!(rt.day, dt.day);
    assert_eq!(rt.hour, dt.hour);
    assert_eq!(rt.min, dt.min);
    assert!((rt.sec - dt.sec).abs() < 0.01);
}

// add_seconds / add_si_seconds

#[test]
fn add_seconds_one_day() {
    let epoch = Epoch::j2000();
    let next_day = epoch.add_seconds(86400.0);
    assert!(
        (next_day.jd() - (J2000_JD + 1.0)).abs() < 1e-12,
        "add 86400s: expected JD {}, got {}",
        J2000_JD + 1.0,
        next_day.jd()
    );
}

#[test]
fn now_returns_reasonable_jd() {
    let epoch = Epoch::now();
    // JD for 2025-01-01 ≈ 2460676, for 2030-01-01 ≈ 2462502
    // Any reasonable current date should be in this range
    assert!(
        epoch.jd() > 2460676.0 && epoch.jd() < 2462502.0,
        "Epoch::now() JD {} is outside 2025–2030 range",
        epoch.jd()
    );
    // Verify to_datetime year is plausible
    let dt = epoch.to_datetime();
    assert!(
        dt.year >= 2025 && dt.year <= 2030,
        "Epoch::now() year {} is outside expected range",
        dt.year
    );
}

#[test]
fn add_seconds_one_hour() {
    let epoch = Epoch::j2000();
    let plus_hour = epoch.add_seconds(3600.0);
    let expected = J2000_JD + 1.0 / 24.0;
    assert!((plus_hour.jd() - expected).abs() < 1e-12);
}

#[test]
fn centuries_since_j2000() {
    let epoch = Epoch::j2000();
    assert!((epoch.centuries_since_j2000() - 0.0).abs() < 1e-15);

    // One Julian century later
    let later = Epoch::from_jd(J2000_JD + JULIAN_CENTURY);
    assert!((later.centuries_since_j2000() - 1.0).abs() < 1e-12);
}

// Discriminating Red tests for Phase 1A
//
// These tests verify behaviors that are only achievable with the new
// Epoch<Scale> design. They would fail with the pre-refactor naive
// `Epoch { jd: f64 }` implementation.

/// **Discriminating test**: `add_si_seconds` crossing the 2017-01-01 leap
/// second boundary must absorb one SI second into the leap, landing one
/// UTC second earlier than naive `add_seconds` would predict.
#[test]
fn leap_second_2017_crossing_si_arithmetic() {
    let before = Epoch::<Utc>::from_iso8601("2016-12-31T23:59:55Z").unwrap();
    let naive = before.add_seconds(10.0);
    let aware = before.add_si_seconds(10.0);

    // Naive arithmetic: 10 "JD-seconds" later = 2017-01-01T00:00:05Z
    // (because UTC JD treats the leap day as exactly 86400 seconds)
    let dt_naive = naive.to_datetime();
    assert_eq!(dt_naive.year, 2017);
    assert_eq!(dt_naive.month, 1);
    assert_eq!(dt_naive.day, 1);
    assert_eq!(dt_naive.hour, 0);
    assert_eq!(dt_naive.min, 0);
    assert!((dt_naive.sec - 5.0).abs() < 0.01);

    // Leap-second-aware: 10 SI seconds from 23:59:55 traverses the
    // 23:59:60 leap, so UTC display shows one fewer second elapsed.
    let dt_aware = aware.to_datetime();
    assert_eq!(dt_aware.year, 2017);
    assert_eq!(dt_aware.month, 1);
    assert_eq!(dt_aware.day, 1);
    assert_eq!(dt_aware.hour, 0);
    assert_eq!(dt_aware.min, 0);
    assert!(
        (dt_aware.sec - 4.0).abs() < 0.01,
        "add_si_seconds should absorb leap second: expected ~4.0 s, got {}",
        dt_aware.sec
    );
}

#[test]
fn from_iso8601_ordinal_day_of_year() {
    // CCSDS ordinal form: 2024-079 is the 79th day of 2024 = March 19.
    let calendar = Epoch::<Utc>::from_iso8601("2024-03-19T12:00:00Z").unwrap();
    let ordinal = Epoch::<Utc>::from_iso8601("2024-079T12:00:00Z").unwrap();
    assert!(
        (calendar.jd() - ordinal.jd()).abs() < 1e-9,
        "ordinal and calendar forms must denote the same instant"
    );
}

#[test]
fn from_iso8601_ordinal_validation() {
    // A truncated calendar date (2-/1-digit field) must NOT be read as ordinal.
    assert!(Epoch::<Utc>::from_iso8601("2024-03T12:00:00Z").is_none());
    assert!(Epoch::<Utc>::from_iso8601("2024-3T12:00:00Z").is_none());
    // Day 366 is valid only in a leap year.
    assert!(Epoch::<Utc>::from_iso8601("2024-366T00:00:00Z").is_some()); // 2024 leap
    assert!(Epoch::<Utc>::from_iso8601("2023-366T00:00:00Z").is_none()); // 2023 common
    // Out-of-range ordinals are rejected.
    assert!(Epoch::<Utc>::from_iso8601("2024-000T00:00:00Z").is_none());
    assert!(Epoch::<Utc>::from_iso8601("2024-367T00:00:00Z").is_none());
}

#[test]
fn from_iso8601_z_suffix_optional() {
    let with_z = Epoch::<Utc>::from_iso8601("2024-03-19T12:00:00Z").unwrap();
    let no_z = Epoch::<Utc>::from_iso8601("2024-03-19T12:00:00").unwrap();
    assert_eq!(with_z.jd(), no_z.jd());
    // Fractional seconds without 'Z' (CelesTrak OMM style).
    let frac = Epoch::<Utc>::from_iso8601("2024-03-19T12:00:00.000000").unwrap();
    assert_eq!(frac.jd(), with_z.jd());
}

#[test]
fn from_year_day_of_year_matches_tle_pivot() {
    // The 4-digit DOY constructor agrees with the 2-digit TLE one for 2024.
    let direct = Epoch::<Utc>::from_year_day_of_year(2024, 79.5);
    let via_tle = Epoch::<Utc>::from_tle_epoch(24, 79.5);
    assert_eq!(direct.jd(), via_tle.jd());
}

/// **Discriminating test**: Converting a UTC epoch to TDB must produce a
/// JD that differs by ~69.184 s (leap count + TT-TAI + Fairhead).
///
/// With the pre-refactor naive implementation, UTC JD was fed directly
/// into Meeus ephemerides as if it were TDB, causing the Artemis 1 69-km
/// position offset described in orts/examples/artemis1/main.rs:29-32.
#[test]
fn utc_to_tdb_applies_expected_offset_2024() {
    let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
    let tdb = utc.to_tdb();
    let delta_sec = (tdb.jd() - utc.jd()) * 86400.0;

    // Expected: 37 (leap) + 32.184 (TT-TAI) + ~1.6 ms (Fairhead) ≈ 69.184 s
    let expected_sec = 37.0 + TT_MINUS_TAI_SEC;
    assert!(
        (delta_sec - expected_sec).abs() < 0.01,
        "TDB - UTC at 2024-03-20: expected ~{} s, got {} s",
        expected_sec,
        delta_sec
    );
}

/// **Discriminating test**: Round-trip `to_tdb().to_tt().to_tai().to_utc()`
/// from a UTC epoch must recover the original UTC JD bit-for-bit
/// (within f64 precision).
#[test]
fn utc_tdb_tt_tai_roundtrip() {
    let original = Epoch::<Utc>::from_iso8601("2024-06-15T08:30:45Z").unwrap();
    let tdb = original.to_tdb();
    let tt = tdb.to_tt();
    let tai = tt.to_tai();
    let utc = tai.to_utc();
    assert!(
        (utc.jd() - original.jd()).abs() < 1e-10,
        "UTC→TDB→TT→TAI→UTC roundtrip diverged: original={} recovered={}",
        original.jd(),
        utc.jd()
    );
}

/// **Discriminating test**: `Epoch<Tt>::centuries_since_j2000` and
/// `Epoch<Tdb>::centuries_since_j2000` must differ from each other AND
/// from `Epoch<Utc>::centuries_since_j2000` by the expected offsets.
///
/// With the pre-refactor implementation all three were the same method
/// returning the same value (ignoring scale), which was the root cause
/// of the UTC-as-TDB silent bug.
#[test]
fn centuries_since_j2000_differs_per_scale() {
    let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
    let tt = utc.to_tt();
    let tdb = utc.to_tdb();

    let c_utc = utc.centuries_since_j2000();
    let c_tt = tt.centuries_since_j2000();
    let c_tdb = tdb.centuries_since_j2000();

    // TT - UTC ≈ 69.184 s = 2.19e-8 centuries
    let dc_tt_utc = c_tt - c_utc;
    let expected_tt_utc = 69.184 / (86400.0 * 36525.0);
    assert!(
        (dc_tt_utc - expected_tt_utc).abs() < 1e-14,
        "TT - UTC centuries: expected {:e}, got {:e}",
        expected_tt_utc,
        dc_tt_utc
    );

    // TDB - TT ≈ few ms peak-to-peak (Fairhead periodic), ~5e-13 centuries.
    // Much smaller than TT - UTC (~2.2e-8) but still detectable.
    let dc_tdb_tt = c_tdb - c_tt;
    assert!(
        dc_tdb_tt.abs() < 1e-11,
        "TDB - TT centuries should be ~ms → ~5e-13 scale, got {:e}",
        dc_tdb_tt
    );
    assert!(
        dc_tdb_tt.abs() < dc_tt_utc.abs() * 0.001,
        "TDB - TT should be much smaller than TT - UTC: dc_tdb_tt={:e}, dc_tt_utc={:e}",
        dc_tdb_tt,
        dc_tt_utc
    );
}

// ISO 8601 parsing

#[test]
fn iso8601_valid() {
    let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
    let expected = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    assert!(
        (epoch.jd() - expected.jd()).abs() < 1e-10,
        "ISO parse mismatch"
    );
}

#[test]
fn iso8601_with_seconds() {
    let epoch = Epoch::from_iso8601("2000-01-01T12:00:00Z").unwrap();
    assert!((epoch.jd() - J2000_JD).abs() < 1e-6);
}

#[test]
fn iso8601_no_z_is_accepted() {
    // 'Z' is now optional — a bare UTC timestamp parses (CCSDS OMM omits it).
    assert!(Epoch::from_iso8601("2024-03-20T12:00:00").is_some());
}

#[test]
fn iso8601_invalid_format() {
    assert!(Epoch::from_iso8601("not-a-date").is_none());
    assert!(Epoch::from_iso8601("2024-13-01T00:00:00Z").is_none()); // month 13
    assert!(Epoch::from_iso8601("2024-01-32T00:00:00Z").is_none()); // day 32
}

// ERA / legacy GMST

#[test]
fn gmst_at_j2000() {
    let epoch = Epoch::j2000();
    let gmst = epoch.gmst();
    // At J2000.0, ERA ≈ 280.46° = 4.8949 rad
    let expected = TAU * 0.7790572732640;
    assert!(
        (gmst - expected).abs() < 0.01,
        "GMST at J2000: expected {:.4} rad, got {:.4} rad",
        expected,
        gmst
    );
}

#[test]
fn gmst_increases_one_sidereal_day() {
    // One sidereal day ≈ 86164.0905 seconds
    // After one solar day (86400s), GMST should increase by ~360.9856° ≈ ~2π + 0.0172 rad
    let epoch = Epoch::j2000();
    let gmst0 = epoch.gmst();
    let next_day = epoch.add_seconds(86400.0);
    let gmst1 = next_day.gmst();

    let delta = if gmst1 > gmst0 {
        gmst1 - gmst0
    } else {
        gmst1 + TAU - gmst0
    };
    let expected_delta = TAU * 1.002_737_811_911_354_6;
    let expected_delta_mod = expected_delta % TAU;
    assert!(
        (delta - expected_delta_mod).abs() < 0.001,
        "GMST daily increase: expected {:.6} rad, got {:.6} rad",
        expected_delta_mod,
        delta
    );
}

#[test]
fn gmst_normalized() {
    // GMST should always be in [0, 2π)
    for days in [0.0, 0.5, 1.0, 100.0, 365.25, 3652.5] {
        let epoch = Epoch::j2000().add_seconds(days * 86400.0);
        let gmst = epoch.gmst();
        assert!(
            gmst >= 0.0 && gmst < TAU,
            "GMST at +{days} days: {gmst} not in [0, 2π)"
        );
    }
}

#[test]
fn era_on_ut1_matches_legacy_gmst() {
    // Utc.to_ut1_naive().era() should equal the legacy gmst() on the same
    // UTC epoch bit-for-bit (since both use the same ERA formula and naive
    // UT1 = UTC assumption).
    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let ut1 = utc.to_ut1_naive();
    assert_eq!(ut1.era(), utc.gmst());
}

// to_ut1 with EOP provider

#[test]
fn to_ut1_applies_dut1_offset() {
    // A mock EOP provider supplying a fixed dUT1 of -0.250 s should
    // shift the UT1 JD by exactly -0.250/86400 days relative to UTC.
    struct FixedDut1(f64);
    impl crate::earth::eop::Ut1Offset for FixedDut1 {
        fn dut1(&self, _utc_mjd: f64) -> f64 {
            self.0
        }
    }

    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let eop = FixedDut1(-0.250);
    let ut1 = utc.to_ut1(&eop);

    // JD around 2.46e6 has ~1 ULP ≈ 5.6e-10 days ≈ 4.8e-5 s resolution,
    // so the reconstructed delta is accurate only to ~10 μs.
    let delta_s = (ut1.jd() - utc.jd()) * 86400.0;
    assert!(
        (delta_s - (-0.250)).abs() < 1e-4,
        "expected -0.250 s shift, got {delta_s}"
    );
}

#[test]
fn to_ut1_naive_is_equivalent_to_zero_dut1_provider() {
    // to_ut1_naive() == to_ut1(&provider with dut1 == 0).
    struct ZeroDut1;
    impl crate::earth::eop::Ut1Offset for ZeroDut1 {
        fn dut1(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let naive = utc.to_ut1_naive();
    let precise = utc.to_ut1(&ZeroDut1);
    assert_eq!(naive.jd(), precise.jd());
}

#[test]
fn to_ut1_accepts_trait_object_provider() {
    // The `?Sized` bound on `to_ut1<P>` lets callers pass `&dyn Ut1Offset`
    // / `Box<dyn Ut1Offset>` directly, which is essential for runtime
    // provider selection (e.g. a plugin-supplied EOP source).
    struct Fixed(f64);
    impl crate::earth::eop::Ut1Offset for Fixed {
        fn dut1(&self, _: f64) -> f64 {
            self.0
        }
    }
    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
    let boxed: Box<dyn crate::earth::eop::Ut1Offset> = Box::new(Fixed(-0.100));
    let _ut1_box: Ut1Epoch = utc.to_ut1(boxed.as_ref());
    let dyn_ref: &dyn crate::earth::eop::Ut1Offset = &Fixed(-0.100);
    let _ut1_dyn: Ut1Epoch = utc.to_ut1(dyn_ref);
}

#[test]
fn to_ut1_passes_utc_mjd_to_provider() {
    // Verify the UTC MJD passed to the provider matches `epoch.mjd()`.
    use std::cell::Cell;
    struct Recording(Cell<f64>);
    impl crate::earth::eop::Ut1Offset for Recording {
        fn dut1(&self, utc_mjd: f64) -> f64 {
            self.0.set(utc_mjd);
            0.0
        }
    }
    let utc = Epoch::<Utc>::from_gregorian(2024, 1, 1, 0, 0, 0.0);
    let r = Recording(Cell::new(f64::NAN));
    let _ = utc.to_ut1(&r);
    assert_eq!(r.0.get(), utc.mjd());
}

// TLE epoch

#[test]
fn tle_epoch_iss_2024() {
    // ISS TLE epoch: 24079.50000000 → 2024 day 79.5 → 2024-03-19 12:00:00 UTC
    let epoch = Epoch::from_tle_epoch(24, 79.5);
    let dt = epoch.to_datetime();
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 3);
    assert_eq!(dt.day, 19);
    assert_eq!(dt.hour, 12);
}

#[test]
fn tle_epoch_year_2000() {
    // Year 00 → 2000, day 1.0 → 2000-01-01 00:00:00
    let epoch = Epoch::from_tle_epoch(0, 1.0);
    let dt = epoch.to_datetime();
    assert_eq!(dt.year, 2000);
    assert_eq!(dt.month, 1);
    assert_eq!(dt.day, 1);
    assert_eq!(dt.hour, 0);
}

#[test]
fn tle_epoch_year_1999() {
    // Year 99 → 1999, day 365.0 → 1999-12-31 00:00:00
    let epoch = Epoch::from_tle_epoch(99, 365.0);
    let dt = epoch.to_datetime();
    assert_eq!(dt.year, 1999);
    assert_eq!(dt.month, 12);
    assert_eq!(dt.day, 31);
}

#[test]
fn tle_epoch_year_57() {
    // Year 57 → 1957 (Sputnik era)
    let epoch = Epoch::from_tle_epoch(57, 1.0);
    let dt = epoch.to_datetime();
    assert_eq!(dt.year, 1957);
    assert_eq!(dt.month, 1);
    assert_eq!(dt.day, 1);
}

#[test]
fn tle_epoch_year_56() {
    // Year 56 → 2056
    let epoch = Epoch::from_tle_epoch(56, 1.0);
    let dt = epoch.to_datetime();
    assert_eq!(dt.year, 2056);
}

#[test]
fn tle_epoch_matches_iso8601() {
    // TLE epoch 24001.50000000 → 2024-01-01 12:00:00 UTC
    let tle_epoch = Epoch::from_tle_epoch(24, 1.5);
    let iso_epoch = Epoch::from_iso8601("2024-01-01T12:00:00Z").unwrap();
    assert!(
        (tle_epoch.jd() - iso_epoch.jd()).abs() < 1e-6,
        "TLE epoch {} vs ISO epoch {}",
        tle_epoch.jd(),
        iso_epoch.jd()
    );
}

// JD → UTC string end-to-end

#[test]
fn jd_to_utc_string_j2000() {
    let s = Epoch::from_jd(J2000_JD).to_datetime().to_string();
    assert_eq!(s, "2000-01-01T12:00:00Z");
}

#[test]
fn jd_to_utc_string_2024_march() {
    let s = Epoch::from_jd(2460390.0).to_datetime().to_string();
    assert_eq!(s, "2024-03-20T12:00:00Z");
}

#[test]
fn jd_to_utc_string_with_offset_1h() {
    // J2000 + 3600s = 2000-01-01T13:00:00Z
    let s = Epoch::from_jd(J2000_JD)
        .add_seconds(3600.0)
        .to_datetime()
        .to_string();
    assert_eq!(s, "2000-01-01T13:00:00Z");
}

#[test]
fn jd_to_utc_string_with_offset_1day() {
    // J2000 + 86400s = 2000-01-02T12:00:00Z
    let s = Epoch::from_jd(J2000_JD)
        .add_seconds(86400.0)
        .to_datetime()
        .to_string();
    assert_eq!(s, "2000-01-02T12:00:00Z");
}

#[test]
fn jd_to_utc_string_no_fractional_seconds() {
    // Fractional seconds should be truncated (format uses {:02.0})
    let s = Epoch::from_jd(J2000_JD)
        .add_seconds(0.5)
        .to_datetime()
        .to_string();
    assert!(
        s.ends_with("Z") && !s.contains('.'),
        "Should not contain fractional seconds: {s}"
    );
}

#[test]
fn gmst_works_with_simple_eci_ecef() {
    // Verify that Epoch::gmst() produces valid angles for
    // SimpleEci↔SimpleEcef conversion via Rotation<SimpleEci, SimpleEcef>.
    use crate::SimpleEci;
    use crate::frame::{Rotation, SimpleEcef as SimpleEcefMarker, SimpleEci as SimpleEciMarker};
    let epoch = Epoch::from_gregorian(2024, 6, 21, 12, 0, 0.0);
    let era = epoch.gmst();

    let eci = SimpleEci::new(7000.0, 1000.0, 500.0);
    let ecef = Rotation::<SimpleEciMarker, SimpleEcefMarker>::from_era(era).transform(&eci);
    let roundtrip = Rotation::<SimpleEcefMarker, SimpleEciMarker>::from_era(era).transform(&ecef);

    let eps = 1e-10;
    assert!((roundtrip.x() - eci.x()).abs() < eps);
    assert!((roundtrip.y() - eci.y()).abs() < eps);
    assert!((roundtrip.z() - eci.z()).abs() < eps);
}

// Leap second table sanity

#[test]
fn leap_second_table_monotonic() {
    // TAI-UTC should strictly increase over time.
    let mut prev_mjd = 0.0;
    let mut prev_offset = 0.0;
    for &(mjd, offset) in LEAP_SECONDS {
        assert!(mjd > prev_mjd, "Leap table MJD not monotonic: {mjd}");
        assert!(offset > prev_offset, "Leap offset not monotonic: {offset}");
        prev_mjd = mjd;
        prev_offset = offset;
    }
}

#[test]
fn leap_second_2024_is_37() {
    // MJD 60000 ≈ 2023-02-25, well after the 2017-01-01 leap entry (MJD 57754)
    assert_eq!(tai_minus_utc_at_mjd(60000.0), 37.0);
}

#[test]
fn leap_second_before_1972_is_10() {
    // Pre-1972: default to the first table value.
    assert_eq!(tai_minus_utc_at_mjd(40000.0), 10.0);
}

#[test]
fn duration_si_seconds() {
    assert_eq!(Duration::from_si_seconds(60.0).as_si_seconds(), 60.0);
    assert_eq!(Duration::from_minutes(1.0).as_si_seconds(), 60.0);
    assert_eq!(Duration::from_hours(1.0).as_si_seconds(), 3600.0);
}

#[test]
fn fixed_offset_from_tai_constants() {
    assert_eq!(<Tai as FixedOffsetFromTai>::SECONDS_AFTER_TAI, 0.0);
    assert_eq!(
        <Tt as FixedOffsetFromTai>::SECONDS_AFTER_TAI,
        TT_MINUS_TAI_SEC
    );
    assert_eq!(<Gps as FixedOffsetFromTai>::SECONDS_AFTER_TAI, -19.0);
}

// GPS Time

#[test]
fn gps_scale_name() {
    assert_eq!(Epoch::<Gps>::scale_name(), "GPS");
}

// Tolerance for sub-second offsets recovered by subtracting two ~2.46e6-day
// Julian Dates: the difference inherits ~1 ULP of the operands (≈48 µs at
// modern JDs). This f64 single-JD floor is removed by the planned two-part
// representation; until then offset assertions stay above it.
const OFFSET_TOL_SEC: f64 = 1e-4;

#[test]
fn gps_is_tai_minus_19s() {
    // TAI − GPS = 19 s exactly, with no leap-second dependence.
    let tai = Epoch::<Tai>::from_jd_tai(2460000.5);
    let gps = tai.to_gps();
    let delta_sec = (tai.jd() - gps.jd()) * 86400.0;
    assert!(
        (delta_sec - 19.0).abs() < OFFSET_TOL_SEC,
        "TAI − GPS: expected 19 s, got {delta_sec} s"
    );
}

#[test]
fn gps_tai_roundtrip_is_bit_exact() {
    // GPS is a fixed offset, so GPS → TAI → GPS recovers the input exactly.
    let gps = Epoch::<Gps>::from_jd_gps(2460123.456);
    assert_eq!(gps.to_tai().to_gps().jd().to_bits(), gps.jd().to_bits());
}

#[test]
fn gps_minus_utc_is_18s_after_2017() {
    // 2017-01-01 onward: leap = 37 s, so GPS − UTC = 37 − 19 = 18 s.
    let utc = Epoch::<Utc>::from_iso8601("2024-03-20T12:00:00Z").unwrap();
    let gps = utc.to_gps();
    let delta_sec = (gps.jd() - utc.jd()) * 86400.0;
    assert!(
        (delta_sec - 18.0).abs() < OFFSET_TOL_SEC,
        "GPS − UTC in 2024: expected 18 s, got {delta_sec} s"
    );
}

/// **Discriminating test**: GPS tracks TAI continuously (no leap second), so
/// `GPS − UTC` must *step* by exactly the leap-second insertion (17 s → 18 s)
/// across the 2017-01-01 boundary, while GPS itself stays uniform.
#[test]
fn gps_minus_utc_steps_across_2017_leap() {
    let before = Epoch::<Utc>::from_iso8601("2016-06-01T00:00:00Z").unwrap();
    let after = Epoch::<Utc>::from_iso8601("2017-06-01T00:00:00Z").unwrap();
    let d_before = (before.to_gps().jd() - before.jd()) * 86400.0;
    let d_after = (after.to_gps().jd() - after.jd()) * 86400.0;
    assert!(
        (d_before - 17.0).abs() < OFFSET_TOL_SEC,
        "GPS − UTC before 2017 leap: expected 17 s, got {d_before} s"
    );
    assert!(
        (d_after - 18.0).abs() < OFFSET_TOL_SEC,
        "GPS − UTC after 2017 leap: expected 18 s, got {d_after} s"
    );
}

#[test]
fn gps_utc_roundtrip() {
    let utc = Epoch::<Utc>::from_iso8601("2024-06-15T08:30:45Z").unwrap();
    let recovered = utc.to_gps().to_utc();
    assert!(
        (recovered.jd() - utc.jd()).abs() < 1e-10,
        "UTC → GPS → UTC diverged: original={}, recovered={}",
        utc.jd(),
        recovered.jd()
    );
}

// Characterization of the scale conversion graph against a reference.
//
// Each row holds the f64 output of every conversion edge for a UTC input given
// by `cols[0]`, captured from the pre-canonical single-f64 implementation.
// The canonical-TAI representation computes these in two parts (higher
// precision), so the values are no longer *bit*-exact — they match the
// reference within a tight tolerance (~ULP). Inputs include leap-second
// boundaries, a pre-1972 date, and non-finite values (NaN, ±∞).
//
// Column order:
//   0: utc.jd()                       5: utc.to_tai().to_tt().jd()
//   1: utc.to_tai().jd()              6: utc.to_tai().to_utc().jd()
//   2: utc.to_tt().jd()               7: utc.to_tt().to_tai().jd()
//   3: utc.to_tdb().jd()              8: utc.to_tt().to_tdb().jd()
//   4: utc.to_ut1_naive().jd()        9: utc.to_tdb().to_tt().jd()
#[rustfmt::skip]
const CONVERSION_GOLDEN: &[(&str, [u64; 10])] = &[
    ("j2000",         [0x4142b42c80000000, 0x4142b42c800c22e4, 0x4142b42c801857a6, 0x4142b42c801857a4, 0x4142b42c80000000, 0x4142b42c801857a6, 0x4142b42c80000000, 0x4142b42c800c22e4, 0x4142b42c801857a4, 0x4142b42c801857a6]),
    ("y2024",         [0x4142c57300000000, 0x4142c573000e0858, 0x4142c573001a3d1a, 0x4142c573001a3d42, 0x4142c57300000000, 0x4142c573001a3d1a, 0x4142c57300000000, 0x4142c573000e0858, 0x4142c573001a3d42, 0x4142c573001a3d1a]),
    ("pre_leap_2017", [0x4142c04d3fff9ee9, 0x4142c04d400d462a, 0x4142c04d40197aec, 0x4142c04d40197aea, 0x4142c04d3fff9ee9, 0x4142c04d40197aec, 0x4142c04d3fff9ee9, 0x4142c04d400d462a, 0x4142c04d40197aea, 0x4142c04d40197aec]),
    ("apollo11",      [0x41429e73ac2d82d8, 0x41429e73ac314dbf, 0x41429e73ac3d8281, 0x41429e73ac3d8276, 0x41429e73ac2d82d8, 0x41429e73ac3d8281, 0x41429e73ac2d82d8, 0x41429e73ac314dbf, 0x41429e73ac3d8276, 0x41429e73ac3d8281]),
    ("nan",           [0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000, 0x7ff8000000000000]),
    ("pinf",          [0x7ff0000000000000, 0x7ff0000000000000, 0x7ff0000000000000, 0xfff8000000000000, 0x7ff0000000000000, 0x7ff0000000000000, 0x7ff0000000000000, 0x7ff0000000000000, 0xfff8000000000000, 0xfff8000000000000]),
    ("ninf",          [0xfff0000000000000, 0xfff0000000000000, 0xfff0000000000000, 0xfff8000000000000, 0xfff0000000000000, 0xfff0000000000000, 0xfff0000000000000, 0xfff0000000000000, 0xfff8000000000000, 0xfff8000000000000]),
];

#[test]
fn conversion_graph_matches_reference() {
    const LABELS: [&str; 10] = [
        "utc.jd",
        "utc.to_tai",
        "utc.to_tt",
        "utc.to_tdb",
        "utc.to_ut1_naive",
        "utc.to_tai.to_tt",
        "utc.to_tai.to_utc",
        "utc.to_tt.to_tai",
        "utc.to_tt.to_tdb",
        "utc.to_tdb.to_tt",
    ];
    // The two-part canonical result differs from the single-f64 reference by at
    // most ~1 ULP of a modern JD (~5.5e-10 day); 1e-9 day is a safe bound.
    const TOL_DAY: f64 = 1e-9;
    for (name, want) in CONVERSION_GOLDEN {
        let utc = Epoch::<Utc>::from_jd(f64::from_bits(want[0]));
        let tai = utc.to_tai();
        let tt = utc.to_tt();
        let tdb = utc.to_tdb();
        let got: [f64; 10] = [
            utc.jd(),
            tai.jd(),
            tt.jd(),
            tdb.jd(),
            utc.to_ut1_naive().jd(),
            tai.to_tt().jd(),
            tai.to_utc().jd(),
            tt.to_tai().jd(),
            tt.to_tdb().jd(),
            tdb.to_tt().jd(),
        ];
        for i in 0..10 {
            let want_v = f64::from_bits(want[i]);
            // Non-finite reference (NaN / ±∞) comes from a non-finite *input*
            // (the nan/pinf/ninf rows). Two-part arithmetic on non-finite values
            // yields NaN where single-f64 yielded ±∞ — both are garbage-in /
            // garbage-out; the meaningful guarantee is that no non-finite input
            // silently produces a finite result. Finite values match within tol.
            if !want_v.is_finite() {
                assert!(
                    !got[i].is_finite(),
                    "{name}: {} non-finite input produced finite {}",
                    LABELS[i],
                    got[i]
                );
            } else {
                assert!(
                    (got[i] - want_v).abs() < TOL_DAY,
                    "{name}: {} diverged from reference: want {want_v}, got {} (Δ {:e} day)",
                    LABELS[i],
                    got[i],
                    got[i] - want_v
                );
            }
        }
    }
}

/// The canonical-TAI representation's payoff: `duration_since` recovers a
/// sub-µs interval that the single-f64 JD floor (~tens of µs near modern
/// epochs) would lose entirely.
#[test]
fn duration_since_recovers_sub_microsecond_interval() {
    let e0 = Epoch::<Utc>::from_iso8601("2024-06-15T08:30:45Z").unwrap();
    let e1 = e0.add_si_seconds(1e-9); // 1 ns later
    let d = e1.duration_since(&e0).as_si_seconds();
    assert!(
        (d - 1e-9).abs() < 1e-12,
        "1 ns interval not recovered: got {d} s"
    );
    // Precondition: differencing the f64 read-outs floors the 1 ns to 0.
    assert_eq!((e1.jd() - e0.jd()) * 86400.0, 0.0);
}

/// `add_si_seconds` is an exact SI add on the uniform TAI line: `duration_since`
/// recovers exactly the added amount, including across a leap-second boundary.
/// (The pre-canonical UTC-iteration version drifted by ~1 s for results landing
/// inside the inserted second; the canonical uniform-TAI add is correct, as the
/// time-systems guidance prescribes — "do duration math in TAI/TT".)
#[test]
fn add_si_seconds_is_exact_si_across_leap() {
    // Starts just before the 2017-01-01 leap so several steps land in/after it.
    let e = Epoch::<Utc>::from_iso8601("2016-12-31T23:59:59Z").unwrap();
    for dt in [0.5_f64, 1.0, 1.5, 2.0, 10.0, 86_400.0] {
        let elapsed = e.add_si_seconds(dt).duration_since(&e).as_si_seconds();
        assert!(
            (elapsed - dt).abs() < 1e-6,
            "add_si_seconds({dt}) → elapsed {elapsed} s"
        );
    }
}
