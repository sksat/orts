use nalgebra::Vector3;
use kaname::epoch::Epoch;
use orts_integrator::State;

use crate::atmosphere;
use crate::constants::R_EARTH;
use crate::perturbations::ForceModel;

/// Earth rotation rate [rad/s] (IERS 2010)
pub const OMEGA_EARTH: f64 = 7.2921159e-5;

/// Default ballistic coefficient for LEO satellites \[m²/kg\].
///
/// Typical ranges:
/// - ISS (high-mass, large area): B ≈ 0.005 m²/kg
/// - Compact satellite: B ≈ 0.01-0.02 m²/kg
/// - CubeSat (low-mass, high area/mass): B ≈ 0.05-0.1 m²/kg
pub const DEFAULT_BALLISTIC_COEFF: f64 = 0.01;

/// Atmospheric drag perturbation.
///
/// Uses a piecewise exponential atmosphere model and computes drag
/// acceleration based on the ballistic coefficient B = Cd*A/(2*m) \[m²/kg\].
pub struct AtmosphericDrag {
    /// Central body equatorial radius [km]
    pub body_radius: f64,
    /// Central body rotation rate [rad/s]
    pub omega_body: f64,
    /// Ballistic coefficient Cd*A/(2*m) [m²/kg]
    pub ballistic_coeff: f64,
}

impl AtmosphericDrag {
    /// Create drag model for Earth orbit with an optional explicit ballistic coefficient.
    ///
    /// When `ballistic_coeff` is `None`, uses [`DEFAULT_BALLISTIC_COEFF`] (0.01 m²/kg).
    pub fn for_earth(ballistic_coeff: Option<f64>) -> Self {
        Self {
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: ballistic_coeff.unwrap_or(DEFAULT_BALLISTIC_COEFF),
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
            body_radius,
            omega_body: OMEGA_EARTH,
            ballistic_coeff,
        }
    }
}

impl ForceModel for AtmosphericDrag {
    fn acceleration(&self, _t: f64, state: &State, _epoch: Option<&Epoch>) -> Vector3<f64> {
        let alt = state.position.magnitude() - self.body_radius;

        // No drag above atmosphere
        if alt < 0.0 {
            return Vector3::zeros();
        }

        let rho = atmosphere::density(alt); // kg/m³
        if rho == 0.0 {
            return Vector3::zeros();
        }

        // Relative velocity: v_rel = v - ω × r (atmosphere co-rotates with body)
        let omega = Vector3::new(0.0, 0.0, self.omega_body);
        let v_rel = state.velocity - omega.cross(&state.position);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MU_EARTH, R_EARTH};
    use nalgebra::vector;

    fn iss_drag() -> AtmosphericDrag {
        AtmosphericDrag {
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: 0.005, // physical ISS: Cd*A/(2m) ≈ 2.2*2000/(2*420000)
        }
    }

    #[test]
    fn drag_opposes_relative_velocity() {
        let drag = iss_drag();
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };

        let a = drag.acceleration(0.0, &state, None);

        // v_rel = v - ω×r. At (r,0,0), ω×r = (0,0,ω)×(r,0,0) = (0, ω*r, 0)
        // v_rel = (0, v - ω*r, 0)
        let v_rel_y = v - OMEGA_EARTH * r;
        // Drag should be in -y direction (opposing v_rel)
        assert!(a.y < 0.0, "Drag should oppose velocity, got a.y={}", a.y);
        // x and z components should be near zero
        assert!(a.x.abs() < a.y.abs() * 1e-10, "a.x should be ~0, got {}", a.x);
        assert!(a.z.abs() < a.y.abs() * 1e-10, "a.z should be ~0, got {}", a.z);

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
        let state = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };

        let a = drag.acceleration(0.0, &state, None);
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

        let state_high = State {
            position: vector![R_EARTH + 600.0, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };
        let state_low = State {
            position: vector![R_EARTH + 300.0, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };

        let a_high = drag.acceleration(0.0, &state_high, None).magnitude();
        let a_low = drag.acceleration(0.0, &state_low, None).magnitude();

        assert!(
            a_low > a_high * 10.0,
            "Drag at 300km ({a_low:.6e}) should be much larger than at 600km ({a_high:.6e})"
        );
    }

    #[test]
    fn no_drag_above_atmosphere() {
        let drag = iss_drag();
        let state = State {
            position: vector![R_EARTH + 3000.0, 0.0, 0.0],
            velocity: vector![0.0, 5.0, 0.0],
        };

        let a = drag.acceleration(0.0, &state, None);
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
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            ballistic_coeff: 0.005,
        };
        let drag_static = AtmosphericDrag {
            body_radius: R_EARTH,
            omega_body: 0.0,
            ballistic_coeff: 0.005,
        };

        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0], // prograde orbit
        };

        let a_rotating = drag_rotating.acceleration(0.0, &state, None).magnitude();
        let a_static = drag_static.acceleration(0.0, &state, None).magnitude();

        // For prograde orbit, co-rotating atmosphere means lower relative velocity → less drag
        assert!(
            a_rotating < a_static,
            "Prograde drag with rotation ({a_rotating:.6e}) should be less than without ({a_static:.6e})"
        );
    }
}
