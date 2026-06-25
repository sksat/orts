//! IAU-76/FK5 equinox-based Earth-orientation math (the classical reduction).
//!
//! These are the building blocks for the TEME↔GCRS transform (see
//! [`crate::earth::teme`]): the IAU 1976 precession, the IAU 1980 nutation
//! (full 106-term series, [`nutation_table`]), the IAU 1980 mean obliquity, the
//! equation of the equinoxes, and GMST 1982. They reproduce ERFA's `pmat76` /
//! `nut80` / `obl80` / `eqeq94` / `gmst82` and are cross-validated against ERFA
//! in `arika/tests/teme_vs_erfa.rs`.
//!
//! This is the *equinox-based* (classical) reduction, distinct from the CIO
//! chain in [`crate::earth::iau2006`]; it exists specifically to interpret
//! TEME (the SGP4/TLE output frame), which is defined in this framework.
//!
//! All angle quantities are radians; the time argument `t` is TT Julian
//! centuries since J2000.0 (except GMST 1982, which is a function of UT1).

mod nutation_table;

use nalgebra::Matrix3;
use nutation_table::IAU1980_NUTATION;

// In `no_std` builds the trig methods resolve via libm through this trait;
// under `std` the inherent methods shadow it.
#[allow(unused_imports)]
use crate::math::F64Ext;

/// Arcseconds to radians.
const DAS2R: f64 = core::f64::consts::PI / (180.0 * 3600.0);
/// 0.1 milliarcseconds (the nutation-table unit) to radians.
const U2R: f64 = DAS2R * 1.0e-4;
const TAU: f64 = core::f64::consts::TAU;
/// Seconds of time to radians (`2π / 86400`).
const DS2R: f64 = TAU / 86400.0;
const J2000_JD: f64 = 2451545.0;
const JULIAN_CENTURY_DAYS: f64 = 36525.0;
const DAY_SECONDS: f64 = 86400.0;

/// The five IAU 1980 Delaunay fundamental arguments (l, l', F, D, Ω) [rad].
///
/// Reproduces the argument expressions in ERFA `nut80`. (These differ from the
/// IAU 2006 expressions in [`crate::earth::iau2006`] at the sub-mas level, so
/// they are kept separate to match the IAU 1980 nutation series exactly.) The
/// arguments enter only as `sin`/`cos` of integer combinations, so they are not
/// range-reduced here.
fn delaunay_arguments(t: f64) -> [f64; 5] {
    // `x % 1.0` matches C `fmod(x, 1.0)` (sign of the dividend), so the
    // integer-revolution term reproduces ERFA exactly.
    let rev = |n: f64| (n * t % 1.0) * TAU;
    [
        // l: mean anomaly of the Moon.
        (485866.733 + (715922.633 + (31.310 + 0.064 * t) * t) * t) * DAS2R + rev(1325.0),
        // l': mean anomaly of the Sun.
        (1287099.804 + (1292581.224 + (-0.577 - 0.012 * t) * t) * t) * DAS2R + rev(99.0),
        // F: mean longitude of the Moon minus that of its node.
        (335778.877 + (295263.137 + (-13.257 + 0.011 * t) * t) * t) * DAS2R + rev(1342.0),
        // D: mean elongation of the Moon from the Sun.
        (1072261.307 + (1105601.328 + (-6.891 + 0.019 * t) * t) * t) * DAS2R + rev(1236.0),
        // Ω: mean longitude of the ascending node of the lunar orbit.
        (450160.280 + (-482890.539 + (7.455 + 0.008 * t) * t) * t) * DAS2R + rev(-5.0),
    ]
}

/// IAU 1980 nutation in longitude and obliquity `(Δψ, Δε)` [rad] at TT
/// centuries `t`. Reproduces ERFA `nut80`.
pub fn nutation(t: f64) -> (f64, f64) {
    let [l, lp, f, d, om] = delaunay_arguments(t);
    let mut dpsi = 0.0;
    let mut deps = 0.0;
    // Accumulate smallest term first (table is ordered largest-first), matching
    // ERFA's summation order for bit-level agreement.
    for term in IAU1980_NUTATION.iter().rev() {
        let [ml, mlp, mf, md, mom] = term.mult;
        let arg = ml as f64 * l + mlp as f64 * lp + mf as f64 * f + md as f64 * d + mom as f64 * om;
        dpsi += (term.longitude[0] + term.longitude[1] * t) * arg.sin();
        deps += (term.obliquity[0] + term.obliquity[1] * t) * arg.cos();
    }
    (dpsi * U2R, deps * U2R)
}

/// IAU 1980 mean obliquity of the ecliptic `ε̄` [rad]. Reproduces ERFA `obl80`.
pub fn mean_obliquity(t: f64) -> f64 {
    DAS2R * (84381.448 + (-46.8150 + (-0.00059 + 0.001813 * t) * t) * t)
}

/// IAU 1976 accumulated precession angles `(ζ, θ, z)` [rad] from J2000 to TT
/// centuries `t`. Reproduces ERFA `prec76` with a J2000 fixed epoch.
pub fn precession_angles(t: f64) -> (f64, f64, f64) {
    let zeta = (2306.2181 + (0.30188 + 0.017998 * t) * t) * t * DAS2R;
    let z = (2306.2181 + (1.09468 + 0.018203 * t) * t) * t * DAS2R;
    let theta = (2004.3109 + (-0.42665 - 0.041833 * t) * t) * t * DAS2R;
    (zeta, theta, z)
}

/// Equation of the equinoxes [rad] at TT centuries `t` (IAU 1994 model, ERFA
/// `eqeq94`): `Δψ·cos ε̄` plus the two complementary node terms.
pub fn equation_of_equinoxes(t: f64) -> f64 {
    let (dpsi, _) = nutation(t);
    let eps0 = mean_obliquity(t);
    let om = delaunay_arguments(t)[4];
    dpsi * eps0.cos() + DAS2R * (0.00264 * om.sin() + 0.000063 * (om + om).sin())
}

/// Greenwich Mean Sidereal Time [rad] from a UT1 Julian date (IAU 1982
/// expression, ERFA `gmst82`), normalized to `[0, 2π)`.
pub fn gmst1982(ut1_jd: f64) -> f64 {
    // The leading constant carries a −12 h offset because UT1 is a Julian date
    // (which begins at noon).
    const A: f64 = 24110.54841 - DAY_SECONDS / 2.0;
    const B: f64 = 8640184.812866;
    const C: f64 = 0.093104;
    const D: f64 = -6.2e-6;
    let t = (ut1_jd - J2000_JD) / JULIAN_CENTURY_DAYS;
    let day_fraction_seconds = DAY_SECONDS * (ut1_jd % 1.0);
    let gmst = DS2R * ((A + (B + (C + D * t) * t) * t) + day_fraction_seconds);
    let w = gmst % TAU;
    if w < 0.0 { w + TAU } else { w }
}

// ─── Elemental rotations (ERFA / SOFA passive convention) ───
// `v' = R · v` where R rotates the coordinate frame about the axis.

fn rot_x(phi: f64) -> Matrix3<f64> {
    let (s, c) = (phi.sin(), phi.cos());
    Matrix3::new(1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c)
}

fn rot_y(theta: f64) -> Matrix3<f64> {
    let (s, c) = (theta.sin(), theta.cos());
    Matrix3::new(c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c)
}

fn rot_z(psi: f64) -> Matrix3<f64> {
    let (s, c) = (psi.sin(), psi.cos());
    Matrix3::new(c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0)
}

/// IAU 1976 precession matrix `r_MOD = P · r_J2000` (ERFA `pmat76`):
/// `R3(−z) · R2(θ) · R3(−ζ)`.
pub(crate) fn precession_matrix(t: f64) -> Matrix3<f64> {
    let (zeta, theta, z) = precession_angles(t);
    rot_z(-z) * rot_y(theta) * rot_z(-zeta)
}

/// IAU 1980 nutation matrix `r_TOD = N · r_MOD` (ERFA `numat`):
/// `R1(−ε̄−Δε) · R3(−Δψ) · R1(ε̄)`.
pub(crate) fn nutation_matrix(t: f64) -> Matrix3<f64> {
    let eps0 = mean_obliquity(t);
    let (dpsi, deps) = nutation(t);
    rot_x(-eps0 - deps) * rot_z(-dpsi) * rot_x(eps0)
}

/// Combined IAU-76/80 precession-nutation matrix `r_TOD = M · r_J2000`
/// (ERFA `pnm80`): `N · P`.
pub(crate) fn prec_nut_matrix(t: f64) -> Matrix3<f64> {
    nutation_matrix(t) * precession_matrix(t)
}

/// TEME → J2000 (≈GCRS) rotation matrix at TT centuries `t`.
///
/// `r_J2000 = pnm80ᵀ · R3(−Eqe) · r_TEME`: TEME differs from the true equator,
/// true equinox (TOD) frame by the equation of the equinoxes about the pole,
/// then the IAU-76/80 precession-nutation maps TOD to J2000. This is the J2000
/// (FK5) dynamical frame; the J2000→GCRS frame bias (~tens of mas) is neglected
/// (≪ the SGP4 error this serves).
pub(crate) fn teme_to_j2000_matrix(t: f64) -> Matrix3<f64> {
    prec_nut_matrix(t).transpose() * rot_z(-equation_of_equinoxes(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nutation_at_j2000_matches_erfa() {
        // ERFA nut80 at J2000.0: Δψ ≈ −13.923″, Δε ≈ −5.774″ (the −17.2″/+9.2″
        // coefficients weighted by sin/cos of Ω ≈ 125°). Tight cross-validation
        // across epochs is in `tests/teme_vs_erfa.rs`.
        let (dpsi, deps) = nutation(0.0);
        assert!(
            (dpsi / DAS2R + 13.923).abs() < 0.01,
            "Δψ ≈ −13.923″, got {}",
            dpsi / DAS2R
        );
        assert!(
            (deps / DAS2R + 5.774).abs() < 0.01,
            "Δε ≈ −5.774″, got {}",
            deps / DAS2R
        );
    }

    #[test]
    fn mean_obliquity_at_j2000() {
        // ε̄(J2000) ≈ 84381.448″ ≈ 23.439°.
        assert!((mean_obliquity(0.0) / DAS2R - 84381.448).abs() < 1e-6);
    }

    #[test]
    fn teme_to_j2000_near_identity_at_j2000() {
        // At J2000 the precession/nutation since epoch is tiny, so the rotation
        // is within ~arcminutes of identity.
        let m = teme_to_j2000_matrix(0.0);
        let off_diag = m[(0, 1)].abs() + m[(0, 2)].abs() + m[(1, 2)].abs();
        assert!(
            off_diag < 1e-3,
            "near-identity at J2000, off-diagonal {off_diag}"
        );
    }
}
