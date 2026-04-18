//! IAU 2006 CIP `X`, `Y` coordinates and CIO locator `s`.
//!
//! # Source
//!
//! - [IERS Conventions 2010 (TN36)](https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html)
//!   Chapter 5, Eq. (5.16) and Tables 5.2a, 5.2b, 5.2d
//! - Companion data tables in the crate-private [`super::tables_gen`]
//!   module, generated from the IERS Conventions Centre electronic
//!   tables by `arika/tools/generate_iau2006_tables.py`
//!
//! # What this file provides
//!
//! - [`cip_xy`] — compute the CIP position `(X, Y)` at a given TT
//!   instant. Matches ERFA `xy06`
//! - [`cio_locator_s`] — compute the CIO locator `s` given `X` and
//!   `Y` at the same TT instant. Matches ERFA `s06`
//! - [`CipCoordinates`] and [`cip_coordinates`] — one-shot convenience
//!   wrapper that returns `(X, Y, s)` together
//! - [`gcrs_to_cirs_matrix`] and [`gcrs_to_cirs_matrix_at`] — assemble
//!   the celestial-to-intermediate rotation matrix from `X`, `Y`, `s`.
//!   Matches ERFA `c2ixys`
//!
//! None of these are public [`crate::frame::Rotation`] constructors —
//! those are Phase 3B. Phase 3A-3 ships the raw scalar evaluators, and
//! Phase 3A-4 adds the 3×3 matrix composition that Phase 3B will wrap.
//!
//! # Independent variable
//!
//! Every function takes `t`, **TT Julian centuries since J2000.0**.
//! Callers obtain this from
//! [`crate::epoch::Epoch::<crate::epoch::Tt>::centuries_since_j2000`].
//!
//! # Algorithm (TN36 Eq. 5.16)
//!
//! Each of the three quantities has the same structural form:
//!
//! ```text
//! Q(t) = polynomial_Q(t)
//!      + Σ_{j=0..4} t^j × Σ_i [ a_{s,j}_i sin(ARG_i) + a_{c,j}_i cos(ARG_i) ]
//! ```
//!
//! where `ARG_i = Σ_{k=0..13} N_{i,k} × F_k(t)` and `F_0..F_13` are the
//! fourteen fundamental arguments from TN36 Eq. (5.43) / (5.44), already
//! implemented in [`super::fundamental_arguments`].
//!
//! `cio_locator_s` additionally subtracts `X·Y/2` because the TN36 Table
//! 5.2d tabulates `s + X·Y/2`, not `s` directly. This subtraction
//! mirrors the ERFA `s06` internal implementation.

use nalgebra::Matrix3;

#[allow(unused_imports)]
use crate::math::F64Ext;

use super::fundamental_arguments::FundamentalArguments;
use super::tables_gen::{
    CipTerm, SXY2_POLY_UAS, SXY2_TERMS_0, SXY2_TERMS_1, SXY2_TERMS_2, SXY2_TERMS_3, SXY2_TERMS_4,
    X_POLY_UAS, X_TERMS_0, X_TERMS_1, X_TERMS_2, X_TERMS_3, X_TERMS_4, Y_POLY_UAS, Y_TERMS_0,
    Y_TERMS_1, Y_TERMS_2, Y_TERMS_3, Y_TERMS_4,
};
use super::{Rad, Uas};

// ─── Public API ──────────────────────────────────────────────────

/// All three CIP / CIO quantities evaluated at a single `t`.
///
/// Returned by [`cip_coordinates`]. `x` and `y` are the celestial
/// intermediate pole coordinates in the GCRS; `s` is the CIO locator
/// that ties the Celestial Intermediate Origin to the true-of-date
/// right ascension.
///
/// Only `Debug` is derived — `Clone`/`Copy` are trivially available via
/// the `Copy`-on-`Rad` pattern but would be unused noise. Downstream
/// consumers destructure or access individual fields.
#[derive(Debug)]
pub struct CipCoordinates {
    /// `X(t)` — CIP X coordinate.
    pub x: Rad,
    /// `Y(t)` — CIP Y coordinate.
    pub y: Rad,
    /// `s(t)` — CIO locator.
    pub s: Rad,
}

/// Evaluate the CIP `(X, Y)` coordinates at TT Julian centuries `t`.
///
/// Equivalent to ERFA's `xy06` when called with `(J2000_JD, t × 36525)`
/// as the two-part TT Julian Date.
pub fn cip_xy(t: f64) -> (Rad, Rad) {
    let fa = FundamentalArguments::evaluate(t);
    let x_uas = evaluate_series(
        t,
        &fa,
        X_POLY_UAS,
        [X_TERMS_0, X_TERMS_1, X_TERMS_2, X_TERMS_3, X_TERMS_4],
    );
    let y_uas = evaluate_series(
        t,
        &fa,
        Y_POLY_UAS,
        [Y_TERMS_0, Y_TERMS_1, Y_TERMS_2, Y_TERMS_3, Y_TERMS_4],
    );
    (Uas::new(x_uas).to_radians(), Uas::new(y_uas).to_radians())
}

/// Evaluate the CIO locator `s(t)` at TT Julian centuries `t`, given
/// the CIP coordinates `(x, y)` at the same instant.
///
/// TN36 Table 5.2d tabulates `s + X·Y/2` as a polynomial plus a
/// trigonometric series; this function evaluates that combined quantity
/// and subtracts `X·Y/2` to isolate `s`. Equivalent to ERFA's `s06`.
///
/// Passing `x` and `y` from a different source than [`cip_xy`] is
/// permitted: the CIO locator depends only on the CIP position in the
/// GCRS and the epoch, so mixing model versions is meaningful and
/// occasionally useful (e.g. applying observed EOP `dX`, `dY`
/// corrections before computing `s`).
pub fn cio_locator_s(t: f64, x: Rad, y: Rad) -> Rad {
    let fa = FundamentalArguments::evaluate(t);
    let sxy2_uas = evaluate_series(
        t,
        &fa,
        SXY2_POLY_UAS,
        [
            SXY2_TERMS_0,
            SXY2_TERMS_1,
            SXY2_TERMS_2,
            SXY2_TERMS_3,
            SXY2_TERMS_4,
        ],
    );
    let sxy2 = Uas::new(sxy2_uas).to_radians();
    Rad::new(sxy2.raw() - x.raw() * y.raw() / 2.0)
}

/// Evaluate `X`, `Y`, and `s` at TT Julian centuries `t` in a single
/// call. Internally calls [`cip_xy`] followed by [`cio_locator_s`].
pub fn cip_coordinates(t: f64) -> CipCoordinates {
    let (x, y) = cip_xy(t);
    let s = cio_locator_s(t, x, y);
    CipCoordinates { x, y, s }
}

// ─── GCRS → CIRS matrix (TN36 Eq. 5.6-5.10 / SOFA iauC2ixys) ─────

/// Assemble the 3×3 **celestial-to-intermediate** rotation matrix `C`
/// from the CIP coordinates `(x, y)` and CIO locator `s`.
///
/// The returned matrix transforms a column vector in the GCRS to the
/// same column vector in the CIRS:
///
/// ```text
/// v_cirs = C · v_gcrs
/// ```
///
/// # Algorithm
///
/// Equivalent to SOFA `iauC2ixys` / ERFA `c2ixys`. Following
/// [IERS Conventions 2010 TN36](https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html)
/// Eq. (5.6) / (5.10), the CIP position `(x, y)` defines spherical
/// angles `(E, d)` via
///
/// ```text
/// x = sin(d) · cos(E)
/// y = sin(d) · sin(E)
/// ```
///
/// from which we recover
///
/// ```text
/// E = atan2(y, x)          (when x² + y² > 0, else 0)
/// d = atan(sqrt(r²) / sqrt(1 − r²))        with r² = x² + y²
/// ```
///
/// and the celestial-to-intermediate matrix is
///
/// ```text
/// C = R_z(−(E + s)) · R_y(d) · R_z(E)
/// ```
///
/// where `R_y`, `R_z` are right-handed rotations about the y- and
/// z-axes with the SOFA sign convention (positive angle rotates a
/// vector clockwise when viewed from the positive axis).
///
/// # Numerical behaviour
///
/// The `r² < 1` guard is implicit: for any realistic CIP offset
/// `|x|, |y| < 10⁻⁴ rad` so `1 − r² > 0.999…`. The `atan(sqrt(r²/(1−r²)))`
/// form matches SOFA and avoids the half-angle truncation that a naive
/// `d = asin(sqrt(r²))` would introduce at sub-µas scales.
pub fn gcrs_to_cirs_matrix(x: Rad, y: Rad, s: Rad) -> Matrix3<f64> {
    let x = x.raw();
    let y = y.raw();
    let s = s.raw();

    let r2 = x * x + y * y;
    let e = if r2 > 0.0 { y.atan2(x) } else { 0.0 };
    let d = (r2 / (1.0 - r2)).sqrt().atan();

    // Explicit `R_z(−(e + s)) · R_y(d) · R_z(e)` with the SOFA sign
    // convention. The matrix is written row-by-row to match ERFA's
    // row-major return layout, so `c2ixys_matrix_matches_erfa` in
    // arika/tests/iau2006_vs_erfa.rs can compare element-for-element.
    rotation_z(-(e + s)) * rotation_y(d) * rotation_z(e)
}

/// Assemble the GCRS→CIRS matrix directly from TT Julian centuries `t`.
/// Combines [`cip_coordinates`] + [`gcrs_to_cirs_matrix`] into a
/// single call.
pub fn gcrs_to_cirs_matrix_at(t: f64) -> Matrix3<f64> {
    let c = cip_coordinates(t);
    gcrs_to_cirs_matrix(c.x, c.y, c.s)
}

/// Right-handed rotation about the z-axis by `psi`, SOFA `iauRz`
/// convention (a positive angle rotates vectors clockwise when
/// viewed from +z).
#[inline]
pub(crate) fn rotation_z(psi: f64) -> Matrix3<f64> {
    let (s, c) = psi.sin_cos();
    Matrix3::new(
        c, s, 0.0, //
        -s, c, 0.0, //
        0.0, 0.0, 1.0,
    )
}

/// Right-handed rotation about the y-axis by `theta`, SOFA `iauRy`
/// convention.
#[inline]
pub(crate) fn rotation_y(theta: f64) -> Matrix3<f64> {
    let (s, c) = theta.sin_cos();
    Matrix3::new(
        c, 0.0, -s, //
        0.0, 1.0, 0.0, //
        s, 0.0, c,
    )
}

/// Right-handed rotation about the x-axis by `phi`, SOFA `iauRx`
/// convention. Shared with [`super::cio_chain`] for the polar motion
/// matrix.
#[inline]
pub(crate) fn rotation_x(phi: f64) -> Matrix3<f64> {
    let (s, c) = phi.sin_cos();
    Matrix3::new(
        1.0, 0.0, 0.0, //
        0.0, c, s, //
        0.0, -s, c,
    )
}

// ─── Shared evaluator ────────────────────────────────────────────

/// Evaluate one CIP / CIO series in microarcseconds.
///
/// `poly_uas` is the polynomial part (six coefficients for `t^0..t^5`),
/// and `terms_by_power` is one slice per power of `t` from `t^0` to
/// `t^4`. The polynomial is evaluated by Horner's rule; each
/// non-polynomial group is summed over its terms and multiplied by
/// `t^j`. Returns the total in microarcseconds so the caller can wrap
/// the result in [`Uas`] and convert to [`Rad`] once at the boundary.
///
/// This helper is private to the `cip` module. All three CIP / CIO
/// series share the same structural shape so a single evaluator keeps
/// behaviour consistent and makes Phase 3A-4 GCRS→CIRS composition
/// trivial to verify.
fn evaluate_series(
    t: f64,
    fa: &FundamentalArguments,
    poly_uas: [f64; 6],
    terms_by_power: [&[CipTerm]; 5],
) -> f64 {
    let mut total = horner6(
        t,
        poly_uas[0],
        poly_uas[1],
        poly_uas[2],
        poly_uas[3],
        poly_uas[4],
        poly_uas[5],
    );
    let mut t_power = 1.0_f64;
    for group in terms_by_power {
        let mut group_sum = 0.0_f64;
        for term in group {
            let arg = compute_argument(fa, &term.arg);
            // sin/cos order matches the TN36 `a_{s,j}) sin + a_{c,j}) cos`
            // form of Eq. (5.16). Y's table header writes the cos term
            // first, but the data columns are still (sin, cos), so this
            // one shared evaluator works for X, Y and s + XY/2 alike.
            let (sin_arg, cos_arg) = arg.sin_cos();
            group_sum += term.sin_uas * sin_arg + term.cos_uas * cos_arg;
        }
        total += group_sum * t_power;
        t_power *= t;
    }
    total
}

/// Compute the argument of one non-polynomial term:
///
/// ```text
/// ARG = Σ_k N_k × F_k
/// ```
///
/// Where `F_0..F_13` are the Delaunay (`l`, `l'`, `F`, `D`, `Ω`) and
/// planetary (`L_Me..L_Ne`, `p_A`) fundamental arguments, and
/// `N_0..N_13` are the integer multipliers stored in `term.arg` in the
/// TN36 column order.
#[inline]
fn compute_argument(fa: &FundamentalArguments, mults: &[i8; 14]) -> f64 {
    // Extract the fourteen fundamental arguments once so the addition
    // chain is cheap and auto-vectorisable.
    let f: [f64; 14] = [
        fa.l.raw(),
        fa.l_prime.raw(),
        fa.f.raw(),
        fa.d.raw(),
        fa.omega.raw(),
        fa.l_me.raw(),
        fa.l_ve.raw(),
        fa.l_e.raw(),
        fa.l_ma.raw(),
        fa.l_j.raw(),
        fa.l_sa.raw(),
        fa.l_u.raw(),
        fa.l_ne.raw(),
        fa.p_a.raw(),
    ];
    let mut arg = 0.0_f64;
    for (n, &fk) in mults.iter().zip(f.iter()) {
        arg += f64::from(*n) * fk;
    }
    arg
}

/// Horner evaluation of `c0 + c1 t + c2 t² + c3 t³ + c4 t⁴ + c5 t⁵`.
#[inline]
fn horner6(t: f64, c0: f64, c1: f64, c2: f64, c3: f64, c4: f64, c5: f64) -> f64 {
    c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * c5))))
}

// ─── Structural tests ────────────────────────────────────────────
//
// Numerical correctness against ERFA `xy06` / `s06` is pinned by the
// integration test `arika/tests/iau2006_vs_erfa.rs`. The unit tests in
// this module cover only structural invariants (finiteness,
// roundtrip shape).

#[cfg(test)]
mod tests {
    use super::*;

    /// Each of `X`, `Y`, `s` must be finite at the J2000 reference
    /// epoch and at typical ±1 century extremes. A botched evaluator
    /// that leaked `NaN` / `Inf` would be immediately caught.
    #[test]
    fn cip_coordinates_are_finite_across_wide_t_range() {
        for &t in &[-1.0, -0.5, -0.1, 0.0, 0.1, 0.5, 1.0] {
            let c = cip_coordinates(t);
            assert!(c.x.is_finite(), "X({t}) = {:?}", c.x);
            assert!(c.y.is_finite(), "Y({t}) = {:?}", c.y);
            assert!(c.s.is_finite(), "s({t}) = {:?}", c.s);
        }
    }

    /// `cip_coordinates` must return the same `(X, Y)` as `cip_xy` and
    /// the same `s` as `cio_locator_s` called with those coordinates.
    /// Pins the one-call helper against its underlying pieces.
    #[test]
    fn cip_coordinates_matches_separate_calls() {
        for &t in &[-0.3, 0.0, 0.2, 0.7] {
            let c = cip_coordinates(t);
            let (x, y) = cip_xy(t);
            let s = cio_locator_s(t, x, y);
            assert_eq!(c.x.raw(), x.raw());
            assert_eq!(c.y.raw(), y.raw());
            assert_eq!(c.s.raw(), s.raw());
        }
    }

    /// At J2000.0 (`t = 0`) the CIP `X` and `Y` are on the order of a
    /// few tens of microarcseconds — small but non-zero, because the
    /// GCRS frame bias (`ξ_0`, `η_0`) already enters the polynomial
    /// part of the TN36 Tables 5.2a/5.2b. A regression that dropped
    /// the constant polynomial term would fail this test.
    #[test]
    fn cip_at_j2000_is_small_but_nonzero() {
        let c = cip_coordinates(0.0);
        // |X|, |Y| are about 2.7e-5 rad at J2000 — bounded above by
        // 1e-4 rad is enough to catch a missing polynomial term
        // without accidentally trigger on ~µas jitter.
        assert!(c.x.raw().abs() < 1e-4);
        assert!(c.y.raw().abs() < 1e-4);
        // s is roughly 1e-8 rad; bound it loosely.
        assert!(c.s.raw().abs() < 1e-6);
        // But neither is exactly zero — the polynomial part is
        // non-trivial.
        assert!(c.x.raw() != 0.0);
        assert!(c.y.raw() != 0.0);
    }

    // ─── GCRS → CIRS matrix (structural) ─────────────────────────

    /// The celestial-to-intermediate matrix must be orthogonal with
    /// determinant +1 (i.e. a proper rotation, not a reflection), for
    /// any `t` — a structural invariant that catches sign errors in
    /// either `R_y` or `R_z` without needing the ERFA oracle.
    #[test]
    fn gcrs_to_cirs_matrix_is_orthogonal_with_determinant_one() {
        for &t in &[-1.0, -0.5, -0.1, 0.0, 0.1, 0.5, 1.0] {
            let m = gcrs_to_cirs_matrix_at(t);
            let mt = m.transpose();
            let should_be_identity = m * mt;
            let identity = Matrix3::<f64>::identity();
            for i in 0..3 {
                for j in 0..3 {
                    let delta = (should_be_identity[(i, j)] - identity[(i, j)]).abs();
                    assert!(delta < 1e-14, "M·Mᵀ at t={t} [{i},{j}] off by {delta}");
                }
            }
            let det = m.determinant();
            assert!(
                (det - 1.0).abs() < 1e-14,
                "det(M) at t={t} = {det}, expected +1"
            );
        }
    }

    /// Applying the matrix to the z-axis should yield the CIP's
    /// direction in the GCRS (which is approximately `(X, Y, √(1−X²−Y²))`).
    /// This pins the sign and axis convention of the rotation.
    #[test]
    fn gcrs_to_cirs_matrix_maps_cip_pole_to_intermediate_pole() {
        use nalgebra::Vector3;

        let c = cip_coordinates(0.5);
        let m = gcrs_to_cirs_matrix(c.x, c.y, c.s);

        // CIP direction in GCRS (the pole of the CIRS equator expressed
        // in GCRS basis): approximately (X, Y, √(1-X²-Y²)).
        let x = c.x.raw();
        let y = c.y.raw();
        let r2 = x * x + y * y;
        let cip_in_gcrs = Vector3::new(x, y, (1.0 - r2).sqrt());

        // Applying M should send this vector to (0, 0, 1) — the z-axis
        // of the CIRS, since the CIP *is* the z-axis of the intermediate
        // frame by construction.
        let mapped = m * cip_in_gcrs;
        assert!((mapped.x).abs() < 1e-13, "mapped.x = {}", mapped.x);
        assert!((mapped.y).abs() < 1e-13, "mapped.y = {}", mapped.y);
        assert!((mapped.z - 1.0).abs() < 1e-13, "mapped.z = {}", mapped.z);
    }

    /// When `(x, y, s)` are all zero the matrix must collapse to the
    /// identity. Catches any accidental non-zero bias that crept into
    /// the sin/cos path via a mis-placed additive constant.
    #[test]
    fn gcrs_to_cirs_matrix_is_identity_when_xys_are_zero() {
        let m = gcrs_to_cirs_matrix(Rad::new(0.0), Rad::new(0.0), Rad::new(0.0));
        let identity = Matrix3::<f64>::identity();
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(m[(i, j)], identity[(i, j)]);
            }
        }
    }
}
