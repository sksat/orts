//! IAU 2009 WGCCRE rotation model for the Moon, including the 13-term
//! libration corrections from Archinal et al. (2011) Table 3a/3b.

use nalgebra::{Matrix3, UnitQuaternion, Vector3};

use crate::epoch::{Epoch, Tdb};
use crate::rotation::IauRotationModel;

/// IAU 2009 rotation model for the Moon (base linear terms).
///
/// For higher accuracy, use [`moon_orientation`] which includes the
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
///
/// # Time scale
///
/// IAU WGCCRE 2009 specifies the W/α/δ polynomials in terms of "interval in
/// Julian days from J2000 in TDB" (Archinal et al. 2011). This function takes
/// `&Epoch<Tdb>` as input; callers starting from `Epoch<Utc>` must invoke
/// `epoch.to_tdb()` explicitly.
pub fn moon_orientation(epoch: &Epoch<Tdb>) -> UnitQuaternion<f64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_orientation_is_unit_quaternion() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0).to_tdb();
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
        let epoch = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
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
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0).to_tdb();
        let w0 = MOON.prime_meridian_angle(&epoch0);

        let sidereal_period_days = 27.322;
        let epoch1 = Epoch::<Tdb>::from_jd_tdb(epoch0.jd() + sidereal_period_days);
        let w1 = MOON.prime_meridian_angle(&epoch1);

        let dw = (w1 - w0).to_degrees();
        let revolutions = dw / 360.0;
        assert!(
            (revolutions - 1.0).abs() < 0.01,
            "Moon should rotate ~1 revolution in {sidereal_period_days} days, got {revolutions:.4}"
        );
    }

    #[test]
    fn moon_near_side_faces_earth_base_model() {
        // Base model (no libration): ~30° tolerance
        let epoch_utc = Epoch::from_gregorian(1969, 7, 20, 20, 17, 0.0);
        let epoch_tdb = epoch_utc.to_tdb();
        let q = MOON.orientation(&epoch_tdb);
        let x_body_eci = q * Vector3::new(1.0, 0.0, 0.0);
        let moon_pos = crate::moon::moon_position_eci(&epoch_utc).into_inner();
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
            let epoch_tdb = epoch.to_tdb();
            let q = moon_orientation(&epoch_tdb);
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
        let epoch = Epoch::from_gregorian(2024, 3, 15, 0, 0, 0.0).to_tdb();
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
            let epoch_tdb = epoch.to_tdb();
            let q = moon_orientation(&epoch_tdb);
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
