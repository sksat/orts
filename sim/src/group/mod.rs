pub mod state;
pub mod dynamics;
pub mod prop_group;
pub mod independent;
pub mod coupled;

pub use state::GroupState;
pub use dynamics::IndependentGroupDynamics;
pub use prop_group::{PropGroup, PropGroupOutcome, SatId, SatelliteTermination, GroupSnapshot};
pub use independent::{IndependentGroup, IntegratorConfig, SatelliteParts};
pub use coupled::{
    CoupledGroup, CoupledGroupDynamics, CoupledGroupParts, InterSatelliteForce, InteractionPair,
    MutualGravity, PairContext,
};

use nalgebra::{Vector3, Vector4};
use orts_integrator::OdeState;

/// Trait for types that expose a 3D position vector.
///
/// Used for inter-group distance queries (e.g., scheduler regime transitions).
pub trait HasPosition {
    fn position(&self) -> Vector3<f64>;
}

impl HasPosition for orts_integrator::State {
    fn position(&self) -> Vector3<f64> {
        self.position
    }
}

impl HasPosition for crate::SpacecraftState {
    fn position(&self) -> Vector3<f64> {
        self.orbit.position
    }
}

/// Create a derivative-form state containing only translational acceleration.
///
/// In the ODE formulation, the derivative state has the same type as the state.
/// `from_acceleration(accel)` produces a delta where only the velocity-slot
/// (which holds acceleration in derivative form) is non-zero.
/// Used to accumulate inter-satellite force contributions:
/// `derivs[i] = derivs[i].axpy(1.0, &S::from_acceleration(a))`
pub trait FromAcceleration: OdeState {
    fn from_acceleration(accel: Vector3<f64>) -> Self;
}

impl FromAcceleration for orts_integrator::State {
    fn from_acceleration(accel: Vector3<f64>) -> Self {
        orts_integrator::State::from_derivative(Vector3::zeros(), accel)
    }
}

impl FromAcceleration for crate::SpacecraftState {
    fn from_acceleration(accel: Vector3<f64>) -> Self {
        crate::SpacecraftState::from_derivative(
            Vector3::zeros(),
            accel,
            Vector4::zeros(),
            Vector3::zeros(),
            0.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector4;
    use orts_attitude::AttitudeState;
    use orts_integrator::State;

    #[test]
    fn has_position_state() {
        let state = State {
            position: Vector3::new(7000.0, 100.0, 50.0),
            velocity: Vector3::new(0.0, 7.5, 0.0),
        };
        assert_eq!(state.position(), Vector3::new(7000.0, 100.0, 50.0));
    }

    #[test]
    fn has_position_spacecraft_state() {
        let sc = crate::SpacecraftState {
            orbit: State {
                position: Vector3::new(7200.0, 0.0, 0.0),
                velocity: Vector3::new(0.0, 7.3, 0.0),
            },
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        };
        assert_eq!(sc.position(), Vector3::new(7200.0, 0.0, 0.0));
    }

    #[test]
    fn from_acceleration_state() {
        let accel = Vector3::new(1.0, -2.0, 3.0);
        let delta = State::from_acceleration(accel);
        // In derivative form: position=velocity(=0), velocity=acceleration
        assert_eq!(delta.position, Vector3::zeros());
        assert_eq!(delta.velocity, accel);
    }

    #[test]
    fn from_acceleration_spacecraft_state() {
        let accel = Vector3::new(0.001, -0.002, 0.003);
        let delta = crate::SpacecraftState::from_acceleration(accel);
        // Only orbit acceleration is set
        assert_eq!(delta.orbit.position, Vector3::zeros());
        assert_eq!(delta.orbit.velocity, accel);
        // Attitude and mass are zero
        assert_eq!(delta.attitude.quaternion, Vector4::zeros());
        assert_eq!(delta.attitude.angular_velocity, Vector3::zeros());
        assert_eq!(delta.mass, 0.0);
    }

    #[test]
    fn from_acceleration_axpy_adds_accel_to_derivative() {
        // Simulate adding inter-satellite acceleration to an existing derivative
        let existing_deriv = State::from_derivative(
            Vector3::new(0.0, 7.5, 0.0),    // velocity
            Vector3::new(-0.008, 0.0, 0.0),  // gravity acceleration
        );
        let inter_sat_accel = Vector3::new(0.0, 0.0, 0.001);
        let delta = State::from_acceleration(inter_sat_accel);
        let combined = existing_deriv.axpy(1.0, &delta);

        // Velocity (position slot) unchanged
        assert_eq!(combined.position, Vector3::new(0.0, 7.5, 0.0));
        // Acceleration (velocity slot) = gravity + inter-satellite
        assert!((combined.velocity[0] - (-0.008)).abs() < 1e-15);
        assert!((combined.velocity[2] - 0.001).abs() < 1e-15);
    }
}
