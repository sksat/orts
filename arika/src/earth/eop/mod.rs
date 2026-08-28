//! Earth Orientation Parameters (EOP) provider traits.
//!
//! # 設計方針
//!
//! EOP provider を **単一の大きな trait** にすると、`NullEop` のような placeholder
//! を渡すコードが compile を通ってしまい、高精度 path が silent に no-op 相当に
//! degrade する危険がある。本モジュールでは代わりに、各 EOP パラメータを提供する
//! trait を個別に分けている:
//!
//! - [`Ut1Offset`] — UT1 − UTC (dUT1) の取得
//! - [`PolarMotion`] — 極運動 `x_pole`, `y_pole` の取得
//! - [`NutationCorrections`] — IAU 2000A 章動補正 dX/dY の取得
//! - [`LengthOfDay`] — LOD (Length of Day) の取得
//! - [`FullEopProvider`] — 上記 4 つを全て実装した型に対する便宜 trait
//!   (auto-blanket impl で自動付与される)
//!
//! 高精度 rotation API ([`Rotation<Gcrs, Cirs>::iau2006`](crate::frame::Rotation::iau2006)
//! など) は必要な trait bound で gate される。`NullEop` を渡すと compile error に
//! なるため、silent degradation は起こらない。
//!
//! # `NullEop`
//!
//! [`NullEop`] は **EOP 系 trait を一つも実装しない** placeholder 型。これを受け付ける
//! のは provider-free な API (例: `Epoch<Utc>::to_ut1_naive`、`Epoch<Tai>::to_tt`) のみ
//! で、EOP trait bound を要求する全ての API では compile error を誘発する。
//!
//! # 有限区間の provider と out-of-range policy
//!
//! 上記 4 trait は infallible (`-> f64`) である。これは高精度 API を trait bound で
//! gate するための設計だが、その代償として **有限区間しか持たないデータ源は
//! これらの trait を直接実装できない** ことになる。[`EopTable`] は finals2000A の
//! 覆う MJD 区間しか答えを持たないので、trait を実装せず、fallible な
//! `*_checked` accessor と、policy を明示した adapter
//! ([`EopTable::clamped`]) だけを提供する。範囲外 epoch は
//! `EopLookupError::OutOfRange` として型に現れ、runtime panic にはならない。
//!
//! # Leap second は別体系
//!
//! Leap second table は arika 内の compiled-in データ
//! ([`crate::epoch`] の `LEAP_SECONDS`) であり、EOP provider 経由では取得しない。
//! 更新 cadence (leap second = 6 ヶ月ごとの IERS Bulletin C、EOP = ほぼ毎日の
//! IERS Bulletin A/B) も意味論も異なるため、完全に別扱い。
//!
//! ただし dUT1 の **補間** は leap second table を必要とする: dUT1 は leap second を
//! 跨ぐと 1 s 跳ぶ不連続量で、連続量は `UT1 − TAI = dUT1 − (TAI − UTC)` の方である。
//! [`EopTable::dut1_checked`] はこの連続量を補間する。

// EOP parameter traits

/// Provides the UT1 − UTC (dUT1) offset.
///
/// `dut1` は通常 `±0.9 s` の範囲内の値で、UTC leap second 追加によってこの範囲
/// に保たれる。時刻系的には UT1 を UTC から導出するために必要:
///
/// ```text
/// UT1 = UTC + dUT1
/// ```
///
/// IERS Bulletin A/B (更新頻度: ほぼ毎日) から取得するのが一般的。
pub trait Ut1Offset {
    /// Return UT1 − UTC [seconds] at the given UTC MJD.
    fn dut1(&self, utc_mjd: f64) -> f64;
}

/// Provides the polar motion components `x_pole`, `y_pole`.
///
/// 極運動は Earth の瞬間的な rotation 軸が CIP (Celestial Intermediate Pole) から
/// どれだけずれているかを表すパラメータで、通常は < 0.5 arcsec の範囲にある。
/// [`Rotation<Tirs, Itrs>::polar_motion`](crate::frame::Rotation::polar_motion) で使用する。
pub trait PolarMotion {
    /// Return the x component of the polar motion [arcsec] at the given UTC MJD.
    fn x_pole(&self, utc_mjd: f64) -> f64;
    /// Return the y component of the polar motion [arcsec] at the given UTC MJD.
    fn y_pole(&self, utc_mjd: f64) -> f64;
}

/// Provides the IAU 2000A nutation corrections `dX`, `dY`.
///
/// IAU 2006 precession + IAU 2000A/B nutation model では、観測値と理論値の
/// 残差を IERS が観測から求めて publish する。この補正を適用することで
/// arcsec 級の高精度 GCRS ↔ CIRS 変換が可能になる。
///
/// 単位は milliarcsec (mas)。
pub trait NutationCorrections {
    /// Return the dX nutation correction [mas] at the given UTC MJD.
    fn dx(&self, utc_mjd: f64) -> f64;
    /// Return the dY nutation correction [mas] at the given UTC MJD.
    fn dy(&self, utc_mjd: f64) -> f64;
}

/// Provides the Length of Day (LOD) parameter.
///
/// LOD は 1 UTC day の長さと 86400 SI seconds の差を秒単位で表したもの
/// (通常 ~1 ms 程度)。Earth の自転速度変動を表し、速度変換 (velocity
/// transformation between inertial and rotating frames) に使われる。
///
/// 現状の position-only な rotation chain では LOD は不要 (どの constructor も
/// 要求しない)。velocity transformation を追加する際の bound として用意してある。
pub trait LengthOfDay {
    /// Return the LOD [seconds] at the given UTC MJD.
    fn lod(&self, utc_mjd: f64) -> f64;
}

/// Convenience alias for a provider that supplies every EOP parameter.
///
/// Implemented automatically via an auto-blanket impl for any type that
/// implements [`Ut1Offset`] + [`PolarMotion`] + [`NutationCorrections`] +
/// [`LengthOfDay`]. position-only な chain は LOD を含まない個別 bound を使うため、
/// この alias は LOD を要する velocity transform 用の便宜 bound。
pub trait FullEopProvider: Ut1Offset + PolarMotion + NutationCorrections + LengthOfDay {}

impl<T> FullEopProvider for T where T: Ut1Offset + PolarMotion + NutationCorrections + LengthOfDay {}

/// Convenience bound for the position-level (no-velocity) rotation chain.
///
/// Object-safe combination of the three parameter traits required by
/// [`Rotation::<Gcrs, Itrs>::iau2006_full_from_utc`](crate::frame::Rotation).
/// Unlike [`FullEopProvider`] it omits [`LengthOfDay`], which is only needed for
/// velocity transformation. Thread-safety is intentionally *not* required here;
/// callers that need to store a provider use the `Send + Sync` boxed
/// [`GcrsEopStorage`].
pub trait PositionEop: Ut1Offset + PolarMotion + NutationCorrections {}

impl<T> PositionEop for T where T: Ut1Offset + PolarMotion + NutationCorrections {}

/// Owned, type-erased [`PositionEop`] provider for the `Gcrs` precise path.
///
/// Wraps a boxed provider and delegates the EOP trait methods, so it can be
/// stored inside a force model and passed directly to arika's rotation
/// constructors (which take `P: Ut1Offset + NutationCorrections + PolarMotion`).
/// `Send + Sync` is carried by the boxed trait object rather than by
/// [`PositionEop`], keeping that trait a pure capability.
#[cfg(feature = "alloc")]
pub struct GcrsEopStorage(alloc::boxed::Box<dyn PositionEop + Send + Sync>);

#[cfg(feature = "alloc")]
impl GcrsEopStorage {
    /// Create from any thread-safe [`PositionEop`] provider.
    pub fn new(provider: impl PositionEop + Send + Sync + 'static) -> Self {
        Self(alloc::boxed::Box::new(provider))
    }
}

#[cfg(feature = "alloc")]
impl core::fmt::Debug for GcrsEopStorage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GcrsEopStorage").finish_non_exhaustive()
    }
}

#[cfg(feature = "alloc")]
impl Ut1Offset for GcrsEopStorage {
    fn dut1(&self, utc_mjd: f64) -> f64 {
        self.0.dut1(utc_mjd)
    }
}

#[cfg(feature = "alloc")]
impl PolarMotion for GcrsEopStorage {
    fn x_pole(&self, utc_mjd: f64) -> f64 {
        self.0.x_pole(utc_mjd)
    }
    fn y_pole(&self, utc_mjd: f64) -> f64 {
        self.0.y_pole(utc_mjd)
    }
}

#[cfg(feature = "alloc")]
impl NutationCorrections for GcrsEopStorage {
    fn dx(&self, utc_mjd: f64) -> f64 {
        self.0.dx(utc_mjd)
    }
    fn dy(&self, utc_mjd: f64) -> f64 {
        self.0.dy(utc_mjd)
    }
}

// NullEop

/// EOP placeholder that implements none of the EOP parameter traits.
///
/// これを受け付けるのは provider-free な API (`Epoch<Utc>::to_ut1_naive` など) のみで、
/// EOP trait bound を要求する全ての API では **compile error** になる。例えば
/// `Epoch<Utc>::to_ut1<P: Ut1Offset>` や
/// [`Rotation<Gcrs, Cirs>::iau2006`](crate::frame::Rotation::iau2006) に `NullEop` を渡すと
/// 型エラーになる。
///
/// # 存在意義
///
/// `NullEop` が直接使われる場面は現状ではほぼない。**意図的に EOP trait を一つも
/// 実装せず、高精度 API が silent に no-op 相当に degrade することを型レベルで
/// 防ぐ compile-error 誘発装置** として存在する。trybuild compile-fail test で
/// この性質を pin している (`arika/tests/trybuild/` 参照)。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullEop;

// Intentionally NO `impl Ut1Offset for NullEop` etc.
// The absence of these impls IS the feature.

// Data loading (requires alloc)

pub mod entry;
#[cfg(feature = "alloc")]
pub mod error;
#[cfg(feature = "alloc")]
pub mod finals2000a;
#[cfg(feature = "alloc")]
pub mod table;

pub use entry::EopEntry;
#[cfg(feature = "alloc")]
pub use error::{EopLookupError, EopParseError};
#[cfg(feature = "alloc")]
pub use finals2000a::Finals2000A;
#[cfg(feature = "alloc")]
pub use table::{ClampedEop, EopTable};

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal mock provider that implements every EOP parameter trait.
    /// Used to verify that (a) user code can define a provider, and (b) the
    /// `FullEopProvider` auto-blanket impl fires correctly.
    struct MockEop {
        dut1: f64,
        xp: f64,
        yp: f64,
        dx: f64,
        dy: f64,
        lod: f64,
    }

    impl Ut1Offset for MockEop {
        fn dut1(&self, _utc_mjd: f64) -> f64 {
            self.dut1
        }
    }

    impl PolarMotion for MockEop {
        fn x_pole(&self, _utc_mjd: f64) -> f64 {
            self.xp
        }
        fn y_pole(&self, _utc_mjd: f64) -> f64 {
            self.yp
        }
    }

    impl NutationCorrections for MockEop {
        fn dx(&self, _utc_mjd: f64) -> f64 {
            self.dx
        }
        fn dy(&self, _utc_mjd: f64) -> f64 {
            self.dy
        }
    }

    impl LengthOfDay for MockEop {
        fn lod(&self, _utc_mjd: f64) -> f64 {
            self.lod
        }
    }

    fn mock() -> MockEop {
        MockEop {
            dut1: -0.123,
            xp: 0.05,
            yp: 0.38,
            dx: 0.12,
            dy: 0.34,
            lod: 0.0015,
        }
    }

    #[test]
    fn mock_implements_all_eop_traits() {
        let m = mock();
        assert_eq!(<MockEop as Ut1Offset>::dut1(&m, 60000.0), -0.123);
        assert_eq!(<MockEop as PolarMotion>::x_pole(&m, 60000.0), 0.05);
        assert_eq!(<MockEop as PolarMotion>::y_pole(&m, 60000.0), 0.38);
        assert_eq!(<MockEop as NutationCorrections>::dx(&m, 60000.0), 0.12);
        assert_eq!(<MockEop as NutationCorrections>::dy(&m, 60000.0), 0.34);
        assert_eq!(<MockEop as LengthOfDay>::lod(&m, 60000.0), 0.0015);
    }

    #[test]
    fn full_eop_provider_blanket_impl_fires_for_mock() {
        // Generic function requiring the combined trait should accept MockEop.
        fn expects_full<P: FullEopProvider>(p: &P) -> f64 {
            p.dut1(60000.0) + p.x_pole(60000.0) + p.dy(60000.0) + p.lod(60000.0)
        }
        let m = mock();
        let sum = expects_full(&m);
        assert!((sum - (-0.123 + 0.05 + 0.34 + 0.0015)).abs() < 1e-12);
    }

    #[test]
    fn partial_provider_only_satisfies_its_subset() {
        // A provider that only implements Ut1Offset must still work with
        // `P: Ut1Offset`-bounded functions.
        struct Dut1Only;
        impl Ut1Offset for Dut1Only {
            fn dut1(&self, _: f64) -> f64 {
                -0.5
            }
        }
        fn expects_ut1<P: Ut1Offset>(p: &P) -> f64 {
            p.dut1(0.0)
        }
        assert_eq!(expects_ut1(&Dut1Only), -0.5);
    }

    #[test]
    fn null_eop_constructs() {
        // NullEop is a ZST; we can create it at will. This test mainly exists
        // to keep `NullEop` in the public API surface. Trybuild compile-fail
        // tests (arika/tests/trybuild/) pin the fact that NullEop does NOT
        // implement any of the EOP parameter traits.
        let _n = NullEop;
        let _n2 = NullEop::default();
    }
}
