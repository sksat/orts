//! EOP lookup table with interpolation.

use super::entry::EopEntry;
use super::error::EopLookupError;

/// Interpolated EOP lookup table.
///
/// Stores sorted EOP entries and provides interpolated parameter
/// lookups at arbitrary MJD values. Implements all four EOP
/// capability traits (`Ut1Offset`, `PolarMotion`, `NutationCorrections`,
/// `LengthOfDay`).
pub struct EopTable {
    entries: Vec<EopEntry>,
}

impl EopTable {
    /// Create an EOP table from a sorted vector of entries.
    ///
    /// Entries must be sorted by MJD and non-empty.
    pub fn new(entries: Vec<EopEntry>) -> Result<Self, EopLookupError> {
        if entries.is_empty() {
            return Err(EopLookupError::Empty);
        }
        Ok(Self { entries })
    }

    /// Convenience: parse finals2000A text and build a table.
    pub fn from_finals2000a(text: &str) -> Result<Self, super::error::EopParseError> {
        let entries = super::finals2000a::Finals2000A::parse(text)?;
        Self::new(entries).map_err(|_| super::error::EopParseError::Empty)
    }

    /// MJD range covered by this table.
    pub fn mjd_range(&self) -> (f64, f64) {
        (
            self.entries.first().unwrap().mjd,
            self.entries.last().unwrap().mjd,
        )
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Checked UT1-UTC lookup with linear interpolation.
    pub fn dut1_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.dut1)
    }

    /// Checked x-pole lookup.
    pub fn xp_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.xp)
    }

    /// Checked y-pole lookup.
    pub fn yp_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.yp)
    }

    /// Checked dX nutation correction lookup.
    pub fn dx_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.dx.unwrap_or(0.0))
    }

    /// Checked dY nutation correction lookup.
    pub fn dy_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.dy.unwrap_or(0.0))
    }

    /// Checked LOD lookup.
    pub fn lod_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        self.interpolate(utc_mjd, |e| e.lod.unwrap_or(0.0))
    }

    /// Linear interpolation between bracketing entries.
    fn interpolate(
        &self,
        utc_mjd: f64,
        field: impl Fn(&EopEntry) -> f64,
    ) -> Result<f64, EopLookupError> {
        let first = self.entries.first().unwrap();
        let last = self.entries.last().unwrap();

        if utc_mjd < first.mjd || utc_mjd > last.mjd {
            return Err(EopLookupError::OutOfRange {
                mjd: utc_mjd,
                start: first.mjd,
                end: last.mjd,
            });
        }

        // Binary search for the bracketing interval
        let idx = self
            .entries
            .partition_point(|e| e.mjd <= utc_mjd)
            .saturating_sub(1);

        let e0 = &self.entries[idx];

        // Exact match or last entry
        if idx + 1 >= self.entries.len() || (utc_mjd - e0.mjd).abs() < 1e-12 {
            return Ok(field(e0));
        }

        let e1 = &self.entries[idx + 1];
        let frac = (utc_mjd - e0.mjd) / (e1.mjd - e0.mjd);
        Ok(field(e0) + frac * (field(e1) - field(e0)))
    }
}

// --- EOP trait implementations ---

use super::{LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset};

impl Ut1Offset for EopTable {
    fn dut1(&self, utc_mjd: f64) -> f64 {
        self.dut1_checked(utc_mjd)
            .expect("EOP UT1-UTC lookup failed")
    }
}

impl PolarMotion for EopTable {
    fn x_pole(&self, utc_mjd: f64) -> f64 {
        self.xp_checked(utc_mjd).expect("EOP x-pole lookup failed")
    }
    fn y_pole(&self, utc_mjd: f64) -> f64 {
        self.yp_checked(utc_mjd).expect("EOP y-pole lookup failed")
    }
}

impl NutationCorrections for EopTable {
    fn dx(&self, utc_mjd: f64) -> f64 {
        self.dx_checked(utc_mjd).expect("EOP dX lookup failed")
    }
    fn dy(&self, utc_mjd: f64) -> f64 {
        self.dy_checked(utc_mjd).expect("EOP dY lookup failed")
    }
}

impl LengthOfDay for EopTable {
    fn lod(&self, utc_mjd: f64) -> f64 {
        self.lod_checked(utc_mjd).expect("EOP LOD lookup failed")
    }
}
