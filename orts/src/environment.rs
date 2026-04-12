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
use arika::epoch::{Epoch, Utc};
use arika::frame::{self, Ecef, Eci, Rotation, Vec3};

// ---------------------------------------------------------------------------
// PositionEop — combined trait for position-level rotation
// ---------------------------------------------------------------------------

/// Combined EOP capability needed for position-level Gcrs↔Itrs rotation.
///
/// Object-safe supertrait of the three EOP parameter traits required by
/// [`Rotation::<Gcrs, Itrs>::iau2006_full_from_utc`](arika::frame::Rotation).
/// LOD (Length of Day) is excluded because it is only needed for velocity
/// transformation.
pub trait PositionEop: Ut1Offset + PolarMotion + NutationCorrections + Send + Sync {}

impl<T: Ut1Offset + PolarMotion + NutationCorrections + Send + Sync> PositionEop for T {}

// ---------------------------------------------------------------------------
// GcrsEopStorage
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// EarthFrameBridge trait
// ---------------------------------------------------------------------------

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
pub trait EarthFrameBridge: Eci + Sized + 'static {
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

// ---------------------------------------------------------------------------
// SimpleEci implementation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Gcrs implementation
// ---------------------------------------------------------------------------

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
}
