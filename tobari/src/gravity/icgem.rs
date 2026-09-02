//! ICGEM `.gfc` gravity-field file parser (static coefficients only).
//!
//! Format reference: ICGEM, "The ICGEM-format" (2023),
//! <https://icgem.gfz-potsdam.de/docs/ICGEM-Format-2023.pdf>. A file is a
//! free-form header of `keyword value` lines ended by `end_of_head`, followed
//! by data records. This parser accepts the static record kind
//!
//! ```text
//! gfc  n  m  C̄nm  S̄nm  [σC  σS]
//! ```
//!
//! and **rejects** the time-variable kinds (`gfct`, `trnd`/`dot`, `asin`,
//! `acos`) with [`IcgemError::TimeVariableUnsupported`]: their meaning depends
//! on a reference epoch and validity interval, so reading `gfct` as a static
//! coefficient would silently fabricate a static model out of a time-variable
//! one. Static models (EGM96, EGM2008, EIGEN-6C4, GOCO06s, …) contain `gfc`
//! records only.
//!
//! Units in the file are SI (`m³/s²`, `m`); the parsed field is in km.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use super::legendre::{tri_index, tri_len};

/// Permanent-tide convention of the C̄20 coefficient, as declared by the file.
///
/// The parser records this and the evaluator does **not** convert between
/// systems. A ~4e-9 difference in C̄20 separates `tide_free` from
/// `zero_tide`; whether that matters depends on which other tide models the
/// caller adds, so the decision is left to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TideSystem {
    /// Permanent tide removed entirely (e.g. EGM2008 as distributed).
    TideFree,
    /// Direct permanent tide removed, indirect (deformation) part kept.
    ZeroTide,
    /// Permanent tide fully included.
    MeanTide,
    /// Header said `unknown`, or had no `tide_system` line.
    Unknown,
}

/// Why an ICGEM file could not be turned into a [`SphericalHarmonicField`](super::SphericalHarmonicField).
#[derive(Debug, Clone, PartialEq)]
pub enum IcgemError {
    /// `end_of_head` never appeared.
    MissingEndOfHead,
    /// A required header keyword is absent.
    MissingHeader(&'static str),
    /// A header value did not parse (`keyword`, `value`).
    InvalidHeader(&'static str, String),
    /// `product_type` was present but not `gravity_field`.
    NotAGravityField(String),
    /// `norm` was something other than `fully_normalized`.
    UnsupportedNormalization(String),
    /// A time-variable record kind (`gfct`, `trnd`, `dot`, `asin`, `acos`).
    TimeVariableUnsupported { line: usize, kind: String },
    /// A data record kind this parser does not know.
    UnknownRecord { line: usize, kind: String },
    /// A `gfc` record with the wrong number of columns or an unparsable field.
    MalformedRecord { line: usize, reason: String },
    /// `n > max_degree` or `m > n`.
    IndexOutOfRange {
        line: usize,
        degree: usize,
        order: usize,
    },
    /// The same `(n, m)` appeared twice.
    DuplicateCoefficient {
        line: usize,
        degree: usize,
        order: usize,
    },
    /// A coefficient value was NaN or infinite.
    NonFiniteCoefficient {
        line: usize,
        degree: usize,
        order: usize,
    },
    /// `C̄00` was present and not 1 (the point-mass term is modelled separately).
    UnexpectedC00(f64),
    /// A degree-1 coefficient was non-zero: that encodes an offset between the
    /// coordinate origin and the centre of mass, which this evaluator does not
    /// model (it starts at degree 2).
    NonZeroDegreeOne { degree: usize, order: usize },
    /// A coefficient with `2 ≤ n ≤ max_degree` never appeared.
    MissingCoefficient { degree: usize, order: usize },
}

impl fmt::Display for IcgemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEndOfHead => write!(f, "ICGEM: no `end_of_head` line"),
            Self::MissingHeader(k) => write!(f, "ICGEM: missing header `{k}`"),
            Self::InvalidHeader(k, v) => write!(f, "ICGEM: header `{k}` has invalid value `{v}`"),
            Self::NotAGravityField(v) => {
                write!(f, "ICGEM: product_type is `{v}`, expected `gravity_field`")
            }
            Self::UnsupportedNormalization(v) => {
                write!(f, "ICGEM: norm `{v}` unsupported (only `fully_normalized`)")
            }
            Self::TimeVariableUnsupported { line, kind } => write!(
                f,
                "ICGEM line {line}: time-variable record `{kind}` unsupported (static `gfc` only)"
            ),
            Self::UnknownRecord { line, kind } => {
                write!(f, "ICGEM line {line}: unknown record kind `{kind}`")
            }
            Self::MalformedRecord { line, reason } => {
                write!(f, "ICGEM line {line}: malformed record: {reason}")
            }
            Self::IndexOutOfRange {
                line,
                degree,
                order,
            } => write!(
                f,
                "ICGEM line {line}: (n={degree}, m={order}) outside max_degree / m ≤ n"
            ),
            Self::DuplicateCoefficient {
                line,
                degree,
                order,
            } => write!(
                f,
                "ICGEM line {line}: duplicate coefficient (n={degree}, m={order})"
            ),
            Self::NonFiniteCoefficient {
                line,
                degree,
                order,
            } => write!(
                f,
                "ICGEM line {line}: non-finite coefficient (n={degree}, m={order})"
            ),
            Self::UnexpectedC00(v) => write!(f, "ICGEM: C00 = {v}, expected 1"),
            Self::NonZeroDegreeOne { degree, order } => write!(
                f,
                "ICGEM: non-zero degree-1 coefficient (n={degree}, m={order}) — origin offsets are not modelled"
            ),
            Self::MissingCoefficient { degree, order } => {
                write!(f, "ICGEM: coefficient (n={degree}, m={order}) missing")
            }
        }
    }
}

impl core::error::Error for IcgemError {}

/// Parsed contents of a static ICGEM file, SI units converted to km.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedIcgem {
    pub gm_km3_s2: f64,
    pub radius_km: f64,
    pub max_degree: usize,
    pub tide_system: TideSystem,
    pub model_name: Option<String>,
    /// C̄nm at [`tri_index`]`(n, m)`, `n ≤ max_degree`.
    pub c: Vec<f64>,
    /// S̄nm, same layout.
    pub s: Vec<f64>,
}

/// Parse a Fortran-style float (`0.484D-03` as well as `4.84e-4`).
fn parse_f64(s: &str) -> Option<f64> {
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    let fixed: String = s
        .chars()
        .map(|ch| match ch {
            'D' | 'd' => 'e',
            other => other,
        })
        .collect();
    fixed.parse::<f64>().ok()
}

fn parse_index(s: &str) -> Option<usize> {
    s.parse::<usize>().ok()
}

pub(super) fn parse(text: &str) -> Result<ParsedIcgem, IcgemError> {
    let mut lines = text.lines().enumerate();

    // Header -------------------------------------------------------------
    let mut product_type: Option<&str> = None;
    let mut gm: Option<f64> = None;
    let mut radius: Option<f64> = None;
    let mut max_degree: Option<usize> = None;
    let mut norm: Option<&str> = None;
    let mut tide_system = TideSystem::Unknown;
    let mut model_name: Option<String> = None;
    let mut error_columns = 0usize;
    let mut saw_end_of_head = false;

    for (_, line) in lines.by_ref() {
        let mut tokens = line.split_whitespace();
        let Some(key) = tokens.next() else { continue };
        let value = tokens.next();
        match key {
            "end_of_head" => {
                saw_end_of_head = true;
                break;
            }
            "product_type" => product_type = value,
            // Model names may contain spaces ("EIGEN-6C4 (static part)"), so
            // keep the whole remainder of the line, not the first token.
            "modelname" => {
                model_name = line
                    .split_once(char::is_whitespace)
                    .map(|(_, rest)| rest.trim().to_string())
                    .filter(|v| !v.is_empty());
            }
            "radius" => {
                let v = value.ok_or(IcgemError::MissingHeader("radius"))?;
                radius = Some(
                    parse_f64(v)
                        .ok_or_else(|| IcgemError::InvalidHeader("radius", v.to_string()))?,
                );
            }
            "max_degree" => {
                let v = value.ok_or(IcgemError::MissingHeader("max_degree"))?;
                max_degree = Some(
                    parse_index(v)
                        .ok_or_else(|| IcgemError::InvalidHeader("max_degree", v.to_string()))?,
                );
            }
            "norm" => norm = value,
            "tide_system" => {
                tide_system = match value {
                    Some("tide_free") => TideSystem::TideFree,
                    Some("zero_tide") => TideSystem::ZeroTide,
                    Some("mean_tide") => TideSystem::MeanTide,
                    Some("unknown") | None => TideSystem::Unknown,
                    Some(other) => {
                        return Err(IcgemError::InvalidHeader("tide_system", other.to_string()));
                    }
                }
            }
            "errors" => {
                error_columns = match value {
                    Some("no") | None => 0,
                    Some("calibrated") | Some("formal") => 2,
                    Some("calibrated_and_formal") => 4,
                    Some(other) => {
                        return Err(IcgemError::InvalidHeader("errors", other.to_string()));
                    }
                }
            }
            // Non-Earth bodies use e.g. `gravity_constant`; accept any suffix
            // match like Orekit does.
            k if k.ends_with("gravity_constant") => {
                let v = value.ok_or(IcgemError::MissingHeader("earth_gravity_constant"))?;
                gm = Some(parse_f64(v).ok_or_else(|| {
                    IcgemError::InvalidHeader("earth_gravity_constant", v.to_string())
                })?);
            }
            _ => {}
        }
    }

    if !saw_end_of_head {
        return Err(IcgemError::MissingEndOfHead);
    }
    if let Some(pt) = product_type
        && pt != "gravity_field"
    {
        return Err(IcgemError::NotAGravityField(pt.to_string()));
    }
    match norm {
        Some("fully_normalized") | None => {}
        Some(other) => return Err(IcgemError::UnsupportedNormalization(other.to_string())),
    }
    let gm = gm.ok_or(IcgemError::MissingHeader("earth_gravity_constant"))?;
    let radius = radius.ok_or(IcgemError::MissingHeader("radius"))?;
    let max_degree = max_degree.ok_or(IcgemError::MissingHeader("max_degree"))?;
    if !(gm.is_finite() && gm > 0.0) {
        return Err(IcgemError::InvalidHeader(
            "earth_gravity_constant",
            gm.to_string(),
        ));
    }
    if !(radius.is_finite() && radius > 0.0) {
        return Err(IcgemError::InvalidHeader("radius", radius.to_string()));
    }

    // Data ---------------------------------------------------------------
    let len = tri_len(max_degree);
    let mut c = vec![0.0; len];
    let mut s = vec![0.0; len];
    let mut seen = vec![false; len];
    let expected_columns = 5 + error_columns;

    for (idx, line) in lines {
        let line_no = idx + 1;
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(&kind) = tokens.first() else {
            continue;
        };
        match kind {
            "gfc" => {}
            "gfct" | "trnd" | "dot" | "asin" | "acos" => {
                return Err(IcgemError::TimeVariableUnsupported {
                    line: line_no,
                    kind: kind.to_string(),
                });
            }
            other => {
                return Err(IcgemError::UnknownRecord {
                    line: line_no,
                    kind: other.to_string(),
                });
            }
        }
        if tokens.len() != expected_columns {
            return Err(IcgemError::MalformedRecord {
                line: line_no,
                reason: alloc::format!(
                    "expected {expected_columns} columns, found {}",
                    tokens.len()
                ),
            });
        }
        let n = parse_index(tokens[1]).ok_or_else(|| IcgemError::MalformedRecord {
            line: line_no,
            reason: alloc::format!("degree `{}` is not an integer", tokens[1]),
        })?;
        let m = parse_index(tokens[2]).ok_or_else(|| IcgemError::MalformedRecord {
            line: line_no,
            reason: alloc::format!("order `{}` is not an integer", tokens[2]),
        })?;
        if n > max_degree || m > n {
            return Err(IcgemError::IndexOutOfRange {
                line: line_no,
                degree: n,
                order: m,
            });
        }
        let cnm = parse_f64(tokens[3]).ok_or_else(|| IcgemError::MalformedRecord {
            line: line_no,
            reason: alloc::format!("C `{}` is not a number", tokens[3]),
        })?;
        let snm = parse_f64(tokens[4]).ok_or_else(|| IcgemError::MalformedRecord {
            line: line_no,
            reason: alloc::format!("S `{}` is not a number", tokens[4]),
        })?;
        if !(cnm.is_finite() && snm.is_finite()) {
            return Err(IcgemError::NonFiniteCoefficient {
                line: line_no,
                degree: n,
                order: m,
            });
        }
        let i = tri_index(n, m);
        if seen[i] {
            return Err(IcgemError::DuplicateCoefficient {
                line: line_no,
                degree: n,
                order: m,
            });
        }
        seen[i] = true;
        c[i] = cnm;
        s[i] = snm;
    }

    // Semantic checks ------------------------------------------------------
    if seen[tri_index(0, 0)] && (c[tri_index(0, 0)] - 1.0).abs() > 1e-9 {
        return Err(IcgemError::UnexpectedC00(c[tri_index(0, 0)]));
    }
    if max_degree >= 1 {
        for m in 0..=1 {
            let i = tri_index(1, m);
            if c[i] != 0.0 || s[i] != 0.0 {
                return Err(IcgemError::NonZeroDegreeOne {
                    degree: 1,
                    order: m,
                });
            }
        }
    }
    for n in 2..=max_degree {
        for m in 0..=n {
            if !seen[tri_index(n, m)] {
                return Err(IcgemError::MissingCoefficient {
                    degree: n,
                    order: m,
                });
            }
        }
    }
    // The degree-0/1 slots are exactly 1 / 0 whether or not the file spelled
    // them out.
    c[tri_index(0, 0)] = 1.0;

    Ok(ParsedIcgem {
        gm_km3_s2: gm / 1e9,
        radius_km: radius / 1e3,
        max_degree,
        tide_system,
        model_name,
        c,
        s,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete degree-2 model with Fortran exponents and the header layout
    /// ICGEM actually emits.
    const MINIMAL: &str = "\
begin_of_head
product_type            gravity_field
modelname               TEST2
earth_gravity_constant  0.3986004415D+15
radius                  0.6378136300D+07
max_degree              2
errors                  no
norm                    fully_normalized
tide_system             zero_tide

key   L    M    C                    S
end_of_head
gfc   0    0    1.0D+00              0.0D+00
gfc   1    0    0.0                  0.0
gfc   1    1    0.0                  0.0
gfc   2    0   -0.484165143790815D-03 0.0
gfc   2    1   -0.206615509074176D-09 0.138441389137979D-08
gfc   2    2    0.243938357328313D-05 -0.140027370385934D-05
";

    #[test]
    fn parses_header_and_converts_to_km() {
        let p = parse(MINIMAL).unwrap();
        assert_eq!(p.gm_km3_s2, 398600.4415);
        assert_eq!(p.radius_km, 6378.1363);
        assert_eq!(p.max_degree, 2);
        assert_eq!(p.tide_system, TideSystem::ZeroTide);
        assert_eq!(p.model_name.as_deref(), Some("TEST2"));
        assert_eq!(p.c[tri_index(0, 0)], 1.0);
        assert_eq!(p.c[tri_index(2, 0)], -0.484165143790815e-3);
        assert_eq!(p.s[tri_index(2, 1)], 0.138441389137979e-8);
        assert_eq!(p.c[tri_index(2, 2)], 0.243938357328313e-5);
        assert_eq!(p.s[tri_index(2, 2)], -0.140027370385934e-5);
    }

    /// EGM2008's own header/record lines (as distributed by ICGEM) parse and
    /// keep C̄20 bit-exact: a golden value independent of any Rust code.
    #[test]
    fn egm2008_golden_lines() {
        let text = "\
product_type             gravity_field
modelname                EGM2008
earth_gravity_constant   3.986004415E+14
radius                   6378136.3
max_degree               2
errors                   formal
norm                     fully_normalized
tide_system              tide_free
end_of_head
gfc    0    0  1.000000000000E+00  0.000000000000E+00 0.0000E+00 0.0000E+00
gfc    2    0 -4.841651437908150E-04  0.000000000000E+00 7.4815E-11 0.0000E+00
gfc    2    1 -2.066155090741760E-10  1.384413891379790E-09 7.0630E-11 7.1667E-11
gfc    2    2  2.439383573283130E-06 -1.400273703859340E-06 7.2306E-11 7.3020E-11
";
        let p = parse(text).unwrap();
        assert_eq!(p.c[tri_index(2, 0)], -4.84165143790815e-4);
        assert_eq!(p.tide_system, TideSystem::TideFree);
        // Degree 1 absent → zero.
        assert_eq!(p.c[tri_index(1, 1)], 0.0);
    }

    #[test]
    fn error_column_count_must_match_header() {
        let text = MINIMAL.replace(
            "errors                  no",
            "errors                  calibrated",
        );
        assert!(matches!(
            parse(&text),
            Err(IcgemError::MalformedRecord { line: 13, .. })
        ));
    }

    #[test]
    fn rejects_time_variable_records() {
        for kind in ["gfct", "trnd", "dot", "asin", "acos"] {
            let text = alloc::format!("{MINIMAL}{kind} 2 0 1.0 0.0 20050101\n");
            assert_eq!(
                parse(&text),
                Err(IcgemError::TimeVariableUnsupported {
                    line: 19,
                    kind: kind.to_string()
                })
            );
        }
    }

    #[test]
    fn rejects_unknown_record_kind() {
        let text = alloc::format!("{MINIMAL}xyz 2 0 1.0 0.0\n");
        assert!(matches!(
            parse(&text),
            Err(IcgemError::UnknownRecord { .. })
        ));
    }

    #[test]
    fn rejects_unnormalized_and_non_gravity_products() {
        let text = MINIMAL.replace("fully_normalized", "unnormalized");
        assert_eq!(
            parse(&text),
            Err(IcgemError::UnsupportedNormalization("unnormalized".into()))
        );
        let text = MINIMAL.replace("gravity_field", "topography");
        assert_eq!(
            parse(&text),
            Err(IcgemError::NotAGravityField("topography".into()))
        );
    }

    #[test]
    fn rejects_missing_end_of_head_and_missing_headers() {
        assert_eq!(
            parse("product_type gravity_field\n"),
            Err(IcgemError::MissingEndOfHead)
        );
        let text = MINIMAL.replace("earth_gravity_constant  0.3986004415D+15\n", "");
        assert_eq!(
            parse(&text),
            Err(IcgemError::MissingHeader("earth_gravity_constant"))
        );
        let text = MINIMAL.replace("max_degree              2\n", "");
        assert_eq!(parse(&text), Err(IcgemError::MissingHeader("max_degree")));
    }

    #[test]
    fn rejects_bad_indices_duplicates_and_non_finite() {
        let text = alloc::format!("{MINIMAL}gfc 3 0 1.0 0.0\n");
        assert!(matches!(
            parse(&text),
            Err(IcgemError::IndexOutOfRange {
                degree: 3,
                order: 0,
                ..
            })
        ));
        let text = alloc::format!("{MINIMAL}gfc 2 3 1.0 0.0\n");
        assert!(matches!(
            parse(&text),
            Err(IcgemError::IndexOutOfRange {
                degree: 2,
                order: 3,
                ..
            })
        ));
        let text = alloc::format!("{MINIMAL}gfc 2 2 1.0 0.0\n");
        assert!(matches!(
            parse(&text),
            Err(IcgemError::DuplicateCoefficient {
                degree: 2,
                order: 2,
                ..
            })
        ));
        let text = MINIMAL.replace("0.243938357328313D-05", "NaN");
        assert!(matches!(
            parse(&text),
            Err(IcgemError::NonFiniteCoefficient {
                degree: 2,
                order: 2,
                ..
            })
        ));
    }

    #[test]
    fn rejects_non_unit_c00_and_non_zero_degree_one() {
        let text = MINIMAL.replace("gfc   0    0    1.0D+00", "gfc   0    0    0.5D+00");
        assert!(matches!(parse(&text), Err(IcgemError::UnexpectedC00(v)) if v == 0.5));
        let text = MINIMAL.replace(
            "gfc   1    1    0.0                  0.0",
            "gfc 1 1 0.0 1e-9",
        );
        assert_eq!(
            parse(&text),
            Err(IcgemError::NonZeroDegreeOne {
                degree: 1,
                order: 1
            })
        );
    }

    #[test]
    fn requires_every_coefficient_from_degree_two_up() {
        let text = MINIMAL.replace(
            "gfc   2    1   -0.206615509074176D-09 0.138441389137979D-08\n",
            "",
        );
        assert_eq!(
            parse(&text),
            Err(IcgemError::MissingCoefficient {
                degree: 2,
                order: 1
            })
        );
    }

    #[test]
    fn degree_zero_and_one_may_be_omitted() {
        let text = MINIMAL
            .replace("gfc   0    0    1.0D+00              0.0D+00\n", "")
            .replace("gfc   1    0    0.0                  0.0\n", "")
            .replace("gfc   1    1    0.0                  0.0\n", "");
        let p = parse(&text).unwrap();
        assert_eq!(p.c[tri_index(0, 0)], 1.0);
        assert_eq!(p.c[tri_index(1, 0)], 0.0);
    }

    #[test]
    fn model_name_keeps_the_whole_line_after_the_keyword() {
        let text = MINIMAL.replace(
            "modelname               TEST2",
            "modelname               EIGEN-6C4 (static part)   ",
        );
        let p = parse(&text).unwrap();
        assert_eq!(p.model_name.as_deref(), Some("EIGEN-6C4 (static part)"));
        // A bare keyword with no name is `None`, not `Some("")`.
        let text = MINIMAL.replace("modelname               TEST2", "modelname");
        assert_eq!(parse(&text).unwrap().model_name, None);
    }

    #[test]
    fn fortran_exponent_parsing() {
        assert_eq!(parse_f64("0.5D+01"), Some(5.0));
        assert_eq!(parse_f64("-1.25d-2"), Some(-0.0125));
        assert_eq!(parse_f64("1e3"), Some(1000.0));
        assert_eq!(parse_f64("abc"), None);
    }
}
