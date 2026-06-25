//! TEME ↔ GCRS / SimpleEci frame rotations.
//!
//! TEME (True Equator, Mean Equinox) is the frame SGP4/TLE state vectors are
//! expressed in (see the `sgp4` module). These factories rotate a TEME state
//! into an integration frame:
//! - [`Rotation<Teme, Gcrs>::teme_to_gcrs`] — the IAU-76/FK5 reduction.
//! - [`Rotation<Teme, SimpleEci>::teme_to_simple_eci`] — a GMST-aligned
//!   z-rotation for the ERA-only visualization frame.
//! - [`FrameTransform<Teme, Gcrs>::teme_to_gcrs`] /
//!   [`FrameTransform<Teme, SimpleEci>::teme_to_simple_eci`] — the
//!   position+velocity (state) transforms.

use nalgebra::{Matrix3, Rotation3, UnitQuaternion};

// In `no_std` builds the trig methods resolve via libm through this trait.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::earth::fk5;
use crate::epoch::{Epoch, Tt, Ut1Epoch};
use crate::frame::{FrameTransform, Gcrs, Rotation, SimpleEci, Teme, Vec3};

fn matrix3_to_unit_quaternion(m: Matrix3<f64>) -> UnitQuaternion<f64> {
    UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m))
}

impl Rotation<Teme, Gcrs> {
    /// TEME → GCRS via the IAU-76/FK5 equinox-based reduction at TT epoch `tt`
    /// (equation of the equinoxes + IAU-80 nutation + IAU-76 precession).
    ///
    /// The IAU-76/FK5 reduction lands in the J2000 (FK5) dynamical frame, which
    /// this returns as a GCRS approximation: the J2000→GCRS frame bias (~tens of
    /// mas, ≈ sub-metre at LEO to a few metres at GEO) is neglected — far below
    /// the SGP4 error this serves. Cross-validated against ERFA (components) and
    /// Orekit (`tests/teme_vs_erfa.rs` / `teme_vs_orekit.rs`).
    pub fn teme_to_gcrs(tt: &Epoch<Tt>) -> Self {
        Self::from_raw(matrix3_to_unit_quaternion(fk5::teme_to_j2000_matrix(
            tt.centuries_since_j2000(),
        )))
    }
}

impl Rotation<Teme, SimpleEci> {
    /// TEME → [`SimpleEci`]: a z-rotation by `GMST1982 − ERA` at UT1 epoch `ut1`.
    ///
    /// `SimpleEci` is the ERA-only approximate inertial frame paired with
    /// `SimpleEcef`. This z-rotation makes `TEME → SimpleEci → SimpleEcef` (via
    /// ERA) consistent with the classical `TEME → PEF` (via GMST), so ground
    /// tracks line up. It ignores precession/nutation (as `SimpleEci` does);
    /// use [`Rotation<Teme, Gcrs>::teme_to_gcrs`] for the precise inertial frame.
    pub fn teme_to_simple_eci(ut1: &Ut1Epoch) -> Self {
        let angle = fk5::gmst1982(ut1.jd()) - ut1.era();
        // Passive z-rotation R3(angle), matching the equinox-reduction convention.
        let (s, c) = (angle.sin(), angle.cos());
        let m = Matrix3::new(c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0);
        Self::from_raw(matrix3_to_unit_quaternion(m))
    }
}

impl FrameTransform<Teme, Gcrs> {
    /// TEME → GCRS state (position + velocity) transform at TT epoch `tt`.
    ///
    /// **Approximate velocity**: the angular velocity is taken as zero, so
    /// position and velocity rotate by the same matrix (the standard Vallado
    /// treatment of TEME → inertial). This is not the exact kinematic
    /// derivative — the precession/nutation/equinox rate contributes a velocity
    /// difference of ~1e-8 km/s (LEO) to ~1e-7 km/s (GEO), negligible against
    /// the SGP4 velocity error. For an Earth-fixed target — where Earth's spin
    /// ω matters — use the
    /// [`EarthFixedTransform`](crate::earth::EarthFixedTransform) factories.
    pub fn teme_to_gcrs(tt: &Epoch<Tt>) -> Self {
        Self::new(
            Rotation::<Teme, Gcrs>::teme_to_gcrs(tt),
            Vec3::<Teme>::zeros(),
        )
    }
}

impl FrameTransform<Teme, SimpleEci> {
    /// TEME → [`SimpleEci`] state (position + velocity) transform at UT1 epoch
    /// `ut1`.
    ///
    /// Both frames are inertial, so the angular velocity is zero (the
    /// `GMST − ERA` rate is the precession-in-right-ascension rate, ~1e-12
    /// rad/s); position and velocity rotate by the same z-rotation. Pairs with
    /// [`Rotation<Teme, SimpleEci>::teme_to_simple_eci`] for the visualization
    /// frame.
    pub fn teme_to_simple_eci(ut1: &Ut1Epoch) -> Self {
        Self::new(
            Rotation::<Teme, SimpleEci>::teme_to_simple_eci(ut1),
            Vec3::<Teme>::zeros(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const J2000_JD: f64 = 2451545.0;

    #[test]
    fn teme_to_gcrs_preserves_magnitude() {
        // A rotation is an isometry: |r| is unchanged.
        let tt = Epoch::<Tt>::from_jd_tt(J2000_JD + 0.24 * 36525.0);
        let r = Vec3::<Teme>::new(4500.0, -3000.0, 5000.0);
        let r_gcrs = Rotation::<Teme, Gcrs>::teme_to_gcrs(&tt).transform(&r);
        assert!((r_gcrs.into_inner().norm() - r.into_inner().norm()).abs() < 1e-9);
    }

    #[test]
    fn teme_to_simple_eci_is_a_z_rotation() {
        // GMST − ERA is a rotation about +Z, so the z-component is preserved and
        // the transform is an isometry.
        let ut1 = Ut1Epoch::from_jd_ut1(J2000_JD + 0.1 * 36525.0);
        let r = Vec3::<Teme>::new(4500.0, -3000.0, 5000.0);
        let r_eci = Rotation::<Teme, SimpleEci>::teme_to_simple_eci(&ut1).transform(&r);
        let (a, b) = (r.into_inner(), r_eci.into_inner());
        assert!((b[2] - a[2]).abs() < 1e-9, "z preserved by a z-rotation");
        assert!((b.norm() - a.norm()).abs() < 1e-9, "isometry");
    }

    #[test]
    fn teme_to_gcrs_near_identity_close_to_j2000() {
        // Months from J2000 the precession/nutation is tiny: a TEME vector maps
        // to nearly itself in GCRS.
        let tt = Epoch::<Tt>::from_jd_tt(J2000_JD + 60.0); // ~2 months
        let r = Vec3::<Teme>::new(7000.0, 0.0, 0.0);
        let r_gcrs = Rotation::<Teme, Gcrs>::teme_to_gcrs(&tt).transform(&r);
        assert!(
            (r_gcrs.into_inner() - r.into_inner()).norm() < 5.0,
            "≪ 5 km near J2000"
        );
    }
}
