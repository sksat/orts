//! Orbit Mean-Elements Message (OMM) — the shared mean-element record.
//!
//! [`Omm`] is the in-memory model that every element-set format decodes into
//! (TLE in [`crate::tle`], OMM JSON/KVN/XML in the submodules here). It mirrors
//! the CCSDS OMM mean-element set: a satellite identity, an epoch, and the six
//! SGP4 mean Keplerian elements plus the B* drag term.
//!
//! OMM is the CCSDS standard that Space-Track and CelesTrak recommend as the
//! overflow-proof successor to the legacy TLE catalog (whose 5-digit number is
//! only extended, not fixed, by Alpha-5 — see [`crate::tle`]). Its
//! `NORAD_CAT_ID` has no fixed width.
//!
//! Angles are stored in **radians** and mean motion in **rad/s** (orts
//! conventions), converted from each format's native units (degrees, rev/day)
//! at parse time. Convert to classical elements with
//! [`Omm::to_keplerian_elements`].

pub mod json;
pub mod kvn;
pub mod xml;

use alloc::string::String;
use core::fmt;

// In `no_std` builds f64 transcendentals (`cbrt`) resolve via libm through this
// trait; under `std` the inherent methods shadow it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::epoch::{Epoch, Utc};
use crate::kepler::{KeplerianElements, mean_to_true_anomaly};

/// Mean orbital element set (CCSDS OMM data model).
///
/// The canonical output of all element-set parsers. TLE is treated as a legacy
/// serialization of the same mean-element data, so [`crate::tle`] also produces
/// an `Omm`.
#[derive(Debug, Clone, PartialEq)]
pub struct Omm {
    /// `OBJECT_NAME` — satellite name, if present.
    pub object_name: Option<String>,
    /// `OBJECT_ID` — international designator (e.g. `"1998-067A"`), if present.
    pub object_id: Option<String>,
    /// `NORAD_CAT_ID` — catalog number. Alpha-5 alphanumeric ids are decoded to
    /// their numeric value (e.g. `"A0000"` → `100000`).
    pub norad_cat_id: u32,
    /// Element-set epoch (UTC).
    pub epoch: Epoch<Utc>,
    /// Mean motion [rad/s].
    pub mean_motion: f64,
    /// Eccentricity (dimensionless).
    pub eccentricity: f64,
    /// Inclination [rad].
    pub inclination: f64,
    /// Right ascension of the ascending node [rad].
    pub raan: f64,
    /// Argument of perigee [rad].
    pub argument_of_perigee: f64,
    /// Mean anomaly [rad].
    pub mean_anomaly: f64,
    /// B* drag term [1/earth radii].
    pub bstar: f64,
}

impl Omm {
    /// Semi-major axis [km] from mean motion: `a = (μ/n²)^(1/3)`.
    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        (mu / (self.mean_motion * self.mean_motion)).cbrt()
    }

    /// Convert to classical Keplerian elements.
    ///
    /// Derives the semi-major axis from mean motion and converts the mean
    /// anomaly to true anomaly via Kepler's equation.
    pub fn to_keplerian_elements(&self, mu: f64) -> KeplerianElements {
        KeplerianElements {
            semi_major_axis: self.semi_major_axis(mu),
            eccentricity: self.eccentricity,
            inclination: self.inclination,
            raan: self.raan,
            argument_of_periapsis: self.argument_of_perigee,
            true_anomaly: mean_to_true_anomaly(self.mean_anomaly, self.eccentricity),
        }
    }
}

/// Parse an OMM `EPOCH` value into a UTC [`Epoch`].
///
/// Delegates to [`Epoch::from_iso8601`], which accepts both the calendar and
/// ordinal (day-of-year) forms with an optional `Z` suffix. Shared by the
/// JSON / KVN / XML parsers; returns `None` for malformed timestamps.
pub(crate) fn parse_epoch(raw: &str) -> Option<Epoch<Utc>> {
    Epoch::from_iso8601(raw)
}

/// Element-set serialization formats that [`parse`] can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// NORAD Two-Line Element set (also 2LE / 3LE), incl. Alpha-5 catalog ids.
    Tle,
    /// CCSDS OMM, JSON serialization.
    OmmJson,
    /// CCSDS OMM, KVN (keyword-value) serialization.
    OmmKvn,
    /// CCSDS OMM, XML serialization.
    OmmXml,
}

/// Strip a leading UTF-8 BOM (U+FEFF). It is not whitespace, so `trim_start`
/// alone won't remove it, and a BOM-prefixed file would otherwise confuse both
/// format sniffing and the JSON/XML parsers (failing at byte 0).
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Sniff the element-set format of `text` (cheap structural heuristic, no full
/// parse): leading `{` or `[` → JSON (a single OMM object, or a 1-element
/// array), leading `<` → XML, a `1 `/`2 ` line pair → TLE, otherwise CCSDS
/// keyword-value → KVN. A leading UTF-8 BOM is ignored. `None` if nothing
/// matches.
pub fn detect(text: &str) -> Option<Format> {
    let text = strip_bom(text);
    match text.trim_start().chars().next()? {
        '{' | '[' => return Some(Format::OmmJson),
        '<' => return Some(Format::OmmXml),
        _ => {}
    }
    if text.contains("CCSDS_OMM_VERS") {
        return Some(Format::OmmKvn);
    }
    // TLE has a "1 …" line and a "2 …" line (optionally preceded by a name line).
    let mut has_line1 = false;
    let mut has_line2 = false;
    for line in text.lines() {
        let l = line.trim_start();
        has_line1 |= l.starts_with("1 ");
        has_line2 |= l.starts_with("2 ");
    }
    if has_line1 && has_line2 {
        return Some(Format::Tle);
    }
    // Fallback: CCSDS keyword = value text carrying OMM keys.
    if text.contains('=') && (text.contains("MEAN_MOTION") || text.contains("EPOCH")) {
        return Some(Format::OmmKvn);
    }
    None
}

/// Error returned by the unified [`parse`] entry point.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The format could not be determined from the input.
    UnknownFormat,
    /// TLE parsing failed.
    Tle(crate::tle::TleParseError),
    /// OMM JSON parsing failed.
    Json(json::JsonParseError),
    /// OMM KVN parsing failed.
    Kvn(kvn::KvnParseError),
    /// OMM XML parsing failed.
    Xml(xml::XmlParseError),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownFormat => write!(f, "unrecognized element-set format"),
            ParseError::Tle(e) => write!(f, "{e}"),
            ParseError::Json(e) => write!(f, "{e}"),
            ParseError::Kvn(e) => write!(f, "{e}"),
            ParseError::Xml(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Parse any supported element-set serialization into an [`Omm`], detecting the
/// format from the input via [`detect`]. A leading UTF-8 BOM is stripped before
/// dispatch so the JSON/XML parsers see clean input.
pub fn parse(text: &str) -> Result<Omm, ParseError> {
    let text = strip_bom(text);
    match detect(text).ok_or(ParseError::UnknownFormat)? {
        Format::Tle => crate::tle::parse(text).map_err(ParseError::Tle),
        Format::OmmJson => json::parse(text).map_err(ParseError::Json),
        Format::OmmKvn => kvn::parse(text).map_err(ParseError::Kvn),
        Format::OmmXml => xml::parse(text).map_err(ParseError::Xml),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;
    use core::f64::consts::PI;

    /// ISS-like OMM, values from the canonical ISS TLE at epoch 2024-079.5.
    fn iss_omm() -> Omm {
        Omm {
            object_name: Some(String::from("ISS (ZARYA)")),
            object_id: Some(String::from("1998-067A")),
            norad_cat_id: 25544,
            epoch: Epoch::from_tle_epoch(24, 79.5),
            mean_motion: 15.49561654 * 2.0 * PI / 86400.0,
            eccentricity: 0.0007417,
            inclination: 51.6400_f64.to_radians(),
            raan: 208.6520_f64.to_radians(),
            argument_of_perigee: 35.3910_f64.to_radians(),
            mean_anomaly: 324.7580_f64.to_radians(),
            bstar: 3.0e-5,
        }
    }

    #[test]
    fn iss_semi_major_axis() {
        let a = iss_omm().semi_major_axis(MU_EARTH);
        // ISS orbits at ~420 km altitude → a ≈ 6796 km.
        assert!(
            (a - 6796.0).abs() < 5.0,
            "ISS semi-major axis should be ≈6796 km, got {a}"
        );
    }

    #[test]
    fn iss_to_keplerian_elements() {
        let omm = iss_omm();
        let kep = omm.to_keplerian_elements(MU_EARTH);

        assert!((kep.semi_major_axis - omm.semi_major_axis(MU_EARTH)).abs() < 1e-9);
        assert_eq!(kep.eccentricity, omm.eccentricity);
        assert_eq!(kep.inclination, omm.inclination);
        assert_eq!(kep.raan, omm.raan);
        assert_eq!(kep.argument_of_periapsis, omm.argument_of_perigee);
        // Near-circular orbit (e ≈ 0.0007): true anomaly ≈ mean anomaly.
        let d_nu = (kep.true_anomaly - omm.mean_anomaly).abs();
        assert!(
            d_nu < 0.01,
            "ν should be ≈ M for near-circular orbit, Δ={d_nu}"
        );
    }

    // ── Format detection / dispatch ──────────────────────────────
    // The same ISS element set (NORAD 25544, i = 51.64°) in each serialization.

    const TLE_2L: &str = "\
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000";

    const TLE_3L: &str = "\
ISS (ZARYA)
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000";

    const JSON: &str = r#"{"NORAD_CAT_ID":25544,"EPOCH":"2024-03-19T12:00:00",
        "MEAN_MOTION":15.49561654,"ECCENTRICITY":0.0007417,"INCLINATION":51.64,
        "RA_OF_ASC_NODE":208.652,"ARG_OF_PERICENTER":35.391,"MEAN_ANOMALY":324.758}"#;

    const KVN: &str = "\
CCSDS_OMM_VERS = 2.0
EPOCH = 2024-03-19T12:00:00
MEAN_MOTION = 15.49561654
ECCENTRICITY = 0.0007417
INCLINATION = 51.64
RA_OF_ASC_NODE = 208.652
ARG_OF_PERICENTER = 35.391
MEAN_ANOMALY = 324.758
NORAD_CAT_ID = 25544";

    const XML: &str = r#"<omm><EPOCH>2024-03-19T12:00:00</EPOCH>
        <MEAN_MOTION>15.49561654</MEAN_MOTION><ECCENTRICITY>0.0007417</ECCENTRICITY>
        <INCLINATION>51.64</INCLINATION><RA_OF_ASC_NODE>208.652</RA_OF_ASC_NODE>
        <ARG_OF_PERICENTER>35.391</ARG_OF_PERICENTER><MEAN_ANOMALY>324.758</MEAN_ANOMALY>
        <NORAD_CAT_ID>25544</NORAD_CAT_ID></omm>"#;

    #[test]
    fn detect_formats() {
        assert_eq!(detect(TLE_2L), Some(Format::Tle));
        assert_eq!(detect(TLE_3L), Some(Format::Tle));
        assert_eq!(detect(JSON), Some(Format::OmmJson));
        assert_eq!(detect("  [ {} ]"), Some(Format::OmmJson)); // 1-element array
        assert_eq!(detect(KVN), Some(Format::OmmKvn));
        assert_eq!(detect(XML), Some(Format::OmmXml));
        assert_eq!(detect("garbage, no markers"), None);
    }

    #[test]
    fn parse_dispatches_every_format() {
        for src in [TLE_2L, TLE_3L, JSON, KVN, XML] {
            let omm = parse(src).unwrap();
            assert_eq!(omm.norad_cat_id, 25544);
            assert!((omm.inclination.to_degrees() - 51.64).abs() < 1e-6);
        }
    }

    #[test]
    fn bom_prefixed_input_detects_and_parses() {
        // A UTF-8 BOM (not whitespace!) must not break sniffing or parsing.
        let bom_json = ["\u{feff}", JSON].concat();
        assert_eq!(detect(&bom_json), Some(Format::OmmJson));
        assert_eq!(parse(&bom_json).unwrap().norad_cat_id, 25544);
        let bom_xml = ["\u{feff}", XML].concat();
        assert_eq!(detect(&bom_xml), Some(Format::OmmXml));
        let bom_tle = ["\u{feff}", TLE_2L].concat();
        assert_eq!(detect(&bom_tle), Some(Format::Tle));
    }

    #[test]
    fn parse_unknown_format_errors() {
        assert_eq!(
            parse("definitely not an element set"),
            Err(ParseError::UnknownFormat)
        );
    }
}
