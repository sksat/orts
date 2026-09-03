//! Fully normalized associated Legendre functions in the Holmes–Featherstone
//! "u^m factored out" form.
//!
//! For colatitude `θ` with `t = cos θ` and `u = sin θ`, the fully normalized
//! function `P̄nm(t)` contains the factor `u^m`, which underflows near the
//! poles at high order and makes the θ-derivative singular there. Holmes &
//! Featherstone (2002, J. Geod. 76:279–299) therefore work with
//!
//! ```text
//! P̃nm(t) = P̄nm(t) / u^m
//! ```
//!
//! which is a polynomial in `t` — finite everywhere, poles included. The
//! standard forward-column recursion (their eqs. 11–13) applies unchanged:
//!
//! ```text
//! P̃mm     = √((2m+1)/(2m)) · P̃m-1,m-1          (P̃00 = 1, P̃11 = √3)
//! P̃nm     = a_nm · t · P̃n-1,m − b_nm · P̃n-2,m   (n > m)
//! a_nm    = √((2n+1)(2n−1) / ((n−m)(n+m)))
//! b_nm    = √((2n+1)(n+m−1)(n−m−1) / ((n−m)(n+m)(2n−3)))
//! ```
//!
//! and the θ-derivative (their eq. 30) splits into two pieces whose `u`
//! powers differ by two, so the caller can keep both regular:
//!
//! ```text
//! dP̄nm/dθ = u^(m−1) · [m · t · P̃nm]  −  u^(m+1) · [e_nm · P̃n,m+1]
//! e_nm    = √((n−m)(n+m+1) / j),   j = 2 if m = 0 else 1
//! ```
//!
//! Every value is scaled by [`SCALE_DOWN`] = 2⁻⁹³⁰ (Orekit uses the same
//! power of two) so that `P̃nm(±1)`, which grows factorially with the degree,
//! stays representable up to degree ~2000. The scale is an exact power of two,
//! so it introduces no rounding; the caller multiplies the finished sums by
//! [`SCALE_UP`].

use alloc::vec;
use alloc::vec::Vec;

// Used only on no_std (libm-backed `.sqrt()`); std uses the inherent method.
#[allow(unused_imports)]
use crate::math::F64Ext;

/// 2⁻⁹³⁰, exactly (IEEE-754 bits: biased exponent 1023 − 930 = 93).
pub(crate) const SCALE_DOWN: f64 = f64::from_bits(93u64 << 52);
/// 2⁺⁹³⁰, exactly (biased exponent 1023 + 930 = 1953).
pub(crate) const SCALE_UP: f64 = f64::from_bits(1953u64 << 52);

/// Triangular index of `(n, m)` with `m ≤ n`.
#[inline]
pub(crate) const fn tri_index(n: usize, m: usize) -> usize {
    n * (n + 1) / 2 + m
}

/// Number of `(n, m)` pairs with `n ≤ max_degree`.
#[inline]
pub(crate) const fn tri_len(max_degree: usize) -> usize {
    (max_degree + 1) * (max_degree + 2) / 2
}

/// Precomputed recursion coefficients for degrees `0..=max_degree`.
///
/// Built once per field (the coefficients depend only on `(n, m)`), then
/// [`column`](Self::column) evaluates one order at a time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HfRecursion {
    max_degree: usize,
    /// `a_nm` at [`tri_index`]`(n, m)`; unused entries (n ≤ m) are 0.
    a: Vec<f64>,
    /// `b_nm`, same layout.
    b: Vec<f64>,
    /// `e_nm`, same layout (defined for every `m ≤ n`; `e_nn = 0`).
    e: Vec<f64>,
    /// `P̃mm · 2⁻⁹³⁰` for `m = 0..=max_degree`.
    sectorial: Vec<f64>,
}

impl HfRecursion {
    pub(crate) fn new(max_degree: usize) -> Self {
        let len = tri_len(max_degree);
        let mut a = vec![0.0; len];
        let mut b = vec![0.0; len];
        let mut e = vec![0.0; len];
        for n in 0..=max_degree {
            for m in 0..=n {
                let (nf, mf) = (n as f64, m as f64);
                let j = if m == 0 { 2.0 } else { 1.0 };
                e[tri_index(n, m)] = (((nf - mf) * (nf + mf + 1.0)) / j).sqrt();
                if n > m {
                    let denom = (nf - mf) * (nf + mf);
                    a[tri_index(n, m)] = ((2.0 * nf + 1.0) * (2.0 * nf - 1.0) / denom).sqrt();
                    if n > m + 1 {
                        b[tri_index(n, m)] = ((2.0 * nf + 1.0) * (nf + mf - 1.0) * (nf - mf - 1.0)
                            / (denom * (2.0 * nf - 3.0)))
                            .sqrt();
                    }
                }
            }
        }

        let mut sectorial = vec![0.0; max_degree + 1];
        sectorial[0] = SCALE_DOWN;
        if max_degree >= 1 {
            sectorial[1] = 3.0f64.sqrt() * SCALE_DOWN;
        }
        for m in 2..=max_degree {
            let mf = m as f64;
            sectorial[m] = ((2.0 * mf + 1.0) / (2.0 * mf)).sqrt() * sectorial[m - 1];
        }

        Self {
            max_degree,
            a,
            b,
            e,
            sectorial,
        }
    }

    #[cfg(test)]
    pub(crate) fn max_degree(&self) -> usize {
        self.max_degree
    }

    /// `e_nm` (derivative coupling to order `m + 1`).
    #[inline]
    pub(crate) fn e(&self, n: usize, m: usize) -> f64 {
        self.e[tri_index(n, m)]
    }

    /// Fill `out[n] = P̃nm(t) · 2⁻⁹³⁰` for `n = m..=max_degree`.
    ///
    /// Entries below `m` are left untouched; `out.len()` must be at least
    /// `max_degree + 1`.
    pub(crate) fn column(&self, m: usize, t: f64, out: &mut [f64]) {
        debug_assert!(m <= self.max_degree);
        debug_assert!(out.len() > self.max_degree);
        out[m] = self.sectorial[m];
        if m < self.max_degree {
            // First off-diagonal term: b_{m+1,m} = 0 (there is no P̃m-1,m).
            out[m + 1] = self.a[tri_index(m + 1, m)] * t * out[m];
        }
        for n in (m + 2)..=self.max_degree {
            let i = tri_index(n, m);
            out[n] = self.a[i] * t * out[n - 1] - self.b[i] * out[n - 2];
        }
    }
}

#[cfg(test)]
// Index-parallel (n, m) loops over several tables read more clearly than
// iterator zips here.
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::magnetic::igrf::schmidt_legendre;

    fn p_tilde(rec: &HfRecursion, m: usize, t: f64) -> Vec<f64> {
        let mut col = vec![0.0; rec.max_degree() + 1];
        rec.column(m, t, &mut col);
        col.iter().map(|v| v * SCALE_UP).collect()
    }

    #[test]
    fn scale_constants_are_exact_inverse_powers_of_two() {
        assert_eq!(SCALE_DOWN * SCALE_UP, 1.0);
        assert_eq!(SCALE_UP, 2.0f64.powi(930));
        assert_eq!(SCALE_DOWN, 2.0f64.powi(-930));
    }

    #[test]
    fn tri_index_is_dense_and_ordered() {
        let mut expected = 0;
        for n in 0..10 {
            for m in 0..=n {
                assert_eq!(tri_index(n, m), expected);
                expected += 1;
            }
        }
        assert_eq!(tri_len(9), expected);
    }

    /// Low-degree closed forms of the fully normalized functions, divided by
    /// `u^m`: pins the normalization convention that ICGEM coefficients
    /// assume, `P̄nm = √((2−δm0)(2n+1)(n−m)!/(n+m)!) · Pnm` (e.g.
    /// P̄31/u = (3/2)√(7/6)(5t²−1) = √(21/8)(5t²−1), P̄33/u³ = 15√(7/360) = √(35/8)).
    #[test]
    fn matches_closed_forms_up_to_degree_3() {
        let rec = HfRecursion::new(3);
        for &t in &[-0.9, -0.3, 0.0, 0.4, 0.95, 1.0, -1.0] {
            let p0 = p_tilde(&rec, 0, t);
            let p1 = p_tilde(&rec, 1, t);
            let p2 = p_tilde(&rec, 2, t);
            let p3 = p_tilde(&rec, 3, t);
            let cases: [(f64, f64, &str); 10] = [
                (p0[0], 1.0, "P00"),
                (p0[1], 3.0f64.sqrt() * t, "P10"),
                (p1[1], 3.0f64.sqrt(), "P11/u"),
                (p0[2], 5.0f64.sqrt() * (3.0 * t * t - 1.0) / 2.0, "P20"),
                (p1[2], 15.0f64.sqrt() * t, "P21/u"),
                (p2[2], 15.0f64.sqrt() / 2.0, "P22/u²"),
                (
                    p0[3],
                    7.0f64.sqrt() * (5.0 * t * t * t - 3.0 * t) / 2.0,
                    "P30",
                ),
                (p1[3], (21.0f64 / 8.0).sqrt() * (5.0 * t * t - 1.0), "P31/u"),
                (p2[3], (105.0f64 / 4.0).sqrt() * t, "P32/u²"),
                (p3[3], (35.0f64 / 8.0).sqrt(), "P33/u³"),
            ];
            for (got, want, name) in cases {
                assert!(
                    (got - want).abs() <= 1e-14 * want.abs().max(1.0),
                    "{name} at t={t}: got {got}, want {want}"
                );
            }
        }
    }

    /// Fully normalized = Schmidt semi-normalized × √(2n+1). The IGRF
    /// recursion is independent code with its own tests, so agreement up to
    /// its degree 13 pins the normalization and sign conventions.
    #[test]
    fn matches_schmidt_recursion_times_sqrt_2n_plus_1() {
        const N: usize = 13;
        let rec = HfRecursion::new(N);
        for &theta in &[0.1, 0.7, 1.2, core::f64::consts::FRAC_PI_2, 2.0, 2.9] {
            let (t, u) = (theta.cos(), theta.sin());
            let (schmidt, _) = schmidt_legendre(t, u, N);
            for m in 0..=N {
                let col = p_tilde(&rec, m, t);
                for n in m..=N {
                    let want = schmidt[n][m] * ((2 * n + 1) as f64).sqrt();
                    let got = col[n] * u.powi(m as i32);
                    assert!(
                        (got - want).abs() <= 1e-12 * want.abs().max(1e-3),
                        "n={n} m={m} θ={theta}: got {got:e}, want {want:e}"
                    );
                }
            }
        }
    }

    /// Textbook fully normalized forward-column recursion on `P̄nm` itself
    /// (no `u^m` factoring, no scaling): an independent evaluation of the same
    /// functions to degree 70, at colatitudes where `u^m` is far from
    /// underflow.
    fn naive_fully_normalized(n_max: usize, t: f64, u: f64) -> Vec<Vec<f64>> {
        let mut p = vec![vec![0.0; n_max + 1]; n_max + 1];
        p[0][0] = 1.0;
        for m in 1..=n_max {
            let mf = m as f64;
            let f = if m == 1 {
                3.0f64.sqrt()
            } else {
                ((2.0 * mf + 1.0) / (2.0 * mf)).sqrt()
            };
            p[m][m] = f * u * p[m - 1][m - 1];
        }
        for m in 0..n_max {
            for n in (m + 1)..=n_max {
                let (nf, mf) = (n as f64, m as f64);
                let a = ((2.0 * nf + 1.0) * (2.0 * nf - 1.0) / ((nf - mf) * (nf + mf))).sqrt();
                let b = if n >= m + 2 {
                    ((2.0 * nf + 1.0) * (nf + mf - 1.0) * (nf - mf - 1.0)
                        / ((nf - mf) * (nf + mf) * (2.0 * nf - 3.0)))
                        .sqrt()
                } else {
                    0.0
                };
                let prev2 = if n >= m + 2 { p[n - 2][m] } else { 0.0 };
                p[n][m] = a * t * p[n - 1][m] - b * prev2;
            }
        }
        p
    }

    #[test]
    fn matches_unfactored_recursion_to_degree_70() {
        const N: usize = 70;
        let rec = HfRecursion::new(N);
        for &theta in &[0.6, 1.0, core::f64::consts::FRAC_PI_2, 2.3] {
            let (t, u) = (theta.cos(), theta.sin());
            let naive = naive_fully_normalized(N, t, u);
            for m in 0..=N {
                let col = p_tilde(&rec, m, t);
                let um = u.powi(m as i32);
                for n in m..=N {
                    let want = naive[n][m];
                    let got = col[n] * um;
                    assert!(
                        got.is_finite() && (got - want).abs() <= 1e-11 * want.abs().max(1e-6),
                        "n={n} m={m} θ={theta}: got {got:e}, want {want:e}"
                    );
                }
            }
        }
    }

    /// The factored functions are finite at the poles for every order, which
    /// is the whole reason for the `u^m` split.
    #[test]
    fn factored_values_are_finite_at_the_poles_to_degree_360() {
        let rec = HfRecursion::new(360);
        for &t in &[1.0, -1.0] {
            for m in [0usize, 1, 2, 50, 180, 359, 360] {
                let col = p_tilde(&rec, m, t);
                assert!(
                    col[m..].iter().all(|v| v.is_finite()),
                    "non-finite P̃ at t={t}, m={m}"
                );
            }
        }
    }

    /// `e_nm` closed form and the m = 0 factor √2 (Orekit's `j`).
    #[test]
    fn derivative_coupling_coefficient() {
        let rec = HfRecursion::new(5);
        assert!((rec.e(2, 0) - (6.0f64 / 2.0).sqrt()).abs() < 1e-15);
        assert!((rec.e(2, 1) - 4.0f64.sqrt()).abs() < 1e-15);
        assert_eq!(rec.e(3, 3), 0.0);
    }
}
