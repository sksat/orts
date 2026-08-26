//! Test-only helpers shared by the module test suites.

use arika::earth::GcrsEopStorage;
use arika::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};

/// EOP provider with every correction zero: the IAU 2006 **model** chain only
/// (no observed dX/dY, no dUT1, no polar motion).
///
/// A deterministic model-only fixture, not an accuracy claim: zeroing dUT1
/// alone displaces ERA by up to ~0.4 arcsecond, so these values do not track
/// observed Earth orientation. The point is that `Gcrs` tests are reproducible
/// without shipping an EOP table — the same provider the `oracle_gcrf` suite
/// uses.
pub(crate) struct ZeroEop;

impl Ut1Offset for ZeroEop {
    fn dut1(&self, _: f64) -> f64 {
        0.0
    }
}

impl PolarMotion for ZeroEop {
    fn x_pole(&self, _: f64) -> f64 {
        0.0
    }
    fn y_pole(&self, _: f64) -> f64 {
        0.0
    }
}

impl NutationCorrections for ZeroEop {
    fn dx(&self, _: f64) -> f64 {
        0.0
    }
    fn dy(&self, _: f64) -> f64 {
        0.0
    }
}

/// `GcrsEopStorage` over [`ZeroEop`].
pub(crate) fn zero_eop() -> GcrsEopStorage {
    GcrsEopStorage::new(ZeroEop)
}
