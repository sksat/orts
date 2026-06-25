use arika::body::KnownBody;
use arika::earth::ellipsoid::{WGS84_A, WGS84_B};
use arika::earth::geodetic::Geodetic;
use arika::earth::{OMEGA as OMEGA_EARTH, R as R_EARTH};
use arika::epoch::Epoch;
use arika::frame::{self, Vec3};
use nalgebra::Vector3;
use tobari::{AtmosphereInput, AtmosphereModel, Exponential};

use crate::model::ExternalLoads;
use crate::model::{HasOrbit, Model};
use crate::orbital::OrbitalState;
use arika::earth::EarthFixedTransform;

/// Default ballistic coefficient for LEO satellites \[m²/kg\].
///
/// Typical ranges:
/// - ISS (high-mass, large area): B ≈ 0.005 m²/kg
/// - Compact satellite: B ≈ 0.01-0.02 m²/kg
/// - CubeSat (low-mass, high area/mass): B ≈ 0.05-0.1 m²/kg
pub const DEFAULT_BALLISTIC_COEFF: f64 = 0.01;

/// Atmospheric drag perturbation.
///
/// Computes drag acceleration based on the ballistic coefficient B = Cd*A/(2*m) \[m²/kg\]
/// and a pluggable atmospheric density model.
///
/// The frame parameter `F` selects how ECI positions are converted to
/// geodetic coordinates (for density lookup, via the ECEF rotation) and
/// which spin axis the atmosphere co-rotates about. `SimpleEci` uses the
/// ERA-only ECEF rotation and a +Z spin axis; `Gcrs` uses the full IAU 2006
/// CIO chain and the true CIP spin axis.
pub struct AtmosphericDrag<F: EarthFixedTransform = frame::SimpleEci> {
    /// Central body (enables WGS-84 geodetic altitude for Earth)
    pub body: Option<KnownBody>,
    /// Central body equatorial radius [km] (fallback for non-Earth bodies)
    pub body_radius: f64,
    /// Central body rotation rate [rad/s]
    pub omega_body: f64,
    /// Ballistic coefficient Cd*A/(2*m) [m²/kg]
    pub ballistic_coeff: f64,
    /// Atmospheric density model.
    pub atmosphere: Box<dyn AtmosphereModel>,
    /// EOP storage for the frame adapter. `()` for `SimpleEci`.
    pub eop: F::EopStorage,
}

impl AtmosphericDrag<frame::SimpleEci> {
    /// Create drag model for Earth orbit with an optional explicit ballistic coefficient.
    ///
    /// Uses the piecewise exponential atmosphere model by default.
    /// When `ballistic_coeff` is `None`, uses [`DEFAULT_BALLISTIC_COEFF`] (0.01 m²/kg).
    pub fn for_earth(ballistic_coeff: Option<f64>) -> Self {
        Self {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: ballistic_coeff.unwrap_or(DEFAULT_BALLISTIC_COEFF),
            atmosphere: Box::new(Exponential),
            eop: (),
        }
    }

    /// Create drag model for Earth orbit from B* (TLE drag term).
    ///
    /// **Deprecated**: B* is an SGP4 fitting parameter tied to SGP4's internal
    /// analytical density model. It cannot be meaningfully converted to a physical
    /// ballistic coefficient for use with a different atmospheric model.
    /// The resulting ballistic coefficient is typically ~1000x too large,
    /// causing unrealistically fast orbital decay.
    ///
    /// Use [`AtmosphericDrag::for_earth`] with a physical ballistic coefficient instead.
    #[deprecated(
        since = "0.1.0",
        note = "B* cannot be converted to physical ballistic coefficient. Use AtmosphericDrag::for_earth() instead."
    )]
    pub fn from_bstar(bstar: f64, body_radius: f64) -> Self {
        let rho0 = 2.461e-5;
        let ballistic_coeff = bstar / rho0;
        Self {
            body: Some(KnownBody::Earth),
            body_radius,
            omega_body: OMEGA_EARTH,
            ballistic_coeff,
            atmosphere: Box::new(Exponential),
            eop: (),
        }
    }
}

impl<F: EarthFixedTransform> AtmosphericDrag<F> {
    /// Create drag model for Earth orbit in any frame.
    pub fn for_earth_in_frame(ballistic_coeff: Option<f64>, eop: F::EopStorage) -> Self {
        Self {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: ballistic_coeff.unwrap_or(DEFAULT_BALLISTIC_COEFF),
            atmosphere: Box::new(Exponential),
            eop,
        }
    }

    /// Replace the atmospheric density model (builder pattern).
    pub fn with_atmosphere(mut self, model: Box<dyn AtmosphereModel>) -> Self {
        self.atmosphere = model;
        self
    }
}

impl<F: EarthFixedTransform> AtmosphericDrag<F> {
    /// Compute drag acceleration [km/s²] from orbital state.
    pub(crate) fn acceleration(
        &self,
        state: &OrbitalState<F>,
        epoch: Option<&Epoch<arika::epoch::Utc>>,
    ) -> Vector3<f64> {
        let pos = state.position();
        // Check if inside the body (ellipsoid test for Earth, spherical for others)
        let inside = match self.body {
            Some(KnownBody::Earth) => {
                let p2 = pos.x * pos.x + pos.y * pos.y;
                let z2 = pos.z * pos.z;
                p2 / (WGS84_A * WGS84_A) + z2 / (WGS84_B * WGS84_B) < 1.0
            }
            _ => pos.magnitude() < self.body_radius,
        };
        if inside {
            return Vector3::zeros();
        }

        // TODO: OrbitalSystem::epoch_0 を Option ではなく required にすれば
        // この dummy epoch は不要になる。現状は epoch なしで呼ばれる unit test
        // パスのためだけに存在する一時的な妥協。
        let dummy_epoch = arika::epoch::Epoch::from_jd(2451545.0); // J2000.0
        let utc = epoch.unwrap_or(&dummy_epoch);

        let geodetic = match self.body {
            Some(KnownBody::Earth) => {
                let pos_vec = Vec3::<F>::from_raw(*pos);
                F::to_geodetic(&pos_vec, utc, &self.eop)
            }
            _ => {
                let r_mag = pos.magnitude();
                Geodetic {
                    latitude: (pos.z / r_mag).asin(),
                    longitude: pos.y.atan2(pos.x),
                    altitude: r_mag - self.body_radius,
                }
            }
        };

        let input = AtmosphereInput { geodetic, utc };
        let rho = self.atmosphere.density(&input); // kg/m³
        if rho == 0.0 {
            return Vector3::zeros();
        }

        // Atmosphere co-rotation velocity Ω × r, with Ω along the central body's
        // spin axis (magnitude `omega_body`). For Earth the spin axis is the IAU
        // 2006 CIP, which `EarthRotationPole` expresses in the integration frame
        // `F`: +Z for `SimpleEci` (so this reduces exactly to the classic
        // [0, 0, ω] × r), the true CIP for `Gcrs` (correcting the ~0.1–0.3°
        // offset of the spin axis from the GCRS Z axis that a +Z assumption
        // incurs). Using the CIP — the rotation axis itself — is more faithful
        // than rotating the ITRS figure axis via `fixed_to_inertial` (which
        // differs by polar motion); LOD variation in |Ω| is omitted.
        //
        // `earth_pole` is Earth-specific, so for any other central body we keep
        // the frame Z axis with that body's `omega_body` (the pre-existing
        // behavior, matching the spherical geodetic fallback above — `F` is an
        // Earth ECI frame regardless). A non-Earth body's true spin-axis
        // orientation is not modeled: it would need a per-body pole and a
        // body-fixed frame, so non-Earth drag here stays a coarse approximation.
        // Tracked in #210.
        let omega = match self.body {
            Some(KnownBody::Earth) => F::earth_pole(utc).into_inner() * self.omega_body,
            _ => Vector3::new(0.0, 0.0, self.omega_body),
        };
        let v_rel = *state.velocity() - omega.cross(pos);

        // Convert v_rel from km/s to m/s for consistent units with ρ [kg/m³] and B [m²/kg]
        let v_rel_m = v_rel * 1000.0; // km/s → m/s
        let v_rel_mag = v_rel_m.magnitude();

        if v_rel_mag < 1e-10 {
            return Vector3::zeros();
        }

        // a_drag = -B * ρ * |v_rel| * v_rel  [m/s²]
        let a_drag_m = -self.ballistic_coeff * rho * v_rel_mag * v_rel_m;

        // Convert back to km/s²
        a_drag_m / 1000.0
    }
}

impl<F: EarthFixedTransform, S: HasOrbit<Frame = F>> Model<S, F> for AtmosphericDrag<F> {
    fn name(&self) -> &str {
        "drag"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        ExternalLoads::acceleration(self.acceleration(state.orbit(), epoch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::{MU as MU_EARTH, R as R_EARTH};
    use nalgebra::vector;

    fn iss_drag() -> AtmosphericDrag {
        AtmosphericDrag {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: 0.005, // physical ISS: Cd*A/(2m) ≈ 2.2*2000/(2*420000)
            atmosphere: Box::new(Exponential),
            eop: (),
        }
    }

    #[test]
    fn drag_opposes_relative_velocity() {
        let drag = iss_drag();
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0]);

        let a = drag.acceleration(&state, None);

        // v_rel = v - ω×r. At (r,0,0), ω×r = (0,0,ω)×(r,0,0) = (0, ω*r, 0)
        // v_rel = (0, v - ω*r, 0)
        let v_rel_y = v - OMEGA_EARTH * r;
        // Drag should be in -y direction (opposing v_rel)
        assert!(a.y < 0.0, "Drag should oppose velocity, got a.y={}", a.y);
        // x and z components should be near zero
        assert!(
            a.x.abs() < a.y.abs() * 1e-10,
            "a.x should be ~0, got {}",
            a.x
        );
        assert!(
            a.z.abs() < a.y.abs() * 1e-10,
            "a.z should be ~0, got {}",
            a.z
        );

        // Check that drag accounts for atmosphere co-rotation
        // v_rel < v_inertial because atmosphere moves with Earth
        assert!(
            v_rel_y < v,
            "Relative velocity ({v_rel_y:.4}) should be less than inertial ({v:.4})"
        );
    }

    #[test]
    fn drag_magnitude_at_iss() {
        let drag = iss_drag();
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0]);

        let a = drag.acceleration(&state, None);
        let a_mag = a.magnitude();

        // At ISS altitude (400km):
        // ρ ≈ 3.7e-12 kg/m³
        // v_rel ≈ 7.66 km/s ≈ 7660 m/s
        // B = 0.005 m²/kg
        // |a| = B * ρ * v² ≈ 0.005 * 3.7e-12 * 7660² ≈ 1.1e-6 m/s² ≈ 1.1e-9 km/s²
        assert!(
            a_mag > 1e-11 && a_mag < 1e-7,
            "ISS drag magnitude should be ~1e-10 to 1e-8 km/s², got {a_mag:.6e}"
        );
    }

    #[test]
    fn drag_increases_at_lower_altitude() {
        let drag = iss_drag();
        let v = 7.5; // approximate LEO velocity

        let state_high =
            OrbitalState::new(vector![R_EARTH + 600.0, 0.0, 0.0], vector![0.0, v, 0.0]);
        let state_low = OrbitalState::new(vector![R_EARTH + 300.0, 0.0, 0.0], vector![0.0, v, 0.0]);

        let a_high = drag.acceleration(&state_high, None).magnitude();
        let a_low = drag.acceleration(&state_low, None).magnitude();

        assert!(
            a_low > a_high * 10.0,
            "Drag at 300km ({a_low:.6e}) should be much larger than at 600km ({a_high:.6e})"
        );
    }

    #[test]
    fn no_drag_above_atmosphere() {
        let drag = iss_drag();
        let state = OrbitalState::new(vector![R_EARTH + 3000.0, 0.0, 0.0], vector![0.0, 5.0, 0.0]);

        let a = drag.acceleration(&state, None);
        assert_eq!(a, Vector3::zeros(), "No drag above atmosphere");
    }

    #[test]
    fn for_earth_default_ballistic_coeff() {
        let drag = AtmosphericDrag::for_earth(None);
        assert!(
            (drag.ballistic_coeff - DEFAULT_BALLISTIC_COEFF).abs() < 1e-15,
            "Default should be {DEFAULT_BALLISTIC_COEFF}, got {}",
            drag.ballistic_coeff
        );
        assert!((drag.body_radius - R_EARTH).abs() < 1e-10);
        assert!((drag.omega_body - OMEGA_EARTH).abs() < 1e-15);
    }

    #[test]
    fn for_earth_explicit_ballistic_coeff() {
        let drag = AtmosphericDrag::for_earth(Some(0.005));
        assert!(
            (drag.ballistic_coeff - 0.005).abs() < 1e-15,
            "Explicit B should be 0.005, got {}",
            drag.ballistic_coeff
        );
    }

    #[test]
    fn earth_rotation_effect() {
        // Verify that Earth rotation reduces the relative velocity
        let drag_rotating = AtmosphericDrag {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: 0.005,
            atmosphere: Box::new(Exponential),
            eop: (),
        };
        let drag_static = AtmosphericDrag {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: 0.0,
            ballistic_coeff: 0.005,
            atmosphere: Box::new(Exponential),
            eop: (),
        };

        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(
            vector![r, 0.0, 0.0],
            vector![0.0, v, 0.0], // prograde orbit
        );

        let a_rotating = drag_rotating.acceleration(&state, None).magnitude();
        let a_static = drag_static.acceleration(&state, None).magnitude();

        // For prograde orbit, co-rotating atmosphere means lower relative velocity → less drag
        assert!(
            a_rotating < a_static,
            "Prograde drag with rotation ({a_rotating:.6e}) should be less than without ({a_static:.6e})"
        );
    }

    #[test]
    fn with_atmosphere_builder() {
        use tobari::HarrisPriester;

        let drag = AtmosphericDrag::for_earth(Some(0.005))
            .with_atmosphere(Box::new(HarrisPriester::new()));

        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0]);

        // Without epoch, HP returns average density — should still produce non-zero drag
        let a = drag.acceleration(&state, None);
        assert!(
            a.magnitude() > 0.0,
            "HP drag should be non-zero at ISS altitude"
        );
    }

    #[test]
    fn gcrs_co_rotation_uses_true_cip_spin_axis() {
        // The atmosphere co-rotates about Earth's spin axis. For `Gcrs` that axis
        // is the IAU 2006 CIP, offset from the GCRS Z axis by precession/nutation.
        // Pin that drag computes the co-rotation velocity about the CIP (not +Z)
        // by reconstructing the acceleration from `EarthRotationPole::earth_pole`.
        use arika::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};
        use arika::earth::{EarthRotationPole, GcrsEopStorage};

        // Zero-EOP: model CIP only (no observed dX/dY, dUT1, polar motion) — the
        // same provider oracle_gcrf uses; keeps the CIP at sub-mas accuracy.
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

        let utc = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        // The check is only meaningful if the CIP differs from +Z at this epoch.
        let pole = <frame::Gcrs as EarthRotationPole>::earth_pole(&utc).into_inner();
        let off_axis = (pole - vector![0.0, 0.0, 1.0]).norm();
        assert!(
            off_axis > 1e-9,
            "Gcrs CIP should be measurably offset from +Z at 2024, got {off_axis:e}"
        );

        // 3D LEO state (all components non-zero so the spin-axis tilt is exercised).
        let r = R_EARTH + 400.0;
        let pos = vector![0.5_f64, 0.3, 0.8].normalize() * r;
        let v = (MU_EARTH / r).sqrt();
        let vel = vector![-v * 0.6, v * 0.5, v * 0.2];
        let state = OrbitalState::<frame::Gcrs>::new_in_frame(pos, vel);

        let drag = AtmosphericDrag::<frame::Gcrs>::for_earth_in_frame(
            Some(0.005),
            GcrsEopStorage::new(ZeroEop),
        );
        let a = drag.acceleration(&state, Some(&utc));

        // Reconstruct the expected acceleration with Ω along the CIP spin axis.
        let omega = pole * OMEGA_EARTH;
        let v_rel = vel - omega.cross(&pos);
        let geod = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &Vec3::from_raw(pos),
            &utc,
            &GcrsEopStorage::new(ZeroEop),
        );
        let rho = Exponential.density(&AtmosphereInput {
            geodetic: geod,
            utc: &utc,
        });
        assert!(rho > 0.0, "expected non-zero density at 400 km");
        let v_rel_m = v_rel * 1000.0;
        let expected = (-0.005 * rho * v_rel_m.magnitude() * v_rel_m) / 1000.0;

        // If drag had kept the +Z assumption, it would disagree with this CIP
        // reconstruction by ~ω·r·offset; a tight match proves the CIP is used.
        assert!(
            (a - expected).norm() < 1e-18,
            "Gcrs drag must use the CIP spin axis for co-rotation; err={:e}",
            (a - expected).norm()
        );
    }
}
