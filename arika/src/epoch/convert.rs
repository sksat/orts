//! Scale conversions between `Epoch<S>` variants and scale-specific JD
//! constructors.
//!
//! # Conversion model: explicit one-hop edges
//!
//! Conversions are modelled as the **edges of the scale graph**, each a single
//! `to_*()` method between physically adjacent scales:
//!
//! ```text
//! UTC ──leap──→ TAI ──+32.184s──→ TT ──Fairhead──→ TDB
//!                 └────−19s────→ GPS
//! ```
//!
//! There is no hidden TAI pivot abstraction — each edge computes its own one
//! step directly. The few multi-step conversions that span several edges
//! (`Utc::to_tt`, `Utc::to_tdb`, `Utc::to_gps`, `Gps::to_utc`) are **thin
//! convenience methods that compose the edges explicitly** (e.g. `to_tdb` is
//! literally `self.to_tai().to_tt().to_tdb()`), not a generic conversion
//! engine.
//!
//! `UT1` is reachable only through a `dUT1` provider
//! ([`Epoch::<Utc>::to_ut1`]); there is no `Epoch<Ut1>::to_tai()`, which keeps
//! the Earth-rotation scale isolated from the atomic/dynamical edges.

use core::f64::consts::TAU;

#[allow(unused_imports)]
use crate::math::F64Ext;

use super::TimeScale;
use super::leap::tai_minus_utc_at_mjd;
use super::{Epoch, Gps, Tai, Tdb, Tt, Ut1, Utc};
use super::{GPS_MINUS_TAI_SEC, J2000_JD, JULIAN_CENTURY, MJD_OFFSET, TT_MINUS_TAI_SEC};

/// Scales whose offset from TAI is a fixed number of SI seconds:
///
/// ```text
/// JD_scale = JD_TAI + SECONDS_AFTER_TAI / 86400
/// ```
///
/// Holds for TAI (`0`), TT (`+32.184`), and GPS (`−19`). UTC (leap-second
/// piecewise), TDB (periodic series), and UT1 (EOP-dependent) deliberately do
/// **not** implement this — their offset from TAI is not constant. The constant
/// is the single source of truth for the corresponding edge methods.
pub trait FixedOffsetFromTai: TimeScale {
    /// `scale − TAI` in SI seconds.
    const SECONDS_AFTER_TAI: f64;
}

impl FixedOffsetFromTai for Tai {
    const SECONDS_AFTER_TAI: f64 = 0.0;
}
impl FixedOffsetFromTai for Tt {
    const SECONDS_AFTER_TAI: f64 = TT_MINUS_TAI_SEC;
}
impl FixedOffsetFromTai for Gps {
    const SECONDS_AFTER_TAI: f64 = GPS_MINUS_TAI_SEC;
}

// UTC edges

impl Epoch<Utc> {
    /// Convert to TAI by applying the current leap-second offset (one hop).
    pub fn to_tai(&self) -> Epoch<Tai> {
        let utc_mjd = self.jd() - MJD_OFFSET;
        let leap = tai_minus_utc_at_mjd(utc_mjd);
        Epoch::<Tai>::from_jd_raw(self.jd() + leap / 86400.0)
    }

    /// Convert to TT. Convenience for the two-edge path `UTC → TAI → TT`.
    pub fn to_tt(&self) -> Epoch<Tt> {
        self.to_tai().to_tt()
    }

    /// Convert to TDB. Convenience for the path `UTC → TAI → TT → TDB`.
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        self.to_tai().to_tt().to_tdb()
    }

    /// Convert to GPS Time. Convenience for the path `UTC → TAI → GPS`.
    /// `GPS − UTC` equals the current leap count minus 19 s (18 s since 2017).
    pub fn to_gps(&self) -> Epoch<Gps> {
        self.to_tai().to_gps()
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

// TAI edges

impl Epoch<Tai> {
    /// Create a TAI epoch from a Julian Date value interpreted as TAI JD.
    pub fn from_jd_tai(jd: f64) -> Self {
        Epoch::<Tai>::from_jd_raw(jd)
    }

    /// Convert to UTC by subtracting the current leap-second offset (one hop).
    ///
    /// Iterates to land on the correct leap count at the resulting UTC instant.
    pub fn to_utc(&self) -> Epoch<Utc> {
        let mut guess_utc_jd = self.jd() - 37.0 / 86400.0; // initial guess
        for _ in 0..3 {
            let guess_mjd = guess_utc_jd - MJD_OFFSET;
            let leap = tai_minus_utc_at_mjd(guess_mjd);
            guess_utc_jd = self.jd() - leap / 86400.0;
        }
        Epoch::<Utc>::from_jd_raw(guess_utc_jd)
    }

    /// Convert to TT by adding the constant 32.184 s offset (one hop).
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_jd_raw(
            self.jd() + <Tt as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0,
        )
    }

    /// Convert to GPS Time by subtracting the constant 19 s offset (one hop).
    pub fn to_gps(&self) -> Epoch<Gps> {
        Epoch::<Gps>::from_jd_raw(
            self.jd() + <Gps as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0,
        )
    }
}

// GPS edges

impl Epoch<Gps> {
    /// Create a GPS epoch from a Julian Date value interpreted as GPS JD.
    pub fn from_jd_gps(jd: f64) -> Self {
        Epoch::<Gps>::from_jd_raw(jd)
    }

    /// Convert to TAI by adding the constant 19 s offset (one hop).
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_jd_raw(
            self.jd() - <Gps as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0,
        )
    }

    /// Convert to UTC. Convenience for the path `GPS → TAI → UTC`.
    pub fn to_utc(&self) -> Epoch<Utc> {
        self.to_tai().to_utc()
    }
}

// TT edges

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

    /// Convert to TAI by subtracting the constant 32.184 s offset (one hop).
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_jd_raw(
            self.jd() - <Tt as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0,
        )
    }

    /// Convert to TDB via the Fairhead-Bretagnon periodic correction (one hop).
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        let delta = tdb_minus_tt(self.jd());
        Epoch::<Tdb>::from_jd_raw(self.jd() + delta / 86400.0)
    }
}

// TDB edges

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

    /// Convert to TT by applying the inverse Fairhead-Bretagnon correction
    /// (one hop). Since `|TDB − TT| < 2 ms`, a single-step inversion suffices.
    pub fn to_tt(&self) -> Epoch<Tt> {
        let delta = tdb_minus_tt(self.jd());
        Epoch::<Tt>::from_jd_raw(self.jd() - delta / 86400.0)
    }
}

// UT1 API

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
