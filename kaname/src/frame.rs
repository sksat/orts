//! Coordinate frame markers and frame-tagged types: `Vec3<F>` and `Rotation<From, To>`.
//!
//! `Vec3<F>` は座標系 `F` で表現される 3 次元ベクトル、`Rotation<From, To>` は
//! `From` → `To` への回転を表す。両者とも `F` は ZST な phantom 型なので、
//! メモリレイアウトは裸の `Vector3<f64>` / `UnitQuaternion<f64>` と同一。
//!
//! 座標系は sealed category trait でカテゴリ分けされており、structural math
//! (magnitude / dot / cross / 変換) は generic に書ける。一方で precision-aware な
//! 変換 (`Rotation<SimpleEci, SimpleEcef>` と `Rotation<Gcrs, Itrs>` など) は
//! concrete 型 API として個別に提供し、近似系と厳密系の silent 混同を防ぐ。
//!
//! # Frame marker
//!
//! - [`SimpleEci`] — 歳差・章動・極運動を無視した近似的な Earth-centered inertial。
//!   ERA-only Z 回転の親フレーム。Meeus ephemeris と可視化グレード計算の出発点
//! - [`SimpleEcef`] — [`SimpleEci`] からの ERA Z 回転先。簡易地球固定系。
//!   極運動や章動を一切適用しない近似的 Earth-fixed
//! - [`Gcrs`] — Geocentric Celestial Reference System (IAU 2006 CIO chain の
//!   celestial side)。Meeus ephemeris の返り型としても使う (strict な GCRS では
//!   なく "geocentric inertial as returned by low-precision analytic models" の
//!   意味。後続 Phase で precession/nutation 補正が加わると厳密な GCRS に近づく)
//! - [`Rsw`] — Radial / Along-track / Cross-track 軌道ローカル系。
//!   軸順は標準 RSW 規約 [R̂, Ŝ, Ŵ] (R̂=normalize(r), Ŵ=normalize(r×v), Ŝ=Ŵ×R̂)
//! - [`Body`] — 宇宙機機体座標系
//!
//! # Category trait
//!
//! - [`Eci`] — structural category for earth-centered inertial frames.
//!   実装者: `SimpleEci`, `Gcrs`
//! - [`Ecef`] — structural category for earth-fixed frames.
//!   実装者: `SimpleEcef`
//! - [`LocalOrbital`] — structural category for local orbital frames.
//!   実装者: `Rsw`
//!
//! category trait は precision-agnostic な generic math (`<F: Eci>` で受ける等)
//! を書くためのものであり、precision-aware な変換 API には concrete 型を使うこと。
//!
//! # 使い方
//!
//! ```
//! use kaname::frame::{Vec3, Rotation, Gcrs, Body};
//!
//! let b_gcrs = Vec3::<Gcrs>::new(1e-5, 2e-5, -3e-5);
//! let r_bg = Rotation::<Gcrs, Body>::from_raw(
//!     nalgebra::UnitQuaternion::identity(),
//! );
//! let b_body: Vec3<Body> = r_bg.transform(&b_gcrs);
//! ```

use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Neg, Sub};

use nalgebra::{UnitQuaternion, Vector3};
use serde::{Deserialize, Serialize};

use crate::epoch::{Epoch, Ut1, Utc};

// ─── Runtime frame descriptor ────────────────────────────────────

/// Category tag for runtime frame identification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrameCategory {
    /// Earth-centered inertial (SimpleEci, Gcrs, ...)
    Eci,
    /// Earth-centered Earth-fixed (SimpleEcef, ...)
    Ecef,
    /// Local orbital (Rsw, ...)
    LocalOrbital,
    /// Spacecraft body-fixed
    Body,
}

/// Concrete frame identifier for runtime identification and serialization.
///
/// Mirrors the compile-time `Frame` marker types. Used by RRD / log / CLI
/// boundaries where a f64 tuple needs to carry its frame interpretation at
/// runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrameDescriptor {
    SimpleEci,
    SimpleEcef,
    Gcrs,
    Rsw,
    Body,
}

impl FrameDescriptor {
    pub const fn name(self) -> &'static str {
        match self {
            FrameDescriptor::SimpleEci => "SimpleEci",
            FrameDescriptor::SimpleEcef => "SimpleEcef",
            FrameDescriptor::Gcrs => "Gcrs",
            FrameDescriptor::Rsw => "Rsw",
            FrameDescriptor::Body => "Body",
        }
    }

    pub const fn category(self) -> FrameCategory {
        match self {
            FrameDescriptor::SimpleEci | FrameDescriptor::Gcrs => FrameCategory::Eci,
            FrameDescriptor::SimpleEcef => FrameCategory::Ecef,
            FrameDescriptor::Rsw => FrameCategory::LocalOrbital,
            FrameDescriptor::Body => FrameCategory::Body,
        }
    }
}

// ─── Sealed trait + Frame / category traits ──────────────────────

mod sealed {
    pub trait Sealed {}
}

/// Top-level frame trait. Implemented by every concrete frame marker.
///
/// Provides `NAME` and `DESCRIPTOR` for runtime identification. Sealed: new
/// frames can only be added inside kaname. No `Copy` / `'static` bound —
/// marker structs derive them themselves.
pub trait Frame: sealed::Sealed {
    const NAME: &'static str;
    const DESCRIPTOR: FrameDescriptor;
}

/// Structural category for earth-centered inertial frames.
///
/// 実装者: [`SimpleEci`], [`Gcrs`]。近似系 (`SimpleEci`) と将来の厳密系
/// (`Gcrs`/`Cirs` 等) の両方を含む category。precision-aware な処理は concrete
/// 型を関数シグネチャに書き、`<F: Eci>` generic bound は precision-agnostic
/// な math (magnitude / dot / 等) のみに使う。
pub trait Eci: Frame {}

/// Structural category for earth-centered earth-fixed frames.
///
/// 実装者: [`SimpleEcef`]。将来 `Itrs`/`Tirs`/`Pef` が追加される。同上の注意。
pub trait Ecef: Frame {}

/// Structural category for local orbital frames.
///
/// 実装者: [`Rsw`]。将来 `Ntw`/`Vvlh`/`Perifocal` 等が追加される。
pub trait LocalOrbital: Frame {}

// ─── Concrete frame markers ──────────────────────────────────────

/// Approximate Earth-centered inertial frame: the "parent frame" for the
/// ERA-only Z rotation used by the simple path. Ignores precession, nutation,
/// polar motion, and frame bias.
///
/// Meeus ephemerides **return [`Gcrs`]** (the analytical "geocentric inertial"),
/// not `SimpleEci`. `SimpleEci` is specifically the complement of [`SimpleEcef`]
/// under the ERA-only rotation; there is no direct relationship between
/// `SimpleEci` and `Gcrs` other than both being Earth-centered inertial in the
/// broad `Eci` category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimpleEci;
impl sealed::Sealed for SimpleEci {}
impl Frame for SimpleEci {
    const NAME: &'static str = "SimpleEci";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::SimpleEci;
}
impl Eci for SimpleEci {}

/// Approximate Earth-centered Earth-fixed frame: the result of applying an
/// ERA-only Z rotation to [`SimpleEci`]. Does not apply polar motion, nutation,
/// or IERS precession. WGS-84 geodetic conversion is defined on this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimpleEcef;
impl sealed::Sealed for SimpleEcef {}
impl Frame for SimpleEcef {
    const NAME: &'static str = "SimpleEcef";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::SimpleEcef;
}
impl Ecef for SimpleEcef {}

/// Geocentric Celestial Reference System. IAU 2006 CIO chain의 celestial side。
///
/// 現 Phase では Meeus ephemeris (低精度 analytic model) の返り型として使用。
/// 厳密な IAU 2006/2000A の precession-nutation 補正は後続 Phase で追加される。
/// `Rotation<Gcrs, Itrs>::iau2006_full` など高精度 chain は Phase 3 で提供予定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gcrs;
impl sealed::Sealed for Gcrs {}
impl Frame for Gcrs {
    const NAME: &'static str = "Gcrs";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Gcrs;
}
impl Eci for Gcrs {}

/// Local orbital frame: Radial / Along-track / Cross-track.
///
/// 軸順は標準 RSW 規約:
/// - R̂ = `normalize(r)` — 地心から衛星方向
/// - Ŵ = `normalize(r × v)` — orbit normal
/// - Ŝ = `Ŵ × R̂` — tangential (円軌道順行なら +v̂ 方向)
///
/// 注意: これは LVLH (業界で変種多数) とは別物。円軌道時の +v̂ 方向が
/// LVLH の X 軸 (or +I 軸) に一致するものがあるが、軸順・符号の選択は
/// 実装によって異なる。kaname は標準 RSW [R̂, Ŝ, Ŵ] で固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rsw;
impl sealed::Sealed for Rsw {}
impl Frame for Rsw {
    const NAME: &'static str = "Rsw";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Rsw;
}
impl LocalOrbital for Rsw {}

/// Spacecraft body-fixed frame.
///
/// Does not implement [`Eci`], [`Ecef`], or [`LocalOrbital`] categories —
/// the body frame is its own thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body;
impl sealed::Sealed for Body {}
impl Frame for Body {
    const NAME: &'static str = "Body";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Body;
}

// ─── Phase 1C legacy bridge (remove in Phase 4) ──────────────────
//
// `Vec3<Gcrs>::force_cast_simple_eci` is a typed no-op that lets call
// sites mix Meeus `Vec3<Gcrs>` output with `Vec3<SimpleEci>` simulation
// state explicitly, instead of silently stripping the phantom tag via
// `.into_inner()`. It exists only because Phase 1B/1C introduces `Gcrs`
// as the return type of Meeus ephemerides while the propagator state
// remains `SimpleEci` — a real `Rotation<Gcrs, SimpleEci>` is
// deliberately NOT provided by the plan (see .claude/plans/
// delegated-chasing-floyd.md § 1 "no upgrade path from SimpleEci to Gcrs").
//
// At Meeus precision the two frames are numerically identical because
// the analytic models apply no precession/nutation/frame-bias, so
// relabeling the tag is semantically honest at the call site.
// Once real GCRS ephemerides (JPL DE430 / Horizons / IAU 2006) land,
// this bridge must be removed — Phase 4 will refactor the propagator
// to either integrate in `Gcrs` directly or to dispatch the force
// model through concrete `Vec3<Gcrs>` / `Vec3<SimpleEci>` overloads.
//
// The method is `#[deprecated]` so each new call site triggers a
// compiler warning, and the name is chosen so `rg force_cast_simple_eci`
// locates every remaining bridge at Phase 4 cleanup time.

impl Vec3<Gcrs> {
    /// Force-cast a `Vec3<Gcrs>` to `Vec3<SimpleEci>` without applying any
    /// frame rotation (a typed no-op). **Phase 1C legacy bridge, remove in
    /// Phase 4.**
    ///
    /// At the current precision (Meeus analytic ephemerides, ~arcminute)
    /// `Gcrs` and `SimpleEci` are numerically indistinguishable — neither
    /// applies precession, nutation, frame bias, or polar motion, so the
    /// raw f64 components agree bit-for-bit. Force-casting is the explicit
    /// way for force-model code to mix Meeus Sun / Moon positions with
    /// `SimpleEci` satellite state and make the "I'm treating these two
    /// frames as numerically equal at this call site" assertion grep-able.
    ///
    /// This is **not** `unsafe` in the Rust sense (no undefined behaviour,
    /// no invariants the compiler relies on). The caution comes from
    /// precision: once real GCRS ephemerides or IAU 2006 precession land,
    /// calling this function bypasses the proper `Rotation<Gcrs, …>`
    /// conversion chain and silently degrades the computation. Phase 4
    /// removes this method entirely.
    #[deprecated(note = "Phase 1C 限定の Gcrs→SimpleEci 型変換 bridge。\
                         Phase 4 で削除予定 (propagator を Gcrs で動かすか、\
                         force model に concrete 型 overload を導入した時点で消す)。")]
    pub fn force_cast_simple_eci(self) -> Vec3<SimpleEci> {
        Vec3::from_raw(self.into_inner())
    }
}

// ─── Vec3<F> ─────────────────────────────────────────────────────

/// Frame-tagged 3D vector.
///
/// `PhantomData<F>` はゼロサイズなのでメモリレイアウトは `Vector3<f64>` と同一。
/// 同一フレーム内の演算のみ許可され、異フレーム間の直接操作は compile error。
#[derive(Clone, Copy, PartialEq)]
pub struct Vec3<F>(Vector3<f64>, PhantomData<F>);

impl<F> Vec3<F> {
    /// 成分から構築。
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self(Vector3::new(x, y, z), PhantomData)
    }

    /// 生の `Vector3<f64>` から構築。
    pub fn from_raw(v: Vector3<f64>) -> Self {
        Self(v, PhantomData)
    }

    /// ゼロベクトル。
    pub fn zeros() -> Self {
        Self(Vector3::zeros(), PhantomData)
    }

    /// 内部の `Vector3<f64>` への参照。
    pub fn inner(&self) -> &Vector3<f64> {
        &self.0
    }

    /// 内部の `Vector3<f64>` を消費して返す。
    pub fn into_inner(self) -> Vector3<f64> {
        self.0
    }

    // ─── 成分アクセサ ────────────────────────────────────────

    pub fn x(&self) -> f64 {
        self.0.x
    }
    pub fn y(&self) -> f64 {
        self.0.y
    }
    pub fn z(&self) -> f64 {
        self.0.z
    }

    // ─── フレーム非依存演算 ──────────────────────────────────

    /// ベクトルの大きさ。
    pub fn magnitude(&self) -> f64 {
        self.0.magnitude()
    }

    /// 大きさの 2 乗。
    pub fn magnitude_squared(&self) -> f64 {
        self.0.magnitude_squared()
    }

    /// 正規化（単位ベクトル化）。
    pub fn normalize(&self) -> Self {
        Self(self.0.normalize(), PhantomData)
    }

    /// 内積。
    pub fn dot(&self, other: &Self) -> f64 {
        self.0.dot(&other.0)
    }

    /// 外積（同一フレーム内）。
    pub fn cross(&self, other: &Self) -> Self {
        Self(self.0.cross(&other.0), PhantomData)
    }

    /// 全成分が有限か。
    pub fn is_finite(&self) -> bool {
        self.0.iter().all(|x| x.is_finite())
    }
}

impl<F: Frame> Vec3<F> {
    /// Frame descriptor (runtime identification).
    pub const fn frame_descriptor() -> FrameDescriptor {
        F::DESCRIPTOR
    }
}

impl<F> std::fmt::Debug for Vec3<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vec3<{}>({}, {}, {})",
            std::any::type_name::<F>()
                .rsplit("::")
                .next()
                .unwrap_or("?"),
            self.0.x,
            self.0.y,
            self.0.z
        )
    }
}

// ─── 同一フレーム演算 ────────────────────────────────────────────

impl<F> Add for Vec3<F> {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0, PhantomData)
    }
}

impl<F> Sub for Vec3<F> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0, PhantomData)
    }
}

impl<F> Neg for Vec3<F> {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0, PhantomData)
    }
}

impl<F> Mul<f64> for Vec3<F> {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self(self.0 * rhs, PhantomData)
    }
}

impl<F> Mul<Vec3<F>> for f64 {
    type Output = Vec3<F>;
    fn mul(self, rhs: Vec3<F>) -> Vec3<F> {
        Vec3(self * rhs.0, PhantomData)
    }
}

impl<F> Div<f64> for Vec3<F> {
    type Output = Self;
    fn div(self, rhs: f64) -> Self {
        Self(self.0 / rhs, PhantomData)
    }
}

impl<F> std::ops::AddAssign for Vec3<F> {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl<F> std::ops::SubAssign for Vec3<F> {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

// ─── Rotation<From, To> ─────────────────────────────────────────

/// 座標系 `From` から `To` への回転。
///
/// Hamilton クォータニオンベース。`transform` でベクトルの
/// フレーム変換を型安全に行う。
#[derive(Clone, Copy, PartialEq)]
pub struct Rotation<From, To>(UnitQuaternion<f64>, PhantomData<(From, To)>);

impl<From, To> Rotation<From, To> {
    /// 生の `UnitQuaternion` から構築。
    pub fn from_raw(q: UnitQuaternion<f64>) -> Self {
        Self(q, PhantomData)
    }

    /// 内部の `UnitQuaternion` への参照。
    pub fn inner(&self) -> &UnitQuaternion<f64> {
        &self.0
    }

    /// 内部の `UnitQuaternion` を消費して返す。
    pub fn into_inner(self) -> UnitQuaternion<f64> {
        self.0
    }

    /// ベクトルを `From` フレームから `To` フレームに変換。
    pub fn transform(&self, v: &Vec3<From>) -> Vec3<To> {
        Vec3(self.0.transform_vector(&v.0), PhantomData)
    }

    /// 逆回転 (`To` → `From`)。
    pub fn inverse(&self) -> Rotation<To, From> {
        Rotation(self.0.inverse(), PhantomData)
    }

    /// 回転の合成: `self` (A→B) と `other` (B→C) → A→C。
    pub fn then<C>(&self, other: &Rotation<To, C>) -> Rotation<From, C> {
        Rotation(other.0 * self.0, PhantomData)
    }
}

// ─── Simple path (SimpleEci ↔ SimpleEcef) rotation constructors ─

impl Rotation<SimpleEci, SimpleEcef> {
    /// Construct from a UT1 epoch using the Earth Rotation Angle (ERA).
    ///
    /// `SimpleEcef = R_z(−ERA(UT1)) × SimpleEci`. Applies only the ERA Z
    /// rotation — no precession, nutation, or polar motion. For high-precision
    /// work use the IAU 2006 CIO chain (not yet implemented: Phase 3).
    pub fn from_ut1(epoch: &Epoch<Ut1>) -> Self {
        Self::from_era(epoch.era())
    }

    /// Legacy helper: construct from a UTC epoch assuming UT1 ≈ UTC.
    ///
    /// This ignores the dUT1 correction (< 0.9 s). Preserves bit-level
    /// compatibility with pre-redesign code that called `Epoch::gmst` on a
    /// UTC epoch.
    pub fn from_utc_assuming_ut1_eq_utc(epoch: &Epoch<Utc>) -> Self {
        Self::from_ut1(&epoch.to_ut1_naive())
    }

    /// Construct from a raw ERA (or GMST) angle [rad].
    ///
    /// Low-level entry point used by the from_ut1 / from_utc helpers, exposed
    /// for tests and for integration with WASM bindings that expose ERA as a
    /// f64 parameter.
    pub fn from_era(era: f64) -> Self {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        Self::from_raw(UnitQuaternion::from_axis_angle(&axis, -era))
    }
}

impl Rotation<SimpleEcef, SimpleEci> {
    /// Inverse of [`Rotation::<SimpleEci, SimpleEcef>::from_ut1`].
    pub fn from_ut1(epoch: &Epoch<Ut1>) -> Self {
        Self::from_era(epoch.era())
    }

    /// Inverse of [`Rotation::<SimpleEci, SimpleEcef>::from_utc_assuming_ut1_eq_utc`].
    pub fn from_utc_assuming_ut1_eq_utc(epoch: &Epoch<Utc>) -> Self {
        Self::from_ut1(&epoch.to_ut1_naive())
    }

    /// Construct from a raw ERA (or GMST) angle [rad].
    pub fn from_era(era: f64) -> Self {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        Self::from_raw(UnitQuaternion::from_axis_angle(&axis, era))
    }
}

impl<From, To> std::fmt::Debug for Rotation<From, To> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let from = std::any::type_name::<From>()
            .rsplit("::")
            .next()
            .unwrap_or("?");
        let to = std::any::type_name::<To>()
            .rsplit("::")
            .next()
            .unwrap_or("?");
        write!(f, "Rotation<{from}, {to}>({:?})", self.0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn vec3_basic_ops() {
        let a = Vec3::<Gcrs>::new(1.0, 2.0, 3.0);
        let b = Vec3::<Gcrs>::new(4.0, 5.0, 6.0);

        let sum = a + b;
        assert_eq!(sum.x(), 5.0);
        assert_eq!(sum.y(), 7.0);
        assert_eq!(sum.z(), 9.0);

        let diff = b - a;
        assert_eq!(diff.x(), 3.0);

        let neg = -a;
        assert_eq!(neg.x(), -1.0);

        let scaled = a * 2.0;
        assert_eq!(scaled.x(), 2.0);

        let scaled2 = 3.0 * a;
        assert_eq!(scaled2.x(), 3.0);

        let div = a / 2.0;
        assert_eq!(div.x(), 0.5);
    }

    #[test]
    fn vec3_magnitude_and_normalize() {
        let v = Vec3::<Body>::new(3.0, 4.0, 0.0);
        assert!((v.magnitude() - 5.0).abs() < 1e-15);
        assert!((v.magnitude_squared() - 25.0).abs() < 1e-15);

        let n = v.normalize();
        assert!((n.magnitude() - 1.0).abs() < 1e-15);
        assert!((n.x() - 0.6).abs() < 1e-15);
    }

    #[test]
    fn vec3_dot_and_cross() {
        let a = Vec3::<Gcrs>::new(1.0, 0.0, 0.0);
        let b = Vec3::<Gcrs>::new(0.0, 1.0, 0.0);

        assert!((a.dot(&b)).abs() < 1e-15);

        let c = a.cross(&b);
        assert!((c.z() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn vec3_add_assign() {
        let mut a = Vec3::<Gcrs>::new(1.0, 2.0, 3.0);
        a += Vec3::new(10.0, 20.0, 30.0);
        assert_eq!(a.x(), 11.0);
    }

    #[test]
    fn vec3_is_finite() {
        assert!(Vec3::<Gcrs>::new(1.0, 2.0, 3.0).is_finite());
        assert!(!Vec3::<Gcrs>::new(f64::NAN, 0.0, 0.0).is_finite());
        assert!(!Vec3::<Gcrs>::new(0.0, f64::INFINITY, 0.0).is_finite());
    }

    #[test]
    fn rotation_identity_is_noop() {
        let r = Rotation::<Gcrs, Body>::from_raw(UnitQuaternion::identity());
        let v = Vec3::<Gcrs>::new(1.0, 2.0, 3.0);
        let result = r.transform(&v);
        assert!((result.x() - 1.0).abs() < 1e-15);
        assert!((result.y() - 2.0).abs() < 1e-15);
        assert!((result.z() - 3.0).abs() < 1e-15);
    }

    #[test]
    fn rotation_90deg_about_z() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 2.0);
        let r = Rotation::<Gcrs, Body>::from_raw(q);

        let v = Vec3::<Gcrs>::new(1.0, 0.0, 0.0);
        let result = r.transform(&v);
        assert!((result.x()).abs() < 1e-15);
        assert!((result.y() - 1.0).abs() < 1e-15);
        assert!((result.z()).abs() < 1e-15);
    }

    #[test]
    fn rotation_inverse() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let r = Rotation::<Gcrs, Body>::from_raw(q);

        let v = Vec3::<Gcrs>::new(1.0, 0.0, 0.0);
        let body = r.transform(&v);
        let back = r.inverse().transform(&body);

        assert!((back.x() - 1.0).abs() < 1e-14);
        assert!((back.y()).abs() < 1e-14);
    }

    #[test]
    fn rotation_compose() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let r_ab =
            Rotation::<Gcrs, Body>::from_raw(UnitQuaternion::from_axis_angle(&axis, PI / 4.0));
        let r_bc =
            Rotation::<Body, Rsw>::from_raw(UnitQuaternion::from_axis_angle(&axis, PI / 4.0));

        let r_ac: Rotation<Gcrs, Rsw> = r_ab.then(&r_bc);

        // 45° + 45° = 90° about Z
        let v = Vec3::<Gcrs>::new(1.0, 0.0, 0.0);
        let result = r_ac.transform(&v);
        assert!((result.x()).abs() < 1e-14);
        assert!((result.y() - 1.0).abs() < 1e-14);
    }

    // ─── Frame descriptor / category ─────────────────────────────

    #[test]
    fn frame_descriptor_name() {
        assert_eq!(FrameDescriptor::SimpleEci.name(), "SimpleEci");
        assert_eq!(FrameDescriptor::SimpleEcef.name(), "SimpleEcef");
        assert_eq!(FrameDescriptor::Gcrs.name(), "Gcrs");
        assert_eq!(FrameDescriptor::Rsw.name(), "Rsw");
        assert_eq!(FrameDescriptor::Body.name(), "Body");
    }

    #[test]
    fn frame_descriptor_category() {
        assert_eq!(FrameDescriptor::SimpleEci.category(), FrameCategory::Eci);
        assert_eq!(FrameDescriptor::Gcrs.category(), FrameCategory::Eci);
        assert_eq!(FrameDescriptor::SimpleEcef.category(), FrameCategory::Ecef);
        assert_eq!(FrameDescriptor::Rsw.category(), FrameCategory::LocalOrbital);
        assert_eq!(FrameDescriptor::Body.category(), FrameCategory::Body);
    }

    #[test]
    fn frame_descriptor_via_trait() {
        assert_eq!(<SimpleEci as Frame>::DESCRIPTOR, FrameDescriptor::SimpleEci);
        assert_eq!(<Gcrs as Frame>::DESCRIPTOR, FrameDescriptor::Gcrs);
        assert_eq!(
            <SimpleEcef as Frame>::DESCRIPTOR,
            FrameDescriptor::SimpleEcef
        );
        assert_eq!(
            Vec3::<SimpleEci>::frame_descriptor(),
            FrameDescriptor::SimpleEci
        );
    }

    #[test]
    fn category_trait_bounds_gate_generic_api() {
        // Structural API using `F: Eci` bound should accept both SimpleEci
        // and Gcrs interchangeably — this is by design.
        fn magnitude_eci<F: Eci>(v: Vec3<F>) -> f64 {
            v.magnitude()
        }
        assert_eq!(magnitude_eci(Vec3::<SimpleEci>::new(3.0, 4.0, 0.0)), 5.0);
        assert_eq!(magnitude_eci(Vec3::<Gcrs>::new(0.0, 0.0, 7.0)), 7.0);
    }

    // ─── Rotation<SimpleEci, SimpleEcef> from_era tests ──────────

    #[test]
    fn from_era_zero_is_identity() {
        let r = Rotation::<SimpleEci, SimpleEcef>::from_era(0.0);
        let v = Vec3::<SimpleEci>::new(1.0, 2.0, 3.0);
        let result = r.transform(&v);
        assert!((result.x() - 1.0).abs() < 1e-14);
        assert!((result.y() - 2.0).abs() < 1e-14);
        assert!((result.z() - 3.0).abs() < 1e-14);
    }

    #[test]
    fn from_era_90deg() {
        let r = Rotation::<SimpleEci, SimpleEcef>::from_era(PI / 2.0);
        let v = Vec3::<SimpleEci>::new(1.0, 0.0, 0.0);
        let result = r.transform(&v);
        // ECEF = R_z(-ERA) × ECI: with ERA=90°, +X_ECI → −Y_ECEF
        assert!(result.x().abs() < 1e-14);
        assert!((result.y() + 1.0).abs() < 1e-14);
        assert!(result.z().abs() < 1e-14);
    }

    #[test]
    fn from_era_roundtrip() {
        let era = 1.234;
        let r_ei = Rotation::<SimpleEci, SimpleEcef>::from_era(era);
        let r_ie = Rotation::<SimpleEcef, SimpleEci>::from_era(era);

        let v = Vec3::<SimpleEci>::new(100.0, 200.0, 300.0);
        let ecef = r_ei.transform(&v);
        let back = r_ie.transform(&ecef);
        assert!((back.x() - v.x()).abs() < 1e-10);
        assert!((back.y() - v.y()).abs() < 1e-10);
        assert!((back.z() - v.z()).abs() < 1e-10);
    }

    #[test]
    fn from_ut1_matches_from_era() {
        use crate::epoch::Epoch;
        let ut1 = Epoch::<Ut1>::from_jd_ut1(2460390.5);
        let era = ut1.era();
        let r_direct = Rotation::<SimpleEci, SimpleEcef>::from_era(era);
        let r_via_ut1 = Rotation::<SimpleEci, SimpleEcef>::from_ut1(&ut1);
        // Both should produce the same quaternion.
        let v = Vec3::<SimpleEci>::new(6778.0, 0.0, 0.0);
        let a = r_direct.transform(&v);
        let b = r_via_ut1.transform(&v);
        assert!((a.x() - b.x()).abs() < 1e-14);
        assert!((a.y() - b.y()).abs() < 1e-14);
        assert!((a.z() - b.z()).abs() < 1e-14);
    }

    #[test]
    fn from_utc_assuming_ut1_eq_utc_matches_legacy_gmst() {
        use crate::epoch::Epoch;
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        // Legacy path: Utc::gmst returns the ERA formula (misnamed).
        let legacy_gmst = utc.gmst();
        let r_new = Rotation::<SimpleEci, SimpleEcef>::from_utc_assuming_ut1_eq_utc(&utc);
        let r_legacy = Rotation::<SimpleEci, SimpleEcef>::from_era(legacy_gmst);
        // Quaternions should be identical (bit-level).
        let v = Vec3::<SimpleEci>::new(7000.0, 1000.0, 500.0);
        let a = r_new.transform(&v);
        let b = r_legacy.transform(&v);
        assert!((a.x() - b.x()).abs() < 1e-14);
        assert!((a.y() - b.y()).abs() < 1e-14);
        assert!((a.z() - b.z()).abs() < 1e-14);
    }
}
