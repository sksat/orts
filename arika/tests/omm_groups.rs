//! Reading the OMMs of a CelesTrak group query as the element sets they are.
//!
//! The fixtures hold two OMMs of CelesTrak's `GROUP=stations` query, fetched
//! 2026-09-24: the first one (ISS) and CSS (TIANHE), in each OMM format it
//! serves. The KVN puts one OMM after another (with CelesTrak's CRLF line
//! endings), the XML is an `<ndm>` holding an `<omm>` for each, and the JSON is
//! an array. The KVN and the XML were fetched together and hold the same
//! element sets; the JSON was fetched later, at later epochs.

use arika::elements::ParsedElementSet;
use arika::omm::{json, kvn, xml};

const KVN: &str = include_str!("fixtures/celestrak_stations_iss_css.kvn");
const XML: &str = include_str!("fixtures/celestrak_stations_iss_css.xml");
const JSON: &str = include_str!("fixtures/celestrak_stations_iss_css.json");

/// Catalog number, name and inclination [deg] of each element set, in order.
fn summary(sets: &[ParsedElementSet]) -> Vec<(u32, Option<&str>, f64)> {
    sets.iter()
        .map(|set| {
            let fields = set.elements.fields();
            (
                fields.norad_cat_id,
                set.object_name.as_deref(),
                fields.inclination.to_degrees(),
            )
        })
        .collect()
}

fn assert_iss_then_css(format: &str, sets: &[ParsedElementSet], inclinations: [f64; 2]) {
    let got = summary(sets);
    assert_eq!(got.len(), 2, "{format}: {got:?}");
    for ((norad, name, inclination), (want_norad, want_name, want_inclination)) in got.iter().zip([
        (25544, "ISS (ZARYA)", inclinations[0]),
        (48274, "CSS (TIANHE)", inclinations[1]),
    ]) {
        assert_eq!((*norad, *name), (want_norad, Some(want_name)), "{format}");
        assert!(
            (inclination - want_inclination).abs() < 1e-9,
            "{format} {want_name}: i = {inclination}"
        );
    }
}

#[test]
fn the_kvn_fixture_keeps_celestrak_line_endings() {
    assert_eq!(KVN.matches("CCSDS_OMM_VERS").count(), 2);
    assert!(
        KVN.contains("\r\n"),
        "the fixture lost its CRLF line endings"
    );
}

#[test]
fn each_format_gives_every_omm_in_order() {
    let from_kvn = kvn::parse_all(KVN).expect("KVN");
    let from_xml = xml::parse_all(XML).expect("XML");
    let from_json = json::parse_all(JSON).expect("JSON");
    assert_iss_then_css("KVN", &from_kvn, [51.6317, 41.4677]);
    assert_iss_then_css("XML", &from_xml, [51.6317, 41.4677]);
    assert_iss_then_css("JSON", &from_json, [51.6318, 41.4677]);
    // Fetched together, the KVN and the XML give the same element sets.
    assert_eq!(from_kvn, from_xml);
}

/// The single-set parsers refuse the same documents (#564): each holds more
/// than the one element set they return.
#[test]
fn the_single_set_parsers_refuse_a_group() {
    assert!(kvn::parse(KVN).is_err());
    assert!(xml::parse(XML).is_err());
    assert!(json::parse(JSON).is_err());
    assert!(arika::elements::parse(KVN).is_err());
}

/// One OMM of the group, on its own, reads as one element set either way.
#[test]
fn a_document_with_one_omm_is_a_list_of_one() {
    let first_kvn = &KVN[..KVN[1..].find("CCSDS_OMM_VERS").expect("a second OMM") + 1];
    let single = kvn::parse(first_kvn).expect("the first OMM on its own");
    assert_eq!(kvn::parse_all(first_kvn), Ok(vec![single]));

    let first_xml = [
        &XML[..XML.find("</omm>").expect("an OMM") + "</omm>".len()],
        "\n</ndm>\n",
    ]
    .concat();
    let single = xml::parse(&first_xml).expect("an NDM with one OMM");
    assert_eq!(xml::parse_all(&first_xml), Ok(vec![single]));
}
