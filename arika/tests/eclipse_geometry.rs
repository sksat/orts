//! Pure-geometry oracle tests for `arika::eclipse`.
//!
//! These tests verify the eclipse illumination computation using
//! hand-designed geometric test vectors. Positions and radii are
//! reverse-engineered from angular parameters (a, b, c) so they
//! are independent of any epoch, orbit, or ephemeris.
//!
//! # Angle-based test vector construction
//!
//! Given desired angular radii a (light), b (occulter), and separation c:
//!
//! ```text
//! observer  = (0, 0, 0)
//! light_pos = (D_l, 0, 0)
//! occulter_pos = (D_o cos c, D_o sin c, 0)
//! light_radius = D_l sin a
//! occulter_radius = D_o sin b
//! ```
//!
//! where D_l, D_o are arbitrary positive distances (chosen for numerical
//! convenience, e.g. 1e6 km for light, 1e4 km for occulter).

use arika::eclipse::{self, ShadowModel};
use nalgebra::Vector3;

// ── helpers ─────────────────────────────────────────────────────

const ZERO: Vector3<f64> = Vector3::new(0.0, 0.0, 0.0);

/// Build a test case from angular parameters.
///
/// - `a`: apparent angular radius of the light source [rad]
/// - `b`: apparent angular radius of the occulter [rad]
/// - `c`: angular separation between light and occulter centers [rad]
/// - `d_light`: distance from observer to light source [km]
/// - `d_occulter`: distance from observer to occulter [km]
struct GeometryCase {
    observer: Vector3<f64>,
    light_pos: Vector3<f64>,
    occulter_pos: Vector3<f64>,
    light_radius: f64,
    occulter_radius: f64,
}

impl GeometryCase {
    fn from_angles(a: f64, b: f64, c: f64, d_light: f64, d_occulter: f64) -> Self {
        Self {
            observer: ZERO,
            light_pos: Vector3::new(d_light, 0.0, 0.0),
            occulter_pos: Vector3::new(d_occulter * c.cos(), d_occulter * c.sin(), 0.0),
            light_radius: d_light * a.sin(),
            occulter_radius: d_occulter * b.sin(),
        }
    }

    fn illumination(&self, model: ShadowModel) -> f64 {
        eclipse::illumination(
            &self.observer,
            &self.light_pos,
            &self.occulter_pos,
            self.light_radius,
            self.occulter_radius,
            model,
        )
    }

    fn illumination_conical(&self) -> f64 {
        self.illumination(ShadowModel::Conical)
    }

    #[allow(dead_code)]
    fn illumination_cylindrical(&self) -> f64 {
        self.illumination(ShadowModel::Cylindrical)
    }
}

fn assert_close(actual: f64, expected: f64, tol: f64, msg: &str) {
    let err = (actual - expected).abs();
    assert!(
        err < tol,
        "{msg}: expected {expected:.12e}, got {actual:.12e}, err={err:.3e} (tol={tol:.0e})"
    );
}

// Typical distances
const D_LIGHT: f64 = 1e6; // 1,000,000 km (Sun-like distance)
const D_OCCULTER: f64 = 1e4; // 10,000 km (Earth-like distance)

// ── Full sun (c >= a + b) ───────────────────────────────────────

#[test]
fn conical_full_sun_wide_separation() {
    // Large separation: occulter is far from the light disk
    let a = 0.01_f64; // ~0.57°
    let b = 0.02;
    let c = 0.5; // well beyond a + b = 0.03
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        1.0,
        1e-12,
        "full sun wide separation",
    );
}

#[test]
fn conical_full_sun_barely_separated() {
    // Just barely separated: c = a + b + small epsilon
    let a = 0.01_f64;
    let b = 0.02;
    let c = a + b + 1e-8;
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        1.0,
        1e-6,
        "full sun barely separated",
    );
}

// ── Total eclipse / umbra (b >= a && c <= b - a) ────────────────

#[test]
fn conical_total_eclipse_centered() {
    // Occulter completely covers light, centered (c = 0)
    let a = 0.005_f64; // light is small
    let b = 0.02; // occulter is large
    let c = 0.0;
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        0.0,
        1e-12,
        "total eclipse centered",
    );
}

#[test]
fn conical_total_eclipse_offset() {
    // Occulter covers light, but offset by less than b - a
    let a = 0.005_f64;
    let b = 0.02;
    let c = 0.01; // c < b - a = 0.015
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        0.0,
        1e-12,
        "total eclipse offset",
    );
}

#[test]
fn conical_total_eclipse_boundary() {
    // Exactly at umbra boundary: c = b - a
    let a = 0.005_f64;
    let b = 0.02;
    let c = b - a; // = 0.015
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    // At the boundary, illumination should be 0.0 (just entering penumbra)
    // Tolerance is generous because this is exactly at the branch point.
    assert_close(
        case.illumination_conical(),
        0.0,
        1e-7,
        "total eclipse boundary",
    );
}

// ── Annular eclipse (a > b && c <= a - b) ───────────────────────

#[test]
fn conical_annular_centered() {
    // Small occulter inside large light disk, centered
    let a = 0.02_f64; // light disk is larger
    let b = 0.01; // occulter is smaller
    let c = 0.0;
    let expected = 1.0 - (b / a).powi(2); // = 1 - 0.25 = 0.75
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        expected,
        1e-10,
        "annular centered",
    );
}

#[test]
fn conical_annular_offset() {
    // Small occulter inside large light disk, offset
    let a = 0.02_f64;
    let b = 0.005;
    let c = 0.005; // c < a - b = 0.015
    let expected = 1.0 - (b / a).powi(2); // = 1 - 0.0625 = 0.9375
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        expected,
        1e-10,
        "annular offset",
    );
}

#[test]
fn conical_annular_boundary() {
    // Exactly at annular boundary: c = a - b
    let a = 0.02_f64;
    let b = 0.01;
    let c = a - b; // = 0.01
    let expected = 1.0 - (b / a).powi(2); // = 0.75
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        expected,
        1e-9,
        "annular boundary",
    );
}

// ── Partial eclipse / penumbra ──────────────────────────────────

#[test]
fn conical_partial_half_overlap_symmetric() {
    // Equal radii, half overlapping: a = b, c = a
    // When a = b and c = a, the overlap area has a known closed form.
    // A = 2a² acos(c/(2a)) - (c/2) sqrt(4a² - c²)
    // With c = a: A = 2a² acos(1/2) - (a/2) sqrt(3a²)
    //            = 2a² (π/3) - (a²√3)/2
    // illumination = 1 - A / (π a²) = 1 - (2/3 - √3/(2π))
    //             = 1/3 + √3/(2π) ≈ 0.6090...
    let a = 0.01_f64;
    let b = a;
    let c = a;
    let expected = 1.0 / 3.0 + 3.0_f64.sqrt() / (2.0 * std::f64::consts::PI);
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        expected,
        1e-6,
        "partial half overlap symmetric",
    );
}

#[test]
fn conical_partial_in_between() {
    // Partial eclipse: illumination should be strictly between 0 and 1
    let a = 0.01_f64;
    let b = 0.015;
    let c = 0.02; // between |b-a|=0.005 and a+b=0.025
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    let illum = case.illumination_conical();
    assert!(
        illum > 0.0 && illum < 1.0,
        "partial eclipse should be in (0,1), got {illum}"
    );
}

// ── Tangent cases ───────────────────────────────────────────────

#[test]
fn conical_tangent_outer() {
    // Disks are externally tangent: c = a + b
    // Illumination should be very close to 1.0 (just touching)
    let a = 0.01_f64;
    let b = 0.02;
    let c = a + b;
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(case.illumination_conical(), 1.0, 1e-6, "tangent outer");
}

#[test]
fn conical_tangent_inner_umbra() {
    // Disks are internally tangent with b > a: c = b - a
    // Illumination should be very close to 0.0 (light disk just fits inside occulter)
    let a = 0.005_f64;
    let b = 0.02;
    let c = b - a;
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        0.0,
        1e-6,
        "tangent inner umbra",
    );
}

#[test]
fn conical_tangent_inner_annular() {
    // Disks are internally tangent with a > b: c = a - b
    // Illumination should be close to the annular value: 1 - (b/a)²
    let a = 0.02_f64;
    let b = 0.005;
    let c = a - b;
    let expected = 1.0 - (b / a).powi(2);
    let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
    assert_close(
        case.illumination_conical(),
        expected,
        1e-6,
        "tangent inner annular",
    );
}

// ── Occulter behind observer ────────────────────────────────────

#[test]
fn conical_occulter_behind_observer() {
    // Occulter is behind the observer (between observer and nothing, away from light)
    // This should result in full illumination — no eclipse possible.
    let case = GeometryCase {
        observer: ZERO,
        light_pos: Vector3::new(1e6, 0.0, 0.0),
        occulter_pos: Vector3::new(-1e4, 0.0, 0.0), // behind observer
        light_radius: 695700.0,
        occulter_radius: 6371.0,
    };
    assert_close(
        case.illumination_conical(),
        1.0,
        1e-12,
        "occulter behind observer",
    );
}

#[test]
fn conical_occulter_beside_observer() {
    // Occulter is at 90° from the light direction — no eclipse
    let case = GeometryCase {
        observer: ZERO,
        light_pos: Vector3::new(1e6, 0.0, 0.0),
        occulter_pos: Vector3::new(0.0, 1e4, 0.0), // perpendicular
        light_radius: 695700.0,
        occulter_radius: 6371.0,
    };
    assert_close(
        case.illumination_conical(),
        1.0,
        1e-12,
        "occulter beside observer",
    );
}

// ── Monotonicity ────────────────────────────────────────────────

#[test]
fn conical_monotonicity_umbra_to_sun() {
    // As c increases from 0 to a + b, illumination should be non-decreasing.
    let a = 0.005_f64;
    let b = 0.02;
    let n = 100;
    let c_max = a + b + 0.001; // slightly beyond full sun
    let mut prev = -1.0_f64;
    for i in 0..=n {
        let c = c_max * (i as f64) / (n as f64);
        let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
        let illum = case.illumination_conical();
        assert!(
            illum >= prev - 1e-10,
            "monotonicity violated at c={c:.6}: prev={prev:.10}, current={illum:.10}"
        );
        prev = illum;
    }
}

#[test]
fn conical_monotonicity_annular_to_sun() {
    // For a > b (annular case), as c increases from 0 to a + b,
    // illumination should be non-decreasing.
    let a = 0.02_f64;
    let b = 0.005;
    let n = 100;
    let c_max = a + b + 0.001;
    let mut prev = -1.0_f64;
    for i in 0..=n {
        let c = c_max * (i as f64) / (n as f64);
        let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
        let illum = case.illumination_conical();
        assert!(
            illum >= prev - 1e-10,
            "monotonicity violated at c={c:.6}: prev={prev:.10}, current={illum:.10}"
        );
        prev = illum;
    }
}

// ── Continuity at boundaries ────────────────────────────────────

#[test]
fn conical_continuity_at_outer_tangent() {
    // illumination should be continuous at c = a + b
    let a = 0.01_f64;
    let b = 0.02;
    let eps = 1e-8;
    let c_boundary = a + b;
    let case_inside = GeometryCase::from_angles(a, b, c_boundary - eps, D_LIGHT, D_OCCULTER);
    let case_outside = GeometryCase::from_angles(a, b, c_boundary + eps, D_LIGHT, D_OCCULTER);
    let illum_inside = case_inside.illumination_conical();
    let illum_outside = case_outside.illumination_conical();
    let diff = (illum_inside - illum_outside).abs();
    assert!(
        diff < 1e-4,
        "discontinuity at outer tangent: inside={illum_inside:.10}, outside={illum_outside:.10}, diff={diff:.3e}"
    );
}

#[test]
fn conical_continuity_at_inner_tangent_umbra() {
    // illumination should be continuous at c = b - a (when b > a)
    let a = 0.005_f64;
    let b = 0.02;
    let eps = 1e-8;
    let c_boundary = b - a;
    let case_inside = GeometryCase::from_angles(a, b, c_boundary - eps, D_LIGHT, D_OCCULTER);
    let case_outside = GeometryCase::from_angles(a, b, c_boundary + eps, D_LIGHT, D_OCCULTER);
    let illum_inside = case_inside.illumination_conical();
    let illum_outside = case_outside.illumination_conical();
    let diff = (illum_inside - illum_outside).abs();
    assert!(
        diff < 1e-4,
        "discontinuity at inner tangent (umbra): inside={illum_inside:.10}, outside={illum_outside:.10}, diff={diff:.3e}"
    );
}

#[test]
fn conical_continuity_at_inner_tangent_annular() {
    // illumination should be continuous at c = a - b (when a > b)
    let a = 0.02_f64;
    let b = 0.005;
    let eps = 1e-8;
    let c_boundary = a - b;
    let case_inside = GeometryCase::from_angles(a, b, c_boundary - eps, D_LIGHT, D_OCCULTER);
    let case_outside = GeometryCase::from_angles(a, b, c_boundary + eps, D_LIGHT, D_OCCULTER);
    let illum_inside = case_inside.illumination_conical();
    let illum_outside = case_outside.illumination_conical();
    let diff = (illum_inside - illum_outside).abs();
    assert!(
        diff < 1e-4,
        "discontinuity at inner tangent (annular): inside={illum_inside:.10}, outside={illum_outside:.10}, diff={diff:.3e}"
    );
}

// ── Earth-like realistic geometry ───────────────────────────────

#[test]
fn conical_leo_full_shadow() {
    // ISS-like orbit behind Earth: should be in full shadow
    // Sun at ~1 AU, Earth radius 6371 km, satellite at 400 km altitude
    // behind Earth (anti-Sun side)
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let earth_pos = ZERO;
    let sat_pos = Vector3::new(-(6371.0 + 400.0), 0.0, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &earth_pos,
        695700.0,
        6371.0,
        ShadowModel::Conical,
    );
    assert_close(illum, 0.0, 1e-10, "LEO full shadow");
}

#[test]
fn conical_leo_sunlit() {
    // ISS-like orbit on Sun side: should be fully sunlit
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let earth_pos = ZERO;
    let sat_pos = Vector3::new(6371.0 + 400.0, 0.0, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &earth_pos,
        695700.0,
        6371.0,
        ShadowModel::Conical,
    );
    assert_close(illum, 1.0, 1e-12, "LEO sunlit");
}

#[test]
fn conical_geo_penumbra_transition_wider_than_leo() {
    // The penumbra transition zone (from full shadow to full sun) should
    // be wider in absolute terms at GEO than at LEO, because the Sun's
    // angular radius stays ~constant while Earth's angular radius shrinks.
    //
    // We measure the transition width by finding the perpendicular offset
    // range where illumination goes from ~0.05 to ~0.95.
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let earth_pos = ZERO;
    let r_earth = 6371.0;
    let r_sun = 695700.0;

    // Find penumbra width at a given altitude
    let penumbra_width = |alt: f64| -> f64 {
        let x = -(r_earth + alt);
        let mut y_low = 0.0_f64;
        let mut y_high = 0.0_f64;
        // Sweep perpendicular offset from 0 to 2*R_earth
        for i in 0..20000 {
            let y = r_earth * 2.0 * (i as f64) / 20000.0;
            let sat = Vector3::new(x, y, 0.0);
            let illum = eclipse::illumination(
                &sat,
                &sun_pos,
                &earth_pos,
                r_sun,
                r_earth,
                ShadowModel::Conical,
            );
            if illum > 0.05 && y_low == 0.0 {
                y_low = y;
            }
            if illum > 0.95 {
                y_high = y;
                break;
            }
        }
        y_high - y_low
    };

    let leo_pw = penumbra_width(400.0);
    let geo_pw = penumbra_width(35786.0);

    assert!(
        geo_pw > leo_pw,
        "GEO penumbra transition should be wider: LEO={leo_pw:.1} km, GEO={geo_pw:.1} km"
    );
}

// ── Cylindrical model tests ─────────────────────────────────────

#[test]
fn cylindrical_sunlit() {
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(6371.0 + 400.0, 0.0, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Cylindrical,
    );
    assert_close(illum, 1.0, 1e-12, "cylindrical sunlit");
}

#[test]
fn cylindrical_umbra() {
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(-(6371.0 + 400.0), 0.0, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Cylindrical,
    );
    assert_close(illum, 0.0, 1e-12, "cylindrical umbra");
}

#[test]
fn cylindrical_perpendicular() {
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(0.0, 6371.0 + 400.0, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Cylindrical,
    );
    assert_close(illum, 1.0, 1e-12, "cylindrical perpendicular");
}

#[test]
fn cylindrical_just_inside_shadow() {
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(-(6371.0 + 400.0), 6371.0 * 0.5, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Cylindrical,
    );
    assert_close(illum, 0.0, 1e-12, "cylindrical just inside shadow");
}

#[test]
fn cylindrical_just_outside_shadow() {
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(-(6371.0 + 400.0), 6371.0 * 1.1, 0.0);
    let illum = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Cylindrical,
    );
    assert_close(illum, 1.0, 1e-12, "cylindrical just outside shadow");
}

// ── Cylindrical is binary (no intermediate values) ──────────────

#[test]
fn cylindrical_is_binary() {
    // Sweep through positions and verify cylindrical only returns 0 or 1
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let r_earth = 6371.0;
    for i in 0..360 {
        let angle = (i as f64) * std::f64::consts::PI / 180.0;
        let r = r_earth + 400.0;
        let sat_pos = Vector3::new(r * angle.cos(), r * angle.sin(), 0.0);
        let illum = eclipse::illumination(
            &sat_pos,
            &sun_pos,
            &ZERO,
            695700.0,
            r_earth,
            ShadowModel::Cylindrical,
        );
        assert!(
            illum == 0.0 || illum == 1.0,
            "cylindrical should be binary, got {illum} at angle {i}°"
        );
    }
}

// ── illumination_central convenience wrapper ────────────────────

#[test]
fn illumination_central_matches_full_api() {
    // illumination_central should give the same result as illumination
    // with occulter_pos = [0,0,0]
    let sun_pos = Vector3::new(149_597_870.7, 0.0, 0.0);
    let sat_pos = Vector3::new(-(6371.0 + 400.0), 1000.0, 0.0);

    let full = eclipse::illumination(
        &sat_pos,
        &sun_pos,
        &ZERO,
        695700.0,
        6371.0,
        ShadowModel::Conical,
    );
    let central =
        eclipse::illumination_central(&sat_pos, &sun_pos, 6371.0, 695700.0, ShadowModel::Conical);
    assert_close(
        central,
        full,
        1e-15,
        "illumination_central matches full API",
    );
}

// ── Output range ────────────────────────────────────────────────

#[test]
fn conical_output_always_in_0_1() {
    // Sweep various geometries and verify output is always in [0, 1]
    let a_values = [0.001, 0.005, 0.01, 0.02, 0.05];
    let b_values = [0.001, 0.005, 0.01, 0.02, 0.05];
    let c_values = [0.0, 0.001, 0.005, 0.01, 0.02, 0.03, 0.05, 0.1, 0.5];

    for &a in &a_values {
        for &b in &b_values {
            for &c in &c_values {
                let case = GeometryCase::from_angles(a, b, c, D_LIGHT, D_OCCULTER);
                let illum = case.illumination_conical();
                assert!(
                    (0.0..=1.0).contains(&illum),
                    "illumination out of [0,1]: a={a}, b={b}, c={c}, illum={illum}"
                );
            }
        }
    }
}
