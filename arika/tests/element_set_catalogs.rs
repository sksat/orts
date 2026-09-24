//! Reading a CelesTrak group query in any format as the element sets it holds.
//!
//! The TLE fixtures hold two element sets of CelesTrak's `GROUP=stations`
//! query, fetched 2026-09-24 as `FORMAT=3LE` and `FORMAT=2LE` (with CelesTrak's
//! CRLF line endings): the first one (ISS) and CSS (TIANHE). `FORMAT=TLE` came
//! back byte for byte the same as `FORMAT=3LE`. The OMM fixtures are the same
//! two satellites (see `omm_groups.rs`), fetched earlier the same day.

use arika::elements::{self, ParsedElementSet};
use arika::tle;

const TLE_3LE: &str = include_str!("fixtures/celestrak_stations_iss_css.3le");
const TLE_2LE: &str = include_str!("fixtures/celestrak_stations_iss_css.2le");
const OMM_KVN: &str = include_str!("fixtures/celestrak_stations_iss_css.kvn");
const OMM_XML: &str = include_str!("fixtures/celestrak_stations_iss_css.xml");
const OMM_JSON: &str = include_str!("fixtures/celestrak_stations_iss_css.json");

fn catalog_numbers(sets: &[ParsedElementSet]) -> Vec<u32> {
    sets.iter()
        .map(|set| set.elements.fields().norad_cat_id)
        .collect()
}

#[test]
fn the_tle_fixtures_keep_celestrak_line_endings() {
    for (name, text) in [("3LE", TLE_3LE), ("2LE", TLE_2LE)] {
        assert!(text.contains("\r\n"), "{name} lost its CRLF line endings");
    }
    assert_eq!(TLE_3LE.lines().count(), 6);
    assert_eq!(TLE_2LE.lines().count(), 4);
}

/// The 3LE and the 2LE give the same element sets, in order; only the 3LE
/// names them.
#[test]
fn a_tle_catalog_gives_every_element_set_in_order() {
    let named = tle::parse_all(TLE_3LE).expect("3LE");
    let bare = tle::parse_all(TLE_2LE).expect("2LE");
    assert_eq!(catalog_numbers(&named), [25544, 48274]);
    let names: Vec<_> = named.iter().map(|s| s.object_name.as_deref()).collect();
    assert_eq!(names, [Some("ISS (ZARYA)"), Some("CSS (TIANHE)")]);
    assert!(bare.iter().all(|s| s.object_name.is_none()));
    let elements =
        |sets: &[ParsedElementSet]| -> Vec<_> { sets.iter().map(|s| s.elements.clone()).collect() };
    assert_eq!(elements(&named), elements(&bare));
    // The single-set parser refuses either catalog.
    assert!(tle::parse(TLE_3LE).is_err());
    assert!(tle::parse(TLE_2LE).is_err());
}

/// `elements::parse_all` detects each of the five formats and reads the two
/// satellites from each, in order.
#[test]
fn the_unified_parse_all_reads_every_format() {
    for (format, text) in [
        ("3LE", TLE_3LE),
        ("2LE", TLE_2LE),
        ("KVN", OMM_KVN),
        ("XML", OMM_XML),
        ("JSON", OMM_JSON),
    ] {
        let sets = elements::parse_all(text).unwrap_or_else(|e| panic!("{format}: {e}"));
        assert_eq!(catalog_numbers(&sets), [25544, 48274], "{format}");
        assert!(elements::parse(text).is_err(), "{format}: parse reads one");
    }
}
