//! [`MeanEquinoxOfDate`] ↔ [`Gcrs`] rotations (IAU 1976 precession).
//!
//! The classical analytic ephemerides (Meeus' Sun and Moon series) and GMST are
//! referred to the mean equator and equinox of date, while the force models and
//! the integration frames work in J2000/GCRS. These factories are the single
//! step between the two, so a consumer that needs one frame cannot silently
//! read a value expressed in the other.
//!
//! Only precession is undone: nutation (≤ 17″) and the J2000→GCRS frame bias
//! (~20 mas) stay in, both far below the ~1′ accuracy of the series this serves.

use nalgebra::{Matrix3, Rotation3, UnitQuaternion};

use crate::earth::fk5;
use crate::epoch::{Epoch, Tt};
use crate::frame::{Gcrs, MeanEquinoxOfDate, Rotation};

fn matrix3_to_unit_quaternion(m: Matrix3<f64>) -> UnitQuaternion<f64> {
    // `m` is a product of elemental rotations (a proper orthonormal rotation
    // matrix), so `from_matrix_unchecked` is sound here.
    UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m))
}

impl Rotation<MeanEquinoxOfDate, Gcrs> {
    /// Mean equinox of date → GCRS at TT epoch `tt`, undoing the IAU 1976
    /// precession accumulated since J2000.
    pub fn iau1976_precession(tt: &Epoch<Tt>) -> Self {
        Self::from_raw(matrix3_to_unit_quaternion(
            fk5::mean_of_date_to_j2000_matrix(tt.centuries_since_j2000()),
        ))
    }
}

impl Rotation<Gcrs, MeanEquinoxOfDate> {
    /// GCRS → mean equinox of date at TT epoch `tt`, applying the IAU 1976
    /// precession accumulated since J2000.
    pub fn iau1976_precession(tt: &Epoch<Tt>) -> Self {
        Self::from_raw(matrix3_to_unit_quaternion(fk5::precession_matrix(
            tt.centuries_since_j2000(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Vec3;

    #[test]
    fn the_two_directions_are_inverses() {
        let tt = Epoch::from_gregorian(2035, 8, 9, 3, 0, 0.0).to_tt();
        let fwd = Rotation::<Gcrs, MeanEquinoxOfDate>::iau1976_precession(&tt);
        let back = Rotation::<MeanEquinoxOfDate, Gcrs>::iau1976_precession(&tt);
        for v in [
            Vec3::<Gcrs>::new(1.0, 0.0, 0.0),
            Vec3::<Gcrs>::new(0.0, 1.0, 0.0),
            Vec3::<Gcrs>::new(0.3, -0.7, 0.65),
        ] {
            let round = back.transform(&fwd.transform(&v));
            assert!(
                (round.into_inner() - v.into_inner()).magnitude() < 1e-14,
                "round trip drifted: {:?} -> {:?}",
                v.into_inner(),
                round.into_inner()
            );
        }
    }

    #[test]
    fn the_rotation_angle_is_the_accumulated_precession() {
        // IAU general precession in longitude [arcsec/Julian century], quoted
        // independently of arika's IAU 1976 coefficient (5029.0966″/century).
        const GENERAL_PRECESSION_ARCSEC_PER_CENTURY: f64 = 5028.796195;

        for y in [1970, 2024, 2075] {
            let tt = Epoch::from_gregorian(y, 1, 1, 0, 0, 0.0).to_tt();
            let t = tt.centuries_since_j2000();
            // The vernal equinox direction sits on the ecliptic, so the whole
            // precession in longitude shows up as an angle there.
            let x = Vec3::<MeanEquinoxOfDate>::new(1.0, 0.0, 0.0);
            let rotated = Rotation::<MeanEquinoxOfDate, Gcrs>::iau1976_precession(&tt)
                .transform(&x)
                .into_inner();
            let sep_arcsec = rotated
                .dot(&nalgebra::Vector3::new(1.0, 0.0, 0.0))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees()
                * 3600.0;
            let expected = (GENERAL_PRECESSION_ARCSEC_PER_CENTURY * t).abs();
            assert!(
                (sep_arcsec - expected).abs() < 5.0,
                "{y}: rotation is {sep_arcsec:.1}″, expected ~{expected:.1}″"
            );
        }
    }
}
