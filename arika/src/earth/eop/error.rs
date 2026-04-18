//! Error types for EOP parsing and lookup.

use alloc::string::String;
use core::fmt;

/// Error during EOP data file parsing.
#[derive(Debug)]
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
        }
    }
}

impl core::error::Error for EopParseError {}

/// Error during EOP table lookup.
#[derive(Debug)]
pub enum EopLookupError {
    /// The table is empty.
    Empty,
    /// The requested MJD is outside the table range.
    OutOfRange { mjd: f64, start: f64, end: f64 },
}

impl fmt::Display for EopLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "EOP table is empty"),
            Self::OutOfRange { mjd, start, end } => {
                write!(f, "MJD {mjd} outside EOP range [{start}, {end}]")
            }
        }
    }
}

impl core::error::Error for EopLookupError {}
