//! Two-Line Element set (TLE / 2LE / 3LE) parser.
//!
//! Parses the legacy NORAD fixed-width TLE format into a
//! [`crate::elements::ParsedElementSet`] (the shared
//! [`crate::elements::Sgp4Elements`] plus identity) — TLE is treated as a
//! serialization of the same mean elements that OMM standardizes. Supports the classic
//! 5-digit catalog number and the Alpha-5 alphanumeric extension (the official
//! interim scheme for catalog numbers past 99 999; see `decode_catalog_number`).
//!
//! Alpha-5 only extends the legacy format to 339 999 — for overflow-proof use
//! the CCSDS OMM format ([`crate::omm`]) is the recommended successor.
//!
//! Both of the format's built-in integrity checks are enforced: each line's
//! mod-10 checksum digit, and the catalog number that line 2 repeats from
//! line 1. A TLE whose fields parse but whose lines disagree describes no real
//! object, and reading it would produce a confidently wrong orbit.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::f64::consts::PI;
use core::fmt;
use core::str::FromStr;

// In `no_std` builds `.to_radians()` resolves via libm through this trait;
// under `std` the inherent method shadows it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::elements::{ElementsError, ParsedElementSet, Sgp4Elements, Sgp4ElementsFields};
use crate::epoch::Epoch;

/// Error type for TLE parsing failures.
#[derive(Debug, Clone, PartialEq)]
pub enum TleParseError {
    /// Not enough lines in the input.
    InsufficientLines,
    /// Line 1 does not start with '1'.
    InvalidLine1Prefix,
    /// Line 2 does not start with '2'.
    InvalidLine2Prefix,
    /// A numeric field could not be parsed.
    InvalidField {
        line: u8,
        field: &'static str,
        value: String,
    },
    /// The line's mod-10 checksum digit (column 69) is absent or disagrees with
    /// the line's contents — the line is corrupt or was hand-edited.
    InvalidChecksum {
        line: u8,
        expected: u32,
        found: Option<char>,
    },
    /// Line 1 and line 2 carry different satellite catalog numbers, i.e. the
    /// two lines come from different objects.
    CatalogNumberMismatch { line1: u32, line2: u32 },
    /// The parsed values are not a valid element set (e.g. non-positive mean
    /// motion or out-of-range eccentricity).
    InvalidElements(ElementsError),
}

impl fmt::Display for TleParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TleParseError::InsufficientLines => write!(f, "TLE requires at least 2 lines"),
            TleParseError::InvalidLine1Prefix => write!(f, "TLE line 1 must start with '1'"),
            TleParseError::InvalidLine2Prefix => write!(f, "TLE line 2 must start with '2'"),
            TleParseError::InvalidField { line, field, value } => {
                write!(f, "Invalid {field} on line {line}: '{value}'")
            }
            TleParseError::InvalidChecksum {
                line,
                expected,
                found: Some(found),
            } => write!(
                f,
                "TLE line {line} checksum mismatch: expected {expected}, found '{found}'"
            ),
            TleParseError::InvalidChecksum {
                line,
                expected,
                found: None,
            } => write!(
                f,
                "TLE line {line} has no checksum digit (expected {expected} in column 69)"
            ),
            TleParseError::CatalogNumberMismatch { line1, line2 } => write!(
                f,
                "TLE lines are from different satellites: line 1 has catalog number {line1}, line 2 has {line2}"
            ),
            TleParseError::InvalidElements(e) => write!(f, "invalid TLE element set: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TleParseError {}

/// Parse a TLE into a [`ParsedElementSet`].
///
/// Accepts:
/// - 2 lines: line 1 + line 2
/// - 3 lines: name + line 1 + line 2
///
/// Both lines must carry a correct mod-10 checksum digit and the same catalog
/// number, so a hand-edited digit or a pair of lines from two different
/// satellites is rejected rather than read as an orbit.
pub fn parse(text: &str) -> Result<ParsedElementSet, TleParseError> {
    // BOM-tolerant like the unified `elements::parse` entry point: a BOM is not whitespace,
    // so without this a BOM-prefixed file fails the line-1 prefix check.
    let text = crate::elements::strip_bom(text);
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let (name, line1, line2) = match lines.len() {
        0 | 1 => return Err(TleParseError::InsufficientLines),
        2 => (None, lines[0], lines[1]),
        _ => {
            // First line is a name only if it isn't itself line 1.
            if lines[0].starts_with('1') {
                (None, lines[0], lines[1])
            } else {
                // CelesTrak "3LE" prefixes the name line with the "0 " line
                // number; strip it so names match the OMM OBJECT_NAME form.
                let name = lines[0].strip_prefix("0 ").unwrap_or(lines[0]).trim();
                (Some(name.to_string()), lines[1], lines[2])
            }
        }
    };

    if !line1.starts_with('1') {
        return Err(TleParseError::InvalidLine1Prefix);
    }
    if !line2.starts_with('2') {
        return Err(TleParseError::InvalidLine2Prefix);
    }

    // Both lines carry a mod-10 checksum over their first 68 columns. Verifying
    // it before reading any field turns a corrupt or hand-edited digit into an
    // error instead of a plausible-looking orbit.
    verify_checksum(line1, 1)?;
    verify_checksum(line2, 2)?;

    // ─── Line 1 ───
    let catnum_field = line1.get(2..7).ok_or(TleParseError::InvalidField {
        line: 1,
        field: "satellite_number",
        value: String::new(),
    })?;
    let norad_cat_id = decode_catalog_number(catnum_field, 1)?;

    // The catalog number is repeated on line 2; a mismatch means the two lines
    // describe different objects (the classic copy-paste error), which no
    // checksum can catch because each line is individually well-formed.
    let catnum_field2 = line2.get(2..7).ok_or(TleParseError::InvalidField {
        line: 2,
        field: "satellite_number",
        value: String::new(),
    })?;
    let norad_cat_id_line2 = decode_catalog_number(catnum_field2, 2)?;
    if norad_cat_id_line2 != norad_cat_id {
        return Err(TleParseError::CatalogNumberMismatch {
            line1: norad_cat_id,
            line2: norad_cat_id_line2,
        });
    }

    let object_id = line1
        .get(9..17)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_intl_designator);

    let epoch_year_2digit = parse_field::<u32>(line1, 18, 20, 1, "epoch_year")?;
    let epoch_day = parse_field::<f64>(line1, 20, 32, 1, "epoch_day")?;
    // Validate day-of-year against the (leap-aware) year length so a malformed
    // field (e.g. 0.0 or 367.0) can't silently roll into an adjacent year.
    let year = if epoch_year_2digit >= 57 {
        1900 + epoch_year_2digit
    } else {
        2000 + epoch_year_2digit
    };
    let max_day = if crate::epoch::is_leap_year(year as i32) {
        367.0
    } else {
        366.0
    };
    if !(1.0..max_day).contains(&epoch_day) {
        return Err(TleParseError::InvalidField {
            line: 1,
            field: "epoch_day",
            value: format!("{epoch_day}"),
        });
    }
    let epoch = Epoch::from_tle_epoch(epoch_year_2digit, epoch_day);

    // B* drag term (columns 53-61, assumed decimal point notation):
    // " NNNNN±E" where value = 0.NNNNN * 10^(±E).
    let bstar = parse_assumed_decimal(line1, 53, 61, 1, "bstar")?;

    // ─── Line 2 ───
    let inclination_deg = parse_field::<f64>(line2, 8, 16, 2, "inclination")?;
    let raan_deg = parse_field::<f64>(line2, 17, 25, 2, "raan")?;

    // Eccentricity: implied leading decimal point (e.g. "0007417" → 0.0007417).
    let ecc_str = line2.get(26..33).ok_or(TleParseError::InvalidField {
        line: 2,
        field: "eccentricity",
        value: String::new(),
    })?;
    let eccentricity: f64 =
        format!("0.{}", ecc_str.trim())
            .parse()
            .map_err(|_| TleParseError::InvalidField {
                line: 2,
                field: "eccentricity",
                value: ecc_str.to_string(),
            })?;

    let arg_perigee_deg = parse_field::<f64>(line2, 34, 42, 2, "argument_of_perigee")?;
    let mean_anomaly_deg = parse_field::<f64>(line2, 43, 51, 2, "mean_anomaly")?;
    let mean_motion_rev_day = parse_field::<f64>(line2, 52, 63, 2, "mean_motion")?;

    let elements = Sgp4Elements::try_new(Sgp4ElementsFields {
        norad_cat_id,
        epoch,
        mean_motion: mean_motion_rev_day * 2.0 * PI / 86400.0, // rev/day → rad/s
        eccentricity,
        inclination: inclination_deg.to_radians(),
        raan: raan_deg.to_radians(),
        argument_of_perigee: arg_perigee_deg.to_radians(),
        mean_anomaly: mean_anomaly_deg.to_radians(),
        bstar,
    })
    .map_err(TleParseError::InvalidElements)?;

    Ok(ParsedElementSet {
        elements,
        object_name: name,
        object_id,
    })
}

/// Decode the 5-character satellite catalog number (TLE line 1, columns 3–7).
///
/// Classic TLEs use a 5-digit number (≤ 99999). Because the catalog has
/// exceeded 99999 objects, the **Alpha-5** scheme extends the range to 339999
/// by turning the first column into a letter: the leading character becomes a
/// base value 10–33 using `A`–`Z` **excluding `I` and `O`** (to avoid confusion
/// with `1` and `0`), combined with the trailing 4 decimal digits as
/// `leading * 10000 + trailing`.
///
/// | field   | value    | note          |
/// |---------|----------|---------------|
/// | `25544` | `25544`  | classic       |
/// | `A0000` | `100000` | `A` → 10      |
/// | `E8493` | `148493` | `E` → 14      |
/// | `Z9999` | `339999` | `Z` → 33 (max)|
///
/// Alpha-5 is the **official interim scheme** of the US Space Force /
/// Space-Track (18th Space Defense Squadron): `100000`–`269999` for catalogued
/// objects and `270000`–`339999` for Space-Fence analyst objects. It is a
/// stopgap — Space-Track and CelesTrak recommend the CCSDS OMM format
/// ([`crate::omm`]) as the overflow-proof long-term replacement.
fn decode_catalog_number(field: &str, line_num: u8) -> Result<u32, TleParseError> {
    let field = field.trim();
    let invalid = || TleParseError::InvalidField {
        line: line_num,
        field: "satellite_number",
        value: field.to_string(),
    };

    let mut chars = field.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return Err(invalid()),
    };

    // Classic all-numeric catalog number (≤ 99999).
    if first.is_ascii_digit() {
        return field.parse::<u32>().map_err(|_| invalid());
    }

    // Alpha-5: the leading column is a base-34 "digit" drawn from 0-9 then A-Z
    // with I and O removed (they look like 1 and 0). A character's position in
    // this alphabet is its value (A→10 … Z→33); the trailing 4 columns stay
    // decimal, so value = leading * 10000 + rest.
    const ALPHA5: &str = "0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let leading = match ALPHA5.find(first.to_ascii_uppercase()) {
        Some(pos) => pos as u32,
        None => return Err(invalid()),
    };
    // Alpha-5 is exactly 1 letter + 4 decimal digits; reject short or padded
    // fields (e.g. "A000") instead of silently decoding corrupt input.
    let rest_str = chars.as_str();
    if rest_str.len() != 4 || !rest_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let rest: u32 = rest_str.parse().map_err(|_| invalid())?;
    Ok(leading * 10000 + rest)
}

/// Normalize a TLE international designator (`YYNNNPPP`) to the OMM `OBJECT_ID`
/// form (`YYYY-NNNPPP`), e.g. `"98067A"` → `"1998-067A"`, so the field matches
/// across TLE and OMM (it is the same COSPAR id). The 2-digit launch year uses
/// the NORAD pivot (57-99 → 1900s, else 2000s; Sputnik 1957 is the floor).
/// Inputs that don't start with two digits are returned unchanged.
fn normalize_intl_designator(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if raw.len() > 2 && bytes[0].is_ascii_digit() && bytes[1].is_ascii_digit() {
        let yy: u32 = raw[..2].parse().unwrap_or(0);
        let year = if yy >= 57 { 1900 + yy } else { 2000 + yy };
        format!("{year}-{}", &raw[2..])
    } else {
        String::from(raw)
    }
}

/// Verify a TLE line's mod-10 checksum (column 69).
///
/// The checksum is the last decimal digit of the sum of columns 1–68, counting
/// each digit as its value and each minus sign as 1 (all other characters as 0).
/// Both NORAD lines carry one, and every real feed emits it correctly, so a
/// mismatch means the line was corrupted or hand-edited — the values can no
/// longer be trusted to be the ones the publisher computed.
fn verify_checksum(line: &str, line_num: u8) -> Result<(), TleParseError> {
    let expected = line_checksum(line);
    // Column 69 (index 68) is the checksum digit. Iterating by char keeps a
    // stray multi-byte character from panicking on a non-char-boundary slice;
    // a TLE is ASCII, so char count == column count.
    let found = line.chars().nth(68);
    match found.and_then(|c| c.to_digit(10)) {
        Some(digit) if digit == expected => Ok(()),
        _ => Err(TleParseError::InvalidChecksum {
            line: line_num,
            expected,
            found,
        }),
    }
}

/// The mod-10 checksum of a TLE line's first 68 columns.
fn line_checksum(line: &str) -> u32 {
    line.chars()
        .take(68)
        .map(|c| match c {
            '0'..='9' => c as u32 - '0' as u32,
            '-' => 1,
            _ => 0,
        })
        .sum::<u32>()
        % 10
}

/// Parse a fixed-width numeric field from a TLE line.
fn parse_field<T: FromStr>(
    line: &str,
    start: usize,
    end: usize,
    line_num: u8,
    field: &'static str,
) -> Result<T, TleParseError> {
    let s = line.get(start..end).ok_or(TleParseError::InvalidField {
        line: line_num,
        field,
        value: String::new(),
    })?;
    s.trim().parse().map_err(|_| TleParseError::InvalidField {
        line: line_num,
        field,
        value: s.to_string(),
    })
}

/// Parse a field in "assumed decimal point" notation (e.g. "30000-4" → 0.30000e-4).
///
/// TLE uses this format for B* and the second derivative of mean motion.
/// Format: " NNNNN±E" or "+NNNNN±E" or "-NNNNN±E".
fn parse_assumed_decimal(
    line: &str,
    start: usize,
    end: usize,
    line_num: u8,
    field: &'static str,
) -> Result<f64, TleParseError> {
    let s = line.get(start..end).ok_or(TleParseError::InvalidField {
        line: line_num,
        field,
        value: String::new(),
    })?;
    let s = s.trim();

    if s == "00000-0" || s == "00000+0" || s.is_empty() {
        return Ok(0.0);
    }

    // Find the exponent sign: the last '+' or '-' after the first character
    // (the first char may be the mantissa's own sign). Iterate by char so a
    // malformed multi-byte field can't panic on a non-char-boundary byte slice.
    let exp_sign = s
        .char_indices()
        .skip(1)
        .filter(|&(_, c)| c == '+' || c == '-')
        .last();
    let (mantissa_str, exp_str) = match exp_sign {
        Some((pos, _)) => (&s[..pos], &s[pos..]),
        None => {
            return Err(TleParseError::InvalidField {
                line: line_num,
                field,
                value: s.to_string(),
            });
        }
    };

    // Prepend "0." to the mantissa to apply the assumed decimal point.
    let mantissa: f64 = format!("0.{}", mantissa_str.trim_start_matches(['+', '-', ' ']))
        .parse()
        .map_err(|_| TleParseError::InvalidField {
            line: line_num,
            field,
            value: s.to_string(),
        })?;

    let exp: i32 = exp_str.parse().map_err(|_| TleParseError::InvalidField {
        line: line_num,
        field,
        value: s.to_string(),
    })?;

    let sign = if mantissa_str.starts_with('-') {
        -1.0
    } else {
        1.0
    };

    Ok(sign * mantissa * 10.0_f64.powi(exp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::KnownBody;
    use crate::earth::MU as MU_EARTH;

    const ISS_TLE: &str = "\
ISS (ZARYA)
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008";

    const ISS_TLE_2LINE: &str = "\
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008";

    // GEO satellite (INTELSAT 10-02)
    const GEO_TLE: &str = "\
1 28358U 04022A   24079.50000000  .00000012  00000-0  00000+0 0  9993
2 28358   0.0300 275.4700 0003500 135.2000 224.8000  1.00271000 72001";

    // Alpha-5 catalog number: "A0000" → 100000. Same orbit as the ISS TLE.
    const ALPHA5_TLE: &str = "\
1 A0000U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996
2 A0000  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008";

    #[test]
    fn parse_iss_3line() {
        let set = parse(ISS_TLE).unwrap();
        let omm = set.elements.to_fields();
        assert_eq!(set.object_name.as_deref(), Some("ISS (ZARYA)"));
        assert_eq!(set.object_id.as_deref(), Some("1998-067A"));
        assert_eq!(omm.norad_cat_id, 25544);
        assert!((omm.inclination.to_degrees() - 51.64).abs() < 0.01);
        assert!((omm.raan.to_degrees() - 208.652).abs() < 0.01);
        assert!((omm.eccentricity - 0.0007417).abs() < 1e-8);
        assert!((omm.argument_of_perigee.to_degrees() - 35.391).abs() < 0.01);
        assert!((omm.mean_anomaly.to_degrees() - 324.758).abs() < 0.01);
        let mm_rev_day = omm.mean_motion * 86400.0 / (2.0 * PI);
        assert!(
            (mm_rev_day - 15.4956165).abs() < 0.001,
            "mean motion: {mm_rev_day} rev/day"
        );
    }

    #[test]
    fn parse_iss_2line() {
        let set = parse(ISS_TLE_2LINE).unwrap();
        let omm = set.elements.to_fields();
        assert!(set.object_name.is_none());
        assert_eq!(omm.norad_cat_id, 25544);
        assert!((omm.inclination.to_degrees() - 51.64).abs() < 0.01);
    }

    #[test]
    fn bom_prefixed_tle_parses() {
        // A leading UTF-8 BOM must not break the line-1 prefix check
        // (matches the BOM tolerance of the unified `elements::parse` entry point).
        let bom_tle = ["\u{feff}", ISS_TLE_2LINE].concat();
        assert_eq!(
            parse(&bom_tle).unwrap().elements.fields().norad_cat_id,
            25544
        );
    }

    #[test]
    fn strips_3le_zero_prefix_from_name() {
        // CelesTrak "3LE" prefixes the name line with the "0 " line number; it
        // must be stripped so the name matches the bare-name / OMM forms.
        let tle = "0 ISS (ZARYA)\n\
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996\n\
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008";
        let omm = parse(tle).unwrap();
        assert_eq!(omm.object_name.as_deref(), Some("ISS (ZARYA)"));
    }

    #[test]
    fn parse_geo_satellite() {
        let omm = parse(GEO_TLE).unwrap().elements.to_fields();
        assert_eq!(omm.norad_cat_id, 28358);
        assert!(
            omm.inclination.to_degrees() < 1.0,
            "GEO should have near-zero inclination"
        );
        let mm_rev_day = omm.mean_motion * 86400.0 / (2.0 * PI);
        assert!(
            (mm_rev_day - 1.0027).abs() < 0.01,
            "GEO mean motion: {mm_rev_day} rev/day"
        );
    }

    #[test]
    fn parse_error_insufficient_lines() {
        assert!(parse("only one line").is_err());
    }

    #[test]
    fn parse_error_invalid_prefix() {
        assert!(parse("X invalid line 1\n2 25544  51.6400 ...").is_err());
    }

    #[test]
    fn iss_epoch() {
        let omm = parse(ISS_TLE).unwrap().elements.to_fields();
        let dt = omm.epoch.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 19);
        assert_eq!(dt.hour, 12);
    }

    /// Rewrite every TLE line's checksum digit so a *deliberately* mutated
    /// fixture still exercises the check the test is about rather than tripping
    /// the checksum first.
    fn refresh_checksums(tle: &str) -> String {
        let mut out = String::new();
        for line in tle.lines() {
            if (line.starts_with('1') || line.starts_with('2')) && line.chars().count() >= 69 {
                let digit = line_checksum(line);
                let kept: String = line.chars().take(68).collect();
                out.push_str(&format!("{kept}{digit}"));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn rejects_out_of_range_epoch_day() {
        // day-of-year must be 1.0 ≤ day < 366 (367 in a leap year). The
        // checksums are refreshed so this pins the day-of-year range check
        // itself, not the checksum of the edited line.
        let day_zero =
            refresh_checksums(&ISS_TLE_2LINE.replace("24079.50000000", "24000.00000000"));
        assert!(
            matches!(
                parse(&day_zero),
                Err(TleParseError::InvalidField {
                    field: "epoch_day",
                    ..
                })
            ),
            "day 0.0 must be rejected: {:?}",
            parse(&day_zero)
        );
        let day_high =
            refresh_checksums(&ISS_TLE_2LINE.replace("24079.50000000", "24367.00000000"));
        assert!(
            matches!(
                parse(&day_high),
                Err(TleParseError::InvalidField {
                    field: "epoch_day",
                    ..
                })
            ),
            "day 367.0 must be rejected: {:?}",
            parse(&day_high)
        );
    }

    #[test]
    fn fixture_checksums_are_self_consistent() {
        // Pins every TLE literal in this module against checksum rot: each
        // line's column-69 digit must be the mod-10 sum of columns 1-68.
        for (name, tle) in [
            ("ISS_TLE", ISS_TLE),
            ("ISS_TLE_2LINE", ISS_TLE_2LINE),
            ("GEO_TLE", GEO_TLE),
            ("ALPHA5_TLE", ALPHA5_TLE),
        ] {
            for line in tle.lines() {
                let num = match line.chars().next() {
                    Some('1') => 1,
                    Some('2') => 2,
                    _ => continue,
                };
                assert_eq!(
                    verify_checksum(line, num),
                    Ok(()),
                    "{name} line {num} has a wrong checksum digit (expected {})",
                    line_checksum(line)
                );
            }
        }
    }

    #[test]
    fn single_digit_mutation_is_rejected() {
        // Mutation invariant: bumping any one digit of either line changes the
        // mod-10 sum, so a correct fixture can never survive a one-character
        // edit. Needs no external data.
        assert!(parse(ISS_TLE_2LINE).is_ok(), "base fixture must parse");
        let lines: Vec<&str> = ISS_TLE_2LINE.lines().collect();
        for (li, line) in lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            for (ci, c) in chars.iter().enumerate() {
                let Some(d) = c.to_digit(10) else { continue };
                let mut mutated = chars.clone();
                mutated[ci] = char::from_digit((d + 1) % 10, 10).unwrap();
                let mutated: String = mutated.into_iter().collect();
                let text = if li == 0 {
                    format!("{mutated}\n{}", lines[1])
                } else {
                    format!("{}\n{mutated}", lines[0])
                };
                assert!(
                    parse(&text).is_err(),
                    "digit at line {} column {} bumped from {d}: expected Err, got Ok",
                    li + 1,
                    ci + 1
                );
            }
        }
    }

    #[test]
    fn lines_from_different_satellites_are_rejected() {
        // Cross-line invariant: both lines are individually well-formed (valid
        // checksums), so only the catalog-number cross-check can catch this —
        // and without it the result is an ISS-labelled GEO orbit.
        let iss1 = ISS_TLE_2LINE.lines().next().unwrap();
        let iss2 = ISS_TLE_2LINE.lines().nth(1).unwrap();
        let geo1 = GEO_TLE.lines().next().unwrap();
        let geo2 = GEO_TLE.lines().nth(1).unwrap();
        assert_eq!(
            parse(&format!("{iss1}\n{geo2}")),
            Err(TleParseError::CatalogNumberMismatch {
                line1: 25544,
                line2: 28358,
            })
        );
        assert_eq!(
            parse(&format!("{geo1}\n{iss2}")),
            Err(TleParseError::CatalogNumberMismatch {
                line1: 28358,
                line2: 25544,
            })
        );
    }

    #[test]
    fn checksum_errors_report_the_expected_digit() {
        let lines: Vec<&str> = ISS_TLE_2LINE.lines().collect();
        // Wrong digit in column 69 of line 1 (the pre-fix state of this fixture).
        let bad1: String = lines[0]
            .chars()
            .take(68)
            .chain(core::iter::once('3'))
            .collect();
        assert_eq!(
            parse(&format!("{bad1}\n{}", lines[1])),
            Err(TleParseError::InvalidChecksum {
                line: 1,
                expected: 6,
                found: Some('3'),
            })
        );
        // A line truncated before column 69 has no checksum digit at all.
        let short: String = lines[1].chars().take(68).collect();
        assert_eq!(
            parse(&format!("{}\n{short}", lines[0])),
            Err(TleParseError::InvalidChecksum {
                line: 2,
                expected: 8,
                found: None,
            })
        );
    }

    #[test]
    fn iss_semi_major_axis() {
        let omm = parse(ISS_TLE).unwrap().elements;
        let a = omm.semi_major_axis(MU_EARTH);
        let earth_radius = KnownBody::Earth.properties().radius;
        let altitude = a - earth_radius;
        assert!(
            (400.0 - altitude).abs() < 30.0,
            "ISS altitude should be ~400km, got {altitude:.1}km (a={a:.1}km)"
        );
    }

    #[test]
    fn geo_semi_major_axis() {
        let omm = parse(GEO_TLE).unwrap().elements;
        let a = omm.semi_major_axis(MU_EARTH);
        assert!(
            (a - 42164.0).abs() < 50.0,
            "GEO semi-major axis should be ~42164km, got {a:.1}km"
        );
    }

    #[test]
    fn three_line_and_two_line_produce_same_result() {
        let omm3 = parse(ISS_TLE).unwrap().elements.to_fields();
        let omm2 = parse(ISS_TLE_2LINE).unwrap().elements.to_fields();
        assert_eq!(omm3.norad_cat_id, omm2.norad_cat_id);
        assert!((omm3.inclination - omm2.inclination).abs() < 1e-15);
        assert!((omm3.raan - omm2.raan).abs() < 1e-15);
        assert!((omm3.eccentricity - omm2.eccentricity).abs() < 1e-15);
        assert!((omm3.mean_motion - omm2.mean_motion).abs() < 1e-15);
    }

    #[test]
    fn iss_bstar() {
        // ISS TLE has "30000-4" → 0.30000e-4 = 3.0e-5
        let omm = parse(ISS_TLE).unwrap().elements.to_fields();
        assert!(
            (omm.bstar - 3.0e-5).abs() < 1e-10,
            "ISS B* should be 3.0e-5, got {:.6e}",
            omm.bstar
        );
    }

    #[test]
    fn geo_bstar_zero() {
        // GEO TLE has "00000+0" → 0.0
        let omm = parse(GEO_TLE).unwrap().elements.to_fields();
        assert_eq!(omm.bstar, 0.0, "GEO B* should be 0.0, got {}", omm.bstar);
    }

    #[test]
    fn assumed_decimal_rejects_malformed_without_panicking() {
        // A single sign char, or a leading multi-byte char, must Err — not panic
        // (parsers must tolerate untrusted element-set text).
        assert!(parse_assumed_decimal("+", 0, 1, 1, "bstar").is_err());
        let minus = "\u{2212}"; // U+2212 MINUS SIGN (3 bytes)
        assert!(parse_assumed_decimal(minus, 0, minus.len(), 1, "bstar").is_err());
    }

    #[test]
    fn alpha5_catalog_number() {
        // Alpha-5: leading 'A' = 10 → 10 * 10000 + 0000 = 100000.
        let omm = parse(ALPHA5_TLE).unwrap().elements.to_fields();
        assert_eq!(omm.norad_cat_id, 100000);
        // The rest of the element set must still parse normally.
        assert!((omm.inclination.to_degrees() - 51.64).abs() < 0.01);
    }

    #[test]
    fn decode_catalog_number_cases() {
        // Classic all-numeric (leading zeros allowed).
        assert_eq!(decode_catalog_number("25544", 1), Ok(25544));
        assert_eq!(decode_catalog_number("00005", 1), Ok(5));
        // Alpha-5 boundaries — note the I/O skips.
        assert_eq!(decode_catalog_number("A0000", 1), Ok(100000)); // A = 10
        assert_eq!(decode_catalog_number("E8493", 1), Ok(148493)); // E = 14
        assert_eq!(decode_catalog_number("H0000", 1), Ok(170000)); // H = 17 (before I)
        assert_eq!(decode_catalog_number("J0000", 1), Ok(180000)); // J = 18 (I skipped)
        assert_eq!(decode_catalog_number("N0000", 1), Ok(220000)); // N = 22 (before O)
        assert_eq!(decode_catalog_number("P0000", 1), Ok(230000)); // P = 23 (O skipped)
        assert_eq!(decode_catalog_number("Z9999", 1), Ok(339999)); // Z = 33 (max)
        // I and O are not valid leading characters (look like 1 and 0).
        assert!(decode_catalog_number("I0000", 1).is_err());
        assert!(decode_catalog_number("O0000", 1).is_err());
        // Non-digit trailing is rejected.
        assert!(decode_catalog_number("A00X0", 1).is_err());
        // Alpha-5 must be exactly 1 letter + 4 digits — short fields are
        // corrupt input, not value 100000.
        assert!(decode_catalog_number("A000", 1).is_err());
        assert!(decode_catalog_number("A", 1).is_err());
        assert!(decode_catalog_number("A00000", 1).is_err());
    }

    #[test]
    fn intl_designator_normalized_to_omm_form() {
        assert_eq!(normalize_intl_designator("98067A"), "1998-067A");
        assert_eq!(normalize_intl_designator("04022A"), "2004-022A");
        assert_eq!(normalize_intl_designator("57001B"), "1957-001B"); // Sputnik-era floor
        assert_eq!(normalize_intl_designator("20001AB"), "2020-001AB"); // multi-letter piece
        assert_eq!(normalize_intl_designator("XYZ"), "XYZ"); // unparseable left as-is
    }
}
