use orts_integrator::{DynamicalSystem, State, StateDerivative};

/// Two-body gravitational system.
///
/// Computes gravitational acceleration a = -μ/|r|³ * r
/// for a body orbiting a central mass with gravitational parameter μ.
pub struct TwoBodySystem {
    pub mu: f64,
}

impl DynamicalSystem for TwoBodySystem {
    fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
        let r = &state.position;
        let r_mag = r.magnitude();
        let acceleration = -self.mu / (r_mag * r_mag * r_mag) * r;
        StateDerivative {
            velocity: state.velocity,
            acceleration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MU_EARTH, R_EARTH};
    use nalgebra::vector;

    #[test]
    fn test_acceleration_direction() {
        // Acceleration should be antiparallel to position (pointing toward center)
        let system = TwoBodySystem { mu: MU_EARTH };
        let state = State {
            position: vector![6778.137, 0.0, 0.0],
            velocity: vector![0.0, 7.6693, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        // Dot product of acceleration and position should be negative (antiparallel)
        let dot = deriv.acceleration.dot(&state.position);
        assert!(dot < 0.0, "acceleration should point toward center (dot={dot})");

        // Cross product should be approximately zero (parallel/antiparallel vectors)
        let cross = deriv.acceleration.cross(&state.position);
        assert!(
            cross.magnitude() < 1e-10,
            "acceleration should be collinear with position (cross mag={})",
            cross.magnitude()
        );
    }

    #[test]
    fn test_acceleration_magnitude() {
        // |a| = μ/|r|² for a known position
        let system = TwoBodySystem { mu: MU_EARTH };
        let r = vector![6778.137, 0.0, 0.0];
        let state = State {
            position: r,
            velocity: vector![0.0, 7.6693, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        let r_mag = r.magnitude();
        let expected_mag = MU_EARTH / (r_mag * r_mag);
        let actual_mag = deriv.acceleration.magnitude();

        let rel_err = (actual_mag - expected_mag).abs() / expected_mag;
        assert!(
            rel_err < 1e-12,
            "acceleration magnitude mismatch: expected={expected_mag}, actual={actual_mag}, rel_err={rel_err}"
        );
    }

    #[test]
    fn test_surface_gravity() {
        // At Earth's surface, |a| ≈ 9.798e-3 km/s²
        let system = TwoBodySystem { mu: MU_EARTH };
        let state = State {
            position: vector![R_EARTH, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);

        let g = deriv.acceleration.magnitude();
        let expected_g = 9.798e-3; // km/s²
        assert!(
            (g - expected_g).abs() < 0.01e-3,
            "surface gravity mismatch: expected≈{expected_g}, actual={g}"
        );
    }
}
