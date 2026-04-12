use crate::OrbitalState;
use crate::attitude::AttitudeState;
use crate::model::{HasAttitude, HasMass, HasOrbit};
use nalgebra::{Vector3, Vector4};
use utsuroi::{OdeState, Tolerances};

/// Combined spacecraft state: orbital (6D) + attitude (7D) + mass (1D).
///
/// Used as the ODE state vector for coupled orbit-attitude propagation.
/// Mass is included for future thrust modeling (mass depletion).
#[derive(Debug, Clone, PartialEq)]
pub struct SpacecraftState {
    pub orbit: OrbitalState,
    pub attitude: AttitudeState,
    pub mass: f64,
}

impl SpacecraftState {
    /// Create a derivative state for the ODE formulation.
    ///
    /// In the ODE y = (orbit, attitude, mass), dy/dt = (velocity+accel, q_dot+alpha, mass_rate):
    /// - orbit part: velocity in position slot, acceleration in velocity slot
    /// - attitude part: q_dot in quaternion slot, angular_acceleration in angular_velocity slot
    /// - mass_rate in mass slot (Phase A: always 0)
    pub fn from_derivative(
        velocity: Vector3<f64>,
        acceleration: Vector3<f64>,
        q_dot: Vector4<f64>,
        angular_acceleration: Vector3<f64>,
        mass_rate: f64,
    ) -> Self {
        Self {
            orbit: OrbitalState::from_derivative(velocity, acceleration),
            attitude: AttitudeState::from_derivative(q_dot, angular_acceleration),
            mass: mass_rate,
        }
    }

    /// Create from orbital state only (identity attitude, zero angular velocity).
    pub fn from_orbit(orbit: OrbitalState, mass: f64) -> Self {
        Self {
            orbit,
            attitude: AttitudeState::identity(),
            mass,
        }
    }
}

impl HasOrbit for SpacecraftState {
    type Frame = arika::frame::SimpleEci;

    fn orbit(&self) -> &OrbitalState {
        &self.orbit
    }
}

impl HasAttitude for SpacecraftState {
    fn attitude(&self) -> &AttitudeState {
        &self.attitude
    }
}

impl HasMass for SpacecraftState {
    fn mass(&self) -> f64 {
        self.mass
    }
}

// Delegate capability traits for AugmentedState<SpacecraftState>.
use crate::effector::AugmentedState;

impl HasOrbit for AugmentedState<SpacecraftState> {
    type Frame = arika::frame::SimpleEci;

    fn orbit(&self) -> &OrbitalState {
        &self.plant.orbit
    }
}

impl HasAttitude for AugmentedState<SpacecraftState> {
    fn attitude(&self) -> &AttitudeState {
        &self.plant.attitude
    }
}

impl HasMass for AugmentedState<SpacecraftState> {
    fn mass(&self) -> f64 {
        self.plant.mass
    }
}

impl OdeState for SpacecraftState {
    fn zero_like(&self) -> Self {
        Self {
            orbit: self.orbit.zero_like(),
            attitude: self.attitude.zero_like(),
            mass: 0.0,
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Self {
            orbit: self.orbit.axpy(scale, &other.orbit),
            attitude: self.attitude.axpy(scale, &other.attitude),
            mass: self.mass + scale * other.mass,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Self {
            orbit: self.orbit.scale(factor),
            attitude: self.attitude.scale(factor),
            mass: self.mass * factor,
        }
    }

    fn is_finite(&self) -> bool {
        self.orbit.is_finite() && self.attitude.is_finite() && self.mass.is_finite()
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        // Per-substate delegation: each substate computes its own RMS norm,
        // then take the max. This preserves the natural scaling of each subsystem
        // (position km, velocity km/s, quaternion ~1, angular_velocity rad/s)
        // without cross-contamination.
        let orbit_norm = self.orbit.error_norm(&y_next.orbit, &error.orbit, tol);
        let attitude_norm = self
            .attitude
            .error_norm(&y_next.attitude, &error.attitude, tol);

        // Mass: 1D scalar norm
        let sc = tol.atol + tol.rtol * self.mass.abs().max(y_next.mass.abs());
        let mass_norm = (error.mass / sc).abs();

        orbit_norm.max(attitude_norm).max(mass_norm)
    }

    fn project(&mut self, t: f64) {
        // Only attitude needs projection (quaternion normalization).
        // Orbit and mass require no projection.
        self.attitude.project(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    #[test]
    fn zero_like() {
        let state = sample_state();
        let zero = state.zero_like();
        assert_eq!(*zero.orbit.position(), Vector3::zeros());
        assert_eq!(*zero.orbit.velocity(), Vector3::zeros());
        assert_eq!(zero.attitude.quaternion, Vector4::zeros());
        assert_eq!(zero.attitude.angular_velocity, Vector3::zeros());
        assert_eq!(zero.mass, 0.0);
    }

    #[test]
    fn axpy_linear_combination() {
        let a = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(1.0, 2.0, 3.0), Vector3::new(4.0, 5.0, 6.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.0, 0.0),
            },
            mass: 100.0,
        };
        let b = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(10.0, 20.0, 30.0),
                Vector3::new(40.0, 50.0, 60.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(0.0, 1.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.0, 0.2, 0.0),
            },
            mass: 50.0,
        };
        let result = a.axpy(0.5, &b);
        assert_eq!(*result.orbit.position(), Vector3::new(6.0, 12.0, 18.0));
        assert_eq!(*result.orbit.velocity(), Vector3::new(24.0, 30.0, 36.0));
        assert_eq!(result.attitude.quaternion, Vector4::new(1.0, 0.5, 0.0, 0.0));
        assert_eq!(
            result.attitude.angular_velocity,
            Vector3::new(0.1, 0.1, 0.0)
        );
        assert!((result.mass - 125.0).abs() < 1e-15);
    }

    #[test]
    fn scale_zero_gives_zeros() {
        let state = sample_state();
        let scaled = state.scale(0.0);
        assert_eq!(*scaled.orbit.position(), Vector3::zeros());
        assert_eq!(*scaled.orbit.velocity(), Vector3::zeros());
        assert_eq!(scaled.attitude.quaternion, Vector4::zeros());
        assert_eq!(scaled.attitude.angular_velocity, Vector3::zeros());
        assert_eq!(scaled.mass, 0.0);
    }

    #[test]
    fn scale_one_identity() {
        let state = sample_state();
        let scaled = state.scale(1.0);
        assert_eq!(scaled, state);
    }

    #[test]
    fn is_finite_normal() {
        assert!(sample_state().is_finite());
    }

    #[test]
    fn is_finite_nan_orbit() {
        let mut state = sample_state();
        state.orbit.position_mut()[0] = f64::NAN;
        assert!(!state.is_finite());
    }

    #[test]
    fn is_finite_nan_attitude() {
        let mut state = sample_state();
        state.attitude.quaternion[0] = f64::NAN;
        assert!(!state.is_finite());
    }

    #[test]
    fn is_finite_nan_mass() {
        let mut state = sample_state();
        state.mass = f64::NAN;
        assert!(!state.is_finite());
    }

    #[test]
    fn is_finite_inf_mass() {
        let mut state = sample_state();
        state.mass = f64::INFINITY;
        assert!(!state.is_finite());
    }

    #[test]
    fn error_norm_orbit_dominant() {
        // Large orbit error, small attitude error → orbit norm dominates
        let y_n = sample_state();
        let y_next = sample_state();
        let error = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(1.0, 1.0, 1.0), // large (km-scale)
                Vector3::new(0.01, 0.01, 0.01),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1e-12, 1e-12, 1e-12, 1e-12),
                angular_velocity: Vector3::new(1e-12, 1e-12, 1e-12),
            },
            mass: 0.0,
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);

        // Orbit-only norm should be close to the composite norm
        let orbit_only = y_n.orbit.error_norm(&y_next.orbit, &error.orbit, &tol);
        assert!((norm - orbit_only).abs() < 1e-10);
        assert!(norm > 0.0);
    }

    #[test]
    fn error_norm_attitude_dominant() {
        // Small orbit error, large attitude error → attitude norm dominates
        let y_n = sample_state();
        let y_next = sample_state();
        let error = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(1e-12, 1e-12, 1e-12),
                Vector3::new(1e-12, 1e-12, 1e-12),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(0.1, 0.1, 0.1, 0.1),
                angular_velocity: Vector3::new(0.1, 0.1, 0.1),
            },
            mass: 0.0,
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);

        let attitude_only = y_n
            .attitude
            .error_norm(&y_next.attitude, &error.attitude, &tol);
        assert!((norm - attitude_only).abs() < 1e-10);
        assert!(norm > 0.0);
    }

    #[test]
    fn error_norm_mass_dominant() {
        // Zero orbit/attitude error, large mass error
        let y_n = sample_state();
        let y_next = sample_state();
        let error = SpacecraftState {
            orbit: OrbitalState::new(Vector3::zeros(), Vector3::zeros()),
            attitude: AttitudeState {
                quaternion: Vector4::zeros(),
                angular_velocity: Vector3::zeros(),
            },
            mass: 10.0, // large mass error
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);

        // Mass norm: |10.0| / (1e-10 + 1e-8 * 500.0) = 10.0 / 5.0000001e-6 ≈ 2e6
        let sc = tol.atol + tol.rtol * 500.0;
        let expected_mass_norm = (10.0 / sc).abs();
        assert!((norm - expected_mass_norm).abs() / expected_mass_norm < 1e-10);
    }

    #[test]
    fn project_normalizes_quaternion() {
        let mut state = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(2.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            mass: 500.0,
        };
        let orbit_before = state.orbit.clone();
        let mass_before = state.mass;

        state.project(0.0);

        // Quaternion should be normalized
        assert!((state.attitude.quaternion.magnitude() - 1.0).abs() < 1e-15);
        // Orbit and mass should be unchanged
        assert_eq!(state.orbit, orbit_before);
        assert_eq!(state.mass, mass_before);
        // Angular velocity should be unchanged
        assert_eq!(state.attitude.angular_velocity, Vector3::new(0.1, 0.2, 0.3));
    }

    #[test]
    fn from_derivative_and_euler_step() {
        // Test that from_derivative + axpy(dt, deriv) gives a correct Euler step
        let state = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.0, 0.0, 0.1),
            },
            mass: 500.0,
        };

        let dt = 1.0;
        let deriv = SpacecraftState::from_derivative(
            Vector3::new(0.0, 7.5, 0.0),       // velocity
            Vector3::new(-0.008, 0.0, 0.0),    // acceleration
            Vector4::new(0.0, 0.0, 0.0, 0.05), // q_dot
            Vector3::new(0.0, 0.0, 0.001),     // angular accel
            -0.1,                              // mass rate
        );

        let new_state = state.axpy(dt, &deriv);

        // Position: (7000, 0, 0) + 1.0 * (0, 7.5, 0) = (7000, 7.5, 0)
        assert!((new_state.orbit.position()[0] - 7000.0).abs() < 1e-10);
        assert!((new_state.orbit.position()[1] - 7.5).abs() < 1e-10);

        // Velocity: (0, 7.5, 0) + 1.0 * (-0.008, 0, 0) = (-0.008, 7.5, 0)
        assert!((new_state.orbit.velocity()[0] - (-0.008)).abs() < 1e-10);
        assert!((new_state.orbit.velocity()[1] - 7.5).abs() < 1e-10);

        // Quaternion: (1, 0, 0, 0) + 1.0 * (0, 0, 0, 0.05) = (1, 0, 0, 0.05)
        assert!((new_state.attitude.quaternion[0] - 1.0).abs() < 1e-10);
        assert!((new_state.attitude.quaternion[3] - 0.05).abs() < 1e-10);

        // Angular velocity: (0, 0, 0.1) + 1.0 * (0, 0, 0.001) = (0, 0, 0.101)
        assert!((new_state.attitude.angular_velocity[2] - 0.101).abs() < 1e-10);

        // Mass: 500 + 1.0 * (-0.1) = 499.9
        assert!((new_state.mass - 499.9).abs() < 1e-10);
    }
}
