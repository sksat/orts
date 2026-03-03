pub mod state;
pub mod dynamics;
pub mod prop_group;
pub mod independent;

pub use state::GroupState;
pub use dynamics::IndependentGroupDynamics;
pub use prop_group::{PropGroup, PropGroupOutcome, SatId, SatelliteTermination, GroupSnapshot};
pub use independent::{IndependentGroup, IntegratorConfig};

use nalgebra::Vector3;

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
}
