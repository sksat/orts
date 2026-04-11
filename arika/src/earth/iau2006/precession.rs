//! IAU 2006 precession polynomial expressions.
//!
//! # Source
//!
//! [IERS Conventions 2010 (TN36)](https://www.iers.org/IERS/EN/Publications/TechnicalNotes/tn36.html)
//! Chapter 5, equations (5.38), (5.39), (5.40), adopted by IAU 2006
//! Resolution B1 (Capitaine, Wallace & Chapront 2003c, "P03 precession").
//!
//! # What this file provides
//!
//! All IAU 2006 precession expressions are 5th-degree polynomials in
//! `t = (JD_TT − 2451545.0) / 36525` (Julian centuries of TT since J2000.0).
//! The values returned are the **secular** / polynomial parts only —
//! nutation and frame bias contributions are added on top by Phase 3A-3
//! when the full GCRS → CIRS matrix is assembled.
//!
//! Two sets of angles are published by TN36 for the same underlying
//! precession theory, and both are needed by downstream code:
//!
//! - [`fukushima_williams`] (TN36 Eq. 5.40, the Williams 1994 / Fukushima
//!   2003 4-rotation decomposition): `γ̄`, `φ̄`, `ψ̄`, `ε_A`. This is the
//!   set ERFA's `pfw06` returns and it is the natural input to the
//!   Fukushima-Williams assembly of the GCRS → ecliptic-of-date frame.
//!
//! - [`ecliptic_precession_angles`] (TN36 Eq. 5.39 + 5.40, the Lieske et
//!   al. 1977 angles): `ψ_A`, `ω_A`, `χ_A`, plus `ε_A`. These are the
//!   classical equinox-based angles. They are exposed for Phase 3A-3's
//!   `X = sin ω sin ψ` decomposition (TN36 Eq. 5.22) and for future
//!   equinox-based code.
//!
//! `ε_A` (the mean obliquity of date) is computed once and returned by
//! both accessors.
//!
//! # Independent variable
//!
//! As elsewhere in this module, `t` is **TT** Julian centuries since
//! J2000.0. Callers obtain it from
//! [`crate::epoch::Epoch::<crate::epoch::Tt>::centuries_since_j2000`].
//!
//! # Units
//!
//! The TN36 polynomials are given with coefficients in arcseconds per
//! century power. Internally the module evaluates each polynomial in
//! arcseconds and wraps the result as [`Arcsec`], then converts to
//! [`Rad`] at the public API boundary.

// Polynomial coefficients are transcribed verbatim from TN36 / ERFA so
// that a diff against the source document stays line-for-line readable.
// The unusual formatting is the natural scientific form and does not
// benefit from clippy's `inconsistent_digit_grouping` rewrite.
#![allow(clippy::inconsistent_digit_grouping)]
#![allow(clippy::excessive_precision)]

use super::{Arcsec, Rad};

// ─── IAU 2006 precession constants ───────────────────────────────

/// Mean obliquity of the ecliptic at J2000.0 (`ε₀`).
///
/// TN36 §5.6.3 p.63: "the IAU 2006 obliquity is different from the IAU
/// 1980 obliquity ... with `ε₀ = 84381.406″` for the mean obliquity at
/// J2000.0 of the ecliptic (while the IAU 2000 value was `84381.448″`)."
pub const EPSILON_0: Arcsec = Arcsec::new(84_381.406);

// ─── 4-rotation Fukushima-Williams angles (TN36 Eq. 5.40) ────────

/// The Fukushima-Williams angles at a given `t`, all as [`Rad`].
///
/// Returned by [`fukushima_williams`]. Field order matches ERFA's
/// `pfw06(date1, date2) -> (gamb, phib, psib, epsa)` for easy cross-check.
///
/// Only `Debug` is derived — consumers destructure or access fields
/// individually through the (`Copy`) [`Rad`] type.
#[derive(Debug)]
pub struct FukushimaWilliamsAngles {
    /// `γ̄` — GCRS right ascension of the intersection of the ecliptic
    /// of date with the GCRS equator. TN36 Eq. (5.40) `γ̄`.
    pub gamma_bar: Rad,
    /// `φ̄` — obliquity of the ecliptic of date on the GCRS equator.
    /// TN36 Eq. (5.40) `φ̄`.
    pub phi_bar: Rad,
    /// `ψ̄` — precession angle plus bias in longitude along the ecliptic
    /// of date. TN36 Eq. (5.40) `ψ̄`.
    pub psi_bar: Rad,
    /// `ε_A` — mean obliquity of date. TN36 Eq. (5.40) `ε_A`, identical
    /// to the one returned in [`EclipticPrecessionAngles::eps_a`].
    pub eps_a: Rad,
}

/// Evaluate the Fukushima-Williams precession angles at TT Julian
/// centuries `t`. Equivalent to ERFA's `pfw06` (used as the oracle in
/// `arika/tests/iau2006_vs_erfa.rs`).
pub fn fukushima_williams(t: f64) -> FukushimaWilliamsAngles {
    FukushimaWilliamsAngles {
        gamma_bar: gamma_bar_arcsec(t).to_radians(),
        phi_bar: phi_bar_arcsec(t).to_radians(),
        psi_bar: psi_bar_arcsec(t).to_radians(),
        eps_a: eps_a_arcsec(t).to_radians(),
    }
}

// ─── Lieske 1977 ecliptic-and-equator angles (TN36 Eq. 5.39/5.40) ─

/// The classical Lieske-style precession angles at a given `t`, all as
/// [`Rad`]. Returned by [`ecliptic_precession_angles`].
///
/// Only `Debug` is derived (see [`FukushimaWilliamsAngles`]).
#[derive(Debug)]
pub struct EclipticPrecessionAngles {
    /// `ψ_A` — precession of the ecliptic in longitude. TN36 Eq. (5.39).
    pub psi_a: Rad,
    /// `ω_A` — obliquity of the mean ecliptic of date on the J2000.0
    /// mean equator. TN36 Eq. (5.39).
    pub omega_a: Rad,
    /// `χ_A` — precession of the ecliptic along the equator
    /// (right-ascension component of ecliptic precession).
    /// TN36 Eq. (5.40).
    pub chi_a: Rad,
    /// `ε_A` — mean obliquity of date. TN36 Eq. (5.40).
    pub eps_a: Rad,
}

/// Evaluate the Lieske ecliptic-and-equator precession angles at TT
/// Julian centuries `t`.
pub fn ecliptic_precession_angles(t: f64) -> EclipticPrecessionAngles {
    EclipticPrecessionAngles {
        psi_a: psi_a_arcsec(t).to_radians(),
        omega_a: omega_a_arcsec(t).to_radians(),
        chi_a: chi_a_arcsec(t).to_radians(),
        eps_a: eps_a_arcsec(t).to_radians(),
    }
}

// ─── Polynomial kernels (all in arcseconds) ──────────────────────
//
// Coefficients transcribed from IERS Conventions 2010 (TN36) Eq. (5.39)
// and (5.40). Each helper evaluates its polynomial with Horner's rule
// and returns the raw value wrapped as `Arcsec`. The wrappers in the
// public accessors (`fukushima_williams`, `ecliptic_precession_angles`)
// convert to `Rad`.

/// Horner evaluation of `c0 + c1 t + c2 t² + c3 t³ + c4 t⁴ + c5 t⁵`.
#[inline]
fn horner6(t: f64, c0: f64, c1: f64, c2: f64, c3: f64, c4: f64, c5: f64) -> f64 {
    c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * c5))))
}

/// `ψ_A` in arcseconds. TN36 Eq. (5.39).
fn psi_a_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        0.0,
        5038.481507,
        -1.0790069,
        -0.00114045,
        0.000132851,
        -0.0000000951,
    ))
}

/// `ω_A` in arcseconds. TN36 Eq. (5.39). Note the constant term is
/// `ε₀` (the IAU 2006 J2000 obliquity).
fn omega_a_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        EPSILON_0.raw(),
        -0.025754,
        0.0512623,
        -0.00772503,
        -0.000000467,
        0.0000003337,
    ))
}

/// `χ_A` in arcseconds. TN36 Eq. (5.40).
fn chi_a_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        0.0,
        10.556403,
        -2.3814292,
        -0.00121197,
        0.000170663,
        -0.0000000560,
    ))
}

/// `ε_A` in arcseconds. TN36 Eq. (5.40). Constant term is `ε₀`.
///
/// # Known TN36 text errata for the `t⁴` coefficient
///
/// The printed TN36 Eq. (5.40) shows `-0.00000576″` for the `t⁴`
/// coefficient. This is a transcription typo: the correct value from
/// IAU 2006 Resolution B1 / Capitaine et al. 2003c / Capitaine &
/// Wallace 2006 is `-0.000000576″` (ten times smaller). SOFA's
/// `iauObl06` and ERFA's `eraObl06` use the correct value, and arika
/// matches them so that the ERFA oracle fixture in
/// `arika/tests/iau2006_vs_erfa.rs` passes to 10⁻¹² rad at `|t| ≤ 1`
/// century. The wrong TN36 literal value would offset `ε_A(1)` by
/// ~5.2 µas, visible at the 10⁻¹¹ rad level.
fn eps_a_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        EPSILON_0.raw(),
        -46.836769,
        -0.0001831,
        0.00200340,
        -0.000000576,
        -0.0000000434,
    ))
}

/// `γ̄` in arcseconds. TN36 Eq. (5.40).
fn gamma_bar_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        -0.052928,
        10.556378,
        0.4932044,
        -0.00031238,
        -0.000002788,
        0.0000000260,
    ))
}

/// `φ̄` in arcseconds. TN36 Eq. (5.40).
fn phi_bar_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        84381.412819,
        -46.811016,
        0.0511268,
        0.00053289,
        -0.000000440,
        -0.0000000176,
    ))
}

/// `ψ̄` in arcseconds. TN36 Eq. (5.40).
fn psi_bar_arcsec(t: f64) -> Arcsec {
    Arcsec::new(horner6(
        t,
        -0.041775,
        5038.481484,
        1.5584175,
        -0.00018522,
        -0.000026452,
        -0.0000000148,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests here only cover *structural* invariants of the
    // polynomials. Cross-validation against ERFA's `pfw06` at multiple
    // `t` values is pinned by the integration test
    // `arika/tests/iau2006_vs_erfa.rs` which loads the fixture at
    // `arika/tests/fixtures/iau2006_erfa_reference.json`.

    /// At J2000.0 the `φ̄` angle must be `84381.412819″` — this value is
    /// published as a literal constant in TN36 Eq. (5.40) and differs
    /// from `ε₀ = 84381.406″` by 6.819 mas. Smart-friend's Phase 3 review
    /// specifically warned that these two angles are easy to confuse.
    #[test]
    fn phi_bar_at_j2000_differs_from_epsilon_zero_by_6819_mas() {
        let fw = fukushima_williams(0.0);
        let phi_bar_arcsec = fw.phi_bar.raw() / super::super::DAS2R;
        let eps_0_arcsec = EPSILON_0.raw();
        let delta_mas = (phi_bar_arcsec - eps_0_arcsec) * 1e3;
        assert!(
            (delta_mas - 6.819).abs() < 1e-6,
            "phi_bar − ε₀ = {delta_mas} mas, expected 6.819 mas"
        );
    }

    /// At J2000.0 the `ε_A` angle is exactly `ε₀`, by construction of
    /// the TN36 Eq. (5.40) polynomial (the constant term is the IAU
    /// 2006 `84381.406″`). This pins the convention used by both
    /// [`fukushima_williams`] and [`ecliptic_precession_angles`].
    #[test]
    fn eps_a_at_j2000_equals_epsilon_zero() {
        let fw = fukushima_williams(0.0);
        let le = ecliptic_precession_angles(0.0);
        assert_eq!(fw.eps_a.raw(), EPSILON_0.to_radians().raw());
        assert_eq!(le.eps_a.raw(), EPSILON_0.to_radians().raw());
    }

    /// At J2000.0, `ψ_A`, `χ_A`, `γ̄`, `ψ̄` have no constant term (or a
    /// sub-arcsecond one in the Fukushima-Williams case). This catches
    /// sign errors and off-by-one index bugs in the Horner evaluator.
    #[test]
    fn zero_constant_term_angles_at_j2000() {
        let le = ecliptic_precession_angles(0.0);
        assert_eq!(le.psi_a.raw(), 0.0);
        assert_eq!(le.chi_a.raw(), 0.0);

        // γ̄ has a −0.052928″ constant term (frame bias contribution).
        let fw = fukushima_williams(0.0);
        let gamma_bar_arcsec = fw.gamma_bar.raw() / super::super::DAS2R;
        assert!((gamma_bar_arcsec - (-0.052928)).abs() < 1e-12);

        // ψ̄ has a −0.041775″ constant term (frame bias contribution).
        let psi_bar_arcsec = fw.psi_bar.raw() / super::super::DAS2R;
        assert!((psi_bar_arcsec - (-0.041775)).abs() < 1e-12);
    }

    /// All precession polynomials must return finite values for `t`
    /// well outside the IAU 2006 validity window (±1 century is the
    /// nominal range; we test ±10 centuries to catch NaN/overflow).
    #[test]
    fn precession_polynomials_are_finite_over_wide_t_range() {
        for &t in &[-10.0, -1.0, -0.5, -0.1, 0.0, 0.1, 0.5, 1.0, 10.0] {
            let fw = fukushima_williams(t);
            assert!(fw.gamma_bar.is_finite());
            assert!(fw.phi_bar.is_finite());
            assert!(fw.psi_bar.is_finite());
            assert!(fw.eps_a.is_finite());

            let le = ecliptic_precession_angles(t);
            assert!(le.psi_a.is_finite());
            assert!(le.omega_a.is_finite());
            assert!(le.chi_a.is_finite());
            assert!(le.eps_a.is_finite());
        }
    }

    /// `ω_A(0) = ε₀` (TN36 Eq. 5.39). Pins the Lieske obliquity
    /// constant term which is easy to confuse with `ε_A`.
    #[test]
    fn omega_a_at_j2000_equals_epsilon_zero() {
        let le = ecliptic_precession_angles(0.0);
        assert_eq!(le.omega_a.raw(), EPSILON_0.to_radians().raw());
    }
}
