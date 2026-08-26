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
//!   ERA-only Z 回転の親フレーム。可視化グレード計算の出発点
//! - [`SimpleEcef`] — [`SimpleEci`] からの ERA-only Z 回転先。近似的 Earth-fixed
//! - [`Gcrs`] — Geocentric Celestial Reference System。IAU 2006 CIO chain の
//!   celestial side。Meeus ephemeris の返り型でもあり、その値は低精度 analytic
//!   model 由来なので strict な GCRS とは限らない (nutation / frame bias 未適用)
//! - [`Cirs`] — Celestial Intermediate Reference System (CIO chain の中間)
//! - [`Tirs`] — Terrestrial Intermediate Reference System (polar motion 未適用)
//! - [`Itrs`] — International Terrestrial Reference System (polar motion 適用済み)。
//!   geodetic 変換はこの frame に紐づく
//! - [`Teme`] — True Equator, Mean Equinox。SGP4 / TLE / OMM の平均要素フレーム
//!   (↔ Gcrs/SimpleEci 回転は IAU-76/FK5 換算で実装済み: [`crate::earth::teme`])
//! - [`Rsw`] — Radial / Along-track / Cross-track 軌道ローカル系。
//!   軸順は標準 RSW 規約 [R̂, Ŝ, Ŵ] (R̂=normalize(r), Ŵ=normalize(r×v), Ŝ=Ŵ×R̂)
//! - [`Body`] — 宇宙機機体座標系
//!
//! # Category trait
//!
//! - [`Eci`] — structural category for earth-centered inertial frames.
//!   実装者: `SimpleEci`, `Gcrs`, `Cirs`, `Teme`
//! - [`Ecef`] — structural category for earth-fixed frames.
//!   実装者: `SimpleEcef`, `Tirs`, `Itrs`
//! - [`LocalOrbital`] — structural category for local orbital frames.
//!   実装者: `Rsw`
//!
//! category trait は precision-agnostic な generic math (`<F: Eci>` で受ける等)
//! を書くためのものであり、precision-aware な変換 API には concrete 型を使うこと。
//!
//! # 使い方
//!
//! ```
//! use arika::frame::{Vec3, Rotation, Gcrs, Body};
//!
//! let b_gcrs = Vec3::<Gcrs>::new(1e-5, 2e-5, -3e-5);
//! let r_bg = Rotation::<Gcrs, Body>::from_raw(
//!     nalgebra::UnitQuaternion::identity(),
//! );
//! let b_body: Vec3<Body> = r_bg.transform(&b_gcrs);
//! ```

use core::marker::PhantomData;
use core::ops::{Add, Div, Mul, Neg, Sub};

use nalgebra::{UnitQuaternion, Vector3};
use serde::{Deserialize, Serialize};

use crate::epoch::{Epoch, Ut1Epoch, Utc};

// Runtime frame descriptor

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
    Cirs,
    Tirs,
    Itrs,
    Teme,
    Rsw,
    Body,
}

impl FrameDescriptor {
    pub const fn name(self) -> &'static str {
        match self {
            FrameDescriptor::SimpleEci => "SimpleEci",
            FrameDescriptor::SimpleEcef => "SimpleEcef",
            FrameDescriptor::Gcrs => "Gcrs",
            FrameDescriptor::Cirs => "Cirs",
            FrameDescriptor::Tirs => "Tirs",
            FrameDescriptor::Itrs => "Itrs",
            FrameDescriptor::Teme => "Teme",
            FrameDescriptor::Rsw => "Rsw",
            FrameDescriptor::Body => "Body",
        }
    }

    pub const fn category(self) -> FrameCategory {
        match self {
            FrameDescriptor::SimpleEci
            | FrameDescriptor::Gcrs
            | FrameDescriptor::Cirs
            | FrameDescriptor::Teme => FrameCategory::Eci,
            FrameDescriptor::SimpleEcef | FrameDescriptor::Tirs | FrameDescriptor::Itrs => {
                FrameCategory::Ecef
            }
            FrameDescriptor::Rsw => FrameCategory::LocalOrbital,
            FrameDescriptor::Body => FrameCategory::Body,
        }
    }
}

// Sealed trait + Frame / category traits

mod sealed {
    pub trait Sealed {}
}

/// Top-level frame trait. Implemented by every concrete frame marker.
///
/// Provides `NAME` and `DESCRIPTOR` for runtime identification. Sealed: new
/// frames can only be added inside arika. No `Copy` / `'static` bound —
/// marker structs derive them themselves.
pub trait Frame: sealed::Sealed {
    const NAME: &'static str;
    const DESCRIPTOR: FrameDescriptor;
}

/// Structural category for earth-centered inertial frames.
///
/// 近似系 (`SimpleEci`)、厳密系 (`Gcrs`/`Cirs`)、SGP4 の quasi-inertial 系 (`Teme`)
/// をまとめて含む category。precision-aware な処理は concrete 型を関数シグネチャに
/// 書き、`<F: Eci>` generic bound は precision-agnostic な math (magnitude / dot /
/// 等) のみに使う。
pub trait Eci: Frame {}

/// Structural category for earth-centered earth-fixed frames.
///
/// 実装者: [`SimpleEcef`] (近似), [`Tirs`], [`Itrs`] (厳密)。同上の注意。
pub trait Ecef: Frame {}

/// Structural category for local orbital frames.
///
/// 実装者: [`Rsw`]。
pub trait LocalOrbital: Frame {}

// Concrete frame markers

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

/// Geocentric Celestial Reference System. IAU 2006 CIO chain の celestial side。
///
/// Meeus ephemeris (低精度 analytic model) の返り型としても使うため、その値は
/// 厳密な GCRS とは限らない。Meeus 側は of-date の級数を IAU 1976 precession で
/// J2000 に戻してから返すので precession は入っているが、nutation (≤ 17″)、
/// J2000→GCRS frame bias (~20 mas)、および model 自身の精度 (~1′) ぶんの残差がある。
/// GCRS → ITRS の高精度変換は
/// [`Rotation::<Gcrs, Itrs>::iau2006_full`] を参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gcrs;
impl sealed::Sealed for Gcrs {}
impl Frame for Gcrs {
    const NAME: &'static str = "Gcrs";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Gcrs;
}
impl Eci for Gcrs {}

/// Celestial Intermediate Reference System. IAU 2006 CIO chain の中間フレーム
/// (precession/nutation 適用後、ERA による Z 回転の直前の celestial side)。
///
/// # Independent variable
///
/// [`Rotation::<Gcrs, Cirs>::iau2006`] は TT (Terrestrial Time) の Julian centuries
/// を独立変数とする — IAU 2006 precession と IAU 2000A/B nutation の series は TT
/// centuries で定義されているため。詳細は [`arika/DESIGN.md`](../../DESIGN.md) の
/// 「Frame rotation の time scale は definitional」を参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cirs;
impl sealed::Sealed for Cirs {}
impl Frame for Cirs {
    const NAME: &'static str = "Cirs";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Cirs;
}
impl Eci for Cirs {}

/// Terrestrial Intermediate Reference System. polar motion 未適用の Earth-fixed
/// 中間フレーム ([`Cirs`] から ERA による Z 回転で得られる)。
///
/// # Independent variable
///
/// [`Cirs`] → [`Tirs`] の変換 ([`Rotation::<Cirs, Tirs>::from_era`]) は UT1 (Earth
/// rotation angle) を独立変数とする — ERA は UT1 の definitional な関数であり、
/// 他の scale では物理的に意味をなさない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tirs;
impl sealed::Sealed for Tirs {}
impl Frame for Tirs {
    const NAME: &'static str = "Tirs";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Tirs;
}
impl Ecef for Tirs {}

/// International Terrestrial Reference System. IAU 2006 CIO chain の Earth-fixed
/// side (polar motion 適用済み)。geodetic 変換 ([`Vec3::to_geodetic`]) は任意の
/// `Ecef` frame で使えるが、高精度 path では通常この ITRS から変換する。
///
/// GCRS からの完全 chain は [`Rotation::<Gcrs, Itrs>::iau2006_full`]、polar motion
/// 単体は [`Rotation::<Tirs, Itrs>::polar_motion`] を参照。
///
/// # 独立性
///
/// [`SimpleEcef`] と [`Itrs`] の間には **型変換を提供しない** — 近似と厳密を silent
/// に混ぜる経路を作らないため。precision-aware な API は concrete 型 (`Vec3<Itrs>`)
/// を関数シグネチャに書き、generic な `<F: Ecef>` bound は precision-agnostic な
/// math にのみ使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Itrs;
impl sealed::Sealed for Itrs {}
impl Frame for Itrs {
    const NAME: &'static str = "Itrs";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Itrs;
}
impl Ecef for Itrs {}

/// True Equator, Mean Equinox — the quasi-inertial frame in which SGP4 / TLE /
/// OMM mean elements are expressed. Belongs to the [`Eci`] category.
///
/// The `sgp4` module (behind the `sgp4` feature) propagates an
/// [`crate::elements::Sgp4Elements`] set to a `Vec3<Teme>` state, and
/// [`crate::earth::teme`] rotates TEME into the
/// integration frames: the precise [`Rotation<Teme, Gcrs>`](Rotation)
/// (IAU-76/FK5 reduction) and the visualization-grade
/// [`Rotation<Teme, SimpleEci>`](Rotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Teme;
impl sealed::Sealed for Teme {}
impl Frame for Teme {
    const NAME: &'static str = "Teme";
    const DESCRIPTOR: FrameDescriptor = FrameDescriptor::Teme;
}
impl Eci for Teme {}

/// Local orbital frame: Radial / Along-track / Cross-track.
///
/// 軸順は標準 RSW 規約:
/// - R̂ = `normalize(r)` — 地心から衛星方向
/// - Ŵ = `normalize(r × v)` — orbit normal
/// - Ŝ = `Ŵ × R̂` — tangential (円軌道順行なら +v̂ 方向)
///
/// 注意: これは LVLH (業界で変種多数) とは別物。円軌道時の +v̂ 方向が
/// LVLH の X 軸 (or +I 軸) に一致するものがあるが、軸順・符号の選択は
/// 実装によって異なる。arika は標準 RSW [R̂, Ŝ, Ŵ] で固定する。
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

    // 成分アクセサ

    pub fn x(&self) -> f64 {
        self.0.x
    }
    pub fn y(&self) -> f64 {
        self.0.y
    }
    pub fn z(&self) -> f64 {
        self.0.z
    }

    // フレーム非依存演算

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

impl<F> core::fmt::Debug for Vec3<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Vec3<{}>({}, {}, {})",
            core::any::type_name::<F>()
                .rsplit("::")
                .next()
                .unwrap_or("?"),
            self.0.x,
            self.0.y,
            self.0.z
        )
    }
}

// 同一フレーム演算

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

impl<F> core::ops::AddAssign for Vec3<F> {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl<F> core::ops::SubAssign for Vec3<F> {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

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

    /// 恒等回転。`From` と `To` が同一視できるフレーム（例: GCRS と、歳差章動を
    /// 無視する可視化グレードの近似フレーム）を型レベルで橋渡しするのに使う。
    pub fn identity() -> Self {
        Self(UnitQuaternion::identity(), PhantomData)
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

// ─── FrameTransform: rotation + angular velocity (state transforms) ─

/// A frame transform carrying both the orientation ([`Rotation`]) and the
/// angular velocity of the `To` frame relative to `From`, so it can transform
/// **velocities** (and full position+velocity states), not just positions.
///
/// The stored angular velocity `ω` is that of `To` relative to `From`,
/// **expressed in `From`**. Velocities follow the transport theorem:
///
/// ```text
/// r_to = R · r_from
/// v_to = R · (v_from − ω × r_from)
/// ```
///
/// [`Rotation`] stays position-only; this is its kinematic (state) companion.
/// Build Earth ECI↔ECEF instances via the
/// [`EarthFixedTransform`](crate::earth::EarthFixedTransform) factories.
#[derive(Clone, Copy, PartialEq)]
pub struct FrameTransform<From, To> {
    rotation: Rotation<From, To>,
    /// Angular velocity of `To` relative to `From`, expressed in `From` [rad/s].
    angular_velocity: Vec3<From>,
}

impl<From, To> FrameTransform<From, To> {
    /// Build from a rotation and the angular velocity of `To` relative to
    /// `From`, expressed in `From` [rad/s].
    pub fn new(rotation: Rotation<From, To>, angular_velocity: Vec3<From>) -> Self {
        Self {
            rotation,
            angular_velocity,
        }
    }

    /// The orientation part (`From` → `To`).
    pub fn rotation(&self) -> &Rotation<From, To> {
        &self.rotation
    }

    /// Angular velocity of `To` relative to `From`, expressed in `From` [rad/s].
    pub fn angular_velocity_in_from(&self) -> &Vec3<From> {
        &self.angular_velocity
    }

    /// Transform a position (identical to [`Rotation::transform`]).
    pub fn transform_position(&self, position: &Vec3<From>) -> Vec3<To> {
        self.rotation.transform(position)
    }

    /// Transform a velocity via the transport theorem `v_to = R·(v_from − ω×r_from)`.
    ///
    /// The position is required because the rotating-frame correction is `ω × r`.
    pub fn transform_velocity(&self, position: &Vec3<From>, velocity: &Vec3<From>) -> Vec3<To> {
        // Transport theorem, kept frame-tagged via rotation linearity:
        // v_to = R·(v_from − ω×r_from) = R·v_from − R·(ω×r_from).
        let corotation = self.angular_velocity.cross(position); // ω × r, in From
        self.rotation.transform(velocity) - self.rotation.transform(&corotation)
    }

    /// Transform a full state (position, velocity).
    pub fn transform_state(
        &self,
        position: &Vec3<From>,
        velocity: &Vec3<From>,
    ) -> (Vec3<To>, Vec3<To>) {
        (
            self.transform_position(position),
            self.transform_velocity(position, velocity),
        )
    }

    /// Inverse transform (`To` → `From`).
    ///
    /// The inverse angular velocity — of `From` relative to `To`, expressed in
    /// `To` — is `−R·ω`.
    pub fn inverse(&self) -> FrameTransform<To, From> {
        let omega_in_to = self.rotation.transform(&self.angular_velocity); // R·ω, in To
        FrameTransform {
            rotation: self.rotation.inverse(),
            angular_velocity: omega_in_to * -1.0,
        }
    }
}

// ─── Simple path (SimpleEci ↔ SimpleEcef) rotation constructors ─

impl Rotation<SimpleEci, SimpleEcef> {
    /// Construct from a UT1 epoch using the Earth Rotation Angle (ERA).
    ///
    /// `SimpleEcef = R_z(−ERA(UT1)) × SimpleEci`. Applies only the ERA Z
    /// rotation — no precession, nutation, or polar motion. For high-precision
    /// work use the IAU 2006 CIO chain ([`Rotation::<Gcrs, Itrs>::iau2006_full`]).
    pub fn from_ut1(epoch: &Ut1Epoch) -> Self {
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
    pub fn from_ut1(epoch: &Ut1Epoch) -> Self {
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

impl<From, To> core::fmt::Debug for Rotation<From, To> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let from = core::any::type_name::<From>()
            .rsplit("::")
            .next()
            .unwrap_or("?");
        let to = core::any::type_name::<To>()
            .rsplit("::")
            .next()
            .unwrap_or("?");
        write!(f, "Rotation<{from}, {to}>({:?})", self.0)
    }
}

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

    // Frame descriptor / category

    #[test]
    fn frame_descriptor_name() {
        assert_eq!(FrameDescriptor::SimpleEci.name(), "SimpleEci");
        assert_eq!(FrameDescriptor::SimpleEcef.name(), "SimpleEcef");
        assert_eq!(FrameDescriptor::Gcrs.name(), "Gcrs");
        assert_eq!(FrameDescriptor::Cirs.name(), "Cirs");
        assert_eq!(FrameDescriptor::Tirs.name(), "Tirs");
        assert_eq!(FrameDescriptor::Itrs.name(), "Itrs");
        assert_eq!(FrameDescriptor::Rsw.name(), "Rsw");
        assert_eq!(FrameDescriptor::Body.name(), "Body");
    }

    #[test]
    fn frame_descriptor_category() {
        assert_eq!(FrameDescriptor::SimpleEci.category(), FrameCategory::Eci);
        assert_eq!(FrameDescriptor::Gcrs.category(), FrameCategory::Eci);
        assert_eq!(FrameDescriptor::Cirs.category(), FrameCategory::Eci);
        assert_eq!(FrameDescriptor::SimpleEcef.category(), FrameCategory::Ecef);
        assert_eq!(FrameDescriptor::Tirs.category(), FrameCategory::Ecef);
        assert_eq!(FrameDescriptor::Itrs.category(), FrameCategory::Ecef);
        assert_eq!(FrameDescriptor::Rsw.category(), FrameCategory::LocalOrbital);
        assert_eq!(FrameDescriptor::Body.category(), FrameCategory::Body);
    }

    #[test]
    fn teme_frame_marker() {
        // TEME (True Equator, Mean Equinox) is the SGP4 / TLE output frame.
        // It is quasi-inertial, so it belongs to the Eci category.
        assert_eq!(<Teme as Frame>::NAME, "Teme");
        assert_eq!(<Teme as Frame>::DESCRIPTOR, FrameDescriptor::Teme);
        assert_eq!(FrameDescriptor::Teme.name(), "Teme");
        assert_eq!(FrameDescriptor::Teme.category(), FrameCategory::Eci);
        assert_eq!(Vec3::<Teme>::frame_descriptor(), FrameDescriptor::Teme);
    }

    #[test]
    fn frame_descriptor_via_trait() {
        assert_eq!(<SimpleEci as Frame>::DESCRIPTOR, FrameDescriptor::SimpleEci);
        assert_eq!(<Gcrs as Frame>::DESCRIPTOR, FrameDescriptor::Gcrs);
        assert_eq!(<Cirs as Frame>::DESCRIPTOR, FrameDescriptor::Cirs);
        assert_eq!(<Tirs as Frame>::DESCRIPTOR, FrameDescriptor::Tirs);
        assert_eq!(<Itrs as Frame>::DESCRIPTOR, FrameDescriptor::Itrs);
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
        // Structural API using `F: Eci` bound should accept SimpleEci, Gcrs,
        // and Cirs interchangeably — this is by design for precision-agnostic
        // math (magnitude / dot / etc.).
        fn magnitude_eci<F: Eci>(v: Vec3<F>) -> f64 {
            v.magnitude()
        }
        assert_eq!(magnitude_eci(Vec3::<SimpleEci>::new(3.0, 4.0, 0.0)), 5.0);
        assert_eq!(magnitude_eci(Vec3::<Gcrs>::new(0.0, 0.0, 7.0)), 7.0);
        assert_eq!(magnitude_eci(Vec3::<Cirs>::new(5.0, 0.0, 12.0)), 13.0);

        // Same for `F: Ecef`.
        fn magnitude_ecef<F: Ecef>(v: Vec3<F>) -> f64 {
            v.magnitude()
        }
        assert_eq!(magnitude_ecef(Vec3::<SimpleEcef>::new(3.0, 4.0, 0.0)), 5.0);
        assert_eq!(magnitude_ecef(Vec3::<Tirs>::new(0.0, 0.0, 7.0)), 7.0);
        assert_eq!(magnitude_ecef(Vec3::<Itrs>::new(5.0, 0.0, 12.0)), 13.0);
    }

    // Rotation<SimpleEci, SimpleEcef> from_era tests

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
        let ut1 = Ut1Epoch::from_jd_ut1(2460390.5);
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

    // FrameTransform (rotation + angular velocity) ─ state transforms

    const OMEGA_E: f64 = 7.2921159e-5;

    #[test]
    fn frame_transform_zero_omega_equals_rotation() {
        // With ω = 0, velocity transforms exactly like position (pure rotation).
        let rot = Rotation::<SimpleEci, SimpleEcef>::from_era(0.7);
        let ft = FrameTransform::new(rot, Vec3::<SimpleEci>::zeros());
        let r = Vec3::<SimpleEci>::new(7000.0, -1200.0, 500.0);
        let v = Vec3::<SimpleEci>::new(1.0, 7.5, -0.3);
        let v_ft = ft.transform_velocity(&r, &v);
        let v_rot = Rotation::<SimpleEci, SimpleEcef>::from_era(0.7).transform(&v);
        assert!((v_ft.x() - v_rot.x()).abs() < 1e-12);
        assert!((v_ft.y() - v_rot.y()).abs() < 1e-12);
        assert!((v_ft.z() - v_rot.z()).abs() < 1e-12);
    }

    #[test]
    fn frame_transform_inverse_state_roundtrip() {
        let ft = FrameTransform::new(
            Rotation::<SimpleEci, SimpleEcef>::from_era(1.1),
            Vec3::<SimpleEci>::new(0.0, 0.0, OMEGA_E),
        );
        let r = Vec3::<SimpleEci>::new(6778.0, 0.0, 0.0);
        let v = Vec3::<SimpleEci>::new(0.0, 7.5, 1.0);
        let (r_e, v_e) = ft.transform_state(&r, &v);
        let (r_back, v_back) = ft.inverse().transform_state(&r_e, &v_e);
        assert!((r_back.inner() - r.inner()).norm() < 1e-9);
        assert!((v_back.inner() - v.inner()).norm() < 1e-12);
    }

    #[test]
    fn frame_transform_corotating_point_is_static_in_ecef() {
        // At ERA = 0 the ECI→ECEF rotation is identity. A point on the equator
        // co-rotating with Earth (inertial velocity ω × r) is static in ECEF.
        let ft = FrameTransform::new(
            Rotation::<SimpleEci, SimpleEcef>::from_era(0.0),
            Vec3::<SimpleEci>::new(0.0, 0.0, OMEGA_E),
        );
        let r_km = 6378.137;
        let r = Vec3::<SimpleEci>::new(r_km, 0.0, 0.0);
        let v = Vec3::<SimpleEci>::new(0.0, OMEGA_E * r_km, 0.0); // ω × r
        let v_ecef = ft.transform_velocity(&r, &v);
        assert!(
            v_ecef.inner().norm() < 1e-12,
            "co-rotating point should be static in ECEF, got {:?}",
            v_ecef.inner()
        );
    }

    #[test]
    fn frame_transform_velocity_matches_finite_difference() {
        // For a point fixed in ECI (v = 0), d/dt[R(ERA(t))·r] = transform_velocity(r, 0).
        let era = 0.9;
        let r = Vec3::<SimpleEci>::new(7000.0, -1500.0, 800.0);
        let ft = FrameTransform::new(
            Rotation::<SimpleEci, SimpleEcef>::from_era(era),
            Vec3::<SimpleEci>::new(0.0, 0.0, OMEGA_E),
        );
        let v_analytic = ft.transform_velocity(&r, &Vec3::<SimpleEci>::zeros());

        let dt = 1.0e-3;
        let p0 = Rotation::<SimpleEci, SimpleEcef>::from_era(era).transform(&r);
        let p1 = Rotation::<SimpleEci, SimpleEcef>::from_era(era + OMEGA_E * dt).transform(&r);
        let v_fd = (p1.inner() - p0.inner()) / dt;
        assert!(
            (v_analytic.inner() - v_fd).norm() < 1e-6,
            "analytic {:?} vs finite-diff {:?}",
            v_analytic.inner(),
            v_fd
        );
    }
}
