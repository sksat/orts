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

/// IAU 2009 rotation model for the Moon.
///
/// Note: This is the base model without libration terms.
/// Physical librations add periodic corrections of up to ~6° in longitude
/// and ~1° in latitude, but are omitted for simplicity.
pub const MOON: IauRotationModel = IauRotationModel {
    alpha0: 269.9949,
    alpha1: 0.0031,
    delta0: 66.5392,
    delta1: 0.0130,
    w0: 38.3213,
    wd: 13.17635815,
};

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

/// Look up the IAU rotation model for a body by name.
pub fn model_for_body(name: &str) -> Option<&'static IauRotationModel> {
    match name {
        "earth" => Some(&EARTH),
        "moon" => Some(&MOON),
        "mars" => Some(&MARS),
        "sun" => Some(&SUN),
        _ => None,
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

    #[test]
    fn moon_near_side_faces_earth() {
        // At any epoch, the Moon's prime meridian (X_body) direction should
        // roughly point toward Earth (origin). This is an approximate check
        // because the IAU model without librations only captures the mean
        // orientation, not the instantaneous tidal lock.
        let epoch = Epoch::from_gregorian(1969, 7, 20, 20, 17, 0.0); // Apollo 11 landing
        let q = MOON.orientation(&epoch);

        // X_body in ECI = q * (1,0,0)
        let x_body_eci = q * Vector3::new(1.0, 0.0, 0.0);

        // Moon position in ECI
        let moon_pos = crate::moon::moon_position_eci(&epoch);
        let earth_dir = -moon_pos.normalize(); // direction from Moon to Earth

        let angle = x_body_eci.angle(&earth_dir).to_degrees();
        // Without librations, this should be within ~15° (mean orientation)
        assert!(
            angle < 30.0,
            "Moon prime meridian should roughly face Earth, angle = {angle:.1}°"
        );
    }
}
