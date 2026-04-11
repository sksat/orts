//! Cross-validation of `arika::earth::iau2006` pure-Rust math against
//! ERFA (liberfa/erfa), a BSD 3-Clause licenced fork of IAU SOFA.
//!
//! ERFA is the accepted reference implementation of the IAU 2006 /
//! 2000A_R06 precession-nutation model used by Astropy, Orekit, and most
//! scientific Python astronomy tooling. It is *not* a runtime dependency
//! of arika or of CI: the reference fixture at
//! `arika/tests/fixtures/iau2006_erfa_reference.json` is generated
//! offline by `arika/tools/generate_iau2006_reference.py` (a `uv run`
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
//! uv run arika/tools/generate_iau2006_reference.py
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

use arika::earth::eop::{LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset};
use arika::earth::iau2006::cip::{cio_locator_s, cip_xy, gcrs_to_cirs_matrix_at};
use arika::earth::iau2006::fundamental_arguments::FundamentalArguments;
use arika::earth::iau2006::precession::{ecliptic_precession_angles, fukushima_williams};
use arika::epoch::{Epoch, Tt, Ut1, Utc};
use arika::frame::{Gcrs, Itrs, Rotation, Vec3};
use serde_json::Value;

/// Minimal EOP provider that returns `0` for every parameter. Lets the
/// `iau2006_full` test compose every Phase 3B constructor without a
/// real EOP table.
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
impl LengthOfDay for ZeroEop {
    fn lod(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
}

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

/// Maximum allowed absolute difference for the full GCRS→ITRS matrix.
/// The chain accumulates three sources of residual:
///   1. The 3000-term CIP series evaluation (inherits the same
///      [`CIP_TOLERANCE_RAD`] = 1e-11 bound used in Phase 3A-3)
///   2. The `Matrix3 → UnitQuaternion → Matrix3` roundtrip that
///      `Rotation<From, To>` performs (sub-ULP, dominated by item 1)
///   3. The ERA composition: ERA reaches ~6.28 rad/century, so
///      multiplying the CIP matrix by `R_z(ERA)` amplifies the scalar
///      residual by up to ~2×. Empirically observed max delta over
///      `|t| ≤ 1` century is ~1.1e-11, so we pin 5e-11 to absorb that
///      plus headroom while still catching a real transcription error.
const FULL_CHAIN_VECTOR_TOLERANCE: f64 = 5e-11;

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
fn iau2006_full_matches_erfa_zero_eop_chain() {
    // End-to-end Phase 3B validation: construct
    // `Rotation<Gcrs, Itrs>::iau2006_full` with the `ZeroEop` provider
    // and verify it maps GCRS unit vectors to the same ITRS vectors
    // that the ERFA-reference fixture computed via the piecewise
    // `c2ixys → rz(era) → pom00(0,0,sp)` chain.
    let fixture = load_fixture();
    let samples = fixture["samples"]
        .as_array()
        .expect("fixture must have a `samples` array");

    let mut failures = 0usize;

    // Three orthogonal reference vectors so a transposition bug in the
    // matrix layout would show up on at least one.
    let reference_inputs: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for sample in samples {
        let t = field_f64(sample, "t_tt_centuries_from_j2000");
        let entry = &sample["gcrs_to_itrs_matrix_zero_eop"];
        let matrix_expected = entry["matrix"]
            .as_array()
            .expect("missing gcrs_to_itrs_matrix_zero_eop.matrix");

        // Build the arika rotation. We use the individual scale
        // constructors rather than `iau2006_full_from_utc` because the
        // fixture `t` is already in TT centuries and we want to pin
        // the raw composition; `from_utc` has its own test in the
        // unit test suite.
        let tt = Epoch::<Tt>::from_jd_tt(2451545.0 + t * 36525.0);
        // With dUT1 = 0, `ut1 = utc` as bit-equal Julian Dates.
        let ut1 = Epoch::<Ut1>::from_jd_ut1(2451545.0 + t * 36525.0);
        let utc = Epoch::<Utc>::from_jd(2451545.0 + t * 36525.0);
        let rot = Rotation::<Gcrs, Itrs>::iau2006_full(&tt, &ut1, &utc, &ZeroEop);

        for input in reference_inputs {
            let v_gcrs = Vec3::<Gcrs>::new(input[0], input[1], input[2]);
            let v_itrs_arika = rot.transform(&v_gcrs);

            // Expected output: matrix_expected · input
            let expected = [
                matrix_expected[0][0].as_f64().unwrap() * input[0]
                    + matrix_expected[0][1].as_f64().unwrap() * input[1]
                    + matrix_expected[0][2].as_f64().unwrap() * input[2],
                matrix_expected[1][0].as_f64().unwrap() * input[0]
                    + matrix_expected[1][1].as_f64().unwrap() * input[1]
                    + matrix_expected[1][2].as_f64().unwrap() * input[2],
                matrix_expected[2][0].as_f64().unwrap() * input[0]
                    + matrix_expected[2][1].as_f64().unwrap() * input[1]
                    + matrix_expected[2][2].as_f64().unwrap() * input[2],
            ];

            for (i, (actual, expected)) in [
                (v_itrs_arika.x(), expected[0]),
                (v_itrs_arika.y(), expected[1]),
                (v_itrs_arika.z(), expected[2]),
            ]
            .iter()
            .enumerate()
            .map(|(i, (a, e))| (i, (*a, *e)))
            {
                let delta = (actual - expected).abs();
                if !delta.is_finite() || delta > FULL_CHAIN_VECTOR_TOLERANCE {
                    eprintln!(
                        "FAIL t={t:+.3} input={input:?} component[{i}]: actual={actual:+.17e} expected={expected:+.17e} Δ={delta:.3e}"
                    );
                    failures += 1;
                }
            }
        }
    }

    assert_eq!(
        failures, 0,
        "{failures} full-chain mismatches exceeded {FULL_CHAIN_VECTOR_TOLERANCE:e} tolerance"
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
