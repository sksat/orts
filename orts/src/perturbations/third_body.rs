use kaname::epoch::Epoch;
use nalgebra::Vector3;

use crate::OrbitalState;
use crate::perturbations::ForceModel;

/// Third-body gravitational perturbation.
///
/// Computes the gravitational acceleration on a satellite due to a third body
/// (e.g., Sun or Moon) using the standard perturbation formula:
///
/// a = μ_3 * [(r_body - r_sat)/|r_body - r_sat|³ - r_body/|r_body|³]
///
/// where r_body is the position of the third body relative to the central body,
/// and r_sat is the satellite position relative to the central body.
pub struct ThirdBodyGravity {
    /// Human-readable name (e.g., "third_body_sun", "third_body_moon")
    pub name: &'static str,
    /// Gravitational parameter of the third body [km³/s²]
    pub mu_body: f64,
    /// Function returning the third body position in ECI [km] at a given epoch
    pub body_position_fn: fn(&Epoch) -> Vector3<f64>,
}

impl ThirdBodyGravity {
    /// Create a Sun third-body perturbation.
    pub fn sun() -> Self {
        Self {
            name: "third_body_sun",
            mu_body: kaname::constants::MU_SUN,
            body_position_fn: kaname::sun::sun_position_eci,
        }
    }

    /// Create a Moon third-body perturbation.
    ///
    /// Moon μ ≈ 4902.8 km³/s² (from body properties).
    pub fn moon() -> Self {
        Self {
            name: "third_body_moon",
            mu_body: 4902.800066, // μ_Moon [km³/s²]
            body_position_fn: kaname::moon::moon_position_eci,
        }
    }
}

impl ForceModel for ThirdBodyGravity {
    fn name(&self) -> &str {
        self.name
    }

    fn acceleration(&self, _t: f64, state: &OrbitalState, epoch: Option<&Epoch>) -> Vector3<f64> {
        let epoch = match epoch {
            Some(e) => e,
            None => return Vector3::zeros(),
        };

        let r_body = (self.body_position_fn)(epoch);
        let r_sat_to_body = r_body - *state.position();
        let d = r_sat_to_body.magnitude();
        let r_body_mag = r_body.magnitude();

        // a = μ₃ * [(r_body - r_sat)/d³ - r_body/R³]
        self.mu_body
            * (r_sat_to_body / (d * d * d) - r_body / (r_body_mag * r_body_mag * r_body_mag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaname::constants::{MU_EARTH, R_EARTH};
    use nalgebra::vector;

    fn iss_state() -> OrbitalState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0])
    }

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    #[test]
    fn sun_perturbation_order_of_magnitude() {
        let tb = ThirdBodyGravity::sun();
        let state = iss_state();
        let epoch = test_epoch();

        let a = tb.acceleration(0.0, &state, Some(&epoch));
        let a_mag = a.magnitude();

        // Sun tidal acceleration on LEO satellite:
        // a ≈ 2*μ_sun*r_sat / d_sun³ ≈ 2*1.327e11*6778 / (1.5e8)³ ≈ 5e-10 km/s²
        assert!(
            a_mag > 1e-11 && a_mag < 1e-8,
            "Sun perturbation should be ~5e-10 km/s², got {a_mag:.6e}"
        );
    }

    #[test]
    fn moon_perturbation_order_of_magnitude() {
        let tb = ThirdBodyGravity::moon();
        let state = iss_state();
        let epoch = test_epoch();

        let a = tb.acceleration(0.0, &state, Some(&epoch));
        let a_mag = a.magnitude();

        // Moon tidal acceleration on LEO satellite:
        // a ≈ 2*μ_moon*r_sat / d_moon³ ≈ 2*4903*6778 / (3.84e5)³ ≈ 1.2e-9 km/s²
        assert!(
            a_mag > 1e-11 && a_mag < 1e-7,
            "Moon perturbation should be ~1e-9 km/s², got {a_mag:.6e}"
        );
    }

    #[test]
    fn no_epoch_returns_zero() {
        let tb = ThirdBodyGravity::sun();
        let state = iss_state();

        let a = tb.acceleration(0.0, &state, None);
        assert_eq!(
            a,
            Vector3::zeros(),
            "No epoch should give zero acceleration"
        );
    }

    #[test]
    fn perturbation_much_smaller_than_central_gravity() {
        let tb_sun = ThirdBodyGravity::sun();
        let tb_moon = ThirdBodyGravity::moon();
        let state = iss_state();
        let epoch = test_epoch();

        let a_sun = tb_sun.acceleration(0.0, &state, Some(&epoch)).magnitude();
        let a_moon = tb_moon.acceleration(0.0, &state, Some(&epoch)).magnitude();

        // Central body gravity: μ/r² ≈ 398600/6778² ≈ 8.7e-3 km/s²
        let r = state.position().magnitude();
        let a_central = MU_EARTH / (r * r);

        // Third-body should be ~6-7 orders of magnitude smaller
        assert!(
            a_sun < a_central * 1e-4,
            "Sun perturbation ({a_sun:.6e}) should be << central gravity ({a_central:.6e})"
        );
        assert!(
            a_moon < a_central * 1e-4,
            "Moon perturbation ({a_moon:.6e}) should be << central gravity ({a_central:.6e})"
        );
    }

    #[test]
    fn sun_perturbation_varies_with_epoch() {
        // Tidal force has 180° symmetry, so compare 90°-apart epochs (March vs June).
        // Place satellite on Y-axis:
        // - March: Sun near +X → satellite perpendicular → tidal compression along Y
        // - June: Sun near +Y → satellite along Sun axis → tidal stretching along Y
        // These give opposite Y-acceleration directions.
        let tb = ThirdBodyGravity::sun();
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(vector![0.0, r, 0.0], vector![-v, 0.0, 0.0]);

        let epoch1 = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 6, 20, 12, 0, 0.0);

        let a1 = tb.acceleration(0.0, &state, Some(&epoch1));
        let a2 = tb.acceleration(0.0, &state, Some(&epoch2));

        // Direction should be very different (perpendicular vs parallel to Sun)
        let cos_angle = a1.normalize().dot(&a2.normalize());
        assert!(
            cos_angle < 0.5,
            "Sun perturbation should differ between March and June, cos={cos_angle:.3}"
        );
    }

    #[test]
    fn geo_larger_perturbation_than_leo() {
        // GEO is farther from Earth center → third-body perturbation is relatively more significant
        let tb_moon = ThirdBodyGravity::moon();
        let epoch = test_epoch();

        let leo_state = iss_state();
        let geo_r = 42164.0; // GEO radius
        let geo_v = (MU_EARTH / geo_r).sqrt();
        let geo_state = OrbitalState::new(vector![geo_r, 0.0, 0.0], vector![0.0, geo_v, 0.0]);

        let a_leo = tb_moon
            .acceleration(0.0, &leo_state, Some(&epoch))
            .magnitude();
        let a_geo = tb_moon
            .acceleration(0.0, &geo_state, Some(&epoch))
            .magnitude();

        // At GEO, satellite is closer to Moon (shorter range) → larger perturbation
        // Also the "indirect" term is larger relative to "direct" term
        // The absolute perturbation may not always be larger, but relative to central gravity it is
        let a_central_leo = MU_EARTH / leo_state.position().magnitude_squared();
        let a_central_geo = MU_EARTH / geo_state.position().magnitude_squared();

        let ratio_leo = a_leo / a_central_leo;
        let ratio_geo = a_geo / a_central_geo;

        assert!(
            ratio_geo > ratio_leo,
            "Moon perturbation ratio at GEO ({ratio_geo:.6e}) should be > LEO ({ratio_leo:.6e})"
        );
    }
}
