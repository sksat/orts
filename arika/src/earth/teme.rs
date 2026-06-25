//! TEME ↔ GCRS / SimpleEci frame rotations.
//!
//! TEME (True Equator, Mean Equinox) is the frame SGP4/TLE state vectors are
//! expressed in (see [`crate::sgp4`]). These factories rotate a TEME state into
//! an integration frame:
//! - [`Rotation<Teme, Gcrs>::teme_to_gcrs`] — the precise IAU-76/FK5 reduction.
//! - [`Rotation<Teme, SimpleEci>::teme_to_simple_eci`] — a GMST-aligned
//!   z-rotation for the ERA-only visualization frame.
//! - [`FrameTransform<Teme, Gcrs>::teme_to_gcrs`] — the position+velocity
//!   (state) transform.

use nalgebra::{Matrix3, Rotation3, UnitQuaternion};

// In `no_std` builds the trig methods resolve via libm through this trait.
#[allow(unused_imports)]
use crate::math::F64Ext;

use crate::earth::iau1980;
use crate::epoch::{Epoch, Tt, Ut1Epoch};
use crate::frame::{FrameTransform, Gcrs, Rotation, SimpleEci, Teme, Vec3};

fn matrix3_to_unit_quaternion(m: Matrix3<f64>) -> UnitQuaternion<f64> {
    UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(m))
}

impl Rotation<Teme, Gcrs> {
    /// TEME → GCRS via the IAU-76/FK5 equinox-based reduction at TT epoch `tt`
    /// (equation of the equinoxes + IAU-80 nutation + IAU-76 precession).
    ///
    /// The J2000→GCRS frame bias (< 1 mas ≈ 3 m at GEO) is neglected — far below
    /// the SGP4 error this serves. Cross-validated against ERFA and Orekit in
    /// `arika/tests/teme_vs_erfa.rs` / `teme_vs_orekit.rs`.
    pub fn teme_to_gcrs(tt: Epoch<Tt>) -> Self {
        Self::from_raw(matrix3_to_unit_quaternion(iau1980::teme_to_j2000_matrix(
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
    pub fn teme_to_simple_eci(ut1: Ut1Epoch) -> Self {
        let angle = iau1980::gmst1982(ut1.jd()) - ut1.era();
        // Passive z-rotation R3(angle), matching the equinox-reduction convention.
        let (s, c) = (angle.sin(), angle.cos());
        let m = Matrix3::new(c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0);
        Self::from_raw(matrix3_to_unit_quaternion(m))
    }
}

impl FrameTransform<Teme, Gcrs> {
    /// TEME → GCRS state (position + velocity) transform at TT epoch `tt`.
    ///
    /// The angular velocity is zero: the precession/nutation rate (~7.7e-12
    /// rad/s) is negligible, so position and velocity rotate by the same matrix
    /// (the standard Vallado treatment of TEME → inertial). For an Earth-fixed
    /// target — where Earth's spin ω matters — use the
    /// [`EarthFixedTransform`](crate::earth::EarthFixedTransform) factories
    /// instead.
    pub fn teme_to_gcrs(tt: Epoch<Tt>) -> Self {
        Self::new(
            Rotation::<Teme, Gcrs>::teme_to_gcrs(tt),
            Vec3::<Teme>::zeros(),
        )
    }
}
