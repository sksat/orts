//! Scale conversions between `Epoch<S>` variants and scale-specific JD
//! constructors.
//!
//! Conversions route through TAI so leap seconds and the Fairhead-Bretagnon
//! TDB correction are applied consistently: `UTC ↔ TAI ↔ TT ↔ TDB`, with
//! `UT1` reached from `UTC` via a `dUT1` offset.

use core::f64::consts::TAU;

#[allow(unused_imports)]
use crate::math::F64Ext;

use super::leap::tai_minus_utc_at_mjd;
use super::{Epoch, Tai, Tdb, Tt, Ut1, Utc};
use super::{J2000_JD, JULIAN_CENTURY, MJD_OFFSET, TT_MINUS_TAI_SEC};

// Epoch<Utc> conversions (outbound from UTC)

impl Epoch<Utc> {
    /// Convert to TAI by applying the current leap-second offset.
    pub fn to_tai(&self) -> Epoch<Tai> {
        let utc_mjd = self.jd() - MJD_OFFSET;
        let leap = tai_minus_utc_at_mjd(utc_mjd);
        Epoch::<Tai>::from_jd_raw(self.jd() + leap / 86400.0)
    }

    /// Convert to TT via UTC → TAI → TT.
    pub fn to_tt(&self) -> Epoch<Tt> {
        self.to_tai().to_tt()
    }

    /// Convert to TDB via UTC → TAI → TT → TDB (Fairhead-Bretagnon periodic).
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        self.to_tt().to_tdb()
    }

    /// Convert to UT1 assuming UT1 ≈ UTC (naive, legacy behavior).
    ///
    /// 真の UT1 が必要な場合は [`Epoch::<Utc>::to_ut1`] (`Ut1Offset` provider を
    /// 引数に取る) を使う。本 method は `NullEop` 相当の `dUT1 = 0` 仮定で、
    /// current arika の `gmst()` 実装との bit-level 互換を保つため提供される。
    pub fn to_ut1_naive(&self) -> Epoch<Ut1> {
        Epoch::<Ut1>::from_jd_raw(self.jd())
    }

    /// Convert to UT1 using the `dUT1 = UT1 − UTC` correction provided by
    /// an EOP provider.
    ///
    /// ```text
    /// JD_UT1 = JD_UTC + dUT1 / 86400
    /// ```
    ///
    /// `dUT1` is looked up at the current UTC MJD and is typically in the
    /// range `±0.9 s`. This is the **precise** UT1 conversion — the `NullEop`
    /// placeholder type does **not** implement
    /// [`Ut1Offset`](crate::earth::eop::Ut1Offset), so passing it is a compile
    /// error (see `arika/tests/trybuild/`).
    ///
    /// The `?Sized` bound lets callers pass trait objects directly
    /// (e.g. `&dyn Ut1Offset` or `Box<dyn Ut1Offset>::as_ref()`) alongside
    /// concrete types.
    ///
    /// For a naive `dUT1 = 0` conversion used by the legacy simple rotation
    /// path, use [`Epoch::<Utc>::to_ut1_naive`] instead.
    pub fn to_ut1<P: crate::earth::eop::Ut1Offset + ?Sized>(&self, eop: &P) -> Epoch<Ut1> {
        let mjd = self.jd() - MJD_OFFSET;
        let dut1 = eop.dut1(mjd);
        Epoch::<Ut1>::from_jd_raw(self.jd() + dut1 / 86400.0)
    }
}

// Epoch<Tai> API

impl Epoch<Tai> {
    /// Create a TAI epoch from a Julian Date value interpreted as TAI JD.
    pub fn from_jd_tai(jd: f64) -> Self {
        Epoch::<Tai>::from_jd_raw(jd)
    }

    /// Convert to TT by adding the constant 32.184 s offset.
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_jd_raw(self.jd() + TT_MINUS_TAI_SEC / 86400.0)
    }

    /// Convert to UTC by subtracting the current leap-second offset.
    pub fn to_utc(&self) -> Epoch<Utc> {
        // Iterate to find the right leap count (guess → refine).
        let mut guess_utc_jd = self.jd() - 37.0 / 86400.0; // initial guess
        for _ in 0..3 {
            let guess_mjd = guess_utc_jd - MJD_OFFSET;
            let leap = tai_minus_utc_at_mjd(guess_mjd);
            guess_utc_jd = self.jd() - leap / 86400.0;
        }
        Epoch::<Utc>::from_jd_raw(guess_utc_jd)
    }
}

// Epoch<Tt> API

impl Epoch<Tt> {
    /// Create a TT epoch from a Julian Date value interpreted as TT JD.
    pub fn from_jd_tt(jd: f64) -> Self {
        Epoch::<Tt>::from_jd_raw(jd)
    }

    /// Return TT Julian centuries since J2000.0.
    ///
    /// この値が IAU 2006 precession / IAU 2000A/B nutation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd() - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TAI by subtracting the constant 32.184 s offset.
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_jd_raw(self.jd() - TT_MINUS_TAI_SEC / 86400.0)
    }

    /// Convert to TDB via the Fairhead-Bretagnon periodic correction.
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        let delta = tdb_minus_tt(self.jd());
        Epoch::<Tdb>::from_jd_raw(self.jd() + delta / 86400.0)
    }
}

// Epoch<Tdb> API

impl Epoch<Tdb> {
    /// Create a TDB epoch from a Julian Date value interpreted as TDB JD.
    ///
    /// JPL DE ephemerides use `Teph` which is for practical purposes
    /// indistinguishable from TDB (IAU 2006 Resolution B3).
    pub fn from_jd_tdb(jd: f64) -> Self {
        Epoch::<Tdb>::from_jd_raw(jd)
    }

    /// Return TDB Julian centuries since J2000.0.
    ///
    /// Meeus / JPL DE ephemeris と IAU 2009 WGCCRE body rotation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd() - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TT by applying the inverse Fairhead-Bretagnon correction.
    pub fn to_tt(&self) -> Epoch<Tt> {
        // Since |TDB - TT| < 2 ms, a single-step inversion is accurate enough.
        let delta = tdb_minus_tt(self.jd());
        Epoch::<Tt>::from_jd_raw(self.jd() - delta / 86400.0)
    }
}

// Epoch<Ut1> API

impl Epoch<Ut1> {
    /// Create a UT1 epoch from a Julian Date value interpreted as UT1 JD.
    pub fn from_jd_ut1(jd: f64) -> Self {
        Epoch::<Ut1>::from_jd_raw(jd)
    }

    /// Earth Rotation Angle (ERA) in radians.
    ///
    /// IAU 2000 Resolution B1.8 / SOFA `iauEra00`:
    /// `ERA(T_u) = 2π × (0.7790572732640 + 1.00273781191135448 × T_u)`
    /// where `T_u = JD_UT1 − 2451545.0`.
    ///
    /// ERA は UT1 の definitional な関数であり、他の scale で計算することは
    /// 意味論的に間違い。したがって `era()` method は `Epoch<Ut1>` にのみ
    /// 提供される (`Epoch<Tdb>::era()` はコンパイルエラー)。
    pub fn era(&self) -> f64 {
        era_formula(self.jd())
    }
}

/// Earth Rotation Angle (ERA) formula, shared by `Epoch<Ut1>::era` and the
/// legacy `Epoch<Utc>::gmst` method.
///
/// Note: the current arika source value `1.002_737_811_911_354_6` differs
/// from the canonical SOFA value `1.00273781191135448` by roughly 1 f64 ULP
/// (~1e-16). Phase 1A keeps the legacy constant for bit-level invariance with
/// pre-refactor tests. The canonical value will be adopted in a later phase.
pub(super) fn era_formula(ut1_jd: f64) -> f64 {
    let du = ut1_jd - J2000_JD;
    let era = TAU * (0.7790572732640 + 1.002_737_811_911_354_6 * du);
    let era = era % TAU;
    if era < 0.0 { era + TAU } else { era }
}

/// TDB - TT [seconds] at the given TT Julian Date.
///
/// Uses a simplified 2-term Fairhead-Bretagnon series:
/// ```text
/// TDB - TT ≈ 0.001658 × sin(g) + 0.000014 × sin(2g)  [seconds]
/// g = 357.53° + 0.98560028° × (JD_TT - 2451545.0)    [Earth mean anomaly]
/// ```
/// Accurate to < 0.1 ms for typical epochs (sufficient for < arcsecond ephemeris).
fn tdb_minus_tt(tt_jd: f64) -> f64 {
    let d = tt_jd - J2000_JD;
    let g_deg = 357.53 + 0.985_600_28 * d;
    let g = g_deg.to_radians();
    0.001_658 * g.sin() + 0.000_014 * (2.0 * g).sin()
}
