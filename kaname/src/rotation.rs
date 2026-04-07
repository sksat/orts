//! IAU rotation models for celestial bodies.
//!
//! Implements the IAU/IAG Working Group on Cartographic Coordinates and
//! Rotational Elements (2009) model for known bodies. Given an epoch,
//! computes the body-fixed → ICRF/J2000 (ECI) orientation as a unit quaternion.
//!
//! Reference: Archinal et al. (2011), "Report of the IAU Working Group on
//! Cartographic Coordinates and Rotational Elements: 2009",
//! Celestial Mechanics and Dynamical Astronomy, 109(2), 101–135.

use nalgebra::{Matrix3, UnitQuaternion, Vector3};

use crate::epoch::Epoch;

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

/// IAU 2009 rotation model for the Moon (base linear terms).
///
/// For higher accuracy, use [`moon_orientation()`] which includes the
/// 13-term periodic libration corrections from the IAU 2009 report.
pub const MOON: IauRotationModel = IauRotationModel {
    alpha0: 269.9949,
    alpha1: 0.0031,
    delta0: 66.5392,
    delta1: 0.0130,
    w0: 38.3213,
    wd: 13.17635815,
};

/// Compute the 13 nutation/libration arguments E1–E13 for the Moon [radians].
///
/// These are linear functions of d (days since J2000) used in the periodic
/// corrections to the Moon's pole direction and prime meridian.
/// Reference: Archinal et al. (2011), Table 2.
fn moon_nutation_args(d: f64) -> [f64; 13] {
    [
        (125.045 - 0.0529921 * d).to_radians(),  // E1
        (250.089 - 0.1059842 * d).to_radians(),  // E2
        (260.008 + 13.0120009 * d).to_radians(), // E3
        (176.625 + 13.3407154 * d).to_radians(), // E4
        (357.529 + 0.9856003 * d).to_radians(),  // E5
        (311.589 + 26.4057084 * d).to_radians(), // E6
        (134.963 + 13.0649930 * d).to_radians(), // E7
        (276.617 + 0.3287146 * d).to_radians(),  // E8
        (34.226 + 1.7484877 * d).to_radians(),   // E9
        (15.134 - 0.1589763 * d).to_radians(),   // E10
        (119.743 + 0.0036096 * d).to_radians(),  // E11
        (239.961 + 0.1643573 * d).to_radians(),  // E12
        (25.053 + 12.9590088 * d).to_radians(),  // E13
    ]
}

/// Moon body-fixed → ECI orientation with IAU 2009 libration corrections.
///
/// Includes the 13-term periodic corrections to α, δ, and W from
/// Archinal et al. (2011), Table 3a/3b. These correct for nutation and
/// precession of the Moon's pole and prime meridian relative to the
/// mean orientation (ME frame).
pub fn moon_orientation(epoch: &Epoch) -> UnitQuaternion<f64> {
    let d = epoch.jd() - 2451545.0;
    let t = d / 36525.0;
    let e = moon_nutation_args(d);

    // Base linear terms
    let mut alpha_deg = 269.9949 + 0.0031 * t;
    let mut delta_deg = 66.5392 + 0.0130 * t;
    let mut w_deg = 38.3213 + 13.17635815 * d + (-1.4e-12) * d * d;

    // Periodic corrections to α (right ascension of pole)
    // Reference: Archinal et al. (2011), Table 3a
    alpha_deg += -3.8787 * e[0].sin() - 0.1204 * e[1].sin() + 0.0700 * e[2].sin()
        - 0.0172 * e[3].sin()
        + 0.0072 * e[5].sin()
        - 0.0052 * e[9].sin()
        + 0.0043 * e[12].sin();

    // Periodic corrections to δ (declination of pole)
    delta_deg += 1.5419 * e[0].cos() + 0.0239 * e[1].cos() - 0.0278 * e[2].cos()
        + 0.0068 * e[3].cos()
        - 0.0029 * e[5].cos()
        + 0.0009 * e[6].cos()
        + 0.0008 * e[9].cos()
        - 0.0009 * e[12].cos();

    // Periodic corrections to W (prime meridian)
    w_deg += 3.5610 * e[0].sin() + 0.1208 * e[1].sin() - 0.0642 * e[2].sin()
        + 0.0158 * e[3].sin()
        + 0.0252 * e[4].sin()
        - 0.0066 * e[5].sin()
        - 0.0047 * e[6].sin()
        - 0.0046 * e[7].sin()
        + 0.0028 * e[8].sin()
        + 0.0052 * e[9].sin()
        + 0.0040 * e[10].sin()
        + 0.0019 * e[11].sin()
        - 0.0044 * e[12].sin();

    // Convert to radians and build body-fixed → ECI rotation
    let alpha = alpha_deg.to_radians();
    let delta = delta_deg.to_radians();
    let w = w_deg.to_radians();

    let z_body = Vector3::new(
        alpha.cos() * delta.cos(),
        alpha.sin() * delta.cos(),
        delta.sin(),
    );
    let node = Vector3::new(-alpha.sin(), alpha.cos(), 0.0);
    let m = z_body.cross(&node);
    let x_body = node * w.cos() + m * w.sin();
    let y_body = z_body.cross(&x_body);
    let rot = Matrix3::from_columns(&[x_body, y_body, z_body]);
    UnitQuaternion::from_rotation_matrix(&nalgebra::Rotation3::from_matrix_unchecked(rot))
}

/// IAU 2009 rotation model for Mars.
pub const MARS: IauRotationModel = IauRotationModel {
    alpha0: 317.68143,
    alpha1: -0.1061,
    delta0: 52.8865,
    delta1: -0.0609,
    w0: 176.630,
    wd: 350.89198226,
};

/// IAU 2009 rotation model for the Earth.
///
/// This model includes pole precession (α₁, δ₁ ≠ 0), so the Earth's pole
/// drifts from the ECI Z-axis over centuries. This is physically more accurate
/// for long-term simulations.
///
/// kaname also provides `Epoch::gmst()` which models Earth rotation as a
/// pure Z-rotation (pole fixed to ECI Z-axis). That simpler model is used by
/// `eci_to_ecef` and geodetic transforms, and is sufficient for short-term
/// (< decades) simulations where precession is negligible.
pub const EARTH: IauRotationModel = IauRotationModel {
    alpha0: 0.0,
    alpha1: -0.641,
    delta0: 90.0,
    delta1: -0.557,
    w0: 190.147,
    wd: 360.9856235,
};

/// IAU 2009 rotation model for the Sun.
pub const SUN: IauRotationModel = IauRotationModel {
    alpha0: 286.13,
    alpha1: 0.0,
    delta0: 63.87,
    delta1: 0.0,
    w0: 84.176,
    wd: 14.1844000,
};

/// Look up the base IAU rotation model for a body by name.
///
/// For the Moon, prefer [`body_orientation()`] which includes libration corrections.
pub fn model_for_body(name: &str) -> Option<&'static IauRotationModel> {
    match name {
        "earth" => Some(&EARTH),
        "moon" => Some(&MOON),
        "mars" => Some(&MARS),
        "sun" => Some(&SUN),
        _ => None,
    }
}

/// Compute the body-fixed → ECI orientation for a named body.
///
/// Uses the best available model: libration-corrected for the Moon,
/// base IAU model for other bodies. Returns `None` for unknown bodies.
pub fn body_orientation(name: &str, epoch: &Epoch) -> Option<UnitQuaternion<f64>> {
    match name {
        "moon" => Some(moon_orientation(epoch)),
        _ => model_for_body(name).map(|m| m.orientation(epoch)),
    }
}

impl IauRotationModel {
    /// Compute the body-fixed → ECI (J2000) unit quaternion at the given epoch.
    ///
    /// The returned quaternion transforms vectors from the body-fixed frame
    /// (Z = north pole, X = prime meridian) to the ECI/ICRF frame.
    pub fn orientation(&self, epoch: &Epoch) -> UnitQuaternion<f64> {
        let d = epoch.jd() - 2451545.0; // days since J2000
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
    pub fn prime_meridian_angle(&self, epoch: &Epoch) -> f64 {
        let d = epoch.jd() - 2451545.0;
        (self.w0 + self.wd * d).to_radians()
    }

    /// Compute the north pole direction in ECI at the given epoch.
    pub fn pole_direction(&self, epoch: &Epoch) -> Vector3<f64> {
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
    fn moon_orientation_is_unit_quaternion() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let q = MOON.orientation(&epoch);
        let norm = q.norm();
        assert!(
            (norm - 1.0).abs() < 1e-12,
            "quaternion norm should be 1, got {norm}"
        );
    }

    #[test]
    fn moon_pole_near_ecliptic_normal() {
        // Moon's north pole is ~1.54° from the ecliptic normal.
        // The ecliptic normal in ECI is approximately (0, -sin(23.44°), cos(23.44°)).
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let pole = MOON.pole_direction(&epoch);

        let obliquity = 23.44_f64.to_radians();
        let ecliptic_normal = Vector3::new(0.0, -obliquity.sin(), obliquity.cos());

        let angle = pole.angle(&ecliptic_normal).to_degrees();
        assert!(
            angle < 10.0,
            "Moon pole should be within ~10° of ecliptic normal, got {angle:.2}°"
        );
    }

    #[test]
    fn moon_rotation_period_matches_orbital_period() {
        // Moon's sidereal rotation period ≈ 27.322 days.
        // Prime meridian should advance ~360° in that time.
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let w0 = MOON.prime_meridian_angle(&epoch0);

        let sidereal_period_days = 27.322;
        let epoch1 = epoch0.add_seconds(sidereal_period_days * 86400.0);
        let w1 = MOON.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        let revolutions = dw / 360.0;
        assert!(
            (revolutions - 1.0).abs() < 0.01,
            "Moon should rotate ~1 revolution in {sidereal_period_days} days, got {revolutions:.4}"
        );
    }

    #[test]
    fn mars_rotation_period_approximately_24h37m() {
        // Mars sidereal rotation period ≈ 24h 37m 22s ≈ 88642 seconds.
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let w0 = MARS.prime_meridian_angle(&epoch0);

        let mars_day_s = 88642.0;
        let epoch1 = epoch0.add_seconds(mars_day_s);
        let w1 = MARS.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        assert!(
            (dw - 360.0).abs() < 1.0,
            "Mars should rotate ~360° in one sol, got {dw:.2}°"
        );
    }

    #[test]
    fn orientation_axes_are_orthonormal() {
        let epoch = Epoch::from_gregorian(2024, 6, 15, 0, 0, 0.0);
        for model in [&MOON, &MARS, &SUN] {
            let q = model.orientation(&epoch);
            let rot = q.to_rotation_matrix();
            let m = rot.matrix();

            // Check orthogonality: M^T * M ≈ I
            let mtm = m.transpose() * m;
            let identity = nalgebra::Matrix3::<f64>::identity();
            let err = (mtm - identity).norm();
            assert!(
                err < 1e-10,
                "rotation matrix should be orthogonal, error = {err}"
            );

            // Check determinant ≈ 1 (proper rotation)
            let det = m.determinant();
            assert!(
                (det - 1.0).abs() < 1e-10,
                "rotation determinant should be 1, got {det}"
            );
        }
    }

    #[test]
    fn earth_rotation_period_approximately_sidereal_day() {
        // Earth sidereal day ≈ 86164.1 seconds
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let w0 = EARTH.prime_meridian_angle(&epoch0);

        let sidereal_day_s = 86164.1;
        let epoch1 = epoch0.add_seconds(sidereal_day_s);
        let w1 = EARTH.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        assert!(
            (dw - 360.0).abs() < 0.5,
            "Earth should rotate ~360° in one sidereal day, got {dw:.2}°"
        );
    }

    #[test]
    fn earth_pole_near_z_axis() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let pole = EARTH.pole_direction(&epoch);
        let z_axis = Vector3::new(0.0, 0.0, 1.0);
        let angle = pole.angle(&z_axis).to_degrees();
        assert!(
            angle < 1.0,
            "Earth pole should be within ~1° of ECI Z-axis, got {angle:.2}°"
        );
    }

    #[test]
    fn earth_prime_meridian_at_j2000() {
        // At J2000.0, Earth's prime meridian (Greenwich) should be at
        // approximately the direction defined by GMST(J2000.0) ≈ 280.46°.
        // IAU model: W₀ = 190.147°, measured from ascending node at RA=90°.
        // So prime meridian RA ≈ 90° + 190.147° = 280.147° ≈ GMST at J2000.
        let epoch = Epoch::from_jd(2451545.0); // J2000.0
        let q = EARTH.orientation(&epoch);
        let x_body_eci = q * Vector3::new(1.0, 0.0, 0.0); // prime meridian in ECI

        // Expected RA of Greenwich at J2000: ~280.46°
        let ra = x_body_eci.y.atan2(x_body_eci.x).to_degrees();
        let ra_pos = if ra < 0.0 { ra + 360.0 } else { ra };
        let expected_ra = 280.46;
        let diff = ((ra_pos - expected_ra + 180.0) % 360.0 - 180.0).abs();
        assert!(
            diff < 5.0,
            "Earth prime meridian RA at J2000 should be ~{expected_ra}°, got {ra_pos:.2}° (diff={diff:.2}°)"
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
            let epoch = Epoch::from_jd(*jd);
            let q = body_orientation(body, &epoch).unwrap();
            println!(
                r#"  {{ body: "{body}", jd: {jd}, label: "{label}", q: [{:.15}, {:.15}, {:.15}, {:.15}] }},"#,
                q.w, q.i, q.j, q.k
            );
        }
        println!("---");
    }

    #[test]
    fn moon_near_side_faces_earth_base_model() {
        // Base model (no libration): ~30° tolerance
        let epoch = Epoch::from_gregorian(1969, 7, 20, 20, 17, 0.0);
        let q = MOON.orientation(&epoch);
        let x_body_eci = q * Vector3::new(1.0, 0.0, 0.0);
        let moon_pos = crate::moon::moon_position_eci(&epoch).into_inner();
        let earth_dir = -moon_pos.normalize();
        let angle = x_body_eci.angle(&earth_dir).to_degrees();
        assert!(
            angle < 30.0,
            "Base model: Moon prime meridian should roughly face Earth, angle = {angle:.1}°"
        );
    }

    #[test]
    fn moon_near_side_faces_earth_with_libration() {
        // With libration corrections: tighter tolerance
        let epochs = [
            Epoch::from_gregorian(1969, 7, 20, 20, 17, 0.0), // Apollo 11 landing
            Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0),
            Epoch::from_gregorian(2000, 1, 1, 12, 0, 0.0), // J2000
        ];
        for epoch in &epochs {
            let q = moon_orientation(epoch);
            let x_body_eci = q * Vector3::new(1.0, 0.0, 0.0);
            let moon_pos = crate::moon::moon_position_eci(epoch).into_inner();
            let earth_dir = -moon_pos.normalize();
            let angle = x_body_eci.angle(&earth_dir).to_degrees();
            assert!(
                angle < 10.0,
                "Libration model at JD {:.1}: Moon prime meridian should face Earth within 10°, got {angle:.2}°",
                epoch.jd()
            );
        }
    }

    #[test]
    fn libration_differs_from_base_model() {
        // The libration model should produce a slightly different orientation
        // from the base model due to periodic corrections.
        // Note: the IAU Moon frame is Mean Earth (ME), so libration corrections
        // do NOT necessarily bring the prime meridian closer to the instantaneous
        // Earth direction. They correct for nutation/precession of the pole and
        // prime meridian relative to the mean orientation.
        let epoch = Epoch::from_gregorian(2024, 3, 15, 0, 0, 0.0);
        let q_base = MOON.orientation(&epoch);
        let q_lib = moon_orientation(&epoch);

        // The two quaternions should differ (libration is nonzero)
        let angle_diff = q_base.angle_to(&q_lib).to_degrees();
        assert!(
            angle_diff > 0.1,
            "Libration corrections should produce measurable difference, got {angle_diff:.4}°"
        );
        // But not by more than ~10° (libration amplitude is bounded)
        assert!(
            angle_diff < 10.0,
            "Libration corrections should be bounded, got {angle_diff:.2}°"
        );
    }

    #[test]
    fn libration_model_is_orthonormal() {
        let epochs = [
            Epoch::from_gregorian(1969, 7, 20, 20, 17, 0.0),
            Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 6, 15, 12, 0, 0.0),
        ];
        for epoch in &epochs {
            let q = moon_orientation(epoch);
            let rot = q.to_rotation_matrix();
            let m = rot.matrix();
            let mtm = m.transpose() * m;
            let err = (mtm - nalgebra::Matrix3::<f64>::identity()).norm();
            assert!(
                err < 1e-10,
                "libration rotation should be orthogonal, error = {err}"
            );
            let det = m.determinant();
            assert!(
                (det - 1.0).abs() < 1e-10,
                "determinant should be 1, got {det}"
            );
        }
    }
}
