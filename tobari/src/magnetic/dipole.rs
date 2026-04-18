use arika::earth::ellipsoid::{WGS84_A, WGS84_E2};
use nalgebra::Vector3;

use super::{MagneticFieldInput, MagneticFieldModel};
#[allow(unused_imports)]
use crate::math::F64Ext;

/// Earth magnetic dipole strength [T*m^3] (= mu_0/(4*pi) * 7.94e22 A*m^2).
const EARTH_DIPOLE_STRENGTH: f64 = 7.94e15;

/// Tilted dipole geomagnetic field model with ECEF-fixed axis.
///
/// Approximates Earth's magnetic field as a tilted dipole, suitable for
/// B-dot detumbling simulations in LEO. The dipole axis is stored in ECEF
/// coordinates and rotated to ECI using the epoch's GMST, correctly
/// accounting for Earth rotation.
///
/// The dipole field at position **r** is:
///
/// **B** = (dipole_strength / r^3) [3(m_hat . r_hat) r_hat - m_hat]
///
/// where m_hat is the dipole axis unit vector and r is in metres.
pub struct TiltedDipole {
    /// Dipole strength [T*m^3] = mu_0 * m / (4*pi), absorbs mu_0/(4*pi) into the constant.
    dipole_strength: f64,
    /// Dipole axis unit vector in ECEF.
    axis_ecef: Vector3<f64>,
}

impl TiltedDipole {
    /// Create a tilted dipole with the given strength and axis in ECEF coordinates.
    ///
    /// # Panics
    /// Panics if `axis_ecef` is zero-length.
    pub fn new(dipole_strength: f64, axis_ecef: Vector3<f64>) -> Self {
        let norm = axis_ecef.magnitude();
        assert!(norm > 1e-15, "Dipole axis must be non-zero");
        Self {
            dipole_strength,
            axis_ecef: axis_ecef / norm,
        }
    }

    /// Earth's tilted dipole (IGRF approximate).
    ///
    /// - Dipole strength: ~7.94e15 T*m^3 (= mu_0/(4*pi) * 7.94e22 A*m^2)
    /// - Axis tilted ~11.5 deg from geographic north (simplified: tilt in x-z plane in ECEF)
    ///
    /// The axis is stored in ECEF coordinates and will be rotated to ECI
    /// using the epoch's GMST when computing the field.
    pub fn earth() -> Self {
        let tilt = 11.5_f64.to_radians();
        Self {
            dipole_strength: EARTH_DIPOLE_STRENGTH,
            axis_ecef: Vector3::new(tilt.sin(), 0.0, tilt.cos()).normalize(),
        }
    }

    /// Compute magnetic field vector in ECEF [T] at ECEF position [km].
    ///
    /// Returns the zero vector for positions inside 1 km from Earth's centre.
    fn compute_field_ecef(&self, position_ecef: &Vector3<f64>) -> Vector3<f64> {
        let r_km = position_ecef.magnitude();
        if r_km < 1.0 {
            return Vector3::zeros();
        }

        let r_m = r_km * 1000.0;
        let r3 = r_m * r_m * r_m;

        let r_hat = position_ecef / r_km;
        let m_hat = &self.axis_ecef;

        let m_dot_r = m_hat.dot(&r_hat);
        self.dipole_strength * (3.0 * m_dot_r * r_hat - m_hat) / r3
    }
}

impl MagneticFieldModel for TiltedDipole {
    fn field_ecef(&self, input: &MagneticFieldInput<'_>) -> [f64; 3] {
        // Geodetic → ECEF Cartesian position
        let lat = input.geodetic.latitude;
        let lon = input.geodetic.longitude;
        let h = input.geodetic.altitude;
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        let pos_ecef = Vector3::new(
            (n + h) * cos_lat * lon.cos(),
            (n + h) * cos_lat * lon.sin(),
            (n * (1.0 - WGS84_E2) + h) * sin_lat,
        );

        let b = self.compute_field_ecef(&pos_ecef);
        [b.x, b.y, b.z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::geodetic::Geodetic;
    use arika::epoch::Epoch;

    fn j2000_epoch() -> Epoch {
        Epoch::j2000()
    }

    fn make_input(geodetic: Geodetic, epoch: &Epoch) -> MagneticFieldInput<'_> {
        MagneticFieldInput {
            geodetic,
            utc: epoch,
        }
    }

    fn b_magnitude(b: &[f64; 3]) -> f64 {
        (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt()
    }

    #[test]
    fn equatorial_field_magnitude_at_leo() {
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        let input = make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 7000.0 - WGS84_A,
            },
            &epoch,
        );
        let b = dipole.field_ecef(&input);
        let b_micro_t = b_magnitude(&b) * 1e6;

        assert!(
            b_micro_t > 20.0 && b_micro_t < 50.0,
            "Equatorial LEO field should be ~25-35 uT, got {b_micro_t:.2} uT"
        );
    }

    #[test]
    fn inverse_cube_scaling() {
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        let b1 = b_magnitude(&dipole.field_ecef(&make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 7000.0 - WGS84_A,
            },
            &epoch,
        )));
        let b2 = b_magnitude(&dipole.field_ecef(&make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 14000.0 - WGS84_A,
            },
            &epoch,
        )));

        let ratio = b1 / b2;
        assert!(
            (ratio - 8.0).abs() < 0.1,
            "Expected ~1/r^3 scaling (ratio ~8.0), got {ratio:.4}"
        );
    }

    #[test]
    fn polar_field_stronger_than_equatorial() {
        // Axis-aligned dipole (z-axis) to test polar/equatorial ratio
        let dipole = TiltedDipole::new(7.94e15, Vector3::new(0.0, 0.0, 1.0));
        let r = 7000.0;
        let epoch = j2000_epoch();

        let b_pole = b_magnitude(&dipole.field_ecef(&make_input(
            Geodetic {
                latitude: std::f64::consts::FRAC_PI_2,
                longitude: 0.0,
                altitude: r - arika::earth::ellipsoid::WGS84_B,
            },
            &epoch,
        )));

        let b_eq = b_magnitude(&dipole.field_ecef(&make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: r - WGS84_A,
            },
            &epoch,
        )));

        let ratio = b_pole / b_eq;
        assert!(
            (ratio - 2.0).abs() < 0.15,
            "Polar/equatorial ratio should be ~2.0, got {ratio:.4}"
        );
    }

    #[test]
    fn zero_inside_earth_guard() {
        let dipole = TiltedDipole::earth();
        // Position at 0.5 km from center → inside guard radius
        let b = dipole.compute_field_ecef(&Vector3::new(0.5, 0.0, 0.0));
        assert_eq!(b, Vector3::zeros());
    }

    #[test]
    fn zero_at_origin() {
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        // Altitude = -WGS84_A puts us at the centre of the Earth
        let input = make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: -WGS84_A,
            },
            &epoch,
        );
        let b = dipole.field_ecef(&input);
        assert_eq!(b, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn field_is_finite() {
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();
        let input = make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 6778.0 - WGS84_A,
            },
            &epoch,
        );
        let b = dipole.field_ecef(&input);
        assert!(
            b[0].is_finite() && b[1].is_finite() && b[2].is_finite(),
            "Field must be finite: {b:?}"
        );
    }

    #[test]
    fn field_ecef_varies_with_longitude() {
        // The tilted dipole axis is fixed in ECEF, so different longitudes
        // should give different field vectors (the old epoch-rotation test
        // is replaced by this longitude-variation test since the model is
        // now frame-agnostic).
        let dipole = TiltedDipole::earth();
        let epoch = j2000_epoch();

        let b1 = dipole.field_ecef(&make_input(
            Geodetic {
                latitude: 0.0,
                longitude: 0.0,
                altitude: 7000.0 - WGS84_A,
            },
            &epoch,
        ));
        let b2 = dipole.field_ecef(&make_input(
            Geodetic {
                latitude: 0.0,
                longitude: std::f64::consts::FRAC_PI_2,
                altitude: 7000.0 - WGS84_A,
            },
            &epoch,
        ));

        let diff =
            ((b1[0] - b2[0]).powi(2) + (b1[1] - b2[1]).powi(2) + (b1[2] - b2[2]).powi(2)).sqrt();
        assert!(
            diff > 1e-10,
            "Field should differ at different longitudes, diff={diff:.3e}"
        );

        let mag_ratio = b_magnitude(&b1) / b_magnitude(&b2);
        assert!(
            (mag_ratio - 1.0).abs() < 0.5,
            "Magnitudes should be similar, ratio={mag_ratio:.3}"
        );
    }
}
