//! OMM XML parser (CCSDS `.xml` OMM form).
//!
//! A deliberately small, dependency-free reader for machine-generated OMM XML.
//! It is *not* a general XML parser: it extracts the text of named leaf
//! elements (`<KEYWORD ...>value</KEYWORD>`) by exact name, skipping any
//! attributes (e.g. `units="deg"`), and assumes well-formed OMM with no
//! namespace prefixes or markup inside leaf values. Comments and processing
//! instructions are skipped, so text inside them is never read as a value, and
//! a document carrying a DOCTYPE or a CDATA section is rejected rather than read
//! past. XML entity references are **not decoded**: an escaped `OBJECT_NAME`
//! like `A &amp; B` is returned verbatim (`A &amp; B`), not unescaped.

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
    /// The document uses a markup declaration this reader does not interpret:
    /// a DOCTYPE (whose internal subset can define entities) or a CDATA
    /// section. Both can carry text that looks like an element, so a reader
    /// that scans for tag names must not read past them.
    UnsupportedMarkup(&'static str),
    /// The parsed values are not a valid element set (e.g. non-positive mean
    /// motion or out-of-range eccentricity).
    InvalidElements(ElementsError),
    /// The document holds more than one `<omm>`, as an NDM that collects
    /// several OMMs does (CelesTrak's group queries return one).
    MultipleMessages { found: usize },
    /// An element this reads or checks appears more than once in the OMM.
    RepeatedElement(&'static str),
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
            XmlParseError::UnsupportedMarkup(what) => {
                write!(f, "unsupported XML markup in OMM document: {what}")
            }
            XmlParseError::InvalidElements(e) => write!(f, "invalid OMM element set: {e}"),
            XmlParseError::MultipleMessages { found } => {
                write!(
                    f,
                    "OMM XML document has {found} <omm> elements; expected exactly one"
                )
            }
            XmlParseError::RepeatedElement(k) => {
                write!(f, "OMM element {k} appears more than once")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for XmlParseError {}

/// Parse an OMM XML document into a [`ParsedElementSet`].
pub fn parse(xml: &str) -> Result<ParsedElementSet, XmlParseError> {
    // BOM-tolerant even when called directly (not via the unified entrypoint).
    let xml = crate::elements::strip_bom(xml);
    reject_unsupported_markup(xml)?;
    reject_repeats(xml)?;
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
    // An OMM that declares SGP4 has to carry the drag term the theory reads;
    // defaulting it to zero propagated a satellite with no drag at all.
    let bstar = parse_num::<f64>("BSTAR", required(xml, "BSTAR")?)?;

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

/// Refuse markup declarations whose *contents* this reader cannot skip safely.
///
/// `element_text` scans for tag names, so any construct that may quote markup as
/// text — a DOCTYPE internal subset defining an entity whose value contains an
/// element, or a CDATA section — could otherwise supply a value the document
/// never declares, defeating the metadata check. Comments carry the same risk
/// and are stepped over instead; everything else opening with `<!` is refused
/// rather than guessed at. No real OMM uses either construct.
fn reject_unsupported_markup(xml: &str) -> Result<(), XmlParseError> {
    let mut from = 0;
    while let Some(at) = xml[from..].find("<!") {
        let decl_at = from + at + "<!".len();
        let decl = &xml[decl_at..];
        // Step over a comment's contents exactly as `element_text` does, so a
        // comment that merely quotes a declaration is not mistaken for one.
        if let Some(after) = decl.strip_prefix("--") {
            from = after
                .find("-->")
                .map_or(xml.len(), |end| decl_at + "--".len() + end + "-->".len());
            continue;
        }
        let what = if decl.starts_with("[CDATA[") {
            "CDATA section"
        } else if decl.starts_with("DOCTYPE") {
            "DOCTYPE declaration"
        } else {
            "markup declaration"
        };
        return Err(XmlParseError::UnsupportedMarkup(what));
    }
    Ok(())
}

/// Refuse a document that holds more than one OMM, or gives an element this
/// reads or checks more than once.
///
/// `element_text` reads the first occurrence, so the other OMMs of an NDM that
/// collects several (CCSDS 502.0-B-3 §8.12; CelesTrak's group queries return
/// one) would be dropped. The `<omm>` elements are counted first, so an NDM is
/// reported as such and not by the first element its OMMs share.
fn reject_repeats(xml: &str) -> Result<(), XmlParseError> {
    let found = start_tags(xml, "omm").count();
    if found > 1 {
        return Err(XmlParseError::MultipleMessages { found });
    }
    match crate::omm::keywords_read().find(|name| start_tags(xml, name).nth(1).is_some()) {
        Some(name) => Err(XmlParseError::RepeatedElement(name)),
        None => Ok(()),
    }
}

fn required<'a>(xml: &'a str, name: &'static str) -> Result<&'a str, XmlParseError> {
    element_text(xml, name).ok_or(XmlParseError::MissingElement(name))
}

/// Extract the trimmed text of the first `<NAME ...>text</NAME>` element.
///
/// The value is read up to the next `<`; an empty element (`<NAME/>`) has none.
fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let rest = &xml[start_tags(xml, name).next()?..];
    let gt = rest.find('>')?;
    if rest[..gt].ends_with('/') {
        return None;
    }
    let content = &rest[gt + 1..];
    let close = content.find('<')?;
    Some(content[..close].trim())
}

/// The byte offset just past the name of each start tag `<NAME ...>` or empty
/// element `<NAME/>`, in document order.
///
/// Matches `name` exactly: the character after the name must be `>`, `/` or
/// whitespace, so a query for `MEAN_MOTION` never matches `MEAN_MOTION_DOT`,
/// and a closing `</NAME>` is not a start tag. Comments (`<!-- … -->`) and
/// processing instructions (`<? … ?>`) are stepped over rather than searched,
/// so markup quoted inside them is not an element.
fn start_tags<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let mut from = 0;
    core::iter::from_fn(move || {
        while let Some(lt) = xml[from..].find('<') {
            let tag = from + lt + 1; // byte index just after '<'
            let markup = &xml[tag..];
            // A comment or a processing instruction declares nothing, but its
            // text can contain what looks like an element. Skipping them
            // wholesale is what keeps a commented-out
            // `<TIME_SYSTEM>UTC</TIME_SYSTEM>` from being read as the
            // document's time system while the real one below it says TAI —
            // the metadata check would then pass on a document it must reject.
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
            from = tag;
            if let Some(rest) = markup.strip_prefix(name)
                && rest.starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace())
            {
                return Some(tag + name.len());
            }
        }
        None
    })
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

    /// SGP4 reads the drag term, and this crate refuses any other mean-element
    /// theory, so an OMM that reaches here declares SGP4 and has to carry
    /// `BSTAR`. A missing element used to read as `0.0`, which propagates the
    /// satellite with no drag at all — a different orbit, reported as success.
    #[test]
    fn a_missing_bstar_is_refused() {
        let without = ISS_OMM_XML.replace("<BSTAR>0.00003</BSTAR>", "");
        assert_ne!(without, ISS_OMM_XML, "fixture no longer carries BSTAR");
        assert_eq!(parse(&without), Err(XmlParseError::MissingElement("BSTAR")));
    }

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
    fn declarations_that_can_quote_markup_are_refused() {
        // A DOCTYPE internal subset can define an entity whose *value* contains
        // an element, and a CDATA section can quote one verbatim. Either would
        // be found by the tag scan before the document's real declaration, so a
        // document that says TIME_SYSTEM = TAI while hiding a UTC lookalike
        // would pass the metadata check. Neither construct appears in real OMM,
        // so both are refused instead of interpreted.
        let doctype = ISS_OMM_XML.replace(
            "<omm id=",
            "<!DOCTYPE omm [<!ENTITY decoy \"<TIME_SYSTEM>UTC</TIME_SYSTEM>\">]><omm id=",
        );
        assert_eq!(
            parse(&doctype),
            Err(XmlParseError::UnsupportedMarkup("DOCTYPE declaration"))
        );
        let cdata = ISS_OMM_XML.replace(
            "<OBJECT_NAME>ISS (ZARYA)</OBJECT_NAME>",
            "<OBJECT_NAME><![CDATA[<TIME_SYSTEM>UTC</TIME_SYSTEM>]]></OBJECT_NAME>",
        );
        assert_eq!(
            parse(&cdata),
            Err(XmlParseError::UnsupportedMarkup("CDATA section"))
        );
        // A comment is the one `<!` form that is safe to step over — including
        // one whose text quotes a declaration, which is not one.
        assert!(parse(&ISS_OMM_XML.replace("<header>", "<!-- note --><header>")).is_ok());
        assert!(
            parse(&ISS_OMM_XML.replace(
                "<header>",
                "<!-- an example: <!DOCTYPE omm> and <![CDATA[x]]> --><header>",
            ))
            .is_ok(),
            "a declaration quoted inside a comment is text, not a declaration"
        );
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

    /// CelesTrak's KVN spelling of the theory, `SGP/SGP4` (#561), reads as
    /// SGP4 in XML too; `SGP` alone is a different theory and stays refused.
    #[test]
    fn the_sgp_sgp4_theory_reads_as_sgp4() {
        let theory = "<MEAN_ELEMENT_THEORY>SGP4</MEAN_ELEMENT_THEORY>";
        let celestrak = ISS_OMM_XML.replace(
            theory,
            "<MEAN_ELEMENT_THEORY>SGP/SGP4</MEAN_ELEMENT_THEORY>",
        );
        assert_ne!(
            celestrak, ISS_OMM_XML,
            "fixture no longer contains the theory"
        );
        assert_eq!(parse(&celestrak).unwrap(), parse(ISS_OMM_XML).unwrap());

        let sgp = ISS_OMM_XML.replace(theory, "<MEAN_ELEMENT_THEORY>SGP</MEAN_ELEMENT_THEORY>");
        match parse(&sgp) {
            Err(XmlParseError::Unsupported(e)) => assert_eq!(e.key, "MEAN_ELEMENT_THEORY"),
            other => panic!("SGP must be rejected, got {other:?}"),
        }
    }

    /// An NDM may collect several OMMs (CCSDS 502.0-B-3 §8.12), and
    /// CelesTrak's group queries (`GROUP=stations&FORMAT=XML`) return one. The
    /// parser read each element from the first place it appears, so it read
    /// the first OMM and dropped the rest (#564).
    #[test]
    fn an_ndm_with_several_omms_is_refused() {
        let omm = ISS_OMM_XML.trim_start_matches(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        assert_ne!(
            omm, ISS_OMM_XML,
            "fixture no longer opens with the XML declaration"
        );
        let other = omm
            .replace("ISS (ZARYA)", "OTHER SAT")
            .replace(">51.6400<", ">97.5000<")
            .replace(">25544<", ">99999<");
        let ndm = |omms: &[&str]| -> String {
            ["<?xml version=\"1.0\"?>\n<ndm>", &omms.concat(), "</ndm>"].concat()
        };
        assert_eq!(
            parse(&ndm(&[omm, &other])),
            Err(XmlParseError::MultipleMessages { found: 2 })
        );
        assert_eq!(
            parse(&ndm(&[omm, &other, omm])),
            Err(XmlParseError::MultipleMessages { found: 3 })
        );
        // One OMM in an NDM is one OMM.
        assert_eq!(parse(&ndm(&[omm])), parse(ISS_OMM_XML));
        // A tag inside a comment, a closing tag and a longer name are no `<omm>`.
        let lookalikes =
            ISS_OMM_XML.replace("<header>", "<!-- <omm id=\"x\"> --><ommExtra/><header>");
        assert_eq!(parse(&lookalikes), parse(ISS_OMM_XML));
    }

    /// In one OMM, an element this reads or checks appears once; a second one
    /// (even with the same value, or empty) leaves which to read undecided.
    #[test]
    fn an_element_it_reads_given_twice_is_refused() {
        for (from, to, repeated) in [
            (
                "<EPOCH>2024-03-19T12:00:00.000000</EPOCH>",
                "<EPOCH>2024-03-19T12:00:00.000000</EPOCH><EPOCH>2024-03-19T12:00:00.000000</EPOCH>",
                "EPOCH",
            ),
            (
                "<REF_FRAME>TEME</REF_FRAME>",
                "<REF_FRAME>TEME</REF_FRAME><REF_FRAME>GCRF</REF_FRAME>",
                "REF_FRAME",
            ),
            (
                "<OBJECT_NAME>ISS (ZARYA)</OBJECT_NAME>",
                "<OBJECT_NAME>ISS (ZARYA)</OBJECT_NAME><OBJECT_NAME>OTHER</OBJECT_NAME>",
                "OBJECT_NAME",
            ),
            (
                "<BSTAR>0.00003</BSTAR>",
                "<BSTAR>0.00003</BSTAR><BSTAR/>",
                "BSTAR",
            ),
        ] {
            let xml = ISS_OMM_XML.replace(from, to);
            assert_ne!(xml, ISS_OMM_XML, "fixture no longer contains '{from}'");
            assert_eq!(
                parse(&xml),
                Err(XmlParseError::RepeatedElement(repeated)),
                "{to}"
            );
        }
        // `COMMENT` and `USER_DEFINED` repeat by definition, and this reads neither.
        let repeatable = ISS_OMM_XML.replace(
            "<metadata>",
            "<metadata><COMMENT>a</COMMENT><COMMENT>b</COMMENT>\
             <USER_DEFINED parameter=\"X\">1</USER_DEFINED>\
             <USER_DEFINED parameter=\"Y\">2</USER_DEFINED>",
        );
        assert_ne!(
            repeatable, ISS_OMM_XML,
            "fixture no longer contains <metadata>"
        );
        assert_eq!(parse(&repeatable), parse(ISS_OMM_XML));
    }

    #[test]
    fn exact_name_match_only() {
        // A document with only MEAN_MOTION_DOT must not satisfy MEAN_MOTION.
        let xml = "<data><MEAN_MOTION_DOT>1.0</MEAN_MOTION_DOT></data>";
        assert!(element_text(xml, "MEAN_MOTION").is_none());
        assert_eq!(element_text(xml, "MEAN_MOTION_DOT"), Some("1.0"));
    }
}
