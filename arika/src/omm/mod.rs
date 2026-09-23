//! CCSDS Orbit Mean-Elements Message (OMM) format decoders.
//!
//! OMM is the CCSDS standard that Space-Track and CelesTrak recommend as the
//! overflow-proof successor to the legacy TLE catalog (whose 5-digit number is
//! only extended, not fixed, by Alpha-5 — see [`crate::tle`]). Its
//! `NORAD_CAT_ID` has no fixed width.
//!
//! These submodules decode the three OMM serializations — [`json`], [`kvn`],
//! [`xml`] — into a [`crate::elements::ParsedElementSet`] (the shared
//! [`crate::elements::Sgp4Elements`] plus identity strings). The
//! format-detecting [`crate::elements::parse`] entry point dispatches to them.
//!
//! Angles are converted to **radians** and mean motion to **rad/s** (orts
//! conventions) from each format's native units (degrees, rev/day) at parse
//! time.
//!
//! All three decoders also *check* the metadata keywords that fix how the
//! elements must be interpreted — `CENTER_NAME`, `REF_FRAME`, `TIME_SYSTEM`,
//! `MEAN_ELEMENT_THEORY` — and reject a document declaring anything but
//! `EARTH` / `TEME` / `UTC` / `SGP4` ([`UnsupportedMetadata`]; `SGP4` may also be
//! spelled `SGP/SGP4`, as CelesTrak's KVN writes it). The record they
//! decode into hard-codes exactly those four assumptions, so honouring the
//! keywords is not optional. CCSDS lists all four as mandatory, but CelesTrak's
//! GP JSON and CSV flavours omit them; an absent keyword is therefore read as
//! the supported value rather than rejected, so those feeds keep parsing. What
//! the checks stop is a document that *states* something else.

pub mod json;
pub mod kvn;
pub mod xml;

use alloc::string::{String, ToString};

/// OMM metadata keywords that fix how the mean elements must be interpreted,
/// paired with the single value this crate can honor.
///
/// [`crate::elements::Sgp4Elements`] hard-codes exactly what these declare: the
/// epoch is a UTC [`crate::epoch::Epoch`], the six elements are SGP4
/// (Brouwer-Kozai) mean elements, and propagating them yields an Earth-centred
/// TEME state. A document declaring anything else describes a record this crate
/// cannot read, so the parsers reject it instead of reinterpreting it — reading
/// a `TIME_SYSTEM = TAI` element set as UTC would displace it by 37 s (≈ 285 km
/// along-track at LEO) with no diagnostic.
const METADATA: [(&str, &str); 4] = [
    ("CENTER_NAME", "EARTH"),
    ("REF_FRAME", "TEME"),
    ("TIME_SYSTEM", "UTC"),
    ("MEAN_ELEMENT_THEORY", "SGP4"),
];

/// Other spellings of a [`METADATA`] value, read as that value.
///
/// CelesTrak's KVN writes `MEAN_ELEMENT_THEORY = SGP/SGP4` for the element sets
/// its XML labels `SGP4` and its JSON leaves unlabelled; the three are the same
/// GP data, and the KVN one parses to the same record once read as SGP4 (#561).
/// An alias matches the whole value, like [`METADATA`]: `SGP` alone, `SGP4-XP`
/// and `SGP/SGP4-XP` are other theories and stay rejected.
const ALIASES: [(&str, &str); 1] = [("MEAN_ELEMENT_THEORY", "SGP/SGP4")];

/// An OMM metadata keyword whose declared value this crate cannot honor.
#[derive(Debug, Clone, PartialEq)]
pub struct UnsupportedMetadata {
    /// The OMM keyword, e.g. `"TIME_SYSTEM"`.
    pub key: &'static str,
    /// The value the document declared.
    pub value: String,
    /// The value this crate reads for that keyword, in its canonical
    /// spelling (`SGP4` also accepts `SGP/SGP4`).
    pub supported: &'static str,
}

impl core::fmt::Display for UnsupportedMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self {
            key,
            value,
            supported,
        } = self;
        write!(
            f,
            "unsupported OMM {key} '{value}' (only {supported} is read)"
        )
    }
}

#[cfg(feature = "std")]
impl std::error::Error for UnsupportedMetadata {}

/// Check one `KEYWORD = VALUE` pair against [`METADATA`] and its [`ALIASES`],
/// case-insensitively.
///
/// Keywords outside the table are not constrained and pass. An absent keyword
/// is never checked at all: CCSDS lists these four as mandatory, but the
/// CelesTrak GP flavours omit them, and for those feeds the omitted value is
/// always the supported one.
pub(crate) fn check_metadata(key: &str, value: &str) -> Result<(), UnsupportedMetadata> {
    let Some(&(key, supported)) = METADATA.iter().find(|(k, _)| *k == key) else {
        return Ok(());
    };
    let value = value.trim();
    let is_alias = ALIASES
        .iter()
        .any(|&(k, alias)| k == key && value.eq_ignore_ascii_case(alias));
    if value.eq_ignore_ascii_case(supported) || is_alias {
        Ok(())
    } else {
        Err(UnsupportedMetadata {
            key,
            value: value.to_string(),
            supported,
        })
    }
}

/// Check every constrained keyword a document defines, looking each value up
/// through `lookup` (which returns `None` for an absent keyword).
pub(crate) fn check_all_metadata<'a>(
    lookup: impl Fn(&'static str) -> Option<&'a str>,
) -> Result<(), UnsupportedMetadata> {
    for (key, _) in METADATA {
        if let Some(value) = lookup(key) {
            check_metadata(key, value)?;
        }
    }
    Ok(())
}
