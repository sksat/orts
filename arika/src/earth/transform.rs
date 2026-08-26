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

// EphemerisFrameBridge

/// ECI frame that a GCRS ephemeris vector can be rotated into, for frame-correct
/// third-body / SRP forces.
///
/// The analytic Sun/Moon/planet ephemerides return positions in
/// [`Gcrs`](crate::frame::Gcrs). A force model integrating in frame `F` must
/// express that position in `F` before differencing it with the satellite state.
/// This trait supplies the `GCRS → F` rotation so the force model can be written
/// once, frame-generically, instead of inheriting a raw-vector mix that is only
/// valid for GCRS-aligned frames. A frame without this impl cannot be used with
/// those forces (a compile error), which is the point: a new of-date frame must
/// state its `GCRS → F` rotation rather than silently reuse the raw treatment.
///
/// # Implementations
///
/// - `SimpleEci` / `Gcrs`: identity. `Gcrs` *is* the ephemeris frame; `SimpleEci`
///   is the simple/visualization-grade frame (no precession/nutation/bias model,
///   and no strict relation to GCRS — see [`SimpleEci`](crate::frame::SimpleEci)),
///   so the analytic ephemeris is treated as already expressed in it, preserving
///   the historical Meeus/simple-path behavior exactly.
/// - `Cirs`: the EOP-free IAU 2006 **model** GCRS→CIRS rotation (precession +
///   nutation), rotating the ephemeris into the of-date intermediate frame.
///
/// EOP is intentionally not a parameter, matching [`EarthRotationPole`]: the
/// model CIP is sub-milliarcsecond, negligible next to the ~arcminute accuracy
/// of the analytic ephemerides this serves.
pub trait EphemerisFrameBridge: Eci + Sized + 'static {
    /// Rotation carrying a GCRS ephemeris vector into this frame at `utc`.
    fn ephemeris_rotation(utc: &Epoch<Utc>) -> Rotation<frame::Gcrs, Self>;
}

impl EphemerisFrameBridge for frame::Gcrs {
    fn ephemeris_rotation(_utc: &Epoch<Utc>) -> Rotation<frame::Gcrs, frame::Gcrs> {
        Rotation::identity()
    }
}

impl EphemerisFrameBridge for frame::SimpleEci {
    fn ephemeris_rotation(_utc: &Epoch<Utc>) -> Rotation<frame::Gcrs, frame::SimpleEci> {
        Rotation::identity()
    }
}

impl EphemerisFrameBridge for frame::Cirs {
    fn ephemeris_rotation(utc: &Epoch<Utc>) -> Rotation<frame::Gcrs, frame::Cirs> {
        Rotation::<frame::Gcrs, frame::Cirs>::iau2006_model(&utc.to_tt())
    }
}

// EarthFixedTransform

/// Everything an [`EarthFixedTransform`] needs to orient frame `F` relative to
/// Earth: the instant, plus that frame's Earth Orientation Parameters.
///
/// No ECI↔ECEF transform needs one without the other, so they travel as one
/// named argument. The EOP side is a borrow (only the `Copy` [`Epoch`] is
/// owned), which makes this a cheap per-call view over storage that lives
/// elsewhere — typically a long-lived field of a force model, which evaluates
/// at a new instant on every step from `&self`.
///
/// Because `F::EopStorage` is an associated type, one frame cannot be oriented
/// with another frame's EOP data — that is a compile error rather than a silent
/// wrong-frame result. For a frame that needs no EOP data at all,
/// [`simple`](Self::simple) says so by name.
///
/// # Examples
///
/// ```
/// # use arika::earth::{EarthFixedTransform, EarthOrientation};
/// # use arika::epoch::Epoch;
/// # use arika::frame::{SimpleEci, Vec3};
/// let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
/// // `SimpleEci` needs no EOP data, and does not mention it:
/// let orientation = EarthOrientation::<SimpleEci>::simple(utc);
/// let pos = Vec3::<SimpleEci>::new(6778.0, 0.0, 0.0);
/// let geodetic = SimpleEci::to_geodetic(&pos, &orientation);
/// # assert!(geodetic.altitude > 0.0);
/// ```
pub struct EarthOrientation<'a, F: EarthFixedTransform> {
    utc: Epoch<Utc>,
    eop: &'a F::EopStorage,
}

impl<'a, F: EarthFixedTransform> EarthOrientation<'a, F> {
    /// Orient `F` at `utc` using `eop`.
    ///
    /// For a frame that needs no EOP data, prefer
    /// [`simple`](Self::simple) — it says so by name.
    pub fn new(utc: Epoch<Utc>, eop: &'a F::EopStorage) -> Self {
        Self { utc, eop }
    }

    /// The instant the frame is oriented at.
    ///
    /// By reference, even though [`Epoch`] is `Copy`, because every consumer
    /// ([`EarthRotationPole::earth_pole`], the IAU 2006 constructors) takes
    /// `&Epoch<Utc>`.
    pub fn utc(&self) -> &Epoch<Utc> {
        &self.utc
    }

    /// The frame's Earth Orientation Parameters.
    pub fn eop(&self) -> &'a F::EopStorage {
        self.eop
    }
}

impl<F: EarthFixedTransform<EopStorage = ()>> EarthOrientation<'static, F> {
    /// Orient a frame that needs no EOP data (the simple, approximate path)
    /// at `utc`.
    ///
    /// Constrained to `EopStorage = ()`, so it does not exist for a frame whose
    /// transform genuinely needs Earth Orientation Parameters — that frame has
    /// to go through [`new`](Self::new) with real data.
    pub fn simple(utc: Epoch<Utc>) -> Self {
        Self { utc, eop: &() }
    }
}

/// ECI frame with a transform to its paired Earth-fixed (ECEF) frame.
///
/// The type-level dispatch point for code that needs geodetic coordinates
/// (atmosphere, magnetic field) or an ECEF↔ECI rotation (atmosphere wind
/// velocity, magnetic field vector transformation).
///
/// Every method takes the Earth orientation as one [`EarthOrientation`]
/// argument — the instant plus this frame's EOP data.
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
    fn to_geodetic(pos: &Vec3<Self>, orientation: &EarthOrientation<'_, Self>) -> Geodetic;

    /// Rotation from the paired ECEF frame to this ECI frame.
    ///
    /// Used to transform ECEF-frame vectors (e.g., magnetic field,
    /// atmosphere co-rotation velocity) back into the propagation frame.
    fn fixed_to_inertial(orientation: &EarthOrientation<'_, Self>) -> Rotation<Self::Fixed, Self>;

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
        orientation: &EarthOrientation<'_, Self>,
    ) -> FrameTransform<Self, Self::Fixed> {
        // ω of ECEF relative to ECI, expressed in ECI: Earth's spin vector.
        let omega = Self::earth_pole(orientation.utc()) * crate::earth::OMEGA;
        let rotation = Self::fixed_to_inertial(orientation).inverse();
        FrameTransform::new(rotation, omega)
    }

    /// ECEF → ECI state transform (inverse of [`inertial_to_fixed_transform`](Self::inertial_to_fixed_transform)).
    fn fixed_to_inertial_transform(
        orientation: &EarthOrientation<'_, Self>,
    ) -> FrameTransform<Self::Fixed, Self> {
        Self::inertial_to_fixed_transform(orientation).inverse()
    }
}

impl EarthFixedTransform for frame::SimpleEci {
    type Fixed = frame::SimpleEcef;
    type EopStorage = ();

    fn to_geodetic(
        pos: &Vec3<frame::SimpleEci>,
        orientation: &EarthOrientation<'_, Self>,
    ) -> Geodetic {
        let era = orientation.utc().to_ut1_naive().era();
        let rot = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(era);
        rot.transform(pos).to_geodetic()
    }

    fn fixed_to_inertial(
        orientation: &EarthOrientation<'_, Self>,
    ) -> Rotation<frame::SimpleEcef, frame::SimpleEci> {
        let era = orientation.utc().to_ut1_naive().era();
        Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(era)
    }
}

#[cfg(feature = "alloc")]
impl EarthFixedTransform for frame::Gcrs {
    type Fixed = frame::Itrs;
    type EopStorage = GcrsEopStorage;

    fn to_geodetic(pos: &Vec3<frame::Gcrs>, orientation: &EarthOrientation<'_, Self>) -> Geodetic {
        let rot = Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(
            orientation.utc(),
            orientation.eop(),
        );
        rot.transform(pos).to_geodetic()
    }

    fn fixed_to_inertial(
        orientation: &EarthOrientation<'_, Self>,
    ) -> Rotation<frame::Itrs, frame::Gcrs> {
        Rotation::<frame::Gcrs, frame::Itrs>::iau2006_full_from_utc(
            orientation.utc(),
            orientation.eop(),
        )
        .inverse()
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

    /// EOP provider with every correction non-zero, so a test can tell an
    /// orientation that actually forwards `eop()` from one that quietly
    /// substitutes zeros. Values are the right order of magnitude for observed
    /// EOP: dUT1 a few tenths of a second, polar motion and the CIP offsets a
    /// fraction of an arcsecond.
    struct NonZeroEop;

    impl Ut1Offset for NonZeroEop {
        fn dut1(&self, _: f64) -> f64 {
            -0.32
        }
    }
    impl PolarMotion for NonZeroEop {
        fn x_pole(&self, _: f64) -> f64 {
            0.161_7_f64.to_radians() / 3600.0
        }
        fn y_pole(&self, _: f64) -> f64 {
            0.436_2_f64.to_radians() / 3600.0
        }
    }
    impl NutationCorrections for NonZeroEop {
        fn dx(&self, _: f64) -> f64 {
            0.000_2_f64.to_radians() / 3600.0
        }
        fn dy(&self, _: f64) -> f64 {
            -0.000_3_f64.to_radians() / 3600.0
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
        let geo = <frame::SimpleEci as EarthFixedTransform>::to_geodetic(
            &pos,
            &EarthOrientation::simple(utc),
        );
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
        let geo = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &pos,
            &EarthOrientation::new(utc, &eop),
        );
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
        let rot = <frame::SimpleEci as EarthFixedTransform>::fixed_to_inertial(
            &EarthOrientation::simple(utc),
        );
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
        let rot = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial(&EarthOrientation::new(
            utc, &eop,
        ));
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
        let geo_simple = <frame::SimpleEci as EarthFixedTransform>::to_geodetic(
            &pos_simple,
            &EarthOrientation::simple(utc),
        );

        let pos_gcrs = Vec3::<frame::Gcrs>::new(R_EARTH + alt_km, 0.0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let geo_gcrs = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &pos_gcrs,
            &EarthOrientation::new(utc, &eop),
        );

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
        let ft = <frame::SimpleEci as EarthFixedTransform>::inertial_to_fixed_transform(
            &EarthOrientation::simple(utc),
        );
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
        let ft = <frame::Gcrs as EarthFixedTransform>::inertial_to_fixed_transform(
            &EarthOrientation::new(utc, &eop),
        );
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
        let ft = <frame::Gcrs as EarthFixedTransform>::inertial_to_fixed_transform(
            &EarthOrientation::new(utc, &eop),
        );
        let r = Vec3::<frame::Gcrs>::new(6778.0, 1200.0, -3400.0);
        let v = Vec3::<frame::Gcrs>::new(-1.2, 7.0, 2.5);
        let (r_e, v_e) = ft.transform_state(&r, &v);
        let (r_back, v_back) = ft.inverse().transform_state(&r_e, &v_e);
        assert!((r_back.inner() - r.inner()).norm() < 1e-9);
        assert!((v_back.inner() - v.inner()).norm() < 1e-12);
        // fixed_to_inertial_transform is the inverse of inertial_to_fixed_transform.
        let ft_inv = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial_transform(
            &EarthOrientation::new(utc, &eop),
        );
        let (r2, v2) = ft_inv.transform_state(&r_e, &v_e);
        assert!((r2.inner() - r.inner()).norm() < 1e-9);
        assert!((v2.inner() - v.inner()).norm() < 1e-12);
    }

    // Characterization snapshots
    //
    // The tests above pin *properties* (magnitude preserved, roundtrip closes, ω
    // direction) with loose tolerances, which a changed rotation chain could
    // still satisfy. These pin the actual numbers the four
    // `EarthFixedTransform` methods return, at one off-axis state and one
    // non-round epoch, so a change in how the Earth-orientation inputs
    // (UTC + EOP) reach them cannot pass unnoticed.
    //
    // `close` compares relatively at 1e-12: far tighter than any plausible
    // change to the rotation chain (a mis-threaded epoch or a dropped EOP moves
    // these by 1e-3 or more) while staying above the last-ULP differences
    // between platform libm implementations of the sin/cos/atan2 underneath.

    /// Off-axis epoch (non-integer second, so ERA is not a round number).
    fn snapshot_epoch() -> Epoch<Utc> {
        Epoch::from_gregorian(2024, 3, 20, 12, 34, 56.789)
    }

    /// Fully 3D position [km] — no zero component, so a dropped rotation shows.
    fn snapshot_pos<F>() -> Vec3<F> {
        Vec3::new(4000.0, -5000.0, 2500.0)
    }

    /// Fully 3D velocity [km/s].
    fn snapshot_vel<F>() -> Vec3<F> {
        Vec3::new(1.0, 2.0, 7.0)
    }

    /// Relative comparison at 1e-12 (see the module note above); exact for 0.
    fn close(got: f64, want: f64) -> bool {
        (got - want).abs() <= 1e-12 * want.abs().max(1.0)
    }

    #[track_caller]
    fn assert_close3(got: [f64; 3], want: [f64; 3], what: &str) {
        assert!(
            close(got[0], want[0]) && close(got[1], want[1]) && close(got[2], want[2]),
            "{what} changed: got {got:?}, want {want:?}"
        );
    }

    #[test]
    fn simple_eci_to_geodetic_snapshot() {
        let geo = <frame::SimpleEci as EarthFixedTransform>::to_geodetic(
            &snapshot_pos(),
            &EarthOrientation::simple(snapshot_epoch()),
        );
        assert_close3(
            [geo.latitude, geo.longitude, geo.altitude],
            [0.3743480895355276, -1.017562528402866, 498.5663922419981],
            "SimpleEci to_geodetic",
        );
    }

    #[test]
    fn simple_eci_fixed_to_inertial_snapshot() {
        let v = Vec3::<frame::SimpleEcef>::new(1.0, 2.0, 3.0);
        let got = <frame::SimpleEci as EarthFixedTransform>::fixed_to_inertial(
            &EarthOrientation::simple(snapshot_epoch()),
        )
        .transform(&v)
        .into_inner();
        assert_close3(
            [got.x, got.y, got.z],
            [0.7502103324902986, 2.10646254583954, 3.0],
            "SimpleEci fixed_to_inertial",
        );
    }

    #[test]
    fn simple_eci_state_transform_snapshot() {
        let utc = snapshot_epoch();
        let (r, v) = (snapshot_pos(), snapshot_vel());
        let (r_f, v_f) = <frame::SimpleEci as EarthFixedTransform>::inertial_to_fixed_transform(
            &EarthOrientation::simple(utc),
        )
        .transform_state(&r, &v);
        assert_close3(
            [r_f.inner().x, r_f.inner().y, r_f.inner().z],
            [3364.46645847656, -5447.968928856532, 2500.0],
            "SimpleEci inertial_to_fixed position",
        );
        assert_close3(
            [v_f.inner().x, v_f.inner().y, v_f.inner().z],
            [0.8377716286892459, 1.6187049999272265, 7.0],
            "SimpleEci inertial_to_fixed velocity",
        );
        // The inverse factory must undo it at the same epoch.
        let (r_b, v_b) = <frame::SimpleEci as EarthFixedTransform>::fixed_to_inertial_transform(
            &EarthOrientation::simple(utc),
        )
        .transform_state(&r_f, &v_f);
        assert!((r_b.inner() - r.inner()).norm() < 1e-9);
        assert!((v_b.inner() - v.inner()).norm() < 1e-12);
    }

    #[test]
    fn gcrs_to_geodetic_snapshot() {
        let geo = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &snapshot_pos(),
            &EarthOrientation::new(snapshot_epoch(), &GcrsEopStorage::new(ZeroEop)),
        );
        assert_close3(
            [geo.latitude, geo.longitude, geo.altitude],
            [0.3757884614713237, -1.0182885501575514, 498.58726984852274],
            "Gcrs to_geodetic",
        );
    }

    /// The `ZeroEop` snapshots above cannot tell a transform that forwards
    /// `orientation.eop()` from one that silently substitutes zeros — with an
    /// all-zero provider both give the same numbers. Pin the forwarding itself:
    /// the same epoch and position through a non-zero provider must differ.
    #[test]
    fn gcrs_forwards_the_eop_it_is_given() {
        let utc = snapshot_epoch();
        let pos = snapshot_pos();
        let zero = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &pos,
            &EarthOrientation::new(utc, &GcrsEopStorage::new(ZeroEop)),
        );
        let observed = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &pos,
            &EarthOrientation::new(utc, &GcrsEopStorage::new(NonZeroEop)),
        );

        // dUT1 of -0.32 s rotates ERA by ~1.4e-6 rad, which moves the
        // sub-satellite longitude by about the same amount; polar motion tilts
        // latitude by ~2e-6 rad. Anything that dropped the provider would land
        // exactly on the ZeroEop numbers.
        assert!(
            (observed.longitude - zero.longitude).abs() > 1e-7,
            "longitude must respond to dUT1: zero={} observed={}",
            zero.longitude,
            observed.longitude
        );
        // Latitude is deliberately not asserted here: how much polar motion moves
        // the geodetic latitude of one particular point depends on that point's
        // longitude, and at this snapshot it happens to be ~1e-11 rad. The
        // rotation below is the frame-level check that does not depend on where
        // the probe sits.

        // The rotation factories must use the provider too, not just to_geodetic.
        let r_zero = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial(
            &EarthOrientation::new(utc, &GcrsEopStorage::new(ZeroEop)),
        );
        let r_observed = <frame::Gcrs as EarthFixedTransform>::fixed_to_inertial(
            &EarthOrientation::new(utc, &GcrsEopStorage::new(NonZeroEop)),
        );
        let probe = frame::Vec3::<frame::Itrs>::new(R_EARTH, 0.0, 0.0);
        assert!(
            (r_observed.transform(&probe).inner() - r_zero.transform(&probe).inner()).norm() > 1e-3,
            "fixed_to_inertial must respond to the EOP it is given"
        );
    }

    #[test]
    fn gcrs_state_transform_snapshot() {
        let utc = snapshot_epoch();
        let eop = GcrsEopStorage::new(ZeroEop);
        let (r, v) = (snapshot_pos(), snapshot_vel());
        let (r_f, v_f) = <frame::Gcrs as EarthFixedTransform>::inertial_to_fixed_transform(
            &EarthOrientation::new(utc, &eop),
        )
        .transform_state(&r, &v);
        assert_close3(
            [r_f.inner().x, r_f.inner().y, r_f.inner().z],
            [3358.6255230092333, -5447.3533661597785, 2509.178331721101],
            "Gcrs inertial_to_fixed position",
        );
        assert_close3(
            [v_f.inner().x, v_f.inner().y, v_f.inner().z],
            [0.8214899007878738, 1.6208520413080323, 7.002402600668856],
            "Gcrs inertial_to_fixed velocity",
        );
    }
}
