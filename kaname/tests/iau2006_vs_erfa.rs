//! Cross-validation of `kaname::earth::iau2006` pure-Rust math against
//! ERFA (liberfa/erfa), a BSD 3-Clause licenced fork of IAU SOFA.
//!
//! ERFA is the accepted reference implementation of the IAU 2006 /
//! 2000A_R06 precession-nutation model used by Astropy, Orekit, and most
//! scientific Python astronomy tooling. It is *not* a runtime dependency
//! of kaname or of CI: the reference fixture at
//! `kaname/tests/fixtures/iau2006_erfa_reference.json` is generated
//! offline by `kaname/tools/generate_iau2006_reference.py` (a `uv run`
//! PEP 723 script that pulls in `pyerfa`), and this integration test
//! loads the committed JSON with `include_str!` and `serde_json`.
//!
//! # When to regenerate
//!
//! - The set of sample `t` values in `generate_iau2006_reference.py`
//!   changes
//! - A new quantity is added to the reference (e.g. the precession
//!   polynomials when Phase 3A-1 extended the fixture)
//! - A new pyerfa version is pinned (update `source` and re-run)
//!
//! Re-run (from the repository root):
//!
//! ```shell
//! uv run kaname/tools/generate_iau2006_reference.py
//! ```
//!
//! # Tolerances
//!
//! The pure Rust and ERFA implementations differ only by floating-point
//! rounding order in Horner evaluation, so agreement is expected at the
//! ~10⁻¹² rad (≈ 0.2 µas) level for fundamental arguments / precession
//! polynomials, and ~10⁻¹² rad for the CIP X, Y and CIO locator s
//! series (which evaluate thousands of trigonometric terms before
//! converting from microarcseconds to radians). The test pins generous
//! tolerances (10⁻¹¹ / 10⁻¹² / 10⁻¹¹) to absorb cross-compiler jitter
//! while still catching any real transcription mistake.

use kaname::earth::iau2006::cip::{cio_locator_s, cip_xy, gcrs_to_cirs_matrix_at};
use kaname::earth::iau2006::fundamental_arguments::FundamentalArguments;
use kaname::earth::iau2006::precession::{ecliptic_precession_angles, fukushima_williams};
use serde_json::Value;

/// The ERFA reference fixture, embedded into the test binary at compile
/// time so CI needs neither a filesystem path nor network access.
const FIXTURE_JSON: &str = include_str!("fixtures/iau2006_erfa_reference.json");

/// Maximum allowed absolute difference (rad) for fundamental arguments.
const FA_TOLERANCE_RAD: f64 = 1e-11;

/// Maximum allowed absolute difference (rad) for Fukushima-Williams
/// precession angles.
const PFW_TOLERANCE_RAD: f64 = 1e-12;

/// Maximum allowed absolute difference (rad) for CIP `X`, `Y` and CIO
/// locator `s` — see the series-total comment above.
const CIP_TOLERANCE_RAD: f64 = 1e-11;

/// Maximum allowed absolute difference for any element of the
/// GCRS→CIRS 3×3 matrix. The matrix elements are either near-unity or
/// near-zero but the rotation composition `R_z(−(E+s)) · R_y(d) · R_z(E)`
/// amplifies scalar residuals by at most `O(1)`, so the same
/// `1e-11` scale that bounds `X/Y/s` is sufficient here.
const C2I_MATRIX_TOLERANCE: f64 = 1e-11;

fn load_fixture() -> Value {
    serde_json::from_str(FIXTURE_JSON).expect("iau2006_erfa_reference.json must be valid JSON")
}

fn field_f64(v: &Value, key: &str) -> f64 {
    v.get(key)
        .and_then(|x| x.as_f64())
        .unwrap_or_else(|| panic!("missing or non-numeric field: {key}"))
}

#[test]
fn fundamental_arguments_match_erfa() {
    let fixture = load_fixture();
    let samples = fixture["samples"]
        .as_array()
        .expect("fixture must have a `samples` array");

    assert!(
        !samples.is_empty(),
        "fixture must contain at least one sample"
    );

    let mut failures = 0usize;

    for sample in samples {
        let t = field_f64(sample, "t_tt_centuries_from_j2000");
        let fa_expected = &sample["fundamental_arguments"];

        let fa = FundamentalArguments::evaluate(t);

        let expectations: [(&str, f64, f64); 14] = [
            ("l", fa.l.raw(), field_f64(fa_expected, "l")),
            (
                "l_prime",
                fa.l_prime.raw(),
                field_f64(fa_expected, "l_prime"),
            ),
            ("f", fa.f.raw(), field_f64(fa_expected, "f")),
            ("d", fa.d.raw(), field_f64(fa_expected, "d")),
            ("omega", fa.omega.raw(), field_f64(fa_expected, "omega")),
            ("l_me", fa.l_me.raw(), field_f64(fa_expected, "l_me")),
            ("l_ve", fa.l_ve.raw(), field_f64(fa_expected, "l_ve")),
            ("l_e", fa.l_e.raw(), field_f64(fa_expected, "l_e")),
            ("l_ma", fa.l_ma.raw(), field_f64(fa_expected, "l_ma")),
            ("l_j", fa.l_j.raw(), field_f64(fa_expected, "l_j")),
            ("l_sa", fa.l_sa.raw(), field_f64(fa_expected, "l_sa")),
            ("l_u", fa.l_u.raw(), field_f64(fa_expected, "l_u")),
            ("l_ne", fa.l_ne.raw(), field_f64(fa_expected, "l_ne")),
            ("p_a", fa.p_a.raw(), field_f64(fa_expected, "p_a")),
        ];

        for (name, actual, expected) in expectations {
            let delta = (actual - expected).abs();
            if !delta.is_finite() || delta > FA_TOLERANCE_RAD {
                eprintln!(
                    "FAIL t={t:+.3} fa.{name}: actual={actual:+.17e} expected={expected:+.17e} Δ={delta:.3e} rad"
                );
                failures += 1;
            }
        }
    }

    assert_eq!(
        failures, 0,
        "{failures} fundamental-argument mismatches exceeded {FA_TOLERANCE_RAD:e} rad tolerance"
    );
}

#[test]
fn fukushima_williams_angles_match_erfa() {
    let fixture = load_fixture();
    let samples = fixture["samples"]
        .as_array()
        .expect("fixture must have a `samples` array");

    let mut failures = 0usize;

    for sample in samples {
        let t = field_f64(sample, "t_tt_centuries_from_j2000");
        let pfw_expected = &sample["precession_fukushima_williams"];

        let pfw = fukushima_williams(t);

        let expectations: [(&str, f64, f64); 4] = [
            (
                "gamma_bar",
                pfw.gamma_bar.raw(),
                field_f64(pfw_expected, "gamma_bar"),
            ),
            (
                "phi_bar",
                pfw.phi_bar.raw(),
                field_f64(pfw_expected, "phi_bar"),
            ),
            (
                "psi_bar",
                pfw.psi_bar.raw(),
                field_f64(pfw_expected, "psi_bar"),
            ),
            ("eps_a", pfw.eps_a.raw(), field_f64(pfw_expected, "eps_a")),
        ];

        for (name, actual, expected) in expectations {
            let delta = (actual - expected).abs();
            if !delta.is_finite() || delta > PFW_TOLERANCE_RAD {
                eprintln!(
                    "FAIL t={t:+.3} pfw.{name}: actual={actual:+.17e} expected={expected:+.17e} Δ={delta:.3e} rad"
                );
                failures += 1;
            }
        }
    }

    assert_eq!(
        failures, 0,
        "{failures} Fukushima-Williams mismatches exceeded {PFW_TOLERANCE_RAD:e} rad tolerance"
    );
}

#[test]
fn cip_xys_match_erfa() {
    let fixture = load_fixture();
    let samples = fixture["samples"]
        .as_array()
        .expect("fixture must have a `samples` array");

    let mut failures = 0usize;

    for sample in samples {
        let t = field_f64(sample, "t_tt_centuries_from_j2000");
        let xys_expected = &sample["cip_xys"];

        let (x, y) = cip_xy(t);
        let s = cio_locator_s(t, x, y);

        let expectations: [(&str, f64, f64); 3] = [
            ("x", x.raw(), field_f64(xys_expected, "x")),
            ("y", y.raw(), field_f64(xys_expected, "y")),
            ("s", s.raw(), field_f64(xys_expected, "s")),
        ];

        for (name, actual, expected) in expectations {
            let delta = (actual - expected).abs();
            if !delta.is_finite() || delta > CIP_TOLERANCE_RAD {
                eprintln!(
                    "FAIL t={t:+.3} cip.{name}: actual={actual:+.17e} expected={expected:+.17e} Δ={delta:.3e} rad"
                );
                failures += 1;
            }
        }
    }

    assert_eq!(
        failures, 0,
        "{failures} CIP X/Y/s mismatches exceeded {CIP_TOLERANCE_RAD:e} rad tolerance"
    );
}

#[test]
fn gcrs_to_cirs_matrix_matches_erfa_c2ixys() {
    let fixture = load_fixture();
    let samples = fixture["samples"]
        .as_array()
        .expect("fixture must have a `samples` array");

    let mut failures = 0usize;

    for sample in samples {
        let t = field_f64(sample, "t_tt_centuries_from_j2000");
        let matrix_expected = sample["gcrs_to_cirs_matrix"]
            .as_array()
            .expect("sample must have a `gcrs_to_cirs_matrix` 3x3 array");
        assert_eq!(matrix_expected.len(), 3, "expected 3 rows");

        let m = gcrs_to_cirs_matrix_at(t);

        for i in 0..3 {
            let row = matrix_expected[i]
                .as_array()
                .expect("matrix row must be an array");
            assert_eq!(row.len(), 3, "expected 3 columns in row {i}");
            for j in 0..3 {
                let expected = row[j].as_f64().expect("matrix element must be numeric");
                let actual = m[(i, j)];
                let delta = (actual - expected).abs();
                if !delta.is_finite() || delta > C2I_MATRIX_TOLERANCE {
                    eprintln!(
                        "FAIL t={t:+.3} M[{i},{j}]: actual={actual:+.17e} expected={expected:+.17e} Δ={delta:.3e}"
                    );
                    failures += 1;
                }
            }
        }
    }

    assert_eq!(
        failures, 0,
        "{failures} GCRS→CIRS matrix element mismatches exceeded {C2I_MATRIX_TOLERANCE:e} tolerance"
    );
}

#[test]
fn lieske_eps_a_matches_fukushima_williams_eps_a() {
    // Both accessors source `ε_A` from the same TN36 Eq. (5.40)
    // polynomial, so they must be bit-identical. A regression here would
    // indicate that one path picked up an edit but not the other.
    for &t in &[-1.0, -0.5, 0.0, 0.2, 0.5, 1.0] {
        let fw = fukushima_williams(t);
        let le = ecliptic_precession_angles(t);
        assert_eq!(
            fw.eps_a.raw(),
            le.eps_a.raw(),
            "eps_A divergence at t={t}: fw={} lieske={}",
            fw.eps_a.raw(),
            le.eps_a.raw()
        );
    }
}
