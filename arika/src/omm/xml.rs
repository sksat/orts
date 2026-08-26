//! OMM XML parser (CCSDS `.xml` OMM form).
//!
//! A deliberately small, dependency-free reader for machine-generated OMM XML.
//! It is *not* a general XML parser: it extracts the text of named leaf
//! elements (`<KEYWORD ...>value</KEYWORD>`) by exact name, skipping any
//! attributes (e.g. `units="deg"`), and assumes well-formed OMM with no
//! namespace prefixes or markup inside leaf values. Comments and processing
//! instructions are skipped, so text inside them is never read as a value. XML
//! entity references are **not decoded**: an escaped `OBJECT_NAME` like
//! `A &amp; B` is returned verbatim (`A &amp; B`), not unescaped.

use alloc::string::{String, ToString};
use core::f64::consts::PI;
use core::fmt;
use core::str::FromStr;

// `.to_radians()` resolves via libm in no_std; under std the inherent shadows it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::elements::{ElementsError, ParsedElementSet, Sgp4Elements, Sgp4ElementsFields};
use crate::omm::{UnsupportedMetadata, check_all_metadata};

/// Error type for OMM XML parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum XmlParseError {
    /// A required element was absent.
    MissingElement(&'static str),
    /// An element's text could not be parsed as the expected number.
    InvalidValue { key: &'static str, value: String },
    /// `EPOCH` was not a parseable ISO-8601 UTC timestamp (calendar or
    /// ordinal / day-of-year form).
    InvalidEpoch(String),
    /// A metadata element declares something this crate cannot read (non-Earth
    /// center, non-TEME frame, non-UTC time system, non-SGP4 theory).
    Unsupported(UnsupportedMetadata),
    /// The parsed values are not a valid element set (e.g. non-positive mean
    /// motion or out-of-range eccentricity).
    InvalidElements(ElementsError),
}

impl fmt::Display for XmlParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlParseError::MissingElement(k) => write!(f, "missing OMM element: {k}"),
            XmlParseError::InvalidValue { key, value } => {
                write!(f, "invalid value for {key}: '{value}'")
            }
            XmlParseError::InvalidEpoch(s) => write!(f, "invalid OMM EPOCH: '{s}'"),
            XmlParseError::Unsupported(e) => write!(f, "{e}"),
            XmlParseError::InvalidElements(e) => write!(f, "invalid OMM element set: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for XmlParseError {}

/// Parse an OMM XML document into a [`ParsedElementSet`].
pub fn parse(xml: &str) -> Result<ParsedElementSet, XmlParseError> {
    // BOM-tolerant even when called directly (not via the unified entrypoint).
    let xml = crate::elements::strip_bom(xml);
    // The metadata elements declare how the mean elements must be interpreted;
    // reject anything this crate does not actually honor (see
    // `crate::omm::METADATA`).
    check_all_metadata(|key| element_text(xml, key)).map_err(XmlParseError::Unsupported)?;
    let epoch_raw = required(xml, "EPOCH")?;
    let epoch = crate::elements::parse_epoch(epoch_raw)
        .ok_or_else(|| XmlParseError::InvalidEpoch(epoch_raw.to_string()))?;

    let mean_motion = parse_num::<f64>("MEAN_MOTION", required(xml, "MEAN_MOTION")?)?;
    let eccentricity = parse_num::<f64>("ECCENTRICITY", required(xml, "ECCENTRICITY")?)?;
    let inclination = parse_num::<f64>("INCLINATION", required(xml, "INCLINATION")?)?;
    let raan = parse_num::<f64>("RA_OF_ASC_NODE", required(xml, "RA_OF_ASC_NODE")?)?;
    let arg_perigee = parse_num::<f64>("ARG_OF_PERICENTER", required(xml, "ARG_OF_PERICENTER")?)?;
    let mean_anomaly = parse_num::<f64>("MEAN_ANOMALY", required(xml, "MEAN_ANOMALY")?)?;
    let norad_cat_id = parse_num::<u32>("NORAD_CAT_ID", required(xml, "NORAD_CAT_ID")?)?;
    let bstar = match element_text(xml, "BSTAR") {
        Some(v) => parse_num::<f64>("BSTAR", v)?,
        None => 0.0,
    };

    let elements = Sgp4Elements::try_new(Sgp4ElementsFields {
        norad_cat_id,
        epoch,
        mean_motion: mean_motion * 2.0 * PI / 86400.0, // rev/day → rad/s
        eccentricity,
        inclination: inclination.to_radians(),
        raan: raan.to_radians(),
        argument_of_perigee: arg_perigee.to_radians(),
        mean_anomaly: mean_anomaly.to_radians(),
        bstar,
    })
    .map_err(XmlParseError::InvalidElements)?;

    Ok(ParsedElementSet {
        elements,
        object_name: element_text(xml, "OBJECT_NAME").map(String::from),
        object_id: element_text(xml, "OBJECT_ID").map(String::from),
    })
}

fn required<'a>(xml: &'a str, name: &'static str) -> Result<&'a str, XmlParseError> {
    element_text(xml, name).ok_or(XmlParseError::MissingElement(name))
}

/// Extract the trimmed text of the first `<NAME ...>text</NAME>` element.
///
/// Matches `name` exactly: the character after the name must be `>` or
/// whitespace, so a query for `MEAN_MOTION` never matches `MEAN_MOTION_DOT`.
/// Attributes are skipped; the value is read up to the next `<`. Comments
/// (`<!-- … -->`) and processing instructions (`<? … ?>`) are stepped over
/// rather than searched, so markup quoted inside them is not a value.
fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(lt) = xml[from..].find('<') {
        let tag = from + lt + 1; // byte index just after '<'
        let markup = &xml[tag..];
        // A comment or a processing instruction declares nothing, but its text
        // can contain what looks like an element. Skipping them wholesale is
        // what keeps a commented-out `<TIME_SYSTEM>UTC</TIME_SYSTEM>` from being
        // read as the document's time system while the real one below it says
        // TAI — the metadata check would then pass on a document it must reject.
        if let Some(after) = markup.strip_prefix("!--") {
            from = after
                .find("-->")
                .map_or(xml.len(), |end| tag + "!--".len() + end + "-->".len());
            continue;
        }
        if let Some(after) = markup.strip_prefix('?') {
            from = after
                .find("?>")
                .map_or(xml.len(), |end| tag + '?'.len_utf8() + end + "?>".len());
            continue;
        }
        if let Some(rest) = markup.strip_prefix(name) {
            match rest.chars().next() {
                Some(c) if c == '>' || c.is_whitespace() => {
                    let gt = rest.find('>')?;
                    let content = &xml[tag + name.len() + gt + 1..];
                    let close = content.find('<')?;
                    return Some(content[..close].trim());
                }
                _ => {}
            }
        }
        from = tag;
    }
    None
}

fn parse_num<T: FromStr>(key: &'static str, value: &str) -> Result<T, XmlParseError> {
    value.parse().map_err(|_| XmlParseError::InvalidValue {
        key,
        value: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;

    // ISS OMM XML. Includes `units` attributes and a MEAN_MOTION_DOT element to
    // exercise attribute-skipping and exact-name matching. Same element set as
    // the ISS fixtures in `crate::tle` / `crate::omm::{json,kvn}`.
    const ISS_OMM_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<omm id="CCSDS_OMM_VERS" version="2.0">
  <header>
    <CREATION_DATE>2024-03-19T00:00:00</CREATION_DATE>
    <ORIGINATOR>test</ORIGINATOR>
  </header>
  <body>
    <segment>
      <metadata>
        <OBJECT_NAME>ISS (ZARYA)</OBJECT_NAME>
        <OBJECT_ID>1998-067A</OBJECT_ID>
        <CENTER_NAME>EARTH</CENTER_NAME>
        <REF_FRAME>TEME</REF_FRAME>
        <TIME_SYSTEM>UTC</TIME_SYSTEM>
        <MEAN_ELEMENT_THEORY>SGP4</MEAN_ELEMENT_THEORY>
      </metadata>
      <data>
        <meanElements>
          <EPOCH>2024-03-19T12:00:00.000000</EPOCH>
          <MEAN_MOTION>15.49561654</MEAN_MOTION>
          <ECCENTRICITY>0.0007417</ECCENTRICITY>
          <INCLINATION units="deg">51.6400</INCLINATION>
          <RA_OF_ASC_NODE units="deg">208.6520</RA_OF_ASC_NODE>
          <ARG_OF_PERICENTER units="deg">35.3910</ARG_OF_PERICENTER>
          <MEAN_ANOMALY units="deg">324.7580</MEAN_ANOMALY>
        </meanElements>
        <tleParameters>
          <NORAD_CAT_ID>25544</NORAD_CAT_ID>
          <ELEMENT_SET_NO>999</ELEMENT_SET_NO>
          <BSTAR>0.00003</BSTAR>
          <MEAN_MOTION_DOT>0.00001234</MEAN_MOTION_DOT>
        </tleParameters>
      </data>
    </segment>
  </body>
</omm>"#;

    #[test]
    fn parse_iss_omm_xml() {
        let set = parse(ISS_OMM_XML).unwrap();
        let omm = set.elements.to_fields();
        assert_eq!(set.object_name.as_deref(), Some("ISS (ZARYA)"));
        assert_eq!(set.object_id.as_deref(), Some("1998-067A"));
        assert_eq!(omm.norad_cat_id, 25544);

        let dt = omm.epoch.to_datetime();
        assert_eq!((dt.year, dt.month, dt.day, dt.hour), (2024, 3, 19, 12));

        assert!((omm.inclination.to_degrees() - 51.64).abs() < 1e-9);
        assert!((omm.raan.to_degrees() - 208.652).abs() < 1e-9);
        assert!((omm.eccentricity - 0.0007417).abs() < 1e-12);
        assert!((omm.argument_of_perigee.to_degrees() - 35.391).abs() < 1e-9);
        assert!((omm.mean_anomaly.to_degrees() - 324.758).abs() < 1e-9);
        // Exact-name match: MEAN_MOTION must be the real one, not MEAN_MOTION_DOT.
        let mm_rev_day = omm.mean_motion * 86400.0 / (2.0 * PI);
        assert!((mm_rev_day - 15.49561654).abs() < 1e-8);
        assert!((omm.bstar - 3.0e-5).abs() < 1e-10);

        assert!((set.elements.semi_major_axis(MU_EARTH) - 6796.0).abs() < 5.0);
    }

    #[test]
    fn missing_required_element_errors() {
        let xml = "<omm><EPOCH>2024-03-19T12:00:00</EPOCH></omm>";
        assert_eq!(
            parse(xml),
            Err(XmlParseError::MissingElement("MEAN_MOTION"))
        );
    }

    #[test]
    fn commented_out_markup_is_not_a_value() {
        // The reader scans for named tags, so text inside a comment used to be
        // indistinguishable from a real element: a document that quotes the
        // conforming value in a comment and then declares TAI must still be
        // rejected, and a commented-out element must not stand in for a missing
        // one. Processing instructions get the same treatment.
        let hidden = ISS_OMM_XML.replace(
            "<TIME_SYSTEM>UTC</TIME_SYSTEM>",
            "<!-- <TIME_SYSTEM>UTC</TIME_SYSTEM> --><TIME_SYSTEM>TAI</TIME_SYSTEM>",
        );
        match parse(&hidden) {
            Err(XmlParseError::Unsupported(e)) => {
                assert_eq!((e.key, e.value.as_str()), ("TIME_SYSTEM", "TAI"))
            }
            other => panic!("a comment must not hide TIME_SYSTEM = TAI, got {other:?}"),
        }
        // A commented-out required element is absent, not present.
        let commented = ISS_OMM_XML.replace(
            "<MEAN_MOTION>15.49561654</MEAN_MOTION>",
            "<!-- <MEAN_MOTION>15.49561654</MEAN_MOTION> -->",
        );
        assert_eq!(
            parse(&commented),
            Err(XmlParseError::MissingElement("MEAN_MOTION"))
        );
        // A value quoted inside a processing instruction is not a value either.
        let pi = ISS_OMM_XML.replace(
            "<INCLINATION units=\"deg\">51.6400</INCLINATION>",
            "<?dump <INCLINATION>0.0</INCLINATION> ?><INCLINATION units=\"deg\">51.6400</INCLINATION>",
        );
        let omm = parse(&pi).unwrap().elements.to_fields();
        assert!((omm.inclination.to_degrees() - 51.64).abs() < 1e-9);
        // An unterminated comment swallows the rest of the document rather than
        // reading through it.
        let unterminated = ISS_OMM_XML.replace("<EPOCH>", "<!-- <EPOCH>");
        assert!(parse(&unterminated).is_err());
    }

    #[test]
    fn rejects_unsupported_metadata() {
        // Same table as the KVN parser: one metadata element mutated at a time.
        for (from, to, key) in [
            (
                "<CENTER_NAME>EARTH</CENTER_NAME>",
                "<CENTER_NAME>MARS</CENTER_NAME>",
                "CENTER_NAME",
            ),
            (
                "<REF_FRAME>TEME</REF_FRAME>",
                "<REF_FRAME>GCRF</REF_FRAME>",
                "REF_FRAME",
            ),
            (
                "<TIME_SYSTEM>UTC</TIME_SYSTEM>",
                "<TIME_SYSTEM>TAI</TIME_SYSTEM>",
                "TIME_SYSTEM",
            ),
            (
                "<MEAN_ELEMENT_THEORY>SGP4</MEAN_ELEMENT_THEORY>",
                "<MEAN_ELEMENT_THEORY>DSST</MEAN_ELEMENT_THEORY>",
                "MEAN_ELEMENT_THEORY",
            ),
        ] {
            let xml = ISS_OMM_XML.replace(from, to);
            assert_ne!(xml, ISS_OMM_XML, "fixture no longer contains '{from}'");
            match parse(&xml) {
                Err(XmlParseError::Unsupported(e)) => assert_eq!(e.key, key),
                other => panic!("'{to}' must be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn exact_name_match_only() {
        // A document with only MEAN_MOTION_DOT must not satisfy MEAN_MOTION.
        let xml = "<data><MEAN_MOTION_DOT>1.0</MEAN_MOTION_DOT></data>";
        assert!(element_text(xml, "MEAN_MOTION").is_none());
        assert_eq!(element_text(xml, "MEAN_MOTION_DOT"), Some("1.0"));
    }
}
