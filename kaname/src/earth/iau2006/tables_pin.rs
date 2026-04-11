//! Structural pin tests for the IAU 2006 CIP / CIO tables.
//!
//! The tables themselves live in the generated file [`super::tables_gen`]
//! and are not modified by hand. This file holds **tests only** — it
//! pins invariants that would change only if:
//!
//! 1. The IERS Conventions Centre publishes an updated Chapter 5 bundle
//!    with different row counts, a different polynomial part, or shifted
//!    first-row amplitudes, or
//! 2. Someone accidentally edits the generated file by hand.
//!
//! In either case regenerate with (from the repository root):
//!
//! ```shell
//! uv run kaname/tools/generate_iau2006_tables.py
//! ```
//!
//! The numerical cross-validation of the **evaluators** that consume
//! these tables is deferred to Phase 3A-3, where the kaname X(t), Y(t),
//! and s(t) evaluators will be compared against ERFA `xy06` / `s06` at
//! multiple epochs via the same fixture mechanism used by Phase 3A-1.
//!
//! The unused-import lint is intentionally relaxed because this file
//! exists solely to wire `tables_gen` into the compilation unit for
//! Phase 3A-2 — Phase 3A-3's evaluator will become the real consumer.
#![cfg(test)]

use super::tables_gen::{
    CipTerm, SXY2_POLY_UAS, SXY2_TERMS_0, SXY2_TERMS_1, SXY2_TERMS_2, SXY2_TERMS_3, SXY2_TERMS_4,
    X_POLY_UAS, X_TERMS_0, X_TERMS_1, X_TERMS_2, X_TERMS_3, X_TERMS_4, Y_POLY_UAS, Y_TERMS_0,
    Y_TERMS_1, Y_TERMS_2, Y_TERMS_3, Y_TERMS_4,
};

// ─── Row counts per power group (TN36 Chapter 5) ─────────────────

/// Table 5.2a (`X`) row counts for `j = 0..4`. Sum = 1600 non-polynomial
/// terms. Matches TN36 Chapter 5 / ERFA `xy06`.
#[test]
fn x_table_row_counts_match_tn36() {
    assert_eq!(X_TERMS_0.len(), 1306);
    assert_eq!(X_TERMS_1.len(), 253);
    assert_eq!(X_TERMS_2.len(), 36);
    assert_eq!(X_TERMS_3.len(), 4);
    assert_eq!(X_TERMS_4.len(), 1);
    assert_eq!(
        total_terms(&[X_TERMS_0, X_TERMS_1, X_TERMS_2, X_TERMS_3, X_TERMS_4]),
        1600
    );
}

/// Table 5.2b (`Y`) row counts for `j = 0..4`. Sum = 1275 non-polynomial
/// terms.
#[test]
fn y_table_row_counts_match_tn36() {
    assert_eq!(Y_TERMS_0.len(), 962);
    assert_eq!(Y_TERMS_1.len(), 277);
    assert_eq!(Y_TERMS_2.len(), 30);
    assert_eq!(Y_TERMS_3.len(), 5);
    assert_eq!(Y_TERMS_4.len(), 1);
    assert_eq!(
        total_terms(&[Y_TERMS_0, Y_TERMS_1, Y_TERMS_2, Y_TERMS_3, Y_TERMS_4]),
        1275
    );
}

/// Table 5.2d (`s + X·Y/2`) row counts for `j = 0..4`. Sum = 66
/// non-polynomial terms.
#[test]
fn sxy2_table_row_counts_match_tn36() {
    assert_eq!(SXY2_TERMS_0.len(), 33);
    assert_eq!(SXY2_TERMS_1.len(), 3);
    assert_eq!(SXY2_TERMS_2.len(), 25);
    assert_eq!(SXY2_TERMS_3.len(), 4);
    assert_eq!(SXY2_TERMS_4.len(), 1);
    assert_eq!(
        total_terms(&[
            SXY2_TERMS_0,
            SXY2_TERMS_1,
            SXY2_TERMS_2,
            SXY2_TERMS_3,
            SXY2_TERMS_4
        ]),
        66,
    );
}

fn total_terms(groups: &[&[CipTerm]]) -> usize {
    groups.iter().map(|g| g.len()).sum()
}

// ─── Polynomial parts ────────────────────────────────────────────
//
// Pin the six coefficients (microarcsec) of each table's polynomial
// part verbatim from the TN36 header lines. The literals below come
// from the `Polynomial part (unit microarcsecond)` line of each source
// file — any future drift between the generator and the upstream
// source produces an immediate failure.

/// TN36 `tab5.2a.txt`: `X = -16617 + 2004191898 t - 429782.9 t² -
/// 198618.34 t³ + 7.578 t⁴ + 5.9285 t⁵` (unit: µas).
#[test]
fn x_polynomial_matches_tn36_header() {
    assert_eq!(
        X_POLY_UAS,
        [-16617.0, 2004191898.0, -429782.9, -198618.34, 7.578, 5.9285]
    );
}

/// TN36 `tab5.2b.txt`: `Y = -6951 - 25896 t - 22407274.7 t² +
/// 1900.59 t³ + 1112.526 t⁴ + 0.1358 t⁵` (unit: µas).
#[test]
fn y_polynomial_matches_tn36_header() {
    assert_eq!(
        Y_POLY_UAS,
        [-6951.0, -25896.0, -22407274.7, 1900.59, 1112.526, 0.1358]
    );
}

/// TN36 `tab5.2d.txt`: `s + XY/2 = 94 + 3808.65 t - 122.68 t² -
/// 72574.11 t³ + 27.98 t⁴ + 15.62 t⁵` (unit: µas).
#[test]
fn sxy2_polynomial_matches_tn36_header() {
    assert_eq!(
        SXY2_POLY_UAS,
        [94.0, 3808.65, -122.68, -72574.11, 27.98, 15.62]
    );
}

// ─── First-row amplitudes for each j = 0 group ───────────────────
//
// The largest (first) term of each `j = 0` series is by far the most
// numerically dominant. Pinning it catches both column-order bugs in
// the generator (e.g. swapped sin / cos) and any upstream revision to
// the canonical amplitudes. Values below are transcribed from TN36
// Table 5.2a / 5.2b / 5.2d extract block (see also
// `iers-conventions.obspm.fr/content/chapter5/additional_info/`).

/// X: `a_{s,0})_1 = -6844318.44 µas`, `a_{c,0})_1 = 1328.67 µas`,
/// multipliers `[0,0,0,0,1, 0,…]` (pure Ω term).
#[test]
fn x_first_row_j0_is_dominant_omega_term() {
    let row = X_TERMS_0[0];
    assert_eq!(row.sin_uas, -6844318.44);
    assert_eq!(row.cos_uas, 1328.67);
    assert_eq!(row.arg, [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

/// Y: `b_{s,0})_1 = 1538.18 µas`, `b_{c,0})_1 = 9205236.26 µas`,
/// same luni-solar multipliers `[0,0,0,0,1, 0,…]`.
///
/// Note: in the Y table the **cosine** amplitude dominates (the cos
/// component of the nutation in obliquity is ~9.2 arcsec at the leading
/// term). The test therefore catches any reintroduction of a
/// sin/cos column swap in the generator.
#[test]
fn y_first_row_j0_is_dominant_omega_term() {
    let row = Y_TERMS_0[0];
    assert_eq!(row.sin_uas, 1538.18);
    assert_eq!(row.cos_uas, 9205236.26);
    assert_eq!(row.arg, [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

/// s+XY/2: `C_{s,0})_1 = -2640.73 µas`, `C_{c,0})_1 = 0.39 µas`,
/// multipliers `[0,0,0,0,1, 0,…]`. This is the largest CIO locator
/// term and was also spot-printed in the TN36 Table 5.2d extract.
#[test]
fn sxy2_first_row_j0_is_dominant_omega_term() {
    let row = SXY2_TERMS_0[0];
    assert_eq!(row.sin_uas, -2640.73);
    assert_eq!(row.cos_uas, 0.39);
    assert_eq!(row.arg, [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

// ─── Highest-power term pins (j = 4 in every table) ──────────────
//
// The `j = 4` section of each table contains exactly one term. Pinning
// its amplitude is a compact way to verify that the generator reached
// the end of the file and did not silently truncate. The multipliers
// are identical across all three tables.

#[test]
fn x_last_group_has_exactly_one_term_with_pinned_amplitude() {
    assert_eq!(X_TERMS_4.len(), 1);
    let row = X_TERMS_4[0];
    assert_eq!(row.sin_uas, -0.10);
    assert_eq!(row.cos_uas, -0.02);
}

#[test]
fn y_last_group_has_exactly_one_term_with_pinned_amplitude() {
    assert_eq!(Y_TERMS_4.len(), 1);
    let row = Y_TERMS_4[0];
    assert_eq!(row.sin_uas, -0.02);
    assert_eq!(row.cos_uas, 0.11);
}

#[test]
fn sxy2_last_group_has_exactly_one_term_with_pinned_amplitude() {
    assert_eq!(SXY2_TERMS_4.len(), 1);
    let row = SXY2_TERMS_4[0];
    assert_eq!(row.sin_uas, -0.26);
    assert_eq!(row.cos_uas, -0.01);
}

// ─── Multiplier sanity ───────────────────────────────────────────
//
// Every `arg` entry is an integer in a small signed range. Empirically
// the entire TN36 Chapter 5 bundle uses multipliers in `[-21, 19]`; we
// pin a loose `±25` bound so legitimate upstream updates (adding a new
// planetary resonance term) do not trip this test, while a parser bug
// that ended up near the `i8` limits (±128) still does.

#[test]
fn every_multiplier_is_within_empirical_tn36_range() {
    let groups: &[&[CipTerm]] = &[
        X_TERMS_0,
        X_TERMS_1,
        X_TERMS_2,
        X_TERMS_3,
        X_TERMS_4,
        Y_TERMS_0,
        Y_TERMS_1,
        Y_TERMS_2,
        Y_TERMS_3,
        Y_TERMS_4,
        SXY2_TERMS_0,
        SXY2_TERMS_1,
        SXY2_TERMS_2,
        SXY2_TERMS_3,
        SXY2_TERMS_4,
    ];
    for group in groups {
        for term in *group {
            for &m in &term.arg {
                assert!(
                    (-25..=25).contains(&m),
                    "multiplier {m} outside the empirical ±25 TN36 range — \
                     suggests a parser bug or an upstream table revision"
                );
            }
        }
    }
}
