//! Julian Date storage for the canonical-TAI [`Epoch`](super::Epoch).
//!
//! [`JdRepr`] abstracts the day-level arithmetic an `Epoch` needs over its two
//! precision tiers (selected by [`Precision`](super::Precision)):
//!
//! - [`f64`] — the [`Coarse`](super::Coarse) tier. A single `f64` Julian Date
//!   floors resolution at tens of µs near modern epochs (the mantissa is spent
//!   on the ~2.46e6 integer part) and cancels catastrophically when differencing
//!   epochs near J2000. Lighter (8 bytes, no residual arithmetic) for wasm /
//!   `no_std` embedded targets that don't need sub-µs time.
//! - [`TwoPartJd`] — the [`Precise`](super::Precise) tier (the default). Carries
//!   the value as two `f64`s, a high part plus a small residual, as SOFA's
//!   two-part date arguments do — keeping the full mantissa available for the
//!   sub-day fraction (sub-nanosecond resolution, exact J2000-relative diffs).

/// Knuth's two-sum: exact `a + b = s + e` with `s = fl(a+b)` and `e` the
/// rounding error. No ordering requirement on `|a|`, `|b|`.
#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    (s, e)
}

/// The Julian Date storage behind a canonical-TAI [`Epoch`](super::Epoch): a JD
/// value plus the day-level arithmetic the epoch applies (lens conversion,
/// `duration_since`, `add_si_seconds`). Implemented by [`f64`] (the coarse tier)
/// and [`TwoPartJd`] (the precise tier); which one an `Epoch` stores is selected
/// by its [`Precision`](super::Precision). The two impls are the only intended
/// ones.
pub trait JdRepr: Copy + PartialEq + core::fmt::Debug {
    /// From a single `f64` JD. For [`TwoPartJd`] the residual is zero — precision
    /// beyond `f64` is only created by the arithmetic methods, never ingested
    /// here.
    fn from_jd(jd: f64) -> Self;

    /// From an explicit high part and residual. For [`f64`] this collapses to
    /// `hi + lo`; for [`TwoPartJd`] it retains the residual (renormalized). This
    /// is the lossless bridge between tiers (see
    /// [`Epoch::to_precision`](super::Epoch::to_precision)).
    fn from_parts(hi: f64, lo: f64) -> Self;

    /// The value as a single `f64` (a lossy collapse for the precise tier).
    fn jd(self) -> f64;

    /// The `(hi, lo)` parts; `lo` is always `0.0` for the coarse `f64` tier.
    fn parts(self) -> (f64, f64);

    /// Add `days`, keeping whatever precision the representation carries.
    fn add_days(self, days: f64) -> Self;

    /// Difference `self - other` in days.
    fn diff_days(self, other: Self) -> f64;
}

/// Coarse tier: a single `f64` JD. ~tens-of-µs resolution near modern epochs.
impl JdRepr for f64 {
    #[inline]
    fn from_jd(jd: f64) -> Self {
        jd
    }

    #[inline]
    fn from_parts(hi: f64, lo: f64) -> Self {
        hi + lo
    }

    #[inline]
    fn jd(self) -> f64 {
        self
    }

    #[inline]
    fn parts(self) -> (f64, f64) {
        (self, 0.0)
    }

    #[inline]
    fn add_days(self, days: f64) -> Self {
        self + days
    }

    #[inline]
    fn diff_days(self, other: Self) -> f64 {
        self - other
    }
}

/// A Julian Date carried as `hi + lo`, normalized so `hi == fl(hi + lo)` and
/// `lo` holds the residual. The represented value is exactly `hi + lo` in
/// extended precision. The [`Precise`](super::Precise) tier's storage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoPartJd {
    hi: f64,
    lo: f64,
}

impl JdRepr for TwoPartJd {
    #[inline]
    fn from_jd(jd: f64) -> Self {
        Self { hi: jd, lo: 0.0 }
    }

    #[inline]
    fn from_parts(hi: f64, lo: f64) -> Self {
        let (s, e) = two_sum(hi, lo);
        Self { hi: s, lo: e }
    }

    #[inline]
    fn jd(self) -> f64 {
        self.hi + self.lo
    }

    #[inline]
    fn parts(self) -> (f64, f64) {
        (self.hi, self.lo)
    }

    #[inline]
    fn add_days(self, days: f64) -> Self {
        let (s, e) = two_sum(self.hi, days);
        <Self as JdRepr>::from_parts(s, e + self.lo)
    }

    /// `two_sum` on the high parts captures their subtraction's rounding error
    /// exactly (for any magnitudes), so no significance is lost even when the
    /// high parts are large and close — the J2000-cancellation case. The
    /// residual difference is then folded into that error term.
    #[inline]
    fn diff_days(self, other: Self) -> f64 {
        let (dh, eh) = two_sum(self.hi, -other.hi);
        // (hi diff) + (hi-diff rounding error) + (lo diff)
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
        // 1e-9 day ≈ 86 µs — a couple of ulp at this JD (ulp ≈ 40 µs), so it
        // does not fit in `hi` alone; `lo` retains it where single-f64 would not.
        let hi = 2_460_000.0;
        let lo = 1e-9;
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
        assert_eq!(
            naive, 0.0,
            "precondition: single-f64 floors a 1 ns step to 0"
        );
    }

    #[test]
    fn add_days_accumulates_without_drift() {
        // Add a micro-day (1e-6 day) a million times → exactly 1.0 day.
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

    #[test]
    fn coarse_f64_tier_collapses_residual() {
        // The coarse (f64) tier carries no residual: from_parts collapses to
        // hi+lo and parts() reports lo == 0. (Contrast from_parts_normalizes.)
        let hi = 2_460_000.0;
        let lo = 1e-9;
        let c = <f64 as JdRepr>::from_parts(hi, lo);
        assert_eq!(c, hi + lo);
        assert_eq!(<f64 as JdRepr>::parts(c), (hi + lo, 0.0));
    }
}
