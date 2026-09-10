//! Parser for IERS finals2000A fixed-column format.
//!
//! Reference: USNO finals2000A format specification.
//! <https://maia.usno.navy.mil/ser7/readme.finals2000A>
//!
//! Column layout (1-indexed):
//!   1-2   : Year (YY)
//!   3-4   : Month
//!   5-6   : Day
//!   8-15  : MJD (F8.2)
//!   16    : I/P flag (polar motion)
//!   18-27 : x pole [arcsec], Bulletin A
//!   37-46 : y pole [arcsec], Bulletin A
//!   57    : I/P flag (UT1-UTC)
//!   58-68 : UT1-UTC [seconds], Bulletin A
//!   79-86 : LOD [milliseconds], Bulletin A
//!   97    : I/P flag (nutation)
//!   98-106: dX [mas], Bulletin A
//!   116-124: dY [mas], Bulletin A
//!
//! Bulletin B values (final quality, preferred when present):
//!   135-144: x pole [arcsec]
//!   145-154: y pole [arcsec]
//!   155-165: UT1-UTC [seconds]
//!   166-175: dX [mas]
//!   176-185: dY [mas]

use alloc::string::ToString;
use alloc::vec::Vec;

use super::entry::EopEntry;
use super::error::EopParseError;

/// Parser for IERS `finals2000A.all` / `finals2000A.data` / `finals2000A.daily`.
pub struct Finals2000A;

impl Finals2000A {
    /// Parse a finals2000A text file into a vector of EOP entries.
    ///
    /// Bulletin B values are preferred when available; otherwise Bulletin A
    /// values are used (matching Orekit's behavior).
    ///
    /// Lines shorter than 68 characters are silently skipped (header lines,
    /// blank lines). Only lines with a valid MJD and at least Bulletin A
    /// pole + UT1-UTC values are included.
    pub fn parse(text: &str) -> Result<Vec<EopEntry>, EopParseError> {
        let mut entries = Vec::new();
        let mut prev_mjd: Option<f64> = None;

        for (line_idx, line) in text.lines().enumerate() {
            let line_num = line_idx + 1;

            // Skip short lines (headers, blanks)
            if line.len() < 68 {
                continue;
            }

            // Parse MJD (cols 8-15, 0-indexed: 7..15)
            let mjd_str = &line[7..15];
            let mjd: f64 = match mjd_str.trim().parse() {
                Ok(v) => v,
                Err(_) => continue, // skip non-data lines
            };

            // A NaN parses as a number and then compares as neither greater
            // nor smaller, so the monotonicity test below cannot see it. Named
            // here, where the line number is still in hand.
            if !mjd.is_finite() {
                return Err(EopParseError::InvalidNumber {
                    line: line_num,
                    column: "MJD",
                    value: mjd_str.trim().to_string(),
                });
            }

            // Check monotonicity
            if let Some(prev) = prev_mjd.filter(|&p| mjd <= p) {
                return Err(EopParseError::NonMonotonicMjd {
                    line: line_num,
                    previous: prev,
                    current: mjd,
                });
            }

            // Parse Bulletin A pole (required)
            let xp_a = parse_col(line, 17, 27, "xp_A", line_num)?;
            let yp_a = parse_col(line, 36, 46, "yp_A", line_num)?;

            // Parse Bulletin A UT1-UTC (required) — cols 59-68 (1-indexed)
            let dut1_a = parse_col(line, 58, 68, "dut1_A", line_num)?;

            // Parse Bulletin A LOD [ms] (optional)
            let lod_a = parse_col_opt(line, 78, 86, "lod_A", line_num)?;

            // Parse Bulletin A nutation (optional, line must be long enough)
            let dx_a = if line.len() >= 106 {
                parse_col_opt(line, 97, 106, "dX_A", line_num)?
            } else {
                None
            };
            let dy_a = if line.len() >= 125 {
                parse_col_opt(line, 116, 125, "dY_A", line_num)?
            } else {
                None
            };

            // Parse Bulletin B values (preferred when present)
            let xp_b = if line.len() >= 144 {
                parse_col_opt(line, 134, 144, "xp_B", line_num)?
            } else {
                None
            };
            let yp_b = if line.len() >= 154 {
                parse_col_opt(line, 144, 154, "yp_B", line_num)?
            } else {
                None
            };
            let dut1_b = if line.len() >= 165 {
                parse_col_opt(line, 154, 165, "dut1_B", line_num)?
            } else {
                None
            };
            let dx_b = if line.len() >= 175 {
                parse_col_opt(line, 165, 175, "dX_B", line_num)?
            } else {
                None
            };
            let dy_b = if line.len() >= 185 {
                parse_col_opt(line, 175, 185, "dY_B", line_num)?
            } else {
                None
            };

            // Prefer Bulletin B when available
            let xp = xp_b.unwrap_or(xp_a);
            let yp = yp_b.unwrap_or(yp_a);
            let dut1 = dut1_b.unwrap_or(dut1_a);
            let dx = dx_b.or(dx_a);
            let dy = dy_b.or(dy_a);
            // LOD: only from Bulletin A (B doesn't have it)
            let lod = lod_a.map(|ms| ms * 1e-3); // ms -> seconds

            entries.push(EopEntry {
                mjd,
                xp,
                yp,
                dut1,
                lod,
                dx,
                dy,
            });

            prev_mjd = Some(mjd);
        }

        if entries.is_empty() {
            return Err(EopParseError::Empty);
        }

        Ok(entries)
    }
}

/// Parse a required fixed-column field.
fn parse_col(
    line: &str,
    start: usize,
    end: usize,
    column: &'static str,
    line_num: usize,
) -> Result<f64, EopParseError> {
    let end = end.min(line.len());
    let s = &line[start..end];
    s.trim()
        .parse::<f64>()
        .map_err(|_| EopParseError::InvalidNumber {
            line: line_num,
            column,
            value: s.trim().to_string(),
        })
}

/// Parse an optional fixed-column field: `None` where the column is blank or
/// the line stops before it, an error where it holds something that is not a
/// number.
///
/// A blank column means IERS published no value there, which a caller can
/// answer with the model alone. A column holding `0.3O0` means the row is
/// corrupt, and reading it as "no value" turned that into the same
/// model-only answer — the required columns have always been an error, and
/// these now agree.
fn parse_col_opt(
    line: &str,
    start: usize,
    end: usize,
    column: &'static str,
    line_num: usize,
) -> Result<Option<f64>, EopParseError> {
    if start >= line.len() {
        return Ok(None);
    }
    let end = end.min(line.len());
    let s = line[start..end].trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<f64>()
        .map(Some)
        .map_err(|_| EopParseError::InvalidNumber {
            line: line_num,
            column,
            value: s.to_string(),
        })
}
