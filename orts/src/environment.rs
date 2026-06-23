//! Frame-aware environment adapter for force models.
//!
//! [`EarthFrameBridge`] bridges an ECI propagation frame to its
//! paired Earth-fixed (ECEF) frame, providing the geodetic conversion
//! and ECEF↔ECI rotation that atmosphere and magnetic field models
//! require.
//!
//! Two implementations are provided:
//!
//! - [`arika::frame::SimpleEci`]: ERA-only Z rotation, no EOP needed.
//!   This is the legacy/approximate path.
//! - [`arika::frame::Gcrs`]: Full IAU 2006 CIO chain
//!   (precession + nutation + ERA + polar motion). Requires an EOP
//!   provider implementing [`PositionEop`].

use arika::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};
use arika::earth::geodetic::Geodetic;
use arika::earth::iau2006::cip::cip_xy;
use arika::epoch::{Epoch, Utc};
use arika::frame::{self, Ecef, Eci, Rotation, Vec3};

// PositionEop — combined trait for position-level rotation

/// Combined EOP capability needed for position-level Gcrs↔Itrs rotation.
///
/// Object-safe supertrait of the three EOP parameter traits required by
/// [`Rotation::<Gcrs, Itrs>::iau2006_full_from_utc`](arika::frame::Rotation).
/// LOD (Length of Day) is excluded because it is only needed for velocity
/// transformation.
pub trait PositionEop: Ut1Offset + PolarMotion + NutationCorrections + Send + Sync {}

impl<T: Ut1Offset + PolarMotion + NutationCorrections + Send + Sync> PositionEop for T {}

// GcrsEopStorage

/// EOP storage for the Gcrs precise path.
///
/// Wraps a boxed [`PositionEop`] provider and delegates the individual
/// EOP trait methods so it can be passed directly to arika's rotation
/// constructors (which require `P: Ut1Offset + NutationCorrections + PolarMotion`).
pub struct GcrsEopStorage(Box<dyn PositionEop>);

impl GcrsEopStorage {
    /// Create from any provider implementing [`PositionEop`].
    pub fn new(provider: impl PositionEop + 'static) -> Self {
        Self(Box::new(provider))
    }
}

impl std::fmt::Debug for GcrsEopStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcrsEopStorage").finish_non_exhaustive()
    }
}

impl Ut1Offset for GcrsEopStorage {
    fn dut1(&self, utc_mjd: f64) -> f64 {
        self.0.dut1(utc_mjd)
    }
}

impl PolarMotion for GcrsEopStorage {
    fn x_pole(&self, utc_mjd: f64) -> f64 {
        self.0.x_pole(utc_mjd)
    }
    fn y_pole(&self, utc_mjd: f64) -> f64 {
        self.0.y_pole(utc_mjd)
    }
}

impl NutationCorrections for GcrsEopStorage {
    fn dx(&self, utc_mjd: f64) -> f64 {
        self.0.dx(utc_mjd)
    }
    fn dy(&self, utc_mjd: f64) -> f64 {
        self.0.dy(utc_mjd)
    }
}

// EarthPoleBridge trait

/// ECI frame that knows Earth's rotation-pole direction expressed in itself.
///
/// Zonal harmonics (J2/J3/J4) are axially symmetric about the true rotation
/// pole, so a zonal gravity model only needs the pole *direction* in the
/// integration frame — not the full Earth-fixed rotation. This is the minimal
/// capability `ZonalGravity` requires, kept separate from the heavier
/// [`EarthFrameBridge`] (which also provides the ECEF rotation that drag,
/// magnetic field, and geodetic conversions need).
///
/// EOP is intentionally not a parameter: the IAU 2006 model CIP is accurate to
/// well under a milliarcsecond without the observed dX/dY corrections, which is
/// negligible next to the ~0.1–0.3° offset of the true pole from the GCRS Z
/// axis that this captures.
///
/// # Implementations
///
/// - `SimpleEci`: pole = `+Z` (the simple frame defines its Z axis as the pole).
/// - `Gcrs`: pole = the IAU 2006 CIP direction `(X, Y, √(1−X²−Y²))` at the epoch.
pub trait EarthPoleBridge: Eci + Sized + 'static {
    /// Unit vector along Earth's rotation pole, expressed in this frame.
    fn earth_pole(utc: &Epoch<Utc>) -> Vec3<Self>;
}

impl EarthPoleBridge for frame::SimpleEci {
    fn earth_pole(_utc: &Epoch<Utc>) -> Vec3<frame::SimpleEci> {
        Vec3::new(0.0, 0.0, 1.0)
    }
}

impl EarthPoleBridge for frame::Gcrs {
    fn earth_pole(utc: &Epoch<Utc>) -> Vec3<frame::Gcrs> {
        // CIP direction cosines (X, Y) in GCRS from the IAU 2006 model; Z closes
        // the unit vector. The model (no observed dX/dY) is sub-mas accurate.
        let t = utc.to_tt().centuries_since_j2000();
        let (x, y) = cip_xy(t);
        let (x, y) = (x.raw(), y.raw());
        // `.max(0.0)` guards against a slightly-negative radicand from f64
        // round-off (x²+y² is ~1e-5 for real CIP values, never near 1).
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        Vec3::new(x, y, z)
    }
}

// EarthFrameBridge trait

/// ECI frame that can bridge to Earth-fixed (ECEF) coordinates.
///
/// This trait is the type-level dispatch point for force models that need
/// geodetic coordinates (atmosphere, magnetic field) or ECEF↔ECI
/// rotation (atmosphere wind velocity, magnetic field vector
/// transformation).
///
/// # Implementations
///
/// - `SimpleEci`: ERA-only Z rotation (`Rotation<SimpleEci, SimpleEcef>`),
///   no EOP needed (`EopStorage = ()`).
/// - `Gcrs`: Full IAU 2006 CIO chain (`Rotation<Gcrs, Itrs>`),
///   requires EOP provider (`EopStorage = GcrsEopStorage`).
pub trait EarthFrameBridge: EarthPoleBridge {
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

// SimpleEci implementation

impl EarthFrameBridge for frame::SimpleEci {
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

// Gcrs implementation

impl EarthFrameBridge for frame::Gcrs {
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

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::R as R_EARTH;
    use arika::earth::eop::LengthOfDay;
    use arika::epoch::Epoch;

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
        let geo = <frame::SimpleEci as EarthFrameBridge>::to_geodetic(&pos, &utc, &());
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
        let geo = <frame::Gcrs as EarthFrameBridge>::to_geodetic(&pos, &utc, &eop);
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
        let rot = <frame::SimpleEci as EarthFrameBridge>::fixed_to_inertial(&utc, &());
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
        let rot = <frame::Gcrs as EarthFrameBridge>::fixed_to_inertial(&utc, &eop);
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
            <frame::SimpleEci as EarthFrameBridge>::to_geodetic(&pos_simple, &utc, &());

        let pos_gcrs = Vec3::<frame::Gcrs>::new(R_EARTH + alt_km, 0.0, 0.0);
        let eop = GcrsEopStorage::new(ZeroEop);
        let geo_gcrs = <frame::Gcrs as EarthFrameBridge>::to_geodetic(&pos_gcrs, &utc, &eop);

        // Altitudes should agree within a few km (different rotation chains)
        assert!(
            (geo_simple.altitude - geo_gcrs.altitude).abs() < 5.0,
            "simple alt={}, gcrs alt={}",
            geo_simple.altitude,
            geo_gcrs.altitude
        );
    }

    // EarthPoleBridge

    #[test]
    fn simple_eci_pole_is_plus_z() {
        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let p = <frame::SimpleEci as EarthPoleBridge>::earth_pole(&utc);
        assert_eq!(p, Vec3::<frame::SimpleEci>::new(0.0, 0.0, 1.0));
    }

    fn pole_offset_from_z_deg<F: EarthPoleBridge>(utc: &Epoch<Utc>) -> f64 {
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
