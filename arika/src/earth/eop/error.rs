//! Error types for EOP parsing and lookup.

use alloc::string::String;
use core::fmt;

/// Error during EOP data file parsing.
///
/// `#[non_exhaustive]`: a downstream exhaustive `match` would otherwise break
/// on every variant added here. Use a wildcard arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum EopParseError {
    /// A line could not be parsed.
    InvalidLine { line: usize, reason: &'static str },
    /// A numeric field could not be parsed.
    InvalidNumber {
        line: usize,
        column: &'static str,
        value: String,
    },
    /// The file contained no valid entries.
    Empty,
    /// MJD values are not monotonically increasing.
    NonMonotonicMjd {
        line: usize,
        previous: f64,
        current: f64,
    },
    /// The rows parsed, and building the table from them was refused. The
    /// parser checks the line-level rules itself, so reaching this means the
    /// two disagree — reported rather than folded into [`Self::Empty`], which
    /// would say the file held no rows at all.
    Table(EopLookupError),
}

impl fmt::Display for EopParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLine { line, reason } => {
                write!(f, "line {line}: {reason}")
            }
            Self::InvalidNumber {
                line,
                column,
                value,
            } => write!(f, "line {line}: invalid number in '{column}': \"{value}\""),
            Self::Empty => write!(f, "no valid EOP entries found"),
            Self::NonMonotonicMjd {
                line,
                previous,
                current,
            } => write!(f, "line {line}: non-monotonic MJD: {previous} -> {current}"),
            Self::Table(e) => write!(f, "EOP rows parsed but the table was refused: {e}"),
        }
    }
}

impl core::error::Error for EopParseError {}

/// Error during EOP table lookup.
///
/// `#[non_exhaustive]`: a downstream exhaustive `match` would otherwise break
/// on every variant added here. Use a wildcard arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum EopLookupError {
    /// The table is empty.
    Empty,
    /// The requested MJD is outside the table range.
    OutOfRange { mjd: f64, start: f64, end: f64 },
    /// The entries are not in increasing MJD order: entry `index` is at
    /// `current`, which does not come after `previous`. Everything past
    /// construction reads that order — `mjd_range` reports the first and last
    /// entry, and the lookup bisects — so a table out of order reports a range
    /// that leaves its own rows out and interpolates over whichever interval
    /// the bisection lands on. The finals2000A parser reports the same
    /// condition as [`EopParseError::NonMonotonicMjd`].
    NonMonotonicMjd {
        index: usize,
        previous: f64,
        current: f64,
    },
}

impl fmt::Display for EopLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "EOP table is empty"),
            Self::OutOfRange { mjd, start, end } => {
                write!(f, "MJD {mjd} outside EOP range [{start}, {end}]")
            }
            Self::NonMonotonicMjd {
                index,
                previous,
                current,
            } => write!(
                f,
                "EOP entry {index} is at MJD {current}, which does not come after \
                 {previous}; entries have to increase"
            ),
        }
    }
}

impl core::error::Error for EopLookupError {}
