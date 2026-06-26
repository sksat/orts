//! Satellite element sets: the no-alloc [`Sgp4Elements`] mean-element set and
//! the unified [`parse`] entry point.
//!
//! [`Sgp4Elements`] holds the numeric SGP4 mean elements an SGP4 propagator
//! consumes — epoch, the six mean elements, the B* drag term, and the catalog
//! number. It is `Copy` and needs no allocator, so the propagation path works
//! in `no_std` builds without `alloc`.
//!
//! Owned satellite identity (`OBJECT_NAME` / `OBJECT_ID`) is split off into
//! [`ParsedElementSet`], the alloc-gated record the text parsers return. Text
//! decoding (TLE in [`crate::tle`], CCSDS OMM JSON/KVN/XML in [`crate::omm`])
//! requires `alloc` regardless, so identity strings live there rather than on
//! the no-alloc element set.
//!
//! These are **mean** elements in the SGP4 (Brouwer-Kozai) sense, *not*
//! osculating Keplerian elements: they are only physically meaningful when fed
//! to the SGP4 propagator. Treating them as classical orbital elements (a
//! two-body interpretation) introduces tens-of-km error at epoch.
//!
//! Angles are stored in **radians** and mean motion in **rad/s** (orts
//! conventions), converted from each format's native units (degrees, rev/day)
//! at parse time.

#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use core::fmt;

// In `no_std` builds f64 transcendentals (`cbrt`) resolve via libm through this
// trait; under `std` the inherent methods shadow it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::epoch::{Epoch, Utc};

/// SGP4 mean orbital element set (no allocator required).
///
/// The numeric core that an SGP4 propagator consumes. Both the legacy TLE and
/// the CCSDS OMM (JSON/KVN/XML) carry the same mean-element data, so
/// [`crate::tle`] and [`crate::omm`] both decode into this type (wrapped with
/// identity strings in [`ParsedElementSet`]).
///
/// The six elements are **SGP4 mean elements** (Brouwer-Kozai), not osculating
/// Keplerian elements — propagate them with SGP4 rather than converting to a
/// classical orbit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sgp4Elements {
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

impl Sgp4Elements {
    /// Semi-major axis [km] from mean motion: `a = (μ/n²)^(1/3)`.
    ///
    /// A rough two-body estimate for display only — it ignores the J2 secular
    /// correction baked into the SGP4 (Kozai) mean motion, so it is not the
    /// semi-major axis SGP4 itself uses.
    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        (mu / (self.mean_motion * self.mean_motion)).cbrt()
    }

    /// Orbital period [s] from the mean motion: `2π / |n|`.
    ///
    /// A non-negative magnitude, so it stays sane even for a malformed element
    /// set with a non-positive mean motion (the text parsers do not reject one).
    /// Exact for the (Kozai) mean motion the set carries, and needs no `mu` —
    /// this is the conventional period reported for a TLE/OMM.
    pub fn period(&self) -> f64 {
        core::f64::consts::TAU / self.mean_motion.abs()
    }
}

/// A parsed element set: the numeric [`Sgp4Elements`] plus owned satellite
/// identity strings.
///
/// The return type of the text parsers ([`parse`], [`crate::tle::parse`], the
/// [`crate::omm`] decoders). Carrying the `OBJECT_NAME` / `OBJECT_ID` strings
/// requires `alloc`; the numeric [`elements`](Self::elements) it wraps does not.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedElementSet {
    /// The numeric mean elements (allocator-free; feed these to SGP4).
    pub elements: Sgp4Elements,
    /// `OBJECT_NAME` — satellite name, if present.
    pub object_name: Option<String>,
    /// `OBJECT_ID` — international designator (e.g. `"1998-067A"`), if present.
    pub object_id: Option<String>,
}

/// Parse an OMM `EPOCH` value into a UTC [`Epoch`].
///
/// Delegates to [`Epoch::from_iso8601`], which accepts both the calendar and
/// ordinal (day-of-year) forms with an optional `Z` suffix. Shared by the
/// JSON / KVN / XML parsers; returns `None` for malformed timestamps.
#[cfg(feature = "alloc")]
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
pub(crate) fn strip_bom(text: &str) -> &str {
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
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The format could not be determined from the input.
    UnknownFormat,
    /// TLE parsing failed.
    Tle(crate::tle::TleParseError),
    /// OMM JSON parsing failed.
    Json(crate::omm::json::JsonParseError),
    /// OMM KVN parsing failed.
    Kvn(crate::omm::kvn::KvnParseError),
    /// OMM XML parsing failed.
    Xml(crate::omm::xml::XmlParseError),
}

#[cfg(feature = "alloc")]
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

#[cfg(all(feature = "std", feature = "alloc"))]
impl std::error::Error for ParseError {}

/// Parse any supported element-set serialization into a [`ParsedElementSet`],
/// detecting the format from the input via [`detect`]. A leading UTF-8 BOM is
/// stripped before dispatch so the JSON/XML parsers see clean input.
#[cfg(feature = "alloc")]
pub fn parse(text: &str) -> Result<ParsedElementSet, ParseError> {
    let text = strip_bom(text);
    match detect(text).ok_or(ParseError::UnknownFormat)? {
        Format::Tle => crate::tle::parse(text).map_err(ParseError::Tle),
        Format::OmmJson => crate::omm::json::parse(text).map_err(ParseError::Json),
        Format::OmmKvn => crate::omm::kvn::parse(text).map_err(ParseError::Kvn),
        Format::OmmXml => crate::omm::xml::parse(text).map_err(ParseError::Xml),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;
    use core::f64::consts::PI;

    /// ISS-like element set, values from the canonical ISS TLE at epoch 2024-079.5.
    fn iss_elements() -> Sgp4Elements {
        Sgp4Elements {
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
        let a = iss_elements().semi_major_axis(MU_EARTH);
        // ISS orbits at ~420 km altitude → a ≈ 6796 km.
        assert!(
            (a - 6796.0).abs() < 5.0,
            "ISS semi-major axis should be ≈6796 km, got {a}"
        );
    }

    #[test]
    fn iss_period() {
        let p = iss_elements().period();
        // ISS mean motion ≈ 15.4956 rev/day → period ≈ 92.9 min ≈ 5576 s.
        assert!(
            (p - 5575.8).abs() < 2.0,
            "ISS period should be ≈5576 s, got {p}"
        );
    }

    #[test]
    fn period_stays_positive_for_negative_mean_motion() {
        // A malformed element set with a non-positive mean motion must not yield
        // a negative period (it flows into run horizons / reset boundaries).
        let mut el = iss_elements();
        el.mean_motion = -el.mean_motion;
        assert!(
            el.period() > 0.0,
            "period must stay positive, got {}",
            el.period()
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

    #[cfg(feature = "alloc")]
    #[test]
    fn parse_dispatches_every_format() {
        for src in [TLE_2L, TLE_3L, JSON, KVN, XML] {
            let set = parse(src).unwrap();
            assert_eq!(set.elements.norad_cat_id, 25544);
            assert!((set.elements.inclination.to_degrees() - 51.64).abs() < 1e-6);
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn bom_prefixed_input_detects_and_parses() {
        // A UTF-8 BOM (not whitespace!) must not break sniffing or parsing.
        let bom_json = ["\u{feff}", JSON].concat();
        assert_eq!(detect(&bom_json), Some(Format::OmmJson));
        assert_eq!(parse(&bom_json).unwrap().elements.norad_cat_id, 25544);
        let bom_xml = ["\u{feff}", XML].concat();
        assert_eq!(detect(&bom_xml), Some(Format::OmmXml));
        let bom_tle = ["\u{feff}", TLE_2L].concat();
        assert_eq!(detect(&bom_tle), Some(Format::Tle));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn parse_unknown_format_errors() {
        assert_eq!(
            parse("definitely not an element set"),
            Err(ParseError::UnknownFormat)
        );
    }
}
