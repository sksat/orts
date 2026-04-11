//! Fundamental arguments of nutation theory (Delaunay + planetary longitudes).
//!
//! # Source
//!
//! [IERS Conventions 2010 (TN36)](https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html)
//! Chapter 5, equations (5.43) and (5.44).
//!
//! # Delaunay arguments (F1..F5)
//!
//! Equations (5.43) give `l`, `l'`, `F`, `D`, `Ω` as 4th-degree polynomials in
//! `t` (Julian centuries of TT since J2000.0). The constant term is given in
//! degrees and the time-varying coefficients in arcseconds per century.
//! Internally we convert the constant term to arcseconds (×3600) and evaluate
//! the entire polynomial in arcseconds, then reduce modulo
//! [`super::TURNAS`] and multiply by [`super::DAS2R`] — the same scheme SOFA
//! uses in `iauFal03`, `iauFalp03`, `iauFaf03`, `iauFad03`, `iauFaom03`.
//!
//! # `l'` and `D`: TN36 exact values
//!
//! TN36 §5.7.2 (p.67) mentions that the historical MHB2000 code rounded the
//! fixed term of `l'` and `D` to five decimal digits (`1287104.79305″` and
//! `1072260.70369″`) for SOFA compatibility. **However, current SOFA and
//! [ERFA](https://github.com/liberfa/erfa) both use the TN36 Eq. (5.43)
//! exact values** (`1287104.793048″` and `1072260.703692″`) — verified by
//! recovering the constants from `erfa.falp03(0.0)` / `erfa.fad03(0.0)`.
//! kaname follows the exact values so that the ERFA-generated fixture in
//! `kaname/tests/fixtures/iau2006_erfa_reference.json` agrees bit-level.
//! The 2 μas historical discrepancy is well below 10⁻⁹ arcsec CIP accuracy.

// Polynomial coefficients are transcribed verbatim from TN36 / ERFA so
// that a diff against the source document stays line-for-line readable.
// The unusual formatting (mixing integer-part digit groups with long
// fractional parts) is the natural scientific form and does not benefit
// from clippy's `inconsistent_digit_grouping` rewrite.
#![allow(clippy::inconsistent_digit_grouping)]
#![allow(clippy::excessive_precision)]
//!
//! # Planetary longitudes (F6..F14)
//!
//! Equations (5.44) give the mean longitudes of the eight planets plus the
//! general precession rate `p_A` from Kinoshita and Souchay (1990). These are
//! expressed in radians with `t` in Julian centuries, so only a linear (F6..F13)
//! or quadratic (F14) evaluation and a 2π reduction are required.

use std::f64::consts::TAU;

use super::{DAS2R, Rad, TURNAS};

/// All fundamental arguments evaluated at a single `t` value, as
/// [`Rad`]-typed angles.
///
/// Fields are grouped as `F1..F5` (Delaunay) and `F6..F14` (planetary
/// longitudes + general precession). Values are reduced with SOFA's `fmod`
/// convention — they may be negative for negative `t` since `fmod`
/// preserves the sign of the dividend.
#[derive(Debug, Clone, Copy)]
pub struct FundamentalArguments {
    /// F1 = `l`, mean anomaly of the Moon (Delaunay).
    pub l: Rad,
    /// F2 = `l'`, mean anomaly of the Sun (Delaunay).
    pub l_prime: Rad,
    /// F3 = `F = L − Ω`, mean argument of latitude of the Moon (Delaunay).
    pub f: Rad,
    /// F4 = `D`, mean elongation of the Moon from the Sun (Delaunay).
    pub d: Rad,
    /// F5 = `Ω`, mean longitude of the Moon's ascending node (Delaunay).
    pub omega: Rad,

    /// F6 = `L_Me`, mean longitude of Mercury.
    pub l_me: Rad,
    /// F7 = `L_Ve`, mean longitude of Venus.
    pub l_ve: Rad,
    /// F8 = `L_E`, mean longitude of Earth.
    pub l_e: Rad,
    /// F9 = `L_Ma`, mean longitude of Mars.
    pub l_ma: Rad,
    /// F10 = `L_J`, mean longitude of Jupiter.
    pub l_j: Rad,
    /// F11 = `L_Sa`, mean longitude of Saturn.
    pub l_sa: Rad,
    /// F12 = `L_U`, mean longitude of Uranus.
    pub l_u: Rad,
    /// F13 = `L_Ne`, mean longitude of Neptune.
    pub l_ne: Rad,
    /// F14 = `p_A`, general precession in longitude
    /// (Kinoshita & Souchay 1990, quadratic in `t`).
    pub p_a: Rad,
}

impl FundamentalArguments {
    /// Evaluate every fundamental argument at TT Julian centuries since
    /// J2000.0.
    pub fn evaluate(t: f64) -> Self {
        Self {
            l: fa_l(t),
            l_prime: fa_l_prime(t),
            f: fa_f(t),
            d: fa_d(t),
            omega: fa_omega(t),
            l_me: fa_l_me(t),
            l_ve: fa_l_ve(t),
            l_e: fa_l_e(t),
            l_ma: fa_l_ma(t),
            l_j: fa_l_j(t),
            l_sa: fa_l_sa(t),
            l_u: fa_l_u(t),
            l_ne: fa_l_ne(t),
            p_a: fa_p_a(t),
        }
    }
}

// ─── Delaunay arguments (TN36 Eq. 5.43) ──────────────────────────
//
// Polynomial in arcseconds, reduced modulo a full turn (1_296_000″), then
// converted to radians. The constant term is converted from degrees to
// arcseconds (×3600).

/// Evaluate a polynomial `c0 + c1·t + c2·t² + c3·t³ + c4·t⁴` via Horner's rule.
#[inline]
fn horner5(t: f64, c0: f64, c1: f64, c2: f64, c3: f64, c4: f64) -> f64 {
    c0 + t * (c1 + t * (c2 + t * (c3 + t * c4)))
}

/// Reduce an angle given in arcseconds to `(−TURNAS, TURNAS)` and convert
/// to a [`Rad`]. Matches SOFA's `fmod(…, TURNAS) * DAS2R`; the sign of the
/// input is preserved.
#[inline]
fn arcsec_mod_turn_to_rad(arcsec: f64) -> Rad {
    Rad::new((arcsec % TURNAS) * DAS2R)
}

/// `F1 = l`, mean anomaly of the Moon. TN36 Eq. (5.43), ERFA `fal03`.
pub fn fa_l(t: f64) -> Rad {
    // 134.96340251° × 3600 = 485868.249036″
    let a = horner5(
        t,
        485868.249036,
        1717915923.2178,
        31.8792,
        0.051635,
        -0.00024470,
    );
    arcsec_mod_turn_to_rad(a)
}

/// `F2 = l'`, mean anomaly of the Sun. TN36 Eq. (5.43), ERFA `falp03`.
pub fn fa_l_prime(t: f64) -> Rad {
    // 357.52910918° × 3600 = 1287104.793048″
    let a = horner5(
        t,
        1287104.793048,
        129596581.0481,
        -0.5532,
        0.000136,
        -0.00001149,
    );
    arcsec_mod_turn_to_rad(a)
}

/// `F3 = F = L − Ω`, mean argument of latitude of the Moon.
/// TN36 Eq. (5.43), ERFA `faf03`.
pub fn fa_f(t: f64) -> Rad {
    // 93.27209062° × 3600 = 335779.526232″
    let a = horner5(
        t,
        335779.526232,
        1739527262.8478,
        -12.7512,
        -0.001037,
        0.00000417,
    );
    arcsec_mod_turn_to_rad(a)
}

/// `F4 = D`, mean elongation of the Moon from the Sun.
/// TN36 Eq. (5.43), ERFA `fad03`.
pub fn fa_d(t: f64) -> Rad {
    // 297.85019547° × 3600 = 1072260.703692″
    let a = horner5(
        t,
        1072260.703692,
        1602961601.2090,
        -6.3706,
        0.006593,
        -0.00003169,
    );
    arcsec_mod_turn_to_rad(a)
}

/// `F5 = Ω`, mean longitude of the Moon's ascending node.
/// TN36 Eq. (5.43), ERFA `faom03`.
pub fn fa_omega(t: f64) -> Rad {
    // 125.04455501° × 3600 = 450160.398036″
    let a = horner5(
        t,
        450160.398036,
        -6962890.5431,
        7.4722,
        0.007702,
        -0.00005939,
    );
    arcsec_mod_turn_to_rad(a)
}

// ─── Planetary longitudes (TN36 Eq. 5.44) ────────────────────────
//
// Already in radians. Linear in t except F14 which is quadratic.

/// `F6 = L_Me`, mean longitude of Mercury. TN36 Eq. (5.44), ERFA `fame03`.
pub fn fa_l_me(t: f64) -> Rad {
    Rad::new((4.402608842 + 2608.7903141574 * t) % TAU)
}

/// `F7 = L_Ve`, mean longitude of Venus. TN36 Eq. (5.44), ERFA `fave03`.
pub fn fa_l_ve(t: f64) -> Rad {
    Rad::new((3.176146697 + 1021.3285546211 * t) % TAU)
}

/// `F8 = L_E`, mean longitude of Earth. TN36 Eq. (5.44), ERFA `fae03`.
pub fn fa_l_e(t: f64) -> Rad {
    Rad::new((1.753470314 + 628.3075849991 * t) % TAU)
}

/// `F9 = L_Ma`, mean longitude of Mars. TN36 Eq. (5.44), ERFA `fama03`.
pub fn fa_l_ma(t: f64) -> Rad {
    Rad::new((6.203480913 + 334.0612426700 * t) % TAU)
}

/// `F10 = L_J`, mean longitude of Jupiter. TN36 Eq. (5.44), ERFA `faju03`.
pub fn fa_l_j(t: f64) -> Rad {
    Rad::new((0.599546497 + 52.9690962641 * t) % TAU)
}

/// `F11 = L_Sa`, mean longitude of Saturn. TN36 Eq. (5.44), ERFA `fasa03`.
pub fn fa_l_sa(t: f64) -> Rad {
    Rad::new((0.874016757 + 21.3299104960 * t) % TAU)
}

/// `F12 = L_U`, mean longitude of Uranus. TN36 Eq. (5.44), ERFA `faur03`.
pub fn fa_l_u(t: f64) -> Rad {
    Rad::new((5.481293872 + 7.4781598567 * t) % TAU)
}

/// `F13 = L_Ne`, mean longitude of Neptune. TN36 Eq. (5.44), ERFA `fane03`.
///
/// Note: TN36 p.68 observes that the SOFA/ERFA implementation uses the
/// original MHB2000 expression `5.311886287 + 3.8133035638 × t` instead of
/// the exact Eq. (5.44) form. The difference produces a CIP error of
/// `< 0.01 μas` per century, well below required accuracy, so we match
/// ERFA.
pub fn fa_l_ne(t: f64) -> Rad {
    Rad::new((5.311886287 + 3.8133035638 * t) % TAU)
}

/// `F14 = p_A`, general precession in longitude. TN36 Eq. (5.44) / Kinoshita
/// and Souchay (1990). Quadratic in `t`.
///
/// Unlike the other planetary longitudes, `p_A` is not reduced modulo 2π —
/// its growth over the lifetime of the series is small relative to a full
/// turn, and ERFA's `fapa03` returns the raw value.
pub fn fa_p_a(t: f64) -> Rad {
    Rad::new(0.02438175 * t + 0.00000538691 * t * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests in this module only cover *structural* properties of the
    // evaluators — returned angles must be finite, bounded where expected,
    // and the struct dispatcher must call the right functions. Numerical
    // correctness (cross-validation against ERFA at multiple t values) is
    // pinned by the integration test at `kaname/tests/iau2006_vs_erfa.rs`,
    // which loads the ERFA-generated fixture in
    // `kaname/tests/fixtures/iau2006_erfa_reference.json`.

    /// All `fa_*` evaluators must return finite values for a wide range of
    /// `t`, including several centuries past the IAU 2000A validity window.
    /// Catches accidental `f64::NAN` / `f64::INFINITY` that a botched
    /// polynomial or bad `fmod` would produce.
    #[test]
    fn all_evaluators_return_finite_across_wide_t_range() {
        for &t in &[-10.0, -1.0, -0.5, -0.1, 0.0, 0.1, 0.5, 1.0, 10.0] {
            let fa = FundamentalArguments::evaluate(t);
            for (name, value) in [
                ("l", fa.l),
                ("l_prime", fa.l_prime),
                ("f", fa.f),
                ("d", fa.d),
                ("omega", fa.omega),
                ("l_me", fa.l_me),
                ("l_ve", fa.l_ve),
                ("l_e", fa.l_e),
                ("l_ma", fa.l_ma),
                ("l_j", fa.l_j),
                ("l_sa", fa.l_sa),
                ("l_u", fa.l_u),
                ("l_ne", fa.l_ne),
                ("p_a", fa.p_a),
            ] {
                assert!(
                    value.is_finite(),
                    "{name} at t={t} was non-finite: {value:?}"
                );
            }
        }
    }

    /// Delaunay arguments pass through `fmod(..., TURNAS) * DAS2R`, so
    /// the magnitude of the returned radian angle must be strictly less
    /// than `2π`. Planetary longitudes are reduced via `% TAU` and share
    /// the same bound.
    ///
    /// The general precession `p_a = F14` is *not* reduced modulo 2π, but
    /// grows slowly enough that `|p_a(t)|` for `|t| ≤ 10` centuries is
    /// below 1 rad — keep it off the bound check.
    #[test]
    fn reduced_arguments_stay_within_one_turn() {
        for &t in &[-10.0, -1.0, 0.0, 1.0, 10.0] {
            let fa = FundamentalArguments::evaluate(t);
            for (name, value) in [
                ("l", fa.l),
                ("l_prime", fa.l_prime),
                ("f", fa.f),
                ("d", fa.d),
                ("omega", fa.omega),
                ("l_me", fa.l_me),
                ("l_ve", fa.l_ve),
                ("l_e", fa.l_e),
                ("l_ma", fa.l_ma),
                ("l_j", fa.l_j),
                ("l_sa", fa.l_sa),
                ("l_u", fa.l_u),
                ("l_ne", fa.l_ne),
            ] {
                assert!(
                    value.raw().abs() < TAU,
                    "{name} at t={t} exceeded 2π: {value:?}"
                );
            }
        }
    }

    /// The struct constructor must dispatch every field to its matching
    /// `fa_*` function. A regression that swaps two fields would silently
    /// feed wrong values to the nutation series in Phase 3A-3, so we pin
    /// the dispatch explicitly rather than relying on the obvious-looking
    /// implementation.
    #[test]
    fn evaluate_dispatches_each_field_to_its_evaluator() {
        let t = 0.1234;
        let fa = FundamentalArguments::evaluate(t);
        assert_eq!(fa.l.raw(), fa_l(t).raw());
        assert_eq!(fa.l_prime.raw(), fa_l_prime(t).raw());
        assert_eq!(fa.f.raw(), fa_f(t).raw());
        assert_eq!(fa.d.raw(), fa_d(t).raw());
        assert_eq!(fa.omega.raw(), fa_omega(t).raw());
        assert_eq!(fa.l_me.raw(), fa_l_me(t).raw());
        assert_eq!(fa.l_ve.raw(), fa_l_ve(t).raw());
        assert_eq!(fa.l_e.raw(), fa_l_e(t).raw());
        assert_eq!(fa.l_ma.raw(), fa_l_ma(t).raw());
        assert_eq!(fa.l_j.raw(), fa_l_j(t).raw());
        assert_eq!(fa.l_sa.raw(), fa_l_sa(t).raw());
        assert_eq!(fa.l_u.raw(), fa_l_u(t).raw());
        assert_eq!(fa.l_ne.raw(), fa_l_ne(t).raw());
        assert_eq!(fa.p_a.raw(), fa_p_a(t).raw());
    }
}
