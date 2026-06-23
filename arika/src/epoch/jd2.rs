//! Two-part Julian Date (`hi + lo`) for sub-nanosecond time precision.
//!
//! A single `f64` Julian Date floors resolution at tens of µs near modern
//! epochs (the mantissa is spent on the ~2.46e6 integer part) and cancels
//! catastrophically when differencing epochs near J2000. Carrying the value as
//! two `f64`s — a high part plus a small residual, as SOFA's two-part date
//! arguments do — keeps the full mantissa available for the sub-day fraction.
//!
//! Internal building block for the canonical-TAI [`Epoch`](super::Epoch); not a
//! public API.

/// Knuth's two-sum: exact `a + b = s + e` with `s = fl(a+b)` and `e` the
/// rounding error. No ordering requirement on `|a|`, `|b|`.
#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    (s, e)
}

/// A Julian Date carried as `hi + lo`, normalized so `hi == fl(hi + lo)` and
/// `lo` holds the residual. The represented value is exactly `hi + lo` in
/// extended precision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TwoPartJd {
    hi: f64,
    lo: f64,
}

impl TwoPartJd {
    /// From a single `f64` JD (residual zero). This is the f64-precision entry
    /// point; precision beyond `f64` requires [`from_parts`](Self::from_parts).
    #[inline]
    pub(crate) fn from_jd(jd: f64) -> Self {
        Self { hi: jd, lo: 0.0 }
    }

    /// From an explicit high part and residual, renormalized so the invariant
    /// `hi == fl(hi + lo)` holds.
    #[inline]
    pub(crate) fn from_parts(hi: f64, lo: f64) -> Self {
        let (s, e) = two_sum(hi, lo);
        Self { hi: s, lo: e }
    }

    /// The value as a single `f64` (lossy combine; for back-compat / display).
    #[inline]
    pub(crate) fn jd(self) -> f64 {
        self.hi + self.lo
    }

    /// The `(hi, lo)` parts (lossless).
    #[inline]
    pub(crate) fn parts(self) -> (f64, f64) {
        (self.hi, self.lo)
    }

    /// Add `days` (a full-precision `f64`) keeping extended precision.
    #[inline]
    pub(crate) fn add_days(self, days: f64) -> Self {
        let (s, e) = two_sum(self.hi, days);
        Self::from_parts(s, e + self.lo)
    }

    /// Difference `self - other` in days, in extended precision.
    ///
    /// The high parts subtract first (exact by Sterbenz when they are within a
    /// factor of two — i.e. epochs close together, the case that matters near
    /// J2000), then the residuals are folded in.
    #[inline]
    pub(crate) fn diff_days(self, other: Self) -> f64 {
        let (dh, eh) = two_sum(self.hi, -other.hi);
        // dh + eh + (self.lo - other.lo)
        dh + (eh + (self.lo - other.lo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_jd_roundtrips() {
        let x = 2_460_000.123_456;
        assert_eq!(TwoPartJd::from_jd(x).jd(), x);
        assert_eq!(TwoPartJd::from_jd(x).parts(), (x, 0.0));
    }

    #[test]
    fn from_parts_normalizes() {
        // hi has no room for a sub-µs residual, but lo carries it exactly.
        let hi = 2_460_000.0;
        let lo = 1e-9; // ~86 µs is 1e-9 day; this is well below single-f64 ulp here
        let t = TwoPartJd::from_parts(hi, lo);
        let (h, l) = t.parts();
        // The residual survives in lo (single-f64 jd() would lose it).
        assert_eq!(h + l, hi + lo);
        assert!(l != 0.0, "residual must be retained in lo");
    }

    #[test]
    fn diff_near_j2000_is_exact() {
        // Two epochs 1 ns apart near a large JD: single-f64 difference floors to
        // 0, two-part keeps it.
        const ONE_NS_DAY: f64 = 1.0 / 86_400.0 / 1e9;
        let a = TwoPartJd::from_parts(2_451_545.0, 0.0);
        let b = a.add_days(ONE_NS_DAY);
        let d = b.diff_days(a);
        let rel_err = ((d - ONE_NS_DAY) / ONE_NS_DAY).abs();
        assert!(rel_err < 1e-6, "1 ns diff lost: got {d}, want {ONE_NS_DAY}");
        // Sanity: naive single-f64 subtraction loses it entirely.
        let naive = (2_451_545.0_f64 + ONE_NS_DAY) - 2_451_545.0_f64;
        assert_eq!(naive, 0.0, "precondition: single-f64 floors a 1 ns step to 0");
    }

    #[test]
    fn add_days_accumulates_without_drift() {
        // Add 1 µs-day a million times; compare to the exact 1e6× value.
        const STEP: f64 = 1e-6;
        let mut t = TwoPartJd::from_jd(2_460_000.5);
        for _ in 0..1_000_000 {
            t = t.add_days(STEP);
        }
        let elapsed = t.diff_days(TwoPartJd::from_jd(2_460_000.5));
        assert!(
            (elapsed - 1.0).abs() < 1e-9,
            "accumulated {elapsed}, want 1.0 day"
        );
    }

    #[test]
    fn non_finite_propagates() {
        assert!(TwoPartJd::from_jd(f64::NAN).jd().is_nan());
        assert!(TwoPartJd::from_jd(f64::INFINITY).jd().is_infinite());
        // diff with NaN is NaN, not a silent finite value.
        let nan = TwoPartJd::from_jd(f64::NAN);
        let ok = TwoPartJd::from_jd(2_460_000.0);
        assert!(nan.diff_days(ok).is_nan());
    }
}
