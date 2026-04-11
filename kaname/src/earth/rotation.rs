//! IAU 2009 WGCCRE rotation model for the Earth.
//!
//! This is the base linear model (α₁, δ₁ ≠ 0, so the pole drifts from the
//! ECI Z-axis over centuries). For short-term simulations where precession is
//! negligible, the simpler ERA-only rotation
//! (`Rotation::<SimpleEci, SimpleEcef>::from_ut1`) is preferred.
//!
//! For Phase 3 the strict IAU 2006 CIO-based precession / nutation chain
//! will be added as a sibling module under `earth/`.

use crate::rotation::IauRotationModel;

/// IAU 2009 rotation model for the Earth.
///
/// This model includes pole precession (α₁, δ₁ ≠ 0), so the Earth's pole
/// drifts from the ECI Z-axis over centuries. This is physically more accurate
/// for long-term simulations than the pure ERA Z-rotation.
pub const EARTH: IauRotationModel = IauRotationModel {
    alpha0: 0.0,
    alpha1: -0.641,
    delta0: 90.0,
    delta1: -0.557,
    w0: 190.147,
    wd: 360.9856235,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::{Epoch, Tdb};
    use nalgebra::Vector3;

    #[test]
    fn earth_rotation_period_approximately_sidereal_day() {
        // Earth sidereal day ≈ 86164.1 seconds
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
        let w0 = EARTH.prime_meridian_angle(&epoch0);

        let sidereal_day_s = 86164.1;
        let epoch1 = Epoch::<Tdb>::from_jd_tdb(epoch0.jd() + sidereal_day_s / 86400.0);
        let w1 = EARTH.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        assert!(
            (dw - 360.0).abs() < 0.5,
            "Earth should rotate ~360° in one sidereal day, got {dw:.2}°"
        );
    }

    #[test]
    fn earth_pole_near_z_axis() {
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
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
        let epoch = Epoch::<Tdb>::from_jd_tdb(2451545.0); // J2000.0 TDB
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
}
