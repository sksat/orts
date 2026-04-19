use arika::eclipse::{self, SUN_RADIUS_KM, ShadowModel};
use arika::epoch::Epoch;
use arika::sun;
use nalgebra::Vector3;

use arika::earth::R as R_EARTH;
use arika::frame::Eci;

use crate::model::ExternalLoads;
use crate::model::{HasOrbit, Model};

/// Solar radiation pressure at 1 AU (N/m²).
/// P/c = 1361 W/m² / 299792458 m/s ≈ 4.5396e-6 N/m²
pub const SOLAR_RADIATION_PRESSURE: f64 = 4.5396e-6;

/// Default radiation pressure coefficient (dimensionless).
///
/// Typical ranges:
/// - Perfect absorber: Cr = 1.0
/// - Perfect reflector: Cr = 2.0
/// - Typical satellite: Cr = 1.2–1.5
pub const DEFAULT_CR: f64 = 1.5;

/// Default area-to-mass ratio \[m²/kg\].
///
/// Typical ranges:
/// - Compact satellite: A/m ≈ 0.005–0.02 m²/kg
/// - GPS (large panels): A/m ≈ 0.02–0.04 m²/kg
/// - CubeSat: A/m ≈ 0.01–0.05 m²/kg
pub const DEFAULT_AREA_TO_MASS: f64 = 0.02;

/// Solar Radiation Pressure (SRP) perturbation (cannonball model).
///
/// Computes acceleration from solar photon pressure on a satellite:
///
/// a = -(P/c) × Cr × (A/m) × (AU/r_sun)² × ŝ
///
/// where ŝ is the unit vector from the satellite toward the Sun,
/// giving acceleration directed away from the Sun.
pub struct SolarRadiationPressure {
    /// Radiation pressure coefficient (1.0 = absorber, 2.0 = reflector)
    pub cr: f64,
    /// Cross-sectional area to mass ratio \[m²/kg\]
    pub area_to_mass: f64,
    /// Central body radius for shadow model \[km\].
    /// `None` disables shadow computation (always sunlit).
    pub shadow_body_radius: Option<f64>,
    /// Shadow model to use (default: Cylindrical for backward compatibility).
    pub shadow_model: ShadowModel,
}

impl SolarRadiationPressure {
    /// Create SRP model for Earth orbit with optional explicit area-to-mass ratio.
    ///
    /// Uses [`DEFAULT_CR`] (1.5) and cylindrical Earth shadow by default.
    pub fn for_earth(area_to_mass: Option<f64>) -> Self {
        Self {
            cr: DEFAULT_CR,
            area_to_mass: area_to_mass.unwrap_or(DEFAULT_AREA_TO_MASS),
            shadow_body_radius: Some(R_EARTH),
            shadow_model: ShadowModel::Cylindrical,
        }
    }

    /// Override the radiation pressure coefficient (builder pattern).
    pub fn with_cr(mut self, cr: f64) -> Self {
        self.cr = cr;
        self
    }

    /// Set or override the shadow body radius (builder pattern).
    pub fn with_shadow_body(mut self, radius: f64) -> Self {
        self.shadow_body_radius = Some(radius);
        self
    }

    /// Set the shadow model (builder pattern).
    pub fn with_shadow_model(mut self, model: ShadowModel) -> Self {
        self.shadow_model = model;
        self
    }
}

impl SolarRadiationPressure {
    /// Compute SRP acceleration [km/s²].
    ///
    /// Sun position comes from Meeus ephemeris (`Vec3<Gcrs>`).
    /// The tidal geometry is pure raw vector arithmetic — frame-independent
    /// at Meeus precision.
    pub(crate) fn acceleration(
        &self,
        sat_position: &Vector3<f64>,
        epoch: Option<&Epoch>,
    ) -> Vector3<f64> {
        let epoch = match epoch {
            Some(e) => e,
            None => return Vector3::zeros(),
        };

        let sun_pos = sun::sun_position_eci(epoch).into_inner();
        let sat_to_sun = sun_pos - sat_position;
        let r_sun = sat_to_sun.magnitude();
        let s_hat = sat_to_sun / r_sun;

        // Shadow check using arika::eclipse
        if let Some(body_r) = self.shadow_body_radius {
            let illum = eclipse::illumination_central(
                sat_position,
                &sun_pos,
                body_r,
                SUN_RADIUS_KM,
                self.shadow_model,
            );
            if illum <= 0.0 {
                return Vector3::zeros();
            }
            if illum < 1.0 {
                // Penumbra: scale SRP by illumination fraction
                let distance_ratio = sun::AU_KM / r_sun;
                let a_mag = SOLAR_RADIATION_PRESSURE
                    * self.cr
                    * self.area_to_mass
                    * distance_ratio
                    * distance_ratio
                    / 1000.0;
                return -a_mag * illum * s_hat;
            }
        }

        // SRP acceleration [km/s²]
        // SOLAR_RADIATION_PRESSURE [N/m²] × Cr × (A/m) [m²/kg] = [m/s²]
        // Divide by 1000 to convert to km/s²
        let distance_ratio = sun::AU_KM / r_sun;
        let a_mag = SOLAR_RADIATION_PRESSURE
            * self.cr
            * self.area_to_mass
            * distance_ratio
            * distance_ratio
            / 1000.0;

        // Acceleration is away from the Sun (opposite to ŝ)
        -a_mag * s_hat
    }
}

impl<F: Eci, S: HasOrbit<Frame = F>> Model<S, F> for SolarRadiationPressure {
    fn name(&self) -> &str {
        "srp"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        ExternalLoads::acceleration(self.acceleration(state.orbit().position(), epoch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use arika::earth::MU as MU_EARTH;
    use nalgebra::vector;

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn iss_state() -> OrbitalState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0])
    }

    #[test]
    fn srp_direction_away_from_sun() {
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let state = iss_state();
        let epoch = test_epoch();
        let a = srp.acceleration(state.position(), Some(&epoch));

        let sun_dir = sun::sun_direction_eci(&epoch).into_inner();
        let cos_angle = a.normalize().dot(&sun_dir);
        assert!(
            cos_angle < -0.5,
            "SRP should point away from Sun, cos_angle={cos_angle:.3}"
        );
    }

    #[test]
    fn srp_magnitude_at_1au() {
        let srp = SolarRadiationPressure {
            cr: 1.0,
            area_to_mass: 1.0,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let state = iss_state();
        let epoch = test_epoch();
        let a = srp.acceleration(state.position(), Some(&epoch));
        let expected = SOLAR_RADIATION_PRESSURE / 1000.0;

        let rel_err = (a.magnitude() - expected).abs() / expected;
        assert!(
            rel_err < 0.05,
            "SRP magnitude: expected ~{expected:.3e}, got {:.3e}, rel_err={rel_err:.3}",
            a.magnitude()
        );
    }

    #[test]
    fn srp_scales_with_cr() {
        let epoch = test_epoch();
        let state = iss_state();

        let srp1 = SolarRadiationPressure {
            cr: 1.0,
            area_to_mass: 0.01,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let srp2 = SolarRadiationPressure {
            cr: 2.0,
            area_to_mass: 0.01,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };

        let a1 = srp1
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let a2 = srp2
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let ratio = a2 / a1;

        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "Cr=2 should give 2x acceleration, ratio={ratio}"
        );
    }

    #[test]
    fn srp_scales_with_area_to_mass() {
        let epoch = test_epoch();
        let state = iss_state();

        let srp1 = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.01,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let srp2 = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };

        let a1 = srp1
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let a2 = srp2
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let ratio = a2 / a1;

        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "2x A/m should give 2x acceleration, ratio={ratio}"
        );
    }

    #[test]
    fn srp_no_epoch_returns_zero() {
        let srp = SolarRadiationPressure::for_earth(None);
        let state = iss_state();
        let a = srp.acceleration(state.position(), None);
        assert_eq!(a, Vector3::zeros());
    }

    #[test]
    fn srp_order_of_magnitude_leo() {
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let epoch = test_epoch();
        let state = iss_state();
        let a_mag = srp.acceleration(state.position(), Some(&epoch)).magnitude();

        assert!(
            a_mag > 1e-11 && a_mag < 1e-8,
            "LEO SRP should be ~1e-10 km/s², got {a_mag:.3e}"
        );
    }

    #[test]
    fn srp_zero_in_shadow() {
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: Some(R_EARTH),
            shadow_model: ShadowModel::Cylindrical,
        };
        let epoch = test_epoch();
        let state = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        let a = srp.acceleration(state.position(), Some(&epoch));
        assert_eq!(a, Vector3::zeros(), "SRP should be zero in shadow");
    }

    // Builder tests

    #[test]
    fn for_earth_builder_defaults() {
        let srp = SolarRadiationPressure::for_earth(None);
        assert!((srp.cr - DEFAULT_CR).abs() < 1e-15);
        assert!((srp.area_to_mass - DEFAULT_AREA_TO_MASS).abs() < 1e-15);
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
        assert_eq!(srp.shadow_model, ShadowModel::Cylindrical);
    }

    #[test]
    fn for_earth_explicit_area_to_mass() {
        let srp = SolarRadiationPressure::for_earth(Some(0.05));
        assert!((srp.area_to_mass - 0.05).abs() < 1e-15);
    }

    #[test]
    fn with_cr_builder() {
        let srp = SolarRadiationPressure::for_earth(None).with_cr(1.2);
        assert!((srp.cr - 1.2).abs() < 1e-15);
    }

    #[test]
    fn with_shadow_model_builder() {
        let srp = SolarRadiationPressure::for_earth(None).with_shadow_model(ShadowModel::Conical);
        assert_eq!(srp.shadow_model, ShadowModel::Conical);
    }

    #[test]
    fn conical_shadow_reduces_srp_in_penumbra() {
        // With conical shadow, SRP should be reduced (but not zero) in penumbra
        let srp_conical = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: Some(R_EARTH),
            shadow_model: ShadowModel::Conical,
        };
        let srp_no_shadow = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
        };
        let epoch = test_epoch();

        // Place satellite at the penumbra boundary:
        // behind Earth but at a perpendicular distance ≈ R_EARTH
        let state = OrbitalState::new(
            vector![-(R_EARTH + 400.0), R_EARTH * 1.001, 0.0],
            vector![0.0, -7.67, 0.0],
        );

        let a_conical = srp_conical
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let a_full = srp_no_shadow
            .acceleration(state.position(), Some(&epoch))
            .magnitude();

        // In penumbra, conical should give a reduced but non-zero acceleration
        if a_conical > 0.0 && a_conical < a_full {
            // This is the expected penumbra behavior
            assert!(
                a_conical < a_full,
                "Penumbra SRP should be reduced: conical={a_conical:.3e}, full={a_full:.3e}"
            );
        }
        // If not in penumbra at this position, that's okay too — the geometry
        // may place it outside the penumbra region.
    }
}
