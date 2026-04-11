//! IAU rotation models for celestial bodies — shared infrastructure.
//!
//! Defines [`IauRotationModel`], the generic (α₀ + α₁·T, δ₀ + δ₁·T, W₀ + Wᵈ·d)
//! linear form of the IAU/IAG Working Group on Cartographic Coordinates and
//! Rotational Elements (2009) body rotation convention, and a runtime
//! dispatcher [`body_orientation`] that resolves a body name to the best
//! available model.
//!
//! Per-body model constants live under each body's own module:
//!
//! - [`crate::earth::rotation::EARTH`]
//! - [`crate::moon::rotation::MOON`] + [`crate::moon::moon_orientation`]
//!   (libration-corrected variant, preferred for the Moon)
//! - [`crate::sun::rotation::SUN`] (via [`crate::sun::SUN`] re-export)
//! - [`crate::planets::rotation::MARS`]
//!
//! Phase 3 will add strict IAU 2006 CIO-based precession / nutation / ERA /
//! polar motion models under `earth/` as a sibling to this dispatcher.
//!
//! Reference: Archinal et al. (2011), "Report of the IAU Working Group on
//! Cartographic Coordinates and Rotational Elements: 2009",
//! Celestial Mechanics and Dynamical Astronomy, 109(2), 101–135.

use nalgebra::{Matrix3, UnitQuaternion, Vector3};

use crate::epoch::{Epoch, Tdb};

/// IAU rotation model parameters for a celestial body.
///
/// The north pole direction (right ascension α, declination δ) and
/// prime meridian angle W are given as linear functions of time:
///
///   α = α₀ + α₁ T   [degrees]
///   δ = δ₀ + δ₁ T   [degrees]
///   W = W₀ + W_d d   [degrees]
///
/// where T = Julian centuries since J2000, d = Julian days since J2000.
#[derive(Debug, Clone, Copy)]
pub struct IauRotationModel {
    /// Right ascension of north pole at J2000 [deg]
    pub alpha0: f64,
    /// Rate of right ascension per Julian century [deg/century]
    pub alpha1: f64,
    /// Declination of north pole at J2000 [deg]
    pub delta0: f64,
    /// Rate of declination per Julian century [deg/century]
    pub delta1: f64,
    /// Prime meridian angle at J2000 [deg]
    pub w0: f64,
    /// Prime meridian rate [deg/day]
    pub wd: f64,
}

/// Look up the base IAU rotation model for a body by name.
///
/// For the Moon, prefer [`body_orientation`] which uses the libration-corrected
/// variant [`crate::moon::moon_orientation`] instead of the base linear model.
pub fn model_for_body(name: &str) -> Option<&'static IauRotationModel> {
    match name {
        "earth" => Some(&crate::earth::rotation::EARTH),
        "moon" => Some(&crate::moon::MOON),
        "mars" => Some(&crate::planets::rotation::MARS),
        "sun" => Some(&crate::sun::SUN),
        _ => None,
    }
}

/// Compute the body-fixed → ECI orientation for a named body.
///
/// Uses the best available model: libration-corrected for the Moon,
/// base IAU model for other bodies. Returns `None` for unknown bodies.
///
/// # Time scale
///
/// Takes `&Epoch<Tdb>` because IAU WGCCRE 2009 specifies the W/α/δ polynomials
/// in TDB days from J2000. See [`crate::moon::moon_orientation`] for details.
pub fn body_orientation(name: &str, epoch: &Epoch<Tdb>) -> Option<UnitQuaternion<f64>> {
    match name {
        "moon" => Some(crate::moon::moon_orientation(epoch)),
        _ => model_for_body(name).map(|m| m.orientation(epoch)),
    }
}

impl IauRotationModel {
    /// Compute the body-fixed → ECI (J2000) unit quaternion at the given epoch.
    ///
    /// The returned quaternion transforms vectors from the body-fixed frame
    /// (Z = north pole, X = prime meridian) to the ECI/ICRF frame.
    ///
    /// Takes `&Epoch<Tdb>` — IAU WGCCRE 2009 (Archinal et al. 2011) defines the
    /// independent variable as "interval in Julian days from J2000 in TDB".
    pub fn orientation(&self, epoch: &Epoch<Tdb>) -> UnitQuaternion<f64> {
        let d = epoch.jd() - 2451545.0; // days since J2000 (TDB)
        let t = d / 36525.0; // Julian centuries since J2000

        let alpha = (self.alpha0 + self.alpha1 * t).to_radians();
        let delta = (self.delta0 + self.delta1 * t).to_radians();
        let w = (self.w0 + self.wd * d).to_radians();

        // Body-fixed frame axes in ECI:
        //   Z_body = pole direction
        //   X_body = prime meridian direction (in equator plane, rotated by W)

        // Pole direction (Z_body in ECI)
        let z_body = Vector3::new(
            alpha.cos() * delta.cos(),
            alpha.sin() * delta.cos(),
            delta.sin(),
        );

        // Node direction: intersection of body equator with ECI equator
        // Perpendicular to pole in ECI equatorial plane
        let node = Vector3::new(-alpha.sin(), alpha.cos(), 0.0);

        // Complete the right-handed frame: m = z_body × node
        let m = z_body.cross(&node);

        // Prime meridian direction: rotate node by W around z_body
        let x_body = node * w.cos() + m * w.sin();

        // Y_body = Z_body × X_body
        let y_body = z_body.cross(&x_body);

        // Rotation matrix: columns are body axes expressed in ECI
        let rot = Matrix3::from_columns(&[x_body, y_body, z_body]);

        UnitQuaternion::from_rotation_matrix(&nalgebra::Rotation3::from_matrix_unchecked(rot))
    }

    /// Compute the prime meridian angle W at the given epoch [radians].
    pub fn prime_meridian_angle(&self, epoch: &Epoch<Tdb>) -> f64 {
        let d = epoch.jd() - 2451545.0;
        (self.w0 + self.wd * d).to_radians()
    }

    /// Compute the north pole direction in ECI at the given epoch.
    pub fn pole_direction(&self, epoch: &Epoch<Tdb>) -> Vector3<f64> {
        let d = epoch.jd() - 2451545.0;
        let t = d / 36525.0;
        let alpha = (self.alpha0 + self.alpha1 * t).to_radians();
        let delta = (self.delta0 + self.delta1 * t).to_radians();
        Vector3::new(
            alpha.cos() * delta.cos(),
            alpha.sin() * delta.cos(),
            delta.sin(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_axes_are_orthonormal_via_dispatch() {
        // Iterate via `model_for_body` so this test also exercises the
        // dispatch path (and naturally covers every body the dispatcher
        // knows about, without touching per-body module consts directly).
        let epoch = Epoch::from_gregorian(2024, 6, 15, 0, 0, 0.0).to_tdb();
        for name in ["moon", "mars", "sun", "earth"] {
            let model = model_for_body(name).expect("known body");
            let q = model.orientation(&epoch);
            let rot = q.to_rotation_matrix();
            let m = rot.matrix();

            // Check orthogonality: M^T * M ≈ I
            let mtm = m.transpose() * m;
            let identity = nalgebra::Matrix3::<f64>::identity();
            let err = (mtm - identity).norm();
            assert!(
                err < 1e-10,
                "{name} rotation matrix should be orthogonal, error = {err}"
            );

            // Check determinant ≈ 1 (proper rotation)
            let det = m.determinant();
            assert!(
                (det - 1.0).abs() < 1e-10,
                "{name} rotation determinant should be 1, got {det}"
            );
        }
    }

    #[test]
    fn model_for_body_unknown_returns_none() {
        assert!(model_for_body("pluto").is_none());
        assert!(model_for_body("").is_none());
    }

    #[test]
    fn body_orientation_unknown_returns_none() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
        assert!(body_orientation("pluto", &epoch).is_none());
    }

    #[test]
    fn body_orientation_moon_uses_libration_variant() {
        // The dispatcher for "moon" should return the libration-corrected
        // variant, which differs measurably from the base linear model.
        let epoch = Epoch::from_gregorian(2024, 3, 15, 0, 0, 0.0).to_tdb();
        let q_dispatch = body_orientation("moon", &epoch).unwrap();
        let q_base = model_for_body("moon").unwrap().orientation(&epoch);
        let angle_diff = q_dispatch.angle_to(&q_base).to_degrees();
        assert!(
            angle_diff > 0.1,
            "dispatcher should apply libration, got angle_diff={angle_diff:.4}°"
        );
    }

    /// Generate fixture quaternions for viewer cross-validation tests.
    /// Run with `cargo test -p kaname generate_fixture -- --nocapture` to see output.
    /// Moon uses the libration model; other bodies use the base model.
    #[test]
    fn generate_fixture_quaternions() {
        let cases = [
            ("moon", 2440418.064 + 723374.0 / 86400.0, "apollo11_end"),
            ("moon", 2440418.064, "apollo11_start"),
            ("mars", 2451545.0, "j2000"),
            ("earth", 2451545.0, "j2000"),
        ];
        println!("--- IAU orientation fixture ---");
        for (body, jd, label) in &cases {
            let epoch = Epoch::<Tdb>::from_jd_tdb(*jd);
            let q = body_orientation(body, &epoch).unwrap();
            println!(
                r#"  {{ body: "{body}", jd: {jd}, label: "{label}", q: [{:.15}, {:.15}, {:.15}, {:.15}] }},"#,
                q.w, q.i, q.j, q.k
            );
        }
        println!("---");
    }
}
