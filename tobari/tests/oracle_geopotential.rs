//! Oracle tests: `SphericalHarmonicField` against Orekit's
//! `HolmesFeatherstoneAttractionModel`, point by point in the body frame.
//!
//! Orekit is an independent implementation of the same Holmes–Featherstone
//! algorithm (different code, different language, its own Legendre recursion
//! ordering), so agreement here pins the parts a self-consistency check
//! cannot: the absolute scale and sign of every tesseral term at high degree.
//!
//! Both sides read the *same* coefficients: the `.gfc` fixture was written
//! from Orekit's provider by `tools/generate_orekit_geopotential_fixtures.py`,
//! so any disagreement is evaluation, not data. The (degree, order) sets in
//! the reference are truncations of that one file on both sides.
//!
//! Not covered: the exact pole. Orekit's spherical-gradient formulation is
//! singular there (`t/u`), so the fixture stops at ρ = 1 mm; the pole itself is
//! pinned by the crate's closed-form and continuity tests.

use nalgebra::Vector3;
use serde::Deserialize;
use tobari::gravity::{SphericalHarmonicField, TideSystem};

#[derive(Deserialize)]
struct Reference {
    mu_km3_s2: f64,
    radius_km: f64,
    sets: Vec<Set>,
}

#[derive(Deserialize)]
struct Set {
    degree: usize,
    order: usize,
    points: Vec<Point>,
}

#[derive(Deserialize)]
struct Point {
    position_km: [f64; 3],
    acceleration_km_s2: [f64; 3],
    potential_km2_s2: f64,
}

fn load_field() -> SphericalHarmonicField {
    SphericalHarmonicField::from_icgem(include_str!("fixtures/orekit_geopotential_70x70.gfc"))
        .expect("fixture gfc must parse")
}

fn load_reference() -> Reference {
    serde_json::from_str(include_str!(
        "fixtures/orekit_geopotential_gradient_reference.json"
    ))
    .expect("fixture json must parse")
}

#[test]
fn fixture_field_metadata_matches_orekit_provider() {
    let field = load_field();
    let reference = load_reference();
    assert_eq!((field.max_degree(), field.max_order()), (70, 70));
    assert_eq!(field.gm(), reference.mu_km3_s2);
    assert_eq!(field.radius(), reference.radius_km);
    assert_eq!(field.tide_system(), TideSystem::TideFree);
    // C̄20 of the orekit-data default field, and its J2, in the WGS-84 ballpark.
    let (c20, _) = field.coefficient(2, 0).unwrap();
    assert!((c20 + 4.8416e-4).abs() < 1e-8, "C20 = {c20}");
    assert!((field.j2() - 1.0826e-3).abs() < 1e-7, "J2 = {}", field.j2());
}

/// Every (degree, order) truncation, every sample point: acceleration and
/// disturbing potential agree with Orekit to f64 round-off. The tolerance is
/// relative to the point-mass acceleration `GM/r²` (potential: `GM/r`) so a
/// point where the harmonic terms nearly cancel does not inflate a relative
/// error; 1e-13 of GM/r² at LEO is ~1e-15 km/s², about 100 ULP of the
/// largest term.
#[test]
fn acceleration_and_potential_match_orekit_pointwise() {
    let full = load_field();
    let reference = load_reference();
    assert!(!reference.sets.is_empty());
    for set in &reference.sets {
        let field = full.truncated(set.degree, set.order);
        assert_eq!(
            (field.max_degree(), field.max_order()),
            (set.degree, set.order)
        );
        assert!(
            set.points.len() >= 30,
            "too few points in {}x{}",
            set.degree,
            set.order
        );
        for point in &set.points {
            let pos = Vector3::from(point.position_km);
            let want_a = Vector3::from(point.acceleration_km_s2);
            let got_a = field.acceleration_ecef(&pos);
            let got_u = field.potential_ecef(&pos);
            let r = pos.norm();
            let (scale_a, scale_u) = (field.gm() / (r * r), field.gm() / r);
            let da = (got_a - want_a).norm();
            assert!(
                da <= 1e-13 * scale_a,
                "{}x{} at {pos:?}: |Δa| = {da:e} km/s² (> 1e-13·GM/r² = {:e}); got {got_a:?}, orekit {want_a:?}",
                set.degree,
                set.order,
                1e-13 * scale_a
            );
            let du = (got_u - point.potential_km2_s2).abs();
            assert!(
                du <= 1e-13 * scale_u,
                "{}x{} at {pos:?}: |ΔU| = {du:e} km²/s²; got {got_u}, orekit {}",
                set.degree,
                set.order,
                point.potential_km2_s2
            );
        }
    }
}

/// The 70×70 terms are not negligible next to J2..J4 at LEO — otherwise the
/// oracle above would pass with the tesserals silently zeroed.
#[test]
fn high_degree_terms_are_resolved_by_the_fixture() {
    let reference = load_reference();
    let full = &reference
        .sets
        .iter()
        .find(|s| s.degree == 70)
        .unwrap()
        .points;
    let zonal = &reference
        .sets
        .iter()
        .find(|s| (s.degree, s.order) == (4, 0))
        .unwrap()
        .points;
    let mut max_rel = 0.0f64;
    for (f, z) in full.iter().zip(zonal) {
        assert_eq!(f.position_km, z.position_km);
        let df = (Vector3::from(f.acceleration_km_s2) - Vector3::from(z.acceleration_km_s2)).norm();
        max_rel = max_rel.max(df / Vector3::from(z.acceleration_km_s2).norm());
    }
    // Tesseral + higher zonal terms are a few 1e-3 of J2 at LEO altitudes.
    assert!(max_rel > 1e-3, "70x70 − 4x0 too small: {max_rel:e}");
}
