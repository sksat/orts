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
use crate::frame::{self, Ecef, Eci, FrameTransform, Rotation, Vec3};
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

    /// ECI → ECEF state transform: the orientation plus Earth's spin angular
    /// velocity, so it transforms velocities (and full position+velocity
    /// states), not just positions.
    ///
    /// The angular velocity is `OMEGA · earth_pole` — Earth's nominal rotation
    /// rate ([`earth::OMEGA`](crate::earth::OMEGA)) about the spin axis from
    /// [`EarthRotationPole`]. This models **Earth spin transport only**: it is
    /// not the full time-derivative of the IAU 2006 W·R·Q chain (the
    /// precession/nutation/polar-motion rates Q̇/Ẇ, ~sub-µrad/s, are omitted),
    /// and it uses the nominal rate with no LOD correction.
    fn inertial_to_fixed_transform(
        utc: &Epoch<Utc>,
        eop: &Self::EopStorage,
    ) -> FrameTransform<Self, Self::Fixed> {
        // ω of ECEF relative to ECI, expressed in ECI: Earth's spin vector.
        let omega = Self::earth_pole(utc) * crate::earth::OMEGA;
        let rotation = Self::fixed_to_inertial(utc, eop).inverse();
        FrameTransform::new(rotation, omega)
    }

    /// ECEF → ECI state transform (inverse of [`inertial_to_fixed_transform`](Self::inertial_to_fixed_transform)).
    fn fixed_to_inertial_transform(
        utc: &Epoch<Utc>,
        eop: &Self::EopStorage,
    ) -> FrameTransform<Self::Fixed, Self> {
        Self::inertial_to_fixed_transform(utc, eop).inverse()
    }
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

    // State transforms (FrameTransform factories)

    #[test]
    fn simple_eci_corotating_point_is_static_in_ecef() {
        // A point on the equator co-rotating with Earth (inertial velocity ω×r)
        // must be static in ECEF, regardless of ERA — validates the factory's
        // ω = OMEGA·(+Z) wiring.
        let utc = Epoch::from_gregorian(2024, 3, 20, 7, 30, 0.0);
        let ft = <frame::SimpleEci as EarthFixedTransform>::inertial_to_fixed_transform(&utc, &());
        let r_km = 6378.137;
        let r = Vec3::<frame::SimpleEci>::new(r_km, 0.0, 0.0);
        let v = Vec3::<frame::SimpleEci>::new(0.0, crate::earth::OMEGA * r_km, 0.0);
        let v_ecef = ft.transform_velocity(&r, &v);
        assert!(
            v_ecef.inner().norm() < 1e-12,
            "co-rotating point should be static in ECEF, got {:?}",
            v_ecef.inner()
        );
    }

    #[test]
    fn gcrs_transform_omega_is_omega_times_cip() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let ft = <frame::Gcrs as EarthFixedTransform>::inertial_to_fixed_transform(&utc, &eop);
        // ω of ITRS relative to GCRS, in GCRS, is OMEGA along the CIP.
        let expected =
            <frame::Gcrs as EarthRotationPole>::earth_pole(&utc).into_inner() * crate::earth::OMEGA;
        assert!(
            (ft.angular_velocity_in_from().inner() - expected).norm() < 1e-18,
            "Gcrs spin angular velocity should be OMEGA·CIP"
        );
        // |ω| ≈ OMEGA (pole is a unit vector).
        assert!((ft.angular_velocity_in_from().inner().norm() - crate::earth::OMEGA).abs() < 1e-16);
    }

    #[test]
    fn gcrs_state_transform_roundtrip() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let ft = <frame::Gcrs as EarthFixedTransform>::inertial_to_fixed_transform(&utc, &eop);
        let r = Vec3::<frame::Gcrs>::new(6778.0, 1200.0, -3400.0);
        let v = Vec3::<frame::Gcrs>::new(-1.2, 7.0, 2.5);
        let (r_e, v_e) = ft.transform_state(&r, &v);
        let (r_back, v_back) = ft.inverse().transform_state(&r_e, &v_e);
        assert!((r_back.inner() - r.inner()).norm() < 1e-9);
        assert!((v_back.inner() - v.inner()).norm() < 1e-12);
        // fixed_to_inertial_transform is the inverse of inertial_to_fixed_transform.
        let ft_inv = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial_transform(&utc, &eop);
        let (r2, v2) = ft_inv.transform_state(&r_e, &v_e);
        assert!((r2.inner() - r.inner()).norm() < 1e-9);
        assert!((v2.inner() - v.inner()).norm() < 1e-12);
    }
}
