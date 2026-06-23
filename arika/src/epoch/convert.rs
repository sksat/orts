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
//! ([`Epoch::<Utc>::to_ut1`]) and lives in its own [`Ut1Epoch`] type — UT1 is
//! not part of the `Epoch<S>` family at all, which keeps
//! the Earth-rotation scale isolated from the atomic/dynamical edges.

use core::f64::consts::TAU;

#[allow(unused_imports)]
use crate::math::F64Ext;

use super::TimeScale;
use super::jd2::TwoPartJd;
use super::leap::tai_minus_utc_at_mjd;
use super::{Epoch, Gps, Tai, Tdb, Tt, Utc};
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

// Canonical-TAI lens
//
// Every scale in the `Epoch<S>` family converts to/from canonical TAI without
// external data. `TaiLens` is that conversion as a pair of pure functions over
// the two-part [`TwoPartJd`] representation: the math (leap table, fixed
// offset, Fairhead periodic) that the canonical-TAI `Epoch` will apply once at
// construction (scale JD → TAI) and once at read-out (TAI → scale JD), keeping
// full precision in between. (UT1 is not in the family — it needs EOP — so it
// has no lens; see `Ut1Epoch`.)

/// Data-free lens between canonical TAI and a scale's own Julian Date.
/// Crate-internal: the canonical-TAI `Epoch<S>` accessors call it per scale.
pub(crate) trait TaiLens: TimeScale {
    /// This scale's JD → the canonical TAI JD.
    fn tai_from_scale(scale_jd: TwoPartJd) -> TwoPartJd;
    /// Canonical TAI JD → this scale's JD.
    fn scale_from_tai(tai: TwoPartJd) -> TwoPartJd;
}

/// Derive [`TaiLens`] for a [`FixedOffsetFromTai`] scale: `scale = TAI + off`.
macro_rules! impl_tai_lens_fixed {
    ($scale:ty) => {
        impl TaiLens for $scale {
            fn tai_from_scale(scale_jd: TwoPartJd) -> TwoPartJd {
                scale_jd.add_days(-<$scale as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0)
            }
            fn scale_from_tai(tai: TwoPartJd) -> TwoPartJd {
                tai.add_days(<$scale as FixedOffsetFromTai>::SECONDS_AFTER_TAI / 86400.0)
            }
        }
    };
}
impl_tai_lens_fixed!(Tai);
impl_tai_lens_fixed!(Tt);
impl_tai_lens_fixed!(Gps);

impl TaiLens for Utc {
    fn tai_from_scale(utc: TwoPartJd) -> TwoPartJd {
        let leap = tai_minus_utc_at_mjd(utc.jd() - MJD_OFFSET);
        utc.add_days(leap / 86400.0)
    }
    fn scale_from_tai(tai: TwoPartJd) -> TwoPartJd {
        // Seed the leap-count search with the current maximum TAI − UTC; any
        // seed within one leap step of the true offset converges in 3 iters.
        const LEAP_SEED_SEC: f64 = 37.0; // TAI − UTC since 2017-01-01
        let mut utc = tai.add_days(-LEAP_SEED_SEC / 86400.0);
        for _ in 0..3 {
            let leap = tai_minus_utc_at_mjd(utc.jd() - MJD_OFFSET);
            utc = tai.add_days(-leap / 86400.0);
        }
        utc
    }
}

impl TaiLens for Tdb {
    fn tai_from_scale(tdb: TwoPartJd) -> TwoPartJd {
        // TDB → TT (single-step inversion, |TDB − TT| < 2 ms) → TAI.
        let tt = tdb.add_days(-tdb_minus_tt(tdb.jd()) / 86400.0);
        tt.add_days(-TT_MINUS_TAI_SEC / 86400.0)
    }
    fn scale_from_tai(tai: TwoPartJd) -> TwoPartJd {
        // TAI → TT → TDB (Fairhead-Bretagnon periodic).
        let tt = tai.add_days(TT_MINUS_TAI_SEC / 86400.0);
        tt.add_days(tdb_minus_tt(tt.jd()) / 86400.0)
    }
}

/// Construct an `Epoch<S>` from a JD interpreted in scale `S`, converting to
/// the stored canonical TAI instant via the lens.
pub(crate) fn from_scale_jd<S: TaiLens>(scale_jd: f64) -> Epoch<S> {
    Epoch::<S>::from_tai_raw(S::tai_from_scale(TwoPartJd::from_jd(scale_jd)))
}

// UTC outbound conversions (re-tags of the shared canonical TAI instant)

impl Epoch<Utc> {
    /// Convert to TAI (re-tag of the shared canonical instant).
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_tai_raw(self.tai_raw())
    }

    /// Convert to TT (re-tag of the shared canonical instant).
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_tai_raw(self.tai_raw())
    }

    /// Convert to TDB (re-tag of the shared canonical instant).
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        Epoch::<Tdb>::from_tai_raw(self.tai_raw())
    }

    /// Convert to GPS Time (re-tag of the shared canonical instant).
    /// `GPS − UTC` equals the current leap count minus 19 s (18 s since 2017).
    pub fn to_gps(&self) -> Epoch<Gps> {
        Epoch::<Gps>::from_tai_raw(self.tai_raw())
    }

    /// Convert to UT1 assuming UT1 ≈ UTC (naive, legacy behavior).
    ///
    /// 真の UT1 が必要な場合は [`Epoch::<Utc>::to_ut1`] (`Ut1Offset` provider を
    /// 引数に取る) を使う。本 method は `NullEop` 相当の `dUT1 = 0` 仮定で、
    /// current arika の `gmst()` 実装との bit-level 互換を保つため提供される。
    pub fn to_ut1_naive(&self) -> Ut1Epoch {
        Ut1Epoch::from_jd_ut1(self.jd())
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
    pub fn to_ut1<P: crate::earth::eop::Ut1Offset + ?Sized>(&self, eop: &P) -> Ut1Epoch {
        let mjd = self.jd() - MJD_OFFSET;
        let dut1 = eop.dut1(mjd);
        Ut1Epoch::from_jd_ut1(self.jd() + dut1 / 86400.0)
    }
}

// TAI edges

impl Epoch<Tai> {
    /// Create a TAI epoch from a Julian Date value interpreted as TAI JD.
    pub fn from_jd_tai(jd: f64) -> Self {
        from_scale_jd::<Tai>(jd)
    }

    /// Convert to UTC (re-tag of the shared canonical instant).
    pub fn to_utc(&self) -> Epoch<Utc> {
        Epoch::<Utc>::from_tai_raw(self.tai_raw())
    }

    /// Convert to TT (re-tag of the shared canonical instant).
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_tai_raw(self.tai_raw())
    }

    /// Convert to GPS Time (re-tag of the shared canonical instant).
    pub fn to_gps(&self) -> Epoch<Gps> {
        Epoch::<Gps>::from_tai_raw(self.tai_raw())
    }
}

// GPS edges

impl Epoch<Gps> {
    /// Create a GPS epoch from a Julian Date value interpreted as GPS JD.
    pub fn from_jd_gps(jd: f64) -> Self {
        from_scale_jd::<Gps>(jd)
    }

    /// Convert to TAI (re-tag of the shared canonical instant).
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_tai_raw(self.tai_raw())
    }

    /// Convert to UTC (re-tag of the shared canonical instant).
    pub fn to_utc(&self) -> Epoch<Utc> {
        Epoch::<Utc>::from_tai_raw(self.tai_raw())
    }
}

// TT edges

impl Epoch<Tt> {
    /// Create a TT epoch from a Julian Date value interpreted as TT JD.
    pub fn from_jd_tt(jd: f64) -> Self {
        from_scale_jd::<Tt>(jd)
    }

    /// Return TT Julian centuries since J2000.0.
    ///
    /// この値が IAU 2006 precession / IAU 2000A/B nutation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd() - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TAI (re-tag of the shared canonical instant).
    pub fn to_tai(&self) -> Epoch<Tai> {
        Epoch::<Tai>::from_tai_raw(self.tai_raw())
    }

    /// Convert to TDB (re-tag of the shared canonical instant).
    pub fn to_tdb(&self) -> Epoch<Tdb> {
        Epoch::<Tdb>::from_tai_raw(self.tai_raw())
    }
}

// TDB edges

impl Epoch<Tdb> {
    /// Create a TDB epoch from a Julian Date value interpreted as TDB JD.
    ///
    /// JPL DE ephemerides use `Teph` which is for practical purposes
    /// indistinguishable from TDB (IAU 2006 Resolution B3).
    pub fn from_jd_tdb(jd: f64) -> Self {
        from_scale_jd::<Tdb>(jd)
    }

    /// Return TDB Julian centuries since J2000.0.
    ///
    /// Meeus / JPL DE ephemeris と IAU 2009 WGCCRE body rotation の独立変数。
    pub fn centuries_since_j2000(&self) -> f64 {
        (self.jd() - J2000_JD) / JULIAN_CENTURY
    }

    /// Convert to TT (re-tag of the shared canonical instant).
    pub fn to_tt(&self) -> Epoch<Tt> {
        Epoch::<Tt>::from_tai_raw(self.tai_raw())
    }
}

// UT1 — isolated from the Epoch<S> family.
//
// UT1 − TAI (ΔUT1) is a *measured* Earth-orientation quantity (EOP), not a
// data-free offset like the other scales, so UT1 cannot live on the canonical
// timeline the `Epoch<S>` family shares. It is therefore its own type, reached
// only through a `dUT1` provider ([`Epoch::<Utc>::to_ut1`]).

/// An epoch on the UT1 (Earth-rotation) time scale, stored as a UT1 Julian Date.
///
/// Separate from [`Epoch`] because UT1 is realized by Earth's rotation, not
/// atomic clocks: reaching it requires a measured `dUT1` (see
/// [`Epoch::<Utc>::to_ut1`]). Carries the definitional Earth Rotation Angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ut1Epoch {
    jd: f64,
}

impl Ut1Epoch {
    /// Create a UT1 epoch from a Julian Date value interpreted as UT1 JD.
    pub fn from_jd_ut1(jd: f64) -> Self {
        Self { jd }
    }

    /// The UT1 Julian Date.
    pub fn jd(&self) -> f64 {
        self.jd
    }

    /// Earth Rotation Angle (ERA) in radians.
    ///
    /// IAU 2000 Resolution B1.8 / SOFA `iauEra00`:
    /// `ERA(T_u) = 2π × (0.7790572732640 + 1.00273781191135448 × T_u)`
    /// where `T_u = JD_UT1 − 2451545.0`.
    ///
    /// ERA は UT1 の definitional な関数。`Ut1Epoch` にのみ提供される。
    pub fn era(&self) -> f64 {
        era_formula(self.jd)
    }
}

/// Earth Rotation Angle (ERA) formula, shared by `Ut1Epoch::era` and the
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

#[cfg(test)]
mod lens_tests {
    //! The `TaiLens` math must agree with the existing one-hop edge methods
    //! (which it will replace once `Epoch<S>` stores canonical TAI), and must
    //! round-trip. Agreement is to f64 noise since the lens does the same
    //! arithmetic in two parts.
    use super::*;

    const TOL_DAY: f64 = 1e-9; // ~86 µs — well above f64 noise, below the regime we care about

    fn tpj(jd: f64) -> TwoPartJd {
        TwoPartJd::from_jd(jd)
    }

    #[test]
    fn lens_tai_from_scale_matches_edges() {
        let x = 2_460_390.5; // 2024-03-20-ish
        // scale-JD x interpreted in each scale → TAI must match the edge `to_tai` path.
        let utc_edge = Epoch::<Utc>::from_jd(x).to_tai().jd();
        assert!((Utc::tai_from_scale(tpj(x)).jd() - utc_edge).abs() < TOL_DAY);

        let tt_edge = Epoch::<Tt>::from_jd_tt(x).to_tai().jd();
        assert!((Tt::tai_from_scale(tpj(x)).jd() - tt_edge).abs() < TOL_DAY);

        let gps_edge = Epoch::<Gps>::from_jd_gps(x).to_tai().jd();
        assert!((Gps::tai_from_scale(tpj(x)).jd() - gps_edge).abs() < TOL_DAY);

        // TDB → TAI edge path is tdb.to_tt().to_tai().
        let tdb_edge = Epoch::<Tdb>::from_jd_tdb(x).to_tt().to_tai().jd();
        assert!((Tdb::tai_from_scale(tpj(x)).jd() - tdb_edge).abs() < TOL_DAY);
    }

    #[test]
    fn lens_scale_from_tai_matches_edges() {
        let tai = 2_460_390.500_8; // a TAI JD
        let utc_edge = Epoch::<Tai>::from_jd_tai(tai).to_utc().jd();
        assert!((Utc::scale_from_tai(tpj(tai)).jd() - utc_edge).abs() < TOL_DAY);

        let tt_edge = Epoch::<Tai>::from_jd_tai(tai).to_tt().jd();
        assert!((Tt::scale_from_tai(tpj(tai)).jd() - tt_edge).abs() < TOL_DAY);

        let gps_edge = Epoch::<Tai>::from_jd_tai(tai).to_gps().jd();
        assert!((Gps::scale_from_tai(tpj(tai)).jd() - gps_edge).abs() < TOL_DAY);
    }

    #[test]
    fn lens_roundtrips_each_scale() {
        let x = 2_460_390.123_456;
        macro_rules! rt {
            ($s:ty) => {{
                let back = <$s as TaiLens>::scale_from_tai(<$s as TaiLens>::tai_from_scale(tpj(x)));
                assert!(
                    (back.jd() - x).abs() < TOL_DAY,
                    "{} round-trip diverged",
                    <$s as TimeScale>::NAME
                );
            }};
        }
        rt!(Utc);
        rt!(Tai);
        rt!(Tt);
        rt!(Gps);
        rt!(Tdb);
    }
}
