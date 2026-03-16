use nalgebra::Vector3;
use kaname::epoch::Epoch;
use kaname::sun;

use kaname::constants::R_EARTH;
use crate::OrbitalState;
use crate::perturbations::ForceModel;

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
    /// Central body radius for cylindrical shadow model \[km\].
    /// `None` disables shadow computation (always sunlit).
    pub shadow_body_radius: Option<f64>,
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
}

/// Cylindrical shadow model.
///
/// Returns 0.0 (umbra/shadow) or 1.0 (sunlit).
/// The shadow cylinder has radius = `body_radius` and axis along the Earth→Sun direction.
/// A satellite is in shadow when it is on the anti-Sun side of the central body
/// and its perpendicular distance to the Sun-Earth line is less than `body_radius`.
fn shadow_function(
    sat_position: &Vector3<f64>,
    sun_position: &Vector3<f64>,
    body_radius: f64,
) -> f64 {
    let sun_dir = sun_position.normalize(); // Earth → Sun unit vector

    // Project satellite position onto the Sun direction
    let projection = sat_position.dot(&sun_dir);

    // If satellite is on the Sun side of Earth, it's sunlit
    if projection >= 0.0 {
        return 1.0;
    }

    // Satellite is behind Earth (anti-Sun side).
    // Compute perpendicular distance to the Earth-Sun axis.
    let perp = sat_position - projection * sun_dir;
    let perp_dist = perp.magnitude();

    if perp_dist < body_radius {
        0.0 // in cylindrical shadow
    } else {
        1.0 // outside shadow cylinder
    }
}

impl ForceModel for SolarRadiationPressure {
    fn name(&self) -> &str {
        "srp"
    }

    fn acceleration(&self, _t: f64, state: &OrbitalState, epoch: Option<&Epoch>) -> Vector3<f64> {
        let epoch = match epoch {
            Some(e) => e,
            None => return Vector3::zeros(),
        };

        let sun_pos = sun::sun_position_eci(epoch);
        let sat_to_sun = sun_pos - *state.position();
        let r_sun = sat_to_sun.magnitude();
        let s_hat = sat_to_sun / r_sun;

        // Shadow check
        if let Some(body_r) = self.shadow_body_radius {
            let illumination = shadow_function(state.position(), &sun_pos, body_r);
            if illumination < 0.5 {
                return Vector3::zeros();
            }
        }

        // SRP acceleration [km/s²]
        // SOLAR_RADIATION_PRESSURE [N/m²] × Cr × (A/m) [m²/kg] = [m/s²]
        // Divide by 1000 to convert to km/s²
        let distance_ratio = sun::AU_KM / r_sun;
        let a_mag = SOLAR_RADIATION_PRESSURE * self.cr * self.area_to_mass
            * distance_ratio * distance_ratio / 1000.0;

        // Acceleration is away from the Sun (opposite to ŝ)
        -a_mag * s_hat
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaname::constants::MU_EARTH;
    use nalgebra::vector;

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn iss_state() -> OrbitalState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(
            vector![r, 0.0, 0.0],
            vector![0.0, v, 0.0],
        )
    }

    #[test]
    fn srp_direction_away_from_sun() {
        // At March equinox, Sun is approximately in +X direction.
        // Satellite at +X: ŝ ≈ +X, so a ≈ -X (away from Sun).
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
        };
        let state = iss_state();
        let epoch = test_epoch();
        let a = srp.acceleration(0.0, &state, Some(&epoch));

        // Sun direction at equinox is roughly +X, so acceleration should be roughly -X
        let sun_dir = sun::sun_direction_eci(&epoch);
        let cos_angle = a.normalize().dot(&sun_dir);
        // Acceleration should point away from Sun (cos < -0.5)
        assert!(
            cos_angle < -0.5,
            "SRP should point away from Sun, cos_angle={cos_angle:.3}"
        );
    }

    #[test]
    fn srp_magnitude_at_1au() {
        // Cr=1, A/m=1 m²/kg at ~1 AU: |a| ≈ 4.54e-6 / 1000 = 4.54e-9 km/s²
        let srp = SolarRadiationPressure {
            cr: 1.0,
            area_to_mass: 1.0,
            shadow_body_radius: None,
        };
        let state = iss_state();
        let epoch = test_epoch();
        let a = srp.acceleration(0.0, &state, Some(&epoch));
        let expected = SOLAR_RADIATION_PRESSURE / 1000.0; // ≈ 4.54e-9 km/s²

        // Distance is approximately 1 AU (satellite offset is negligible)
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

        let srp1 = SolarRadiationPressure { cr: 1.0, area_to_mass: 0.01, shadow_body_radius: None };
        let srp2 = SolarRadiationPressure { cr: 2.0, area_to_mass: 0.01, shadow_body_radius: None };

        let a1 = srp1.acceleration(0.0, &state, Some(&epoch)).magnitude();
        let a2 = srp2.acceleration(0.0, &state, Some(&epoch)).magnitude();
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

        let srp1 = SolarRadiationPressure { cr: 1.5, area_to_mass: 0.01, shadow_body_radius: None };
        let srp2 = SolarRadiationPressure { cr: 1.5, area_to_mass: 0.02, shadow_body_radius: None };

        let a1 = srp1.acceleration(0.0, &state, Some(&epoch)).magnitude();
        let a2 = srp2.acceleration(0.0, &state, Some(&epoch)).magnitude();
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
        let a = srp.acceleration(0.0, &state, None);
        assert_eq!(a, Vector3::zeros());
    }

    #[test]
    fn srp_order_of_magnitude_leo() {
        // Typical LEO: Cr=1.5, A/m=0.02 m²/kg
        // |a| = 4.54e-6 * 1.5 * 0.02 / 1000 ≈ 1.36e-10 km/s²
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: None,
        };
        let epoch = test_epoch();
        let state = iss_state();
        let a_mag = srp.acceleration(0.0, &state, Some(&epoch)).magnitude();

        assert!(
            a_mag > 1e-11 && a_mag < 1e-8,
            "LEO SRP should be ~1e-10 km/s², got {a_mag:.3e}"
        );
    }

    // Shadow function tests

    #[test]
    fn shadow_function_sunlit() {
        // Satellite on the Sun-side of Earth
        let sun_pos = vector![149_597_870.7, 0.0, 0.0];
        let sat_pos = vector![R_EARTH + 400.0, 0.0, 0.0];
        let shadow = shadow_function(&sat_pos, &sun_pos, R_EARTH);
        assert!((shadow - 1.0).abs() < 1e-10);
    }

    #[test]
    fn shadow_function_umbra() {
        // Satellite directly behind Earth (opposite side from Sun)
        let sun_pos = vector![149_597_870.7, 0.0, 0.0];
        let sat_pos = vector![-(R_EARTH + 400.0), 0.0, 0.0];
        let shadow = shadow_function(&sat_pos, &sun_pos, R_EARTH);
        assert!((shadow - 0.0).abs() < 1e-10);
    }

    #[test]
    fn shadow_function_perpendicular() {
        // Satellite at 90° from Sun-Earth line: sunlit
        let sun_pos = vector![149_597_870.7, 0.0, 0.0];
        let sat_pos = vector![0.0, R_EARTH + 400.0, 0.0];
        let shadow = shadow_function(&sat_pos, &sun_pos, R_EARTH);
        assert!((shadow - 1.0).abs() < 1e-10);
    }

    #[test]
    fn shadow_function_just_inside() {
        // Behind Earth, within shadow cylinder (perpendicular dist < R_EARTH)
        let sun_pos = vector![149_597_870.7, 0.0, 0.0];
        let sat_pos = vector![-(R_EARTH + 400.0), R_EARTH * 0.5, 0.0];
        let shadow = shadow_function(&sat_pos, &sun_pos, R_EARTH);
        assert!((shadow - 0.0).abs() < 1e-10);
    }

    #[test]
    fn shadow_function_just_outside() {
        // Behind Earth, outside shadow cylinder (perpendicular dist > R_EARTH)
        let sun_pos = vector![149_597_870.7, 0.0, 0.0];
        let sat_pos = vector![-(R_EARTH + 400.0), R_EARTH * 1.1, 0.0];
        let shadow = shadow_function(&sat_pos, &sun_pos, R_EARTH);
        assert!((shadow - 1.0).abs() < 1e-10);
    }

    #[test]
    fn srp_zero_in_shadow() {
        // At March equinox, Sun is near +X. Place satellite behind Earth at -X.
        let srp = SolarRadiationPressure {
            cr: 1.5,
            area_to_mass: 0.02,
            shadow_body_radius: Some(R_EARTH),
        };
        let epoch = test_epoch();
        let state = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        let a = srp.acceleration(0.0, &state, Some(&epoch));
        assert_eq!(a, Vector3::zeros(), "SRP should be zero in shadow");
    }

    // Builder tests

    #[test]
    fn for_earth_builder_defaults() {
        let srp = SolarRadiationPressure::for_earth(None);
        assert!((srp.cr - DEFAULT_CR).abs() < 1e-15);
        assert!((srp.area_to_mass - DEFAULT_AREA_TO_MASS).abs() < 1e-15);
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
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
}
