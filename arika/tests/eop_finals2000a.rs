//! Tests for IERS finals2000A parser and EOP table.
//!
//! TDD: these tests are written first (Red), then the implementation
//! is added to make them pass (Green).

use arika::earth::eop::{
    EopTable, Finals2000A, LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset,
};

const SAMPLE: &str = include_str!("fixtures/finals2000A.sample");

// ============================================================================
// Parser tests
// ============================================================================

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

// ============================================================================
// EopTable tests
// ============================================================================

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

// ============================================================================
// Interpolation tests
// ============================================================================

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
    // Midpoint between MJD 60389 and 60390 should be average of the two
    let entries = Finals2000A::parse(SAMPLE).unwrap();
    let e0 = entries
        .iter()
        .find(|e| (e.mjd - 60389.0).abs() < 0.01)
        .unwrap();
    let e1 = entries
        .iter()
        .find(|e| (e.mjd - 60390.0).abs() < 0.01)
        .unwrap();
    let expected = (e0.dut1 + e1.dut1) / 2.0;

    let dut1 = table.dut1_checked(60389.5).unwrap();
    assert!(
        (dut1 - expected).abs() < 1e-10,
        "interpolated dut1 should be ~{expected}, got {dut1}"
    );
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

// ============================================================================
// EOP trait implementation tests
// ============================================================================

#[test]
fn trait_ut1_offset() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let dut1 = Ut1Offset::dut1(&table, 60389.0);
    assert!((dut1 - (-0.0091683)).abs() < 1e-6);
}

#[test]
fn trait_polar_motion() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let xp = PolarMotion::x_pole(&table, 60389.0);
    let yp = PolarMotion::y_pole(&table, 60389.0);
    assert!((xp - (-0.013421)).abs() < 1e-6);
    assert!((yp - 0.313052).abs() < 1e-6);
}

#[test]
fn trait_nutation_corrections() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let dx = NutationCorrections::dx(&table, 60389.0);
    assert!((dx - 0.378).abs() < 0.01);
}

#[test]
fn trait_length_of_day() {
    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let lod = LengthOfDay::lod(&table, 60389.0);
    // LOD in seconds
    assert!(
        (lod - 0.1693e-3).abs() < 1e-7,
        "LOD should be ~0.1693 ms, got {lod}"
    );
}

// ============================================================================
// Integration: EopTable works with Rotation chain
// ============================================================================

#[test]
fn eop_table_works_with_iau2006_full() {
    use arika::epoch::{Epoch, Utc};
    use arika::frame::{self, Rotation, Vec3};

    let table = EopTable::from_finals2000a(SAMPLE).unwrap();
    let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);

    // This should compile and not panic — real EOP data flowing through
    // the full IAU 2006 CIO chain.
    let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(&utc, &table);
    let v = Vec3::<frame::Gcrs>::new(1.0, 0.0, 0.0);
    let v_itrs = rot.transform(&v);

    // Magnitude should be preserved
    assert!(
        (v_itrs.magnitude() - 1.0).abs() < 1e-14,
        "rotation should preserve magnitude"
    );
}
