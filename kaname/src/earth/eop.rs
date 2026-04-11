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
//! 高精度 rotation API (Phase 3 で追加予定の `Rotation<Gcrs, Cirs>::iau2006` など)
//! は必要な trait bound で gate される。`NullEop` を渡すと compile error になるため、
//! silent degradation は起こらない。
//!
//! # `NullEop`
//!
//! [`NullEop`] は **EOP 系 trait を一つも実装しない** placeholder 型。これを受け付ける
//! のは provider-free な API (例: `Epoch<Utc>::to_ut1_naive`、`Epoch<Tai>::to_tt`) のみ
//! で、EOP trait bound を要求する全ての API では compile error を誘発する。
//!
//! # Phase 2 の範囲
//!
//! 本 Phase では:
//! - EOP trait 4 種 + `FullEopProvider` + `NullEop` を追加
//! - [`Epoch::<Utc>::to_ut1<P: Ut1Offset>`](crate::epoch::Epoch::to_ut1) を追加
//! - trybuild compile-fail test で `NullEop` が EOP trait を実装しないことを pin
//!
//! Phase 3 で rotation constructor (`iau2006`, `from_era`, `polar_motion`,
//! `iau2006_full`) が追加された段階で、これらの trait bound を要求する。
//!
//! # Leap second は別体系
//!
//! Leap second table は kaname 内の compiled-in データ
//! ([`crate::epoch`] の `LEAP_SECONDS`) であり、EOP provider 経由では取得しない。
//! 更新 cadence (leap second = 6 ヶ月ごとの IERS Bulletin C、EOP = ほぼ毎日の
//! IERS Bulletin A/B) も意味論も異なるため、完全に別扱い。

// ─── EOP parameter traits ────────────────────────────────────────

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
/// `Rotation<Tirs, Itrs>::polar_motion(utc, eop)` (Phase 3) で使用する。
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
/// Phase 3 の position-only rotation では LOD は不要だが、将来 velocity
/// transformation を追加する際にこの trait が役に立つ。
pub trait LengthOfDay {
    /// Return the LOD [seconds] at the given UTC MJD.
    fn lod(&self, utc_mjd: f64) -> f64;
}

/// Convenience alias for a provider that supplies every EOP parameter.
///
/// Implemented automatically via an auto-blanket impl for any type that
/// implements [`Ut1Offset`] + [`PolarMotion`] + [`NutationCorrections`] +
/// [`LengthOfDay`]. Phase 3 で `Rotation<Gcrs, Itrs>::iau2006_full<P: FullEopProvider>`
/// のような完全 chain の bound として使う想定。
pub trait FullEopProvider: Ut1Offset + PolarMotion + NutationCorrections + LengthOfDay {}

impl<T> FullEopProvider for T where T: Ut1Offset + PolarMotion + NutationCorrections + LengthOfDay {}

// ─── NullEop ─────────────────────────────────────────────────────

/// EOP placeholder that implements none of the EOP parameter traits.
///
/// これを受け付けるのは provider-free な API (`Epoch<Utc>::to_ut1_naive` など) のみで、
/// EOP trait bound を要求する全ての API では **compile error** になる。例えば
/// `Epoch<Utc>::to_ut1<P: Ut1Offset>` や Phase 3 で追加予定の
/// `Rotation<Gcrs, Cirs>::iau2006` に `NullEop` を渡すと型エラーになる。
///
/// # 存在意義
///
/// `NullEop` が直接使われる場面は現状ではほぼない。**意図的に EOP trait を一つも
/// 実装せず、高精度 API が silent に no-op 相当に degrade することを型レベルで
/// 防ぐ compile-error 誘発装置** として存在する。trybuild compile-fail test で
/// この性質を pin している (`kaname/tests/trybuild/` 参照)。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullEop;

// Intentionally NO `impl Ut1Offset for NullEop` etc.
// The absence of these impls IS the feature.

// ─── Tests ───────────────────────────────────────────────────────

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
        // tests (kaname/tests/trybuild/) pin the fact that NullEop does NOT
        // implement any of the EOP parameter traits.
        let _n = NullEop;
        let _n2 = NullEop::default();
    }
}
