//! Eclipse / shadow geometry.
//!
//! Computes the illumination fraction of an observer point given a
//! light source and an occulting body. The computation is pure geometry
//! (positions + radii → illumination) with no epoch, frame, or
//! ephemeris dependency.
//!
//! Two models are provided:
//! - [`ShadowModel::Cylindrical`]: Binary 0/1 (infinite shadow cylinder).
//! - [`ShadowModel::Conical`]: Continuous \[0, 1\] with penumbra, using
//!   the apparent-disk overlap area ratio (Montenbruck & Gill §3.4.2).
//!
//! # API
//!
//! The primary entry point is [`illumination`], which takes fully generic
//! positions (observer, light source, occulting body) and radii.
//! [`illumination_central`] is a convenience wrapper for the common case
//! where the occulting body is at the coordinate origin.
//!
//! # References
//!
//! - Montenbruck, O. and Gill, E., *Satellite Orbits*, §3.4.2
//! - Orekit `ConicallyShadowedLightFluxModel`

#[allow(unused_imports)]
use crate::math::F64Ext;
use nalgebra::Vector3;

/// Sun mean radius \[km\].
pub const SUN_RADIUS_KM: f64 = 695700.0;

/// Shadow model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowModel {
    /// Cylindrical shadow: binary 0 (umbra) or 1 (sunlit).
    ///
    /// The shadow cylinder has radius = `occulter_radius` and axis along
    /// the occulter→light direction. Fast but no penumbra.
    Cylindrical,

    /// Conical shadow: continuous illumination \[0, 1\] with penumbra.
    ///
    /// Uses the apparent-disk overlap area ratio (two-circle intersection)
    /// from Montenbruck & Gill. Handles total eclipse (umbra), annular
    /// eclipse (transit), and partial eclipse (penumbra).
    Conical,
}

/// Compute the illumination fraction at `observer_pos`.
///
/// All positions must be in the same coordinate frame and units (km).
///
/// Returns a value in \[0.0, 1.0\]:
/// - 1.0 = fully sunlit (no eclipse)
/// - 0.0 = total eclipse (umbra)
/// - intermediate = partial eclipse (penumbra or annular)
///
/// # Arguments
///
/// - `observer_pos` — position of the observer (e.g. satellite)
/// - `light_pos` — position of the light source (e.g. Sun)
/// - `occulter_pos` — position of the occulting body (e.g. Earth)
/// - `light_radius` — radius of the light source \[km\]
/// - `occulter_radius` — radius of the occulting body \[km\]
/// - `model` — shadow model to use
pub fn illumination(
    observer_pos: &Vector3<f64>,
    light_pos: &Vector3<f64>,
    occulter_pos: &Vector3<f64>,
    light_radius: f64,
    occulter_radius: f64,
    model: ShadowModel,
) -> f64 {
    match model {
        ShadowModel::Cylindrical => {
            illumination_cylindrical(observer_pos, light_pos, occulter_pos, occulter_radius)
        }
        ShadowModel::Conical => illumination_conical(
            observer_pos,
            light_pos,
            occulter_pos,
            light_radius,
            occulter_radius,
        ),
    }
}

/// Convenience wrapper for [`illumination`] when the occulting body is
/// at the coordinate origin.
///
/// This matches the common Earth-centered use case where satellite and
/// Sun positions are both relative to Earth's center.
pub fn illumination_central(
    observer_pos: &Vector3<f64>,
    light_pos: &Vector3<f64>,
    occulter_radius: f64,
    light_radius: f64,
    model: ShadowModel,
) -> f64 {
    illumination(
        observer_pos,
        light_pos,
        &Vector3::zeros(),
        light_radius,
        occulter_radius,
        model,
    )
}

// ── Cylindrical model ───────────────────────────────────────────

/// Cylindrical shadow model (binary 0/1).
///
/// Returns 0.0 if the observer is in the infinite shadow cylinder
/// cast by the occulting body, 1.0 otherwise.
fn illumination_cylindrical(
    observer_pos: &Vector3<f64>,
    light_pos: &Vector3<f64>,
    occulter_pos: &Vector3<f64>,
    occulter_radius: f64,
) -> f64 {
    // Vector from occulter to observer
    let obs_rel = observer_pos - occulter_pos;
    // Vector from occulter to light
    let light_rel = light_pos - occulter_pos;
    let light_dir = light_rel.normalize();

    // Project observer position onto the occulter→light axis
    let projection = obs_rel.dot(&light_dir);

    // If observer is on the light-side of the occulter, it's sunlit
    if projection >= 0.0 {
        return 1.0;
    }

    // Observer is on the anti-light side. Check perpendicular distance.
    let perp = obs_rel - projection * light_dir;
    let perp_dist = perp.magnitude();

    if perp_dist < occulter_radius {
        0.0 // in shadow cylinder
    } else {
        1.0 // outside shadow cylinder
    }
}

// ── Conical model ───────────────────────────────────────────────

/// Conical shadow model with penumbra (Montenbruck & Gill §3.4.2).
///
/// Computes illumination as the fraction of the light disk visible
/// from the observer, considering the occulting body as a sphere.
fn illumination_conical(
    observer_pos: &Vector3<f64>,
    light_pos: &Vector3<f64>,
    occulter_pos: &Vector3<f64>,
    light_radius: f64,
    occulter_radius: f64,
) -> f64 {
    // Vectors from observer to light and occulter
    let obs_to_light = light_pos - observer_pos;
    let obs_to_occulter = occulter_pos - observer_pos;

    let d_light = obs_to_light.magnitude();
    let d_occulter = obs_to_occulter.magnitude();

    // Guard: degenerate geometry
    if d_light < 1e-10 || d_occulter < 1e-10 {
        return 1.0;
    }

    // Check if occulter is behind the observer (away from light)
    // If the angle between obs→light and obs→occulter is > 90°,
    // the occulter cannot occult the light.
    let cos_angle = obs_to_light.dot(&obs_to_occulter) / (d_light * d_occulter);
    if cos_angle < 0.0 {
        return 1.0; // occulter is behind the observer
    }

    // Apparent angular radii as seen from the observer
    let sin_a = (light_radius / d_light).clamp(-1.0, 1.0);
    let sin_b = (occulter_radius / d_occulter).clamp(-1.0, 1.0);
    let a = sin_a.asin(); // apparent angular radius of light source
    let b = sin_b.asin(); // apparent angular radius of occulter

    // Angular separation between centers
    let c = cos_angle.clamp(-1.0, 1.0).acos();

    // Classify eclipse type
    if c >= a + b {
        // No overlap — fully sunlit
        return 1.0;
    }

    if b >= a && c <= b - a {
        // Total eclipse (umbra): occulter completely covers light
        return 0.0;
    }

    if a > b && c <= a - b {
        // Annular eclipse: occulter is inside the light disk
        // Illumination = 1 - (area of occulter disk / area of light disk)
        // = 1 - (b/a)²
        let ratio = b / a;
        return 1.0 - ratio * ratio;
    }

    // Partial eclipse: compute two-circle overlap area
    partial_eclipse_illumination(a, b, c)
}

/// Compute illumination for partial eclipse using two-circle overlap.
///
/// Given apparent angular radii `a` (light) and `b` (occulter) and
/// angular separation `c`, the overlap area of two circles on the
/// unit sphere (small-angle approximation) is:
///
/// ```text
/// A = a² acos((c² + a² - b²) / (2ca))
///   + b² acos((c² + b² - a²) / (2cb))
///   - 0.5 sqrt((-c+a+b)(c+a-b)(c-a+b)(c+a+b))
/// ```
///
/// illumination = 1 - A / (π a²)
fn partial_eclipse_illumination(a: f64, b: f64, c: f64) -> f64 {
    let a2 = a * a;
    let b2 = b * b;
    let c2 = c * c;

    // acos arguments — clamp for numerical safety
    let arg1 = ((c2 + a2 - b2) / (2.0 * c * a)).clamp(-1.0, 1.0);
    let arg2 = ((c2 + b2 - a2) / (2.0 * c * b)).clamp(-1.0, 1.0);

    // Heron-like product under the square root
    let s1 = -c + a + b;
    let s2 = c + a - b;
    let s3 = c - a + b;
    let s4 = c + a + b;
    let product = s1 * s2 * s3 * s4;

    // Guard against negative product due to floating-point rounding
    let sqrt_term = if product > 0.0 { product.sqrt() } else { 0.0 };

    let overlap = a2 * arg1.acos() + b2 * arg2.acos() - 0.5 * sqrt_term;

    let illum = 1.0 - overlap / (core::f64::consts::PI * a2);

    // Clamp to [0, 1] for numerical safety
    illum.clamp(0.0, 1.0)
}
