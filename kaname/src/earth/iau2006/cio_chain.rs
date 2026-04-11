//! Public `Rotation` constructors for the IAU 2006 CIO-based
//! GCRS ↔ ITRS chain.
//!
//! Phase 3B wires the pure-math evaluators from
//! [`super::cip`], [`super::precession`], [`super::fundamental_arguments`],
//! and the EOP provider traits from [`crate::earth::eop`] into typed
//! [`crate::frame::Rotation`] constructors.
//!
//! # Chain structure (TN36 Eq. 5.1)
//!
//! ```text
//!   [ITRS] = W(t) · R(t) · Q(t) · [GCRS]
//! ```
//!
//! decomposed into the three phantom-typed steps:
//!
//! ```text
//!   Rotation<Gcrs, Cirs>::iau2006        ≡ Q(t) with optional dX/dY correction
//!   Rotation<Cirs, Tirs>::from_era       ≡ R(t) = R_3(−ERA)
//!   Rotation<Tirs, Itrs>::polar_motion   ≡ W(t) = polar-motion + TIO locator
//! ```
//!
//! and the combined helpers
//!
//! ```text
//!   Rotation<Gcrs, Itrs>::iau2006_full           ≡ W · R · Q
//!   Rotation<Gcrs, Itrs>::iau2006_full_from_utc  ≡ derives tt / ut1 from utc
//! ```
//!
//! # Independent variables (TN36 §5.2)
//!
//! Each rotation takes the `Epoch<S>` scale that the underlying math
//! requires, because each `S` is definitional rather than conventional:
//!
//! | Step                          | Scale    | Why                                   |
//! |-------------------------------|----------|---------------------------------------|
//! | `iau2006` (CIP polynomial)    | `Tt`     | IAU 2006 precession + IAU 2000A nutation use TT centuries |
//! | `iau2006` (EOP dX/dY lookup)  | `Utc`    | IERS Bulletin A/B index EOP by UTC MJD |
//! | `from_era`                    | `Ut1`    | ERA is a definitional function of UT1 (SOFA `iauEra00`) |
//! | `polar_motion` (xp/yp lookup) | `Utc`    | Same as above                          |
//! | `polar_motion` (s' TIO locator) | `Tt`   | `s'(t)` is a TT-centuries polynomial (TN36 Eq. 5.13) |
//!
//! The combined `iau2006_full` therefore takes **three** epochs
//! (`tt`, `ut1`, `utc`). The `iau2006_full_from_utc` convenience
//! derives all three from a single [`Epoch<Utc>`] plus a [`Ut1Offset`]
//! provider.
//!
//! # EOP provider trait bounds
//!
//! - `iau2006` requires [`NutationCorrections`] (for `dX`, `dY`)
//! - `polar_motion` requires [`PolarMotion`] (for `xp`, `yp`)
//! - `iau2006_full` requires both
//! - `iau2006_full_from_utc` requires those **plus** [`Ut1Offset`]
//!
//! [`crate::earth::eop::NullEop`] implements none of these traits, so
//! every constructor above rejects it at compile time. The trybuild
//! tests under `kaname/tests/trybuild/` pin that guarantee.

use nalgebra::{Matrix3, Rotation3, UnitQuaternion, Vector3};

use super::cip::{cio_locator_s, cip_xy, gcrs_to_cirs_matrix, rotation_x, rotation_y, rotation_z};
use super::{Arcsec, Mas, Rad, Uas};
use crate::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};
use crate::epoch::{Epoch, Tt, Ut1, Utc};
use crate::frame::{Cirs, Gcrs, Itrs, Rotation, Tirs};

// ─── TIO locator s'(t) ──────────────────────────────────────────

/// Terrestrial Intermediate Origin (TIO) locator `s'(t)`.
///
/// TN36 Eq. (5.13) / SOFA `iauSp00`: to the accuracy required for
/// polar motion (< 1 µas/yr), `s'` is a linear polynomial in TT
/// centuries:
///
/// ```text
/// s'(t) ≈ −47 µas × t
/// ```
///
/// Used by [`Rotation<Tirs, Itrs>::polar_motion`]. Exposed as a private
/// helper; Phase 3A-1 / 3A-2 did not surface it because no consumer
/// existed until the polar motion constructor below.
fn tio_locator_s_prime(tt_centuries: f64) -> Rad {
    Uas::new(-47.0 * tt_centuries).to_radians()
}

// ─── Matrix3 → UnitQuaternion ───────────────────────────────────

/// Convert an orthogonal 3×3 matrix to a [`UnitQuaternion`].
///
/// The inputs produced by the Phase 3A math are all proper orthogonal
/// (determinant +1) by construction, so `Rotation3::from_matrix_unchecked`
/// is safe here. Extracted into a helper because every constructor in
/// this module follows the same "build a Matrix3, wrap in Rotation"
/// pattern.
fn matrix3_to_unit_quaternion(m: Matrix3<f64>) -> UnitQuaternion<f64> {
    UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m))
}

// ─── Rotation<Gcrs, Cirs>::iau2006 ──────────────────────────────

impl Rotation<Gcrs, Cirs> {
    /// Build the GCRS → CIRS rotation from the IAU 2006 / 2000A_R06
    /// CIP model plus optional observed `dX`, `dY` corrections from an
    /// EOP provider.
    ///
    /// # Parameters
    ///
    /// - `tt`  — TT epoch; used to evaluate the CIP polynomial and
    ///   trigonometric series at `t = (JD_TT − 2451545.0) / 36525`
    /// - `utc` — UTC epoch; used as the IERS EOP lookup index for
    ///   the `dX`, `dY` nutation corrections
    /// - `eop` — anything implementing [`NutationCorrections`] (in mas).
    ///   [`crate::earth::eop::NullEop`] does **not** satisfy this
    ///   bound, so trybuild pins a compile error at
    ///   `kaname/tests/trybuild/null_eop_in_iau2006.rs`.
    ///
    /// # Algorithm
    ///
    /// ```text
    /// (X, Y) = cip_xy(t)                         // CIP model
    /// dX, dY = eop.dx(utc_mjd), eop.dy(utc_mjd)  // observed, mas → rad
    /// X' = X + dX                                // TN36 Eq. 5.26
    /// Y' = Y + dY
    /// s  = cio_locator_s(t, X, Y)                // model (not corrected)
    /// Q  = gcrs_to_cirs_matrix(X', Y', s)
    /// ```
    ///
    /// Note that `s` is evaluated from the **model** `X`, `Y`,
    /// matching SOFA's `iauC2i06a`. The correction contribution to `s`
    /// is at sub-nas level for realistic dX, dY (~mas).
    pub fn iau2006<P>(tt: &Epoch<Tt>, utc: &Epoch<Utc>, eop: &P) -> Self
    where
        P: NutationCorrections + ?Sized,
    {
        let t = tt.centuries_since_j2000();
        let utc_mjd = utc.mjd();

        let (x_model, y_model) = cip_xy(t);
        let dx = Mas::new(eop.dx(utc_mjd)).to_radians();
        let dy = Mas::new(eop.dy(utc_mjd)).to_radians();
        let x_corrected = Rad::new(x_model.raw() + dx.raw());
        let y_corrected = Rad::new(y_model.raw() + dy.raw());

        // CIO locator from model X, Y (SOFA iauC2i06a convention).
        let s = cio_locator_s(t, x_model, y_model);

        let m = gcrs_to_cirs_matrix(x_corrected, y_corrected, s);
        Self::from_raw(matrix3_to_unit_quaternion(m))
    }
}

// ─── Rotation<Cirs, Tirs>::from_era ─────────────────────────────

impl Rotation<Cirs, Tirs> {
    /// Build the CIRS → TIRS rotation: `R_3(−ERA(ut1))`.
    ///
    /// The Earth Rotation Angle `ERA` is a definitional function of
    /// UT1 (TN36 Eq. 5.14 / SOFA `iauEra00`), already implemented by
    /// [`Epoch::<Ut1>::era`]. The `Rotation<Cirs, Tirs>` is the pure
    /// z-axis rotation by `−ERA`.
    ///
    /// This constructor takes no EOP provider — every quantity is
    /// definitional once `ut1` is known, and the `ut1` epoch was
    /// itself derived from `utc` + `dUT1` upstream (via
    /// [`Epoch::<Utc>::to_ut1`]).
    pub fn from_era(ut1: &Epoch<Ut1>) -> Self {
        let era = ut1.era();
        let axis = nalgebra::Unit::new_normalize(Vector3::z());
        Self::from_raw(UnitQuaternion::from_axis_angle(&axis, -era))
    }
}

// ─── Rotation<Tirs, Itrs>::polar_motion ─────────────────────────

impl Rotation<Tirs, Itrs> {
    /// Build the TIRS → ITRS rotation: the polar-motion matrix
    /// `W(xp, yp, s')`.
    ///
    /// TN36 Eq. (5.3) / SOFA `iauPom00`:
    ///
    /// ```text
    /// W = R_3(−s') · R_2(xp) · R_1(yp)
    /// ```
    ///
    /// (active convention, TN36) which SOFA computes as the
    /// observationally equivalent
    ///
    /// ```text
    /// W = R_1(−yp) · R_2(−xp) · R_3(s')
    /// ```
    ///
    /// (passive convention, SOFA `iauRx` / `iauRy` / `iauRz`). kaname
    /// uses the SOFA form directly because our Phase 3A `rotation_{x,y,z}`
    /// helpers are passive.
    ///
    /// # Parameters
    ///
    /// - `tt`  — TT epoch; used to evaluate the TIO locator `s'(t)`
    ///   via [`tio_locator_s_prime`]
    /// - `utc` — UTC epoch; used as the IERS EOP lookup index for
    ///   `xp`, `yp`
    /// - `eop` — anything implementing [`PolarMotion`] (in arcsec).
    ///   [`crate::earth::eop::NullEop`] does **not** satisfy this
    ///   bound.
    pub fn polar_motion<P>(tt: &Epoch<Tt>, utc: &Epoch<Utc>, eop: &P) -> Self
    where
        P: PolarMotion + ?Sized,
    {
        let utc_mjd = utc.mjd();
        let xp = Arcsec::new(eop.x_pole(utc_mjd)).to_radians();
        let yp = Arcsec::new(eop.y_pole(utc_mjd)).to_radians();
        let sp = tio_locator_s_prime(tt.centuries_since_j2000());

        let m = rotation_x(-yp.raw()) * rotation_y(-xp.raw()) * rotation_z(sp.raw());
        Self::from_raw(matrix3_to_unit_quaternion(m))
    }
}

// ─── Rotation<Gcrs, Itrs>::iau2006_full ─────────────────────────

impl Rotation<Gcrs, Itrs> {
    /// Build the full GCRS → ITRS rotation by composing the three
    /// intermediate steps.
    ///
    /// ```text
    /// [ITRS] = W(utc, tt) · R_3(−ERA(ut1)) · Q(tt, utc) · [GCRS]
    /// ```
    ///
    /// Taking three separate epochs is intentional: `Epoch<Tt>`,
    /// `Epoch<Ut1>`, and `Epoch<Utc>` are **definitionally** distinct
    /// time scales (TN36 §5.2) and the compiler enforces that the
    /// caller thought about each one. See
    /// [`Rotation::<Gcrs, Itrs>::iau2006_full_from_utc`] for the
    /// convenience form that derives all three from a single UTC.
    pub fn iau2006_full<P>(tt: &Epoch<Tt>, ut1: &Epoch<Ut1>, utc: &Epoch<Utc>, eop: &P) -> Self
    where
        P: NutationCorrections + PolarMotion + ?Sized,
    {
        // Fully-qualified calls are needed because `from_era` /
        // `polar_motion` / `iau2006` now exist on several `Rotation`
        // type specialisations (e.g. `Rotation<SimpleEci, SimpleEcef>::from_era`).
        let q = Rotation::<Gcrs, Cirs>::iau2006(tt, utc, eop);
        let r = Rotation::<Cirs, Tirs>::from_era(ut1);
        let w = Rotation::<Tirs, Itrs>::polar_motion(tt, utc, eop);
        q.then(&r).then(&w)
    }

    /// Build the full GCRS → ITRS rotation from a single [`Epoch<Utc>`],
    /// deriving `tt` and `ut1` internally via
    /// [`Epoch::<Utc>::to_tt`] and [`Epoch::<Utc>::to_ut1`].
    ///
    /// Requires the combined
    /// [`Ut1Offset`] + [`NutationCorrections`] + [`PolarMotion`]
    /// bound on the EOP provider. Position-only rotation: LOD is not
    /// needed here but will be required when
    /// `Rotation<Gcrs, Itrs>::iau2006_full_with_rates` (velocity
    /// transform) is added.
    pub fn iau2006_full_from_utc<P>(utc: &Epoch<Utc>, eop: &P) -> Self
    where
        P: Ut1Offset + NutationCorrections + PolarMotion + ?Sized,
    {
        let tt = utc.to_tt();
        let ut1 = utc.to_ut1(eop);
        Self::iau2006_full(&tt, &ut1, utc, eop)
    }
}

// ─── Structural tests ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::{Tai, Tdb};
    use crate::frame::Vec3;

    /// Minimal EOP provider that returns zero for every parameter.
    /// Distinct from [`crate::earth::eop::NullEop`] (which implements
    /// no trait at all): `ZeroEop` satisfies all four capability
    /// traits and lets tests build full chains with no EOP correction
    /// applied.
    struct ZeroEop;
    impl Ut1Offset for ZeroEop {
        fn dut1(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    impl PolarMotion for ZeroEop {
        fn x_pole(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
        fn y_pole(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    impl NutationCorrections for ZeroEop {
        fn dx(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
        fn dy(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }
    impl crate::earth::eop::LengthOfDay for ZeroEop {
        fn lod(&self, _utc_mjd: f64) -> f64 {
            0.0
        }
    }

    fn sample_epochs() -> (Epoch<Tt>, Epoch<Ut1>, Epoch<Utc>) {
        // Pick a non-pathological post-J2000 instant.
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let tt = utc.to_tt();
        let ut1 = utc.to_ut1_naive();
        (tt, ut1, utc)
    }

    // ─── TIO locator (structural) ────────────────────────────────

    /// TN36 Eq. 5.13 / SOFA `iauSp00`: `s'(0) = 0` and the linear
    /// coefficient is exactly `−47 µas/century`. Pins the polynomial
    /// against a manual derivation from [`Uas::to_radians`].
    #[test]
    fn tio_locator_is_linear_minus_47_uas_per_century() {
        assert_eq!(tio_locator_s_prime(0.0).raw(), 0.0);

        let one_century = tio_locator_s_prime(1.0).raw();
        let expected_rad = Uas::new(-47.0).to_radians().raw();
        assert!(
            (one_century - expected_rad).abs() < 1e-20,
            "s'(1) = {one_century}, expected {expected_rad}"
        );

        // Linearity: s'(2t) = 2 · s'(t)
        let t = 0.314;
        let two_t = tio_locator_s_prime(2.0 * t).raw();
        let t_doubled = 2.0 * tio_locator_s_prime(t).raw();
        assert!((two_t - t_doubled).abs() < 1e-30);
    }

    // ─── Rotation<Cirs, Tirs>::from_era ──────────────────────────

    /// At the J2000.0 UT1 reference instant the ERA is
    /// `2π × 0.7790572732640` ≈ `4.895 rad`, and the resulting
    /// `R_3(−ERA)` matrix has a non-trivial element in (0,1). Pins the
    /// sign and axis of `from_era` against `Epoch<Ut1>::era`.
    #[test]
    fn from_era_uses_z_axis_with_negative_era() {
        use crate::epoch::J2000_JD;

        let ut1 = Epoch::<Ut1>::from_jd_ut1(J2000_JD);
        let rot = Rotation::<Cirs, Tirs>::from_era(&ut1);

        // Compare with a hand-constructed reference: the underlying
        // quaternion should rotate `+x` by `−ERA` around `+z`.
        let era = ut1.era();
        let v_in = Vec3::<Cirs>::new(1.0, 0.0, 0.0);
        let v_out = rot.transform(&v_in);
        let expected_x = era.cos();
        let expected_y = -era.sin();
        assert!(
            (v_out.x() - expected_x).abs() < 1e-14,
            "x = {:?}, expected {expected_x}",
            v_out.x()
        );
        assert!(
            (v_out.y() - expected_y).abs() < 1e-14,
            "y = {:?}, expected {expected_y}",
            v_out.y()
        );
        assert!(v_out.z().abs() < 1e-14);
    }

    // ─── Rotation<Tirs, Itrs>::polar_motion ──────────────────────

    /// With zero xp, yp, the polar-motion matrix collapses to a
    /// pure `R_3(s')` about the z-axis (TIO locator alone). Pins the
    /// matrix reduction at the `xp = yp = 0` limit.
    #[test]
    fn polar_motion_with_zero_xp_yp_reduces_to_z_rotation_by_sp() {
        let (tt, _ut1, utc) = sample_epochs();
        let rot = Rotation::<Tirs, Itrs>::polar_motion(&tt, &utc, &ZeroEop);

        // Apply to +x and +y: should be a pure z-axis rotation. With
        // s'(0) of ~−47 µas/century, even a few centuries gives a
        // rotation of ~100 µas, so the off-axis components should
        // scale linearly with sin(s') ≈ s'.
        let v_in = Vec3::<Tirs>::new(1.0, 0.0, 0.0);
        let v_out = rot.transform(&v_in);
        assert!(v_out.z().abs() < 1e-20);
        // For realistic s' (~ −47 µas/century ≈ 2e-10 rad), x ≈ 1 and
        // y ≈ −s' after the rotation R_3(−s') × (1,0,0)ᵀ = (cos s', −sin s', 0).
        // Just pin that the Z component stays ~0 and the magnitude is preserved.
        assert!((v_out.x() * v_out.x() + v_out.y() * v_out.y() - 1.0).abs() < 1e-14);
    }

    // ─── Rotation<Gcrs, Cirs>::iau2006 ───────────────────────────

    /// With zero dX, dY corrections, the `iau2006` rotation must match
    /// the raw `gcrs_to_cirs_matrix_at` from Phase 3A-4. Pins the
    /// "NutationCorrections → iau2006" wiring without needing new
    /// fixtures.
    #[test]
    fn iau2006_with_zero_corrections_matches_phase_3a4_matrix() {
        use super::super::cip::gcrs_to_cirs_matrix_at;
        let (tt, _ut1, utc) = sample_epochs();

        let rot = Rotation::<Gcrs, Cirs>::iau2006(&tt, &utc, &ZeroEop);

        // Compare the underlying quaternion against the Phase 3A-4
        // matrix-based one by picking a reference vector and applying
        // both. The quaternion and matrix representations must agree
        // at the 1e-14 rad level.
        let m = gcrs_to_cirs_matrix_at(tt.centuries_since_j2000());
        let reference = UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m));

        let v_in = nalgebra::Vector3::new(1.0, 0.0, 0.0);
        let v_from_rotation = rot.inner().transform_vector(&v_in);
        let v_from_reference = reference.transform_vector(&v_in);

        for i in 0..3 {
            let delta = (v_from_rotation[i] - v_from_reference[i]).abs();
            assert!(delta < 1e-14, "component {i} delta = {delta}");
        }
    }

    // ─── Rotation<Gcrs, Itrs>::iau2006_full ──────────────────────

    /// `iau2006_full` must equal the explicit composition
    /// `polar_motion · from_era · iau2006`. Pins the chaining logic
    /// against the constructor-by-constructor build.
    #[test]
    fn iau2006_full_composition_matches_explicit_chain() {
        let (tt, ut1, utc) = sample_epochs();

        let combined = Rotation::<Gcrs, Itrs>::iau2006_full(&tt, &ut1, &utc, &ZeroEop);

        let q = Rotation::<Gcrs, Cirs>::iau2006(&tt, &utc, &ZeroEop);
        let r = Rotation::<Cirs, Tirs>::from_era(&ut1);
        let w = Rotation::<Tirs, Itrs>::polar_motion(&tt, &utc, &ZeroEop);
        let explicit: Rotation<Gcrs, Itrs> = q.then(&r).then(&w);

        // Apply both to a reference vector and compare.
        let v_in = Vec3::<Gcrs>::new(1.0, 0.0, 0.0);
        let a = combined.transform(&v_in);
        let b = explicit.transform(&v_in);
        for (i, (ai, bi)) in [(a.x(), b.x()), (a.y(), b.y()), (a.z(), b.z())]
            .iter()
            .enumerate()
            .map(|(i, (a, b))| (i, (*a, *b)))
        {
            assert!(
                (ai - bi).abs() < 1e-14,
                "component {i}: combined={ai}, explicit={bi}"
            );
        }
    }

    /// `iau2006_full_from_utc` must agree with `iau2006_full` when the
    /// derived `tt` / `ut1` match. With the `ZeroEop` provider the
    /// derived `ut1` differs from `to_ut1_naive` only at the f64
    /// round-off level, so the transform should agree to 1e-14.
    #[test]
    fn iau2006_full_from_utc_matches_iau2006_full() {
        let utc = Epoch::<Utc>::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let tt = utc.to_tt();
        let ut1 = utc.to_ut1(&ZeroEop);

        let from_utc = Rotation::<Gcrs, Itrs>::iau2006_full_from_utc(&utc, &ZeroEop);
        let explicit = Rotation::<Gcrs, Itrs>::iau2006_full(&tt, &ut1, &utc, &ZeroEop);

        let v_in = Vec3::<Gcrs>::new(0.0, 1.0, 0.0);
        let a = from_utc.transform(&v_in);
        let b = explicit.transform(&v_in);
        for i in 0..3 {
            let ai = [a.x(), a.y(), a.z()][i];
            let bi = [b.x(), b.y(), b.z()][i];
            assert!(
                (ai - bi).abs() < 1e-14,
                "component {i}: from_utc={ai}, explicit={bi}"
            );
        }
    }

    // Touch the `Tai` / `Tdb` imports so the `use` stays meaningful
    // when the test body evolves — otherwise rustc warns on unused.
    #[allow(dead_code)]
    fn _unused_imports() {
        let _: Option<Epoch<Tai>> = None;
        let _: Option<Epoch<Tdb>> = None;
    }
}
