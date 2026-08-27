//! OMM KVN parser (CCSDS Keyword-Value Notation, the OMM `.kvn` text form).
//!
//! KVN is a sequence of `KEYWORD = VALUE` lines. This hand-rolled reader skips
//! `COMMENT` lines and block markers (`META_START` / `META_STOP`, lines without
//! `=`), strips trailing unit annotations (`51.64 [deg]`), and collects the
//! mean-element keywords into a [`ParsedElementSet`]. Unknown keywords are ignored.

use alloc::string::{String, ToString};
use core::f64::consts::PI;
use core::fmt;
use core::str::FromStr;

// `.to_radians()` resolves via libm in no_std; under std the inherent shadows it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::elements::{ElementsError, ParsedElementSet, Sgp4Elements, Sgp4ElementsFields};
use crate::omm::{UnsupportedMetadata, check_metadata};

/// Error type for OMM KVN parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum KvnParseError {
    /// A required keyword was absent.
    MissingField(&'static str),
    /// A keyword's value could not be parsed as the expected number.
    InvalidValue { key: &'static str, value: String },
    /// `EPOCH` was not a parseable ISO-8601 UTC timestamp (calendar or
    /// ordinal / day-of-year form).
    InvalidEpoch(String),
    /// A metadata keyword declares something this crate cannot read (non-Earth
    /// center, non-TEME frame, non-UTC time system, non-SGP4 theory).
    Unsupported(UnsupportedMetadata),
    /// The parsed values are not a valid element set (e.g. non-positive mean
    /// motion or out-of-range eccentricity).
    InvalidElements(ElementsError),
}

impl fmt::Display for KvnParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KvnParseError::MissingField(k) => write!(f, "missing OMM keyword: {k}"),
            KvnParseError::InvalidValue { key, value } => {
                write!(f, "invalid value for {key}: '{value}'")
            }
            KvnParseError::InvalidEpoch(s) => write!(f, "invalid OMM EPOCH: '{s}'"),
            KvnParseError::Unsupported(e) => write!(f, "{e}"),
            KvnParseError::InvalidElements(e) => write!(f, "invalid OMM element set: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for KvnParseError {}

/// Parse an OMM KVN document into a [`ParsedElementSet`].
pub fn parse(kvn: &str) -> Result<ParsedElementSet, KvnParseError> {
    // BOM-tolerant even when called directly (not via the unified entrypoint).
    let kvn = crate::elements::strip_bom(kvn);
    let mut object_name = None;
    let mut object_id = None;
    let mut norad_cat_id = None;
    let mut epoch_str: Option<&str> = None;
    let mut mean_motion = None; // rev/day
    let mut eccentricity = None;
    let mut inclination = None; // deg
    let mut raan = None; // deg
    let mut arg_perigee = None; // deg
    let mut mean_anomaly = None; // deg
    let mut bstar = 0.0;

    for line in kvn.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("COMMENT") {
            continue;
        }
        // Block markers (META_START / META_STOP / *_START / *_STOP) have no '='.
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        // Trim only — do NOT strip unit annotations here, or a string field
        // like `OBJECT_NAME = SAT [TEST]` would be truncated at '['. Units are
        // stripped per numeric field inside `parse_num`.
        let value = value.trim();
        // The metadata keywords declare how the elements must be interpreted;
        // reject anything this crate does not actually honor before reading a
        // single element (see `crate::omm::METADATA`).
        check_metadata(key, value).map_err(KvnParseError::Unsupported)?;
        // Pass the matched string literal (not the borrowed `key`) so the
        // error type can hold a `&'static str` keyword.
        match key {
            "OBJECT_NAME" => object_name = Some(value.to_string()),
            "OBJECT_ID" => object_id = Some(value.to_string()),
            "NORAD_CAT_ID" => norad_cat_id = Some(parse_num::<u32>("NORAD_CAT_ID", value)?),
            "EPOCH" => epoch_str = Some(value),
            "MEAN_MOTION" => mean_motion = Some(parse_num::<f64>("MEAN_MOTION", value)?),
            "ECCENTRICITY" => eccentricity = Some(parse_num::<f64>("ECCENTRICITY", value)?),
            "INCLINATION" => inclination = Some(parse_num::<f64>("INCLINATION", value)?),
            "RA_OF_ASC_NODE" => raan = Some(parse_num::<f64>("RA_OF_ASC_NODE", value)?),
            "ARG_OF_PERICENTER" => {
                arg_perigee = Some(parse_num::<f64>("ARG_OF_PERICENTER", value)?)
            }
            "MEAN_ANOMALY" => mean_anomaly = Some(parse_num::<f64>("MEAN_ANOMALY", value)?),
            "BSTAR" => bstar = parse_num::<f64>("BSTAR", value)?,
            _ => {} // version / GM / element_set_no / checked metadata / …
        }
    }

    let epoch_str = epoch_str.ok_or(KvnParseError::MissingField("EPOCH"))?;
    let epoch = crate::elements::parse_epoch(epoch_str)
        .ok_or_else(|| KvnParseError::InvalidEpoch(epoch_str.to_string()))?;

    let mean_motion = mean_motion.ok_or(KvnParseError::MissingField("MEAN_MOTION"))?;
    let eccentricity = eccentricity.ok_or(KvnParseError::MissingField("ECCENTRICITY"))?;
    let inclination = inclination.ok_or(KvnParseError::MissingField("INCLINATION"))?;
    let raan = raan.ok_or(KvnParseError::MissingField("RA_OF_ASC_NODE"))?;
    let arg_perigee = arg_perigee.ok_or(KvnParseError::MissingField("ARG_OF_PERICENTER"))?;
    let mean_anomaly = mean_anomaly.ok_or(KvnParseError::MissingField("MEAN_ANOMALY"))?;

    let norad_cat_id = norad_cat_id.ok_or(KvnParseError::MissingField("NORAD_CAT_ID"))?;
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
    .map_err(KvnParseError::InvalidElements)?;

    Ok(ParsedElementSet {
        elements,
        object_name,
        object_id,
    })
}

/// Drop a trailing CCSDS unit annotation, e.g. `"51.64 [deg]"` → `"51.64 "`.
fn strip_units(value: &str) -> &str {
    match value.find('[') {
        Some(i) => &value[..i],
        None => value,
    }
}

fn parse_num<T: FromStr>(key: &'static str, value: &str) -> Result<T, KvnParseError> {
    // Numeric fields may carry a trailing unit annotation (e.g. "51.64 [deg]").
    strip_units(value)
        .trim()
        .parse()
        .map_err(|_| KvnParseError::InvalidValue {
            key,
            value: value.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;
    use alloc::format;

    // ISS OMM KVN with a META block, a COMMENT, and unit-annotated values —
    // same element set as the ISS fixtures in `crate::tle` / `crate::omm::json`.
    const ISS_OMM_KVN: &str = "\
CCSDS_OMM_VERS = 2.0
COMMENT generated by test
CREATION_DATE = 2024-03-19T00:00:00
ORIGINATOR = test
META_START
OBJECT_NAME = ISS (ZARYA)
OBJECT_ID = 1998-067A
CENTER_NAME = EARTH
REF_FRAME = TEME
TIME_SYSTEM = UTC
MEAN_ELEMENT_THEORY = SGP4
META_STOP
EPOCH = 2024-03-19T12:00:00.000000
MEAN_MOTION = 15.49561654 [rev/day]
ECCENTRICITY = 0.0007417
INCLINATION = 51.6400 [deg]
RA_OF_ASC_NODE = 208.6520 [deg]
ARG_OF_PERICENTER = 35.3910 [deg]
MEAN_ANOMALY = 324.7580 [deg]
GM = 398600.8 [km**3/s**2]
NORAD_CAT_ID = 25544
ELEMENT_SET_NO = 999
BSTAR = 0.00003
";

    #[test]
    fn parse_iss_omm_kvn() {
        let set = parse(ISS_OMM_KVN).unwrap();
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
        let mm_rev_day = omm.mean_motion * 86400.0 / (2.0 * PI);
        assert!((mm_rev_day - 15.49561654).abs() < 1e-8);
        assert!((omm.bstar - 3.0e-5).abs() < 1e-10);

        assert!((set.elements.semi_major_axis(MU_EARTH) - 6796.0).abs() < 5.0);
    }

    #[test]
    fn missing_required_field_errors() {
        // Drop the MEAN_MOTION line.
        let kvn: String = ISS_OMM_KVN
            .lines()
            .filter(|l| !l.trim_start().starts_with("MEAN_MOTION"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(parse(&kvn), Err(KvnParseError::MissingField("MEAN_MOTION")));
    }

    #[test]
    fn invalid_value_errors() {
        let kvn = "EPOCH = 2024-03-19T12:00:00\nNORAD_CAT_ID = not_a_number\n";
        assert!(matches!(
            parse(kvn),
            Err(KvnParseError::InvalidValue {
                key: "NORAD_CAT_ID",
                ..
            })
        ));
    }

    #[test]
    fn object_name_with_bracket_not_truncated() {
        // A '[' in a string field must survive (unit stripping is numeric-only).
        let kvn = "\
OBJECT_NAME = SAT [TEST]
EPOCH = 2024-03-19T12:00:00
MEAN_MOTION = 15.0
ECCENTRICITY = 0.0
INCLINATION = 0.0
RA_OF_ASC_NODE = 0.0
ARG_OF_PERICENTER = 0.0
MEAN_ANOMALY = 0.0
NORAD_CAT_ID = 1";
        let set = parse(kvn).unwrap();
        assert_eq!(set.object_name.as_deref(), Some("SAT [TEST]"));
    }

    #[test]
    fn rejects_unsupported_metadata() {
        // Table-driven: mutate one metadata value at a time. Each of these
        // declares an element set this crate cannot read — a TAI epoch read as
        // UTC lands 37 s (≈ 285 km along-track at LEO) from where it belongs,
        // and a DSST / SGP4-XP set is not SGP4 mean elements at all.
        for (from, to, key) in [
            ("CENTER_NAME = EARTH", "CENTER_NAME = MARS", "CENTER_NAME"),
            ("REF_FRAME = TEME", "REF_FRAME = GCRF", "REF_FRAME"),
            ("TIME_SYSTEM = UTC", "TIME_SYSTEM = TAI", "TIME_SYSTEM"),
            (
                "MEAN_ELEMENT_THEORY = SGP4",
                "MEAN_ELEMENT_THEORY = DSST",
                "MEAN_ELEMENT_THEORY",
            ),
            (
                "MEAN_ELEMENT_THEORY = SGP4",
                "MEAN_ELEMENT_THEORY = SGP4-XP",
                "MEAN_ELEMENT_THEORY",
            ),
        ] {
            let kvn = ISS_OMM_KVN.replace(from, to);
            assert_ne!(kvn, ISS_OMM_KVN, "fixture no longer contains '{from}'");
            match parse(&kvn) {
                Err(KvnParseError::Unsupported(e)) => assert_eq!(e.key, key),
                other => panic!("'{to}' must be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn metadata_values_are_case_insensitive() {
        let kvn = ISS_OMM_KVN
            .replace("CENTER_NAME = EARTH", "CENTER_NAME = Earth")
            .replace("TIME_SYSTEM = UTC", "TIME_SYSTEM = utc");
        assert_eq!(parse(&kvn).unwrap().elements.fields().norad_cat_id, 25544);
    }

    #[test]
    fn bom_prefixed_kvn_parses_directly() {
        // Direct calls (not via elements::parse) must also tolerate a leading BOM.
        let bom = ["\u{feff}", ISS_OMM_KVN].concat();
        assert_eq!(parse(&bom).unwrap().elements.fields().norad_cat_id, 25544);
    }
}
