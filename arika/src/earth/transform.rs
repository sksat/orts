//! Per-frame Earth rotation pole and ECI↔ECEF transforms.
//!
//! These traits express, for a given ECI frame, two coordinate facts that
//! force models in downstream crates need:
//!
//! - [`EarthRotationPole`] — the direction of Earth's rotation pole in the frame
//!   (zonal gravity and atmosphere co-rotation only need the axis direction).
//! - [`EarthFixedTransform`] — the paired Earth-fixed (ECEF) frame plus the
//!   geodetic conversion and ECEF↔ECI rotation (atmosphere geodetic lookup,
//!   magnetic field, wind co-rotation).
//!
//! Two implementations are provided:
//!
//! - [`SimpleEci`](crate::frame::SimpleEci): ERA-only Z rotation, pole = `+Z`,
//!   no EOP needed (the approximate, visualization-grade path).
//! - [`Gcrs`](crate::frame::Gcrs): full IAU 2006 CIO chain
//!   (precession + nutation + ERA + polar motion); the pole is the true CIP and
//!   the ECEF transform takes an EOP provider via [`GcrsEopStorage`].

use crate::earth::geodetic::Geodetic;
use crate::earth::iau2006::cip::cip_xy;
use crate::epoch::{Epoch, Utc};
use crate::frame::{self, Ecef, Eci, Rotation, Vec3};
// Used only on no_std (libm-backed `.sqrt()`); std uses the inherent f64 method.
#[allow(unused_imports)]
use crate::math::F64Ext;

#[cfg(feature = "alloc")]
use crate::earth::eop::GcrsEopStorage;

// EarthRotationPole

/// ECI frame that knows Earth's rotation-pole direction expressed in itself.
///
/// Zonal harmonics (J2/J3/J4) are axially symmetric about the true rotation
/// pole, and the atmosphere co-rotates about it, so those models only need the
/// pole *direction* in the integration frame — not the full Earth-fixed
/// rotation. This is the minimal capability they require, kept separate from the
/// heavier [`EarthFixedTransform`] (which also provides the ECEF rotation that
/// geodetic conversions and magnetic field need).
///
/// EOP is intentionally not a parameter: the IAU 2006 model CIP is accurate to
/// well under a milliarcsecond without the observed dX/dY corrections, which is
/// negligible next to the ~0.1–0.3° offset of the true pole from the GCRS Z
/// axis that this captures.
///
/// # Implementations
///
/// - `SimpleEci`: pole = `+Z` (the simple frame defines its Z axis as the pole).
/// - `Gcrs`: the IAU 2006 **model** CIP — precession + nutation, no observed
///   dX/dY, no polar motion — as `(X, Y, √(1−X²−Y²))` at the epoch. For an
///   exact observed pole, a separate EOP-taking API would be needed.
pub trait EarthRotationPole: Eci + Sized + 'static {
    /// Unit vector along Earth's rotation pole, expressed in this frame.
    fn earth_pole(utc: &Epoch<Utc>) -> Vec3<Self>;
}

impl EarthRotationPole for frame::SimpleEci {
    fn earth_pole(_utc: &Epoch<Utc>) -> Vec3<frame::SimpleEci> {
        Vec3::new(0.0, 0.0, 1.0)
    }
}

impl EarthRotationPole for frame::Gcrs {
    fn earth_pole(utc: &Epoch<Utc>) -> Vec3<frame::Gcrs> {
        // CIP direction cosines (X, Y) in GCRS from the IAU 2006 model; Z closes
        // the unit vector. The model (no observed dX/dY) is sub-mas accurate.
        //
        // This is the CIP (precession + nutation), NOT the ITRS figure axis: it
        // omits polar motion, which offsets the figure axis from the CIP by
        // < ~0.3″. That is ~5 orders of magnitude below the ~0.1° precession
        // offset this captures, and keeping it out is what lets the pole be
        // EOP-free. (When cross-validating zonal gravity against Orekit, match
        // its gravity body frame to the CIP rather than full ITRF accordingly.)
        let t = utc.to_tt().centuries_since_j2000();
        let (x, y) = cip_xy(t);
        let (x, y) = (x.raw(), y.raw());
        // `.max(0.0)` guards against a slightly-negative radicand from f64
        // round-off (x²+y² is ~1e-5 for real CIP values, never near 1).
        // `.sqrt()` is the only std-only float op here; `F64Ext` (libm) supplies
        // it on no_std, while `f64::max` is available in `core`.
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        Vec3::new(x, y, z)
    }
}

// EarthFixedTransform

/// ECI frame with a transform to its paired Earth-fixed (ECEF) frame.
///
/// The type-level dispatch point for code that needs geodetic coordinates
/// (atmosphere, magnetic field) or an ECEF↔ECI rotation (atmosphere wind
/// velocity, magnetic field vector transformation).
///
/// # Implementations
///
/// - `SimpleEci`: ERA-only Z rotation (`Rotation<SimpleEci, SimpleEcef>`),
///   no EOP needed (`EopStorage = ()`).
/// - `Gcrs`: full IAU 2006 CIO chain (`Rotation<Gcrs, Itrs>`), requires an EOP
///   provider (`EopStorage = GcrsEopStorage`, available with the `alloc`
///   feature).
pub trait EarthFixedTransform: EarthRotationPole {
    /// The ECEF frame paired with this ECI frame.
    type Fixed: Ecef;

    /// Storage for the EOP provider. `()` for the simple path.
    type EopStorage: Send + Sync + 'static;

    /// Convert an ECI position to geodetic coordinates.
    fn to_geodetic(pos: &Vec3<Self>, utc: &Epoch<Utc>, eop: &Self::EopStorage) -> Geodetic;

    /// Rotation from the paired ECEF frame to this ECI frame.
    ///
    /// Used to transform ECEF-frame vectors (e.g., magnetic field,
    /// atmosphere co-rotation velocity) back into the propagation frame.
    fn fixed_to_inertial(utc: &Epoch<Utc>, eop: &Self::EopStorage) -> Rotation<Self::Fixed, Self>;
}

impl EarthFixedTransform for frame::SimpleEci {
    type Fixed = frame::SimpleEcef;
    type EopStorage = ();

    fn to_geodetic(pos: &Vec3<frame::SimpleEci>, utc: &Epoch<Utc>, _eop: &()) -> Geodetic {
        let era = utc.to_ut1_naive().era();
        let rot = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(era);
        rot.transform(pos).to_geodetic()
    }

    fn fixed_to_inertial(
        utc: &Epoch<Utc>,
        _eop: &(),
    ) -> Rotation<frame::SimpleEcef, frame::SimpleEci> {
        let era = utc.to_ut1_naive().era();
        Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(era)
    }
}

#[cfg(feature = "alloc")]
impl EarthFixedTransform for frame::Gcrs {
    type Fixed = frame::Itrs;
    type EopStorage = GcrsEopStorage;

    fn to_geodetic(pos: &Vec3<frame::Gcrs>, utc: &Epoch<Utc>, eop: &GcrsEopStorage) -> Geodetic {
        let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(utc, eop);
        rot.transform(pos).to_geodetic()
    }

    fn fixed_to_inertial(
        utc: &Epoch<Utc>,
        eop: &GcrsEopStorage,
    ) -> Rotation<frame::Itrs, frame::Gcrs> {
        Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(utc, eop).inverse()
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::earth::R as R_EARTH;
    use crate::earth::eop::{LengthOfDay, NutationCorrections, PolarMotion, Ut1Offset};

    /// Minimal EOP provider for testing.
    struct ZeroEop;

    impl Ut1Offset for ZeroEop {
        fn dut1(&self, _: f64) -> f64 {
            0.0
        }
    }
    impl PolarMotion for ZeroEop {
        fn x_pole(&self, _: f64) -> f64 {
            0.0
        }
        fn y_pole(&self, _: f64) -> f64 {
            0.0
        }
    }
    impl NutationCorrections for ZeroEop {
        fn dx(&self, _: f64) -> f64 {
            0.0
        }
        fn dy(&self, _: f64) -> f64 {
            0.0
        }
    }
    impl LengthOfDay for ZeroEop {
        fn lod(&self, _: f64) -> f64 {
            0.0
        }
    }

    #[test]
    fn simple_eci_to_geodetic_altitude() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let alt_km = 400.0;
        let pos = Vec3::<frame::SimpleEci>::new(R_EARTH + alt_km, 0.0, 0.0);
        let geo = <frame::SimpleEci as EarthFixedTransform>::to_geodetic(&pos, &utc, &());
        // Altitude should be close to 400 km (not exact due to ERA rotation
        // and WGS84 ellipsoidal correction)
        assert!(
            (geo.altitude - alt_km).abs() < 1.0,
            "expected ~{alt_km} km, got {}",
            geo.altitude
        );
    }

    #[test]
    fn gcrs_to_geodetic_altitude() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let alt_km = 400.0;
        let pos = Vec3::<frame::Gcrs>::new(R_EARTH + alt_km, 0.0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let geo = <frame::Gcrs as EarthFixedTransform>::to_geodetic(&pos, &utc, &eop);
        assert!(
            (geo.altitude - alt_km).abs() < 1.0,
            "expected ~{alt_km} km, got {}",
            geo.altitude
        );
    }

    #[test]
    fn simple_eci_fixed_to_inertial_roundtrip() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let v_ecef = Vec3::<frame::SimpleEcef>::new(1.0, 2.0, 3.0);
        let rot = <frame::SimpleEci as EarthFixedTransform>::fixed_to_inertial(&utc, &());
        let v_eci = rot.transform(&v_ecef);
        // Magnitude should be preserved
        assert!(
            (v_eci.magnitude() - v_ecef.magnitude()).abs() < 1e-14,
            "rotation should preserve magnitude"
        );
    }

    #[test]
    fn gcrs_fixed_to_inertial_roundtrip() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let v_itrs = Vec3::<frame::Itrs>::new(1.0, 2.0, 3.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let rot = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial(&utc, &eop);
        let v_gcrs = rot.transform(&v_itrs);
        assert!(
            (v_gcrs.magnitude() - v_itrs.magnitude()).abs() < 1e-14,
            "rotation should preserve magnitude"
        );
    }

    #[test]
    fn simple_and_gcrs_geodetic_close_with_zero_eop() {
        // With zero EOP, Gcrs and SimpleEci should produce similar (not
        // identical) geodetic results — the precession/nutation/frame-bias
        // in IAU 2006 makes a difference of ~arcsec.
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let alt_km = 400.0;

        let pos_simple = Vec3::<frame::SimpleEci>::new(R_EARTH + alt_km, 0.0, 0.0);
        let geo_simple =
            <frame::SimpleEci as EarthFixedTransform>::to_geodetic(&pos_simple, &utc, &());

        let pos_gcrs = Vec3::<frame::Gcrs>::new(R_EARTH + alt_km, 0.0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let geo_gcrs = <frame::Gcrs as EarthFixedTransform>::to_geodetic(&pos_gcrs, &utc, &eop);

        // Altitudes should agree within a few km (different rotation chains)
        assert!(
            (geo_simple.altitude - geo_gcrs.altitude).abs() < 5.0,
            "simple alt={}, gcrs alt={}",
            geo_simple.altitude,
            geo_gcrs.altitude
        );
    }

    #[test]
    fn simple_eci_pole_is_plus_z() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let p = <frame::SimpleEci as EarthRotationPole>::earth_pole(&utc);
        assert_eq!(p, Vec3::<frame::SimpleEci>::new(0.0, 0.0, 1.0));
    }

    fn pole_offset_from_z_deg<F: EarthRotationPole>(utc: &Epoch<Utc>) -> f64 {
        let p = F::earth_pole(utc);
        let z = Vec3::<F>::new(0.0, 0.0, 1.0);
        assert!(
            (p.magnitude() - 1.0).abs() < 1e-12,
            "pole must be a unit vector, got {}",
            p.magnitude()
        );
        p.dot(&z).clamp(-1.0, 1.0).acos().to_degrees()
    }

    #[test]
    fn gcrs_pole_offset_from_z_is_precession_scale_at_2024() {
        // The true CIP is offset from the GCRS Z axis by precession + nutation;
        // at 2024 this is ~0.1°. This is exactly the J2 axis error the simple
        // (frame-Z) treatment ignores.
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let angle = pole_offset_from_z_deg::<frame::Gcrs>(&utc);
        assert!(
            angle > 0.05 && angle < 0.5,
            "2024 CIP offset from GCRS Z should be ~0.1°, got {angle}°"
        );
    }

    #[test]
    fn gcrs_pole_offset_grows_with_precession() {
        // The CIP offset from GCRS Z is precession-dominated and grows with
        // time: ~arcsec (nutation only) near J2000, ~0.1° (precession) by 2024.
        let j2000 =
            pole_offset_from_z_deg::<frame::Gcrs>(&Epoch::from_gregorian(2000, 1, 1, 12, 0, 0.0));
        let y2024 =
            pole_offset_from_z_deg::<frame::Gcrs>(&Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0));
        assert!(
            j2000 < 0.01,
            "J2000 CIP offset should be arcsec-scale, got {j2000}°"
        );
        assert!(
            y2024 > 10.0 * j2000,
            "CIP offset should grow with precession: 2024={y2024}°, J2000={j2000}°"
        );
    }
}
