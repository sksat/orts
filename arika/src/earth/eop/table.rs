//! EOP lookup table with interpolation.

use alloc::vec::Vec;

use super::entry::EopEntry;
use super::error::EopLookupError;
use crate::epoch::tai_minus_utc_at_mjd;

/// Interpolated EOP lookup table.
///
/// Stores sorted EOP entries and provides interpolated parameter lookups at
/// arbitrary MJD values through the fallible `*_checked` accessors, which report
/// [`EopLookupError::OutOfRange`] outside the covered MJD span.
///
/// The table does **not** implement the EOP capability traits (`Ut1Offset`,
/// `PolarMotion`, `NutationCorrections`, `LengthOfDay`) itself: those are
/// infallible, and a table covering a finite span has no correct infallible
/// answer for an epoch it does not cover. Call [`EopTable::clamped`] (or
/// [`EopTable::into_clamped`]) to get a provider with a stated out-of-range
/// policy — see [`ClampedEop`].
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
    ///
    /// # Leap seconds
    ///
    /// dUT1 is *not* the quantity interpolated here. It jumps by a full second
    /// across a leap second (finals2000A rows are daily, so the pair bracketing
    /// 2017-01-01 reads −0.5928 s and +0.4068 s), and interpolating it directly
    /// smears half of that step over the preceding day — a ~0.5 s UT1 error,
    /// i.e. 3.7e-5 rad of ERA and ~230 m of equatorial ITRS displacement.
    ///
    /// `UT1 − TAI = dUT1 − (TAI − UTC)` is continuous by construction, so that
    /// is what gets interpolated, using arika's compiled-in leap-second table
    /// ([`crate::epoch`]) for each bracketing row; the query instant's own
    /// `TAI − UTC` is then added back. Inside a single leap-second regime this
    /// is algebraically identical to interpolating dUT1.
    pub fn dut1_checked(&self, utc_mjd: f64) -> Result<f64, EopLookupError> {
        let ut1_minus_tai = self.interpolate(utc_mjd, |e| e.dut1 - tai_minus_utc_at_mjd(e.mjd))?;
        Ok(ut1_minus_tai + tai_minus_utc_at_mjd(utc_mjd))
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

// Out-of-range policy adapters

use super::{LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset};

/// An [`EopTable`] that answers out-of-range queries with its nearest endpoint.
///
/// The EOP capability traits ([`Ut1Offset`] and friends) are infallible by
/// design, so they gate the high-precision APIs at compile time
/// ([`Epoch::<Utc>::to_ut1`](crate::epoch::Epoch::to_ut1),
/// [`Rotation::<Gcrs, Itrs>::iau2006_full_from_utc`](crate::frame::Rotation)).
/// An `EopTable` covers a finite MJD span and therefore cannot honour an
/// infallible contract; it deliberately does *not* implement those traits, so a
/// caller has to name an out-of-range policy first. This adapter is the
/// "hold the endpoint value" policy, obtained from
/// [`EopTable::clamped`] / [`EopTable::into_clamped`].
///
/// Clamping means dUT1, polar motion and dX/dY are held constant beyond the
/// table — acceptable for a short overshoot past the end of a prediction file,
/// increasingly wrong the further out the query goes. Use the `*_checked`
/// accessors when an out-of-range query must be an error instead.
pub struct ClampedEop<T>(T);

impl<T: core::borrow::Borrow<EopTable>> ClampedEop<T> {
    /// The wrapped table.
    pub fn table(&self) -> &EopTable {
        self.0.borrow()
    }

    /// Clamp `utc_mjd` into the table's covered range.
    ///
    /// Written out rather than using `f64::clamp`, which asserts `min <= max`
    /// (an unsorted table would turn a lookup into a panic) and to leave NaN
    /// alone: NaN cannot be clamped into range, so the checked lookup below
    /// reports it as out of range and the adapter yields NaN instead of a
    /// plausible-looking number.
    fn clamp_mjd(&self, utc_mjd: f64) -> f64 {
        let (start, end) = self.table().mjd_range();
        if utc_mjd < start {
            start
        } else if utc_mjd > end {
            end
        } else {
            utc_mjd
        }
    }
}

impl EopTable {
    /// Borrow this table as an endpoint-clamping EOP provider.
    ///
    /// See [`ClampedEop`] for what the policy means.
    pub fn clamped(&self) -> ClampedEop<&EopTable> {
        ClampedEop(self)
    }

    /// Consume this table into an owned endpoint-clamping EOP provider.
    ///
    /// See [`ClampedEop`] for what the policy means.
    pub fn into_clamped(self) -> ClampedEop<EopTable> {
        ClampedEop(self)
    }
}

impl<T: core::borrow::Borrow<EopTable>> Ut1Offset for ClampedEop<T> {
    fn dut1(&self, utc_mjd: f64) -> f64 {
        self.table()
            .dut1_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
}

impl<T: core::borrow::Borrow<EopTable>> PolarMotion for ClampedEop<T> {
    fn x_pole(&self, utc_mjd: f64) -> f64 {
        self.table()
            .xp_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
    fn y_pole(&self, utc_mjd: f64) -> f64 {
        self.table()
            .yp_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
}

impl<T: core::borrow::Borrow<EopTable>> NutationCorrections for ClampedEop<T> {
    fn dx(&self, utc_mjd: f64) -> f64 {
        self.table()
            .dx_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
    fn dy(&self, utc_mjd: f64) -> f64 {
        self.table()
            .dy_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
}

impl<T: core::borrow::Borrow<EopTable>> LengthOfDay for ClampedEop<T> {
    fn lod(&self, utc_mjd: f64) -> f64 {
        self.table()
            .lod_checked(self.clamp_mjd(utc_mjd))
            .unwrap_or(f64::NAN)
    }
}
