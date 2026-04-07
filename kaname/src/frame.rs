//! 座標系付き型: `Vec3<F>` と `Rotation<From, To>`。
//!
//! 座標系をコンパイル時に型レベルで区別し、異フレーム間の
//! 不正な演算を防ぐ。ゼロサイズの `PhantomData` マーカーなので
//! ランタイムコストはゼロ。
//!
//! # フレームマーカー
//!
//! - [`Eci`] — Earth-Centered Inertial (J2000)
//! - [`Ecef`] — Earth-Centered Earth-Fixed
//! - [`Body`] — 宇宙機機体座標系
//! - [`Lvlh`] — Local Vertical Local Horizontal
//!
//! # 使い方
//!
//! ```
//! use kaname::frame::{Vec3, Rotation, Eci, Body};
//!
//! let b_eci = Vec3::<Eci>::new(1e-5, 2e-5, -3e-5);
//! let r_bi = Rotation::<Eci, Body>::from_raw(
//!     nalgebra::UnitQuaternion::identity(),
//! );
//! let b_body: Vec3<Body> = r_bi.transform(&b_eci);
//! ```

use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Neg, Sub};

use nalgebra::{UnitQuaternion, Vector3};

// ─── フレームマーカー ────────────────────────────────────────────

/// Earth-Centered Inertial (J2000)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eci;

/// Earth-Centered Earth-Fixed。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ecef;

/// 宇宙機機体座標系。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body;

/// Local Vertical Local Horizontal。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lvlh;

// ─── Vec3<F> ─────────────────────────────────────────────────────

/// 座標系 `F` 付き 3 次元ベクトル。
///
/// `PhantomData<F>` はゼロサイズなのでメモリレイアウトは
/// `Vector3<f64>` と同一。同一フレーム内の演算のみ許可。
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

// ─── テスト ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn vec3_basic_ops() {
        let a = Vec3::<Eci>::new(1.0, 2.0, 3.0);
        let b = Vec3::<Eci>::new(4.0, 5.0, 6.0);

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
        let a = Vec3::<Eci>::new(1.0, 0.0, 0.0);
        let b = Vec3::<Eci>::new(0.0, 1.0, 0.0);

        assert!((a.dot(&b)).abs() < 1e-15);

        let c = a.cross(&b);
        assert!((c.z() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn vec3_zeros() {
        let z = Vec3::<Body>::zeros();
        assert_eq!(z.magnitude(), 0.0);
    }

    #[test]
    fn vec3_inner_roundtrip() {
        let v = Vec3::<Eci>::new(1.0, 2.0, 3.0);
        let raw = v.into_inner();
        let v2 = Vec3::<Eci>::from_raw(raw);
        assert_eq!(v, v2);
    }

    #[test]
    fn vec3_add_assign() {
        let mut a = Vec3::<Eci>::new(1.0, 2.0, 3.0);
        a += Vec3::new(10.0, 20.0, 30.0);
        assert_eq!(a.x(), 11.0);
    }

    #[test]
    fn vec3_is_finite() {
        assert!(Vec3::<Eci>::new(1.0, 2.0, 3.0).is_finite());
        assert!(!Vec3::<Eci>::new(f64::NAN, 0.0, 0.0).is_finite());
        assert!(!Vec3::<Eci>::new(0.0, f64::INFINITY, 0.0).is_finite());
    }

    #[test]
    fn rotation_identity_is_noop() {
        let r = Rotation::<Eci, Body>::from_raw(UnitQuaternion::identity());
        let v = Vec3::<Eci>::new(1.0, 2.0, 3.0);
        let result = r.transform(&v);
        assert!((result.x() - 1.0).abs() < 1e-15);
        assert!((result.y() - 2.0).abs() < 1e-15);
        assert!((result.z() - 3.0).abs() < 1e-15);
    }

    #[test]
    fn rotation_90deg_about_z() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 2.0);
        let r = Rotation::<Eci, Body>::from_raw(q);

        let v = Vec3::<Eci>::new(1.0, 0.0, 0.0);
        let result = r.transform(&v);
        assert!((result.x()).abs() < 1e-15);
        assert!((result.y() - 1.0).abs() < 1e-15);
        assert!((result.z()).abs() < 1e-15);
    }

    #[test]
    fn rotation_inverse() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let r = Rotation::<Eci, Body>::from_raw(q);

        let v = Vec3::<Eci>::new(1.0, 0.0, 0.0);
        let body = r.transform(&v);
        let back = r.inverse().transform(&body);

        assert!((back.x() - 1.0).abs() < 1e-14);
        assert!((back.y()).abs() < 1e-14);
    }

    #[test]
    fn rotation_compose() {
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        let r_ab =
            Rotation::<Eci, Body>::from_raw(UnitQuaternion::from_axis_angle(&axis, PI / 4.0));
        let r_bc =
            Rotation::<Body, Lvlh>::from_raw(UnitQuaternion::from_axis_angle(&axis, PI / 4.0));

        let r_ac: Rotation<Eci, Lvlh> = r_ab.then(&r_bc);

        // 45° + 45° = 90° about Z
        let v = Vec3::<Eci>::new(1.0, 0.0, 0.0);
        let result = r_ac.transform(&v);
        assert!((result.x()).abs() < 1e-14);
        assert!((result.y() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn debug_formatting() {
        let v = Vec3::<Eci>::new(1.0, 2.0, 3.0);
        let s = format!("{v:?}");
        assert!(s.contains("Eci"));
        assert!(s.contains("1"));
    }
}
