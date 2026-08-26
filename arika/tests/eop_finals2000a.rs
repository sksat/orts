//! Tests for IERS finals2000A parser and EOP table.
//!
//! TDD: these tests are written first (Red), then the implementation
//! is added to make them pass (Green).

use arika::earth::eop::{
    EopTable, Finals2000A, LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset,
};

const SAMPLE: &str = include_str!("fixtures/finals2000A.sample");

// Parser tests

#[test]
fn parse_sample_returns_entries() {
    let entries = Finals2000A::parse(SAMPLE).expect("parse should succeed");
    assert!(
        entries.len() >= 10,
        "sample has 16 lines, should get at least 10 entries, got {}",
        entries.len()
    );
}

#[test]
fn parse_entries_have_monotonic_mjd() {
    let entries = Finals2000A::parse(SAMPLE).unwrap();
    for w in entries.windows(2) {
        assert!(
            w[1].mjd > w[0].mjd,
            "MJD not monotonic: {} -> {}",
            w[0].mjd,
            w[1].mjd
        );
    }
}

#[test]
fn parse_mjd_matches_expected_epoch() {
    let entries = Finals2000A::parse(SAMPLE).unwrap();
    // First line of our fixture is 2024-03-01, MJD 60370
    assert!(
        (entries[0].mjd - 60370.0).abs() < 0.01,
        "first MJD should be ~60370, got {}",
        entries[0].mjd
    );
}

#[test]
fn parse_2024_03_20_values() {
    // MJD 60389 = 2024-03-20
    // From fixture: xp=-0.013366, yp=0.313043, dut1=-0.0091657, lod=0.1693ms
    // Bulletin A nutation: dX=0.334, dY=-0.130
    // Bulletin B: xp=-0.013421, yp=0.313052, dut1=-0.0091683, dX=0.378, dY=-0.162
    let entries = Finals2000A::parse(SAMPLE).unwrap();
    let entry = entries
        .iter()
        .find(|e| (e.mjd - 60389.0).abs() < 0.01)
        .expect("should find MJD 60389");

    // B values preferred when available (Orekit compat)
    assert!(
        (entry.xp - (-0.013421)).abs() < 1e-6,
        "xp should be B value -0.013421, got {}",
        entry.xp
    );
    assert!(
        (entry.yp - 0.313052).abs() < 1e-6,
        "yp should be B value 0.313052, got {}",
        entry.yp
    );
    assert!(
        (entry.dut1 - (-0.0091683)).abs() < 1e-7,
        "dut1 should be B value -0.0091683, got {}",
        entry.dut1
    );
    // LOD is in ms in the file, stored in seconds
    assert!(entry.lod.is_some(), "LOD should be available for this date");
    assert!(
        (entry.lod.unwrap() - 0.1693e-3).abs() < 1e-7,
        "LOD should be ~0.1693 ms = {:.7e} s, got {:.7e}",
        0.1693e-3,
        entry.lod.unwrap()
    );
    // Nutation: B values preferred
    assert!(entry.dx.is_some(), "dX should be available");
    assert!(
        (entry.dx.unwrap() - 0.378).abs() < 0.01,
        "dX should be B value 0.378 mas, got {}",
        entry.dx.unwrap()
    );
}

#[test]
fn parse_empty_returns_error() {
    let result = Finals2000A::parse("");
    assert!(result.is_err(), "empty input should return error");
}

// EopTable tests

#[test]
fn table_from_finals2000a() {
    let table = EopTable::from_finals2000a(SAMPLE).expect("should build table");
    assert!(table.len() >= 10);
}

#[test]
fn table_mjd_range() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let (start, end) = table.mjd_range();
    assert!(start <= 60384.0 + 0.01);
    assert!(end >= 60399.0 - 0.01);
}

// Interpolation tests

#[test]
fn table_dut1_at_exact_entry() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    // At exact MJD 60389.0 (2024-03-20), should return the entry value
    let dut1 = table.dut1_checked(60389.0).unwrap();
    assert!(
        (dut1 - (-0.0091683)).abs() < 1e-6,
        "dut1 at MJD 60389 should be ~-0.0091683, got {dut1}"
    );
}

#[test]
fn table_dut1_interpolated_midpoint() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let entries = Finals2000A::parse(SAMPLE).unwrap();
    let e0 = entries
        .iter()
        .find(|e| (e.mjd - 60389.0).abs() < 0.01)
        .unwrap();
    let e1 = entries
        .iter()
        .find(|e| (e.mjd - 60390.0).abs() < 0.01)
        .unwrap();

    // The interpolated quantity is `UT1 - TAI = dUT1 - (TAI - UTC)`, which is
    // continuous; dUT1 itself is not. The expectation used to be the average of
    // the two rows' dUT1 — the implementation's own formula, which is why it
    // could not detect that the formula was wrong. It happens to agree here
    // because this fixture (2024-03) sits entirely inside one leap-second
    // regime, where TAI - UTC is the constant 37 s and cancels; the
    // leap-crossing case is pinned by `table_dut1_interpolates_ut1_minus_tai_*`.
    const TAI_MINUS_UTC_2024: f64 = 37.0;
    let ut1_tai_0 = e0.dut1 - TAI_MINUS_UTC_2024;
    let ut1_tai_1 = e1.dut1 - TAI_MINUS_UTC_2024;
    let expected = (ut1_tai_0 + ut1_tai_1) / 2.0 + TAI_MINUS_UTC_2024;

    let dut1 = table.dut1_checked(60389.5).unwrap();
    assert!(
        (dut1 - expected).abs() < 1e-10,
        "interpolated dut1 should be ~{expected}, got {dut1}"
    );
}

// Leap-second handling in the dUT1 interpolation

/// The two IERS rows bracketing the 2017-01-01 leap second (TAI-UTC 36 → 37 s).
/// A positive leap second is inserted as dUT1 approaches −0.9 s and lifts it by
/// a full second, so the daily rows differ by ~1.0 s.
fn leap_crossing_table() -> EopTable {
    use arika::earth::eop::EopEntry;
    let entry = |mjd: f64, dut1: f64| EopEntry {
        mjd,
        xp: 0.0,
        yp: 0.0,
        dut1,
        lod: None,
        dx: None,
        dy: None,
    };
    EopTable::new(vec![entry(57753.0, -0.5928), entry(57754.0, 0.4068)]).unwrap()
}

#[test]
fn table_dut1_interpolates_ut1_minus_tai_across_a_leap_second() {
    let table = leap_crossing_table();
    // Halfway through 2016-12-31 the leap second has not happened yet, so UT1 -
    // UTC is still near −0.593 s. Interpolating dUT1 directly would smear half
    // the 1 s step backwards and return ≈ −0.093 s: a 0.5 s UT1 error, i.e.
    // 3.65e-5 rad of ERA and ~233 m of equatorial ITRS displacement.
    let dut1 = table.dut1_checked(57753.5).unwrap();
    assert!(
        (dut1 - (-0.5928)).abs() < 0.01,
        "dUT1 at MJD 57753.5 should stay near -0.593 s, got {dut1}"
    );

    // Endpoints are still reproduced exactly.
    assert!((table.dut1_checked(57753.0).unwrap() - (-0.5928)).abs() < 1e-12);
    assert!((table.dut1_checked(57754.0).unwrap() - 0.4068).abs() < 1e-12);
}

#[test]
fn table_dut1_interpolates_ut1_minus_tai_continuously() {
    // Data-free invariant: `UT1 - TAI` is continuous, so sampling it finely
    // across the leap second must show no step larger than the day-to-day
    // drift. This holds for any table, without hand-picked expected values.
    let table = leap_crossing_table();
    const TAI_MINUS_UTC: [(f64, f64); 2] = [(57753.0, 36.0), (57754.0, 37.0)];
    let tai_utc = |mjd: f64| {
        if mjd >= TAI_MINUS_UTC[1].0 {
            TAI_MINUS_UTC[1].1
        } else {
            TAI_MINUS_UTC[0].1
        }
    };

    let step = 0.01;
    let mut prev: Option<f64> = None;
    let mut k = 0;
    while k <= 100 {
        let mjd = 57753.0 + step * k as f64;
        let ut1_tai = table.dut1_checked(mjd).unwrap() - tai_utc(mjd);
        if let Some(p) = prev {
            let jump = (ut1_tai - p).abs();
            assert!(
                jump < 1e-3,
                "UT1-TAI jumped by {jump} s between MJD {} and {mjd}",
                mjd - step
            );
        }
        prev = Some(ut1_tai);
        k += 1;
    }
}

#[test]
fn table_polar_motion_at_entry() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let xp = table.xp_checked(60389.0).unwrap();
    let yp = table.yp_checked(60389.0).unwrap();
    assert!((xp - (-0.013421)).abs() < 1e-6);
    assert!((yp - 0.313052).abs() < 1e-6);
}

#[test]
fn table_out_of_range_returns_error() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    assert!(table.dut1_checked(50000.0).is_err());
    assert!(table.dut1_checked(70000.0).is_err());
}

// EOP trait implementation tests

// The capability traits are implemented by the `clamped()` adapter, not by
// `EopTable` itself: a finite-range table has no correct infallible answer
// outside its span, so the out-of-range policy has to be named at the call site.

#[test]
fn trait_ut1_offset() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let dut1 = Ut1Offset::dut1(&table.clamped(), 60389.0);
    assert!((dut1 - (-0.0091683)).abs() < 1e-6);
}

#[test]
fn trait_polar_motion() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let xp = PolarMotion::x_pole(&table.clamped(), 60389.0);
    let yp = PolarMotion::y_pole(&table.clamped(), 60389.0);
    assert!((xp - (-0.013421)).abs() < 1e-6);
    assert!((yp - 0.313052).abs() < 1e-6);
}

#[test]
fn trait_nutation_corrections() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let dx = NutationCorrections::dx(&table.clamped(), 60389.0);
    assert!((dx - 0.378).abs() < 0.01);
}

#[test]
fn trait_length_of_day() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let lod = LengthOfDay::lod(&table.clamped(), 60389.0);
    // LOD in seconds
    assert!(
        (lod - 0.1693e-3).abs() < 1e-7,
        "LOD should be ~0.1693 ms, got {lod}"
    );
}

#[test]
fn out_of_range_epoch_reaches_the_caller_as_an_error_not_a_panic() {
    // The trait methods used to be implemented directly on `EopTable` with
    // `.expect(...)`, so an ordinary out-of-range epoch — a forward propagation
    // running past the end of a finals2000A prediction file — aborted the
    // process from inside `Epoch::to_ut1` / the IAU 2006 full chain.
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let (start, end) = table.mjd_range();

    for mjd in [start - 1.0, end + 1.0, start - 1000.0] {
        match table.dut1_checked(mjd) {
            Err(arika::earth::eop::EopLookupError::OutOfRange { .. }) => {}
            other => panic!("expected OutOfRange at MJD {mjd}, got {other:?}"),
        }
    }

    // The clamping adapter is the explicit "hold the endpoint" policy, and it
    // must not panic either.
    let clamped = table.clamped();
    assert_eq!(
        Ut1Offset::dut1(&clamped, start - 1.0),
        table.dut1_checked(start).unwrap()
    );
    assert_eq!(
        Ut1Offset::dut1(&clamped, end + 1.0),
        table.dut1_checked(end).unwrap()
    );
    assert!(PolarMotion::x_pole(&clamped, end + 500.0).is_finite());
    assert!(LengthOfDay::lod(&clamped, start - 500.0).is_finite());
}

#[test]
fn to_ut1_and_the_full_chain_accept_an_out_of_range_epoch_via_the_policy() {
    use arika::epoch::{Epoch, Utc};
    use arika::frame::{self, Rotation, Vec3};

    // 2024-03-20 is inside the fixture; 2030-01-01 is far past its end.
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let clamped = table.clamped();
    let far = Epoch::<Utc>::from_gregorian(2030, 1, 1, 0, 0, 0.0);

    let ut1 = far.to_ut1(&clamped);
    assert!(ut1.jd().is_finite(), "UT1 conversion must not abort");

    let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(&far, &clamped);
    let v = rot.transform(&Vec3::<frame::Gcrs>::new(1.0, 0.0, 0.0));
    assert!((v.magnitude() - 1.0).abs() < 1e-14);
}

// Integration: EopTable works with Rotation chain

#[test]
fn eop_table_works_with_iau2006_full() {
    use arika::epoch::{Epoch, Utc};
    use arika::frame::{self, Rotation, Vec3};

    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);

    // This should compile and not panic — real EOP data flowing through
    // the full IAU 2006 CIO chain.
    let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(&utc, &table.clamped());
    let v = Vec3::<frame::Gcrs>::new(1.0, 0.0, 0.0);
    let v_itrs = rot.transform(&v);

    // Magnitude should be preserved
    assert!(
        (v_itrs.magnitude() - 1.0).abs() < 1e-14,
        "rotation should preserve magnitude"
    );
}
