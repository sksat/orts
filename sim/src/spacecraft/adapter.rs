use kaname::epoch::Epoch;
use nalgebra::Vector3;
use crate::attitude::TorqueModel;
use orts_orbits::perturbations::ForceModel;

use super::{ExternalLoads, LoadModel, SpacecraftState};

/// Adapts a `ForceModel` (translational acceleration only) into a `LoadModel`.
///
/// The force acts at the center of mass, producing zero torque.
pub struct ForceModelAtCoM(pub Box<dyn ForceModel>);

impl LoadModel for ForceModelAtCoM {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn loads(&self, t: f64, state: &SpacecraftState, epoch: Option<&Epoch>) -> ExternalLoads {
        ExternalLoads {
            acceleration_inertial: self.0.acceleration(t, &state.orbit, epoch),
            torque_body: Vector3::zeros(),
            mass_rate: 0.0,
        }
    }
}

/// Adapts a `TorqueModel` (rotational torque only) into a `LoadModel`.
///
/// Only produces torque; translational acceleration is zero.
pub struct TorqueModelOnly(pub Box<dyn TorqueModel>);

impl LoadModel for TorqueModelOnly {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn loads(&self, t: f64, state: &SpacecraftState, epoch: Option<&Epoch>) -> ExternalLoads {
        ExternalLoads {
            acceleration_inertial: Vector3::zeros(),
            torque_body: self.0.torque(t, &state.attitude, epoch),
            mass_rate: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use orts_integrator::State;

    /// Mock ForceModel that returns a fixed acceleration and records the inputs.
    struct MockForce {
        accel: Vector3<f64>,
    }

    impl ForceModel for MockForce {
        fn name(&self) -> &str {
            "mock_force"
        }
        fn acceleration(&self, _t: f64, _state: &State, _epoch: Option<&Epoch>) -> Vector3<f64> {
            self.accel
        }
    }

    /// Mock TorqueModel that returns a fixed torque.
    struct MockTorque {
        torque_val: Vector3<f64>,
    }

    impl TorqueModel for MockTorque {
        fn name(&self) -> &str {
            "mock_torque"
        }
        fn torque(
            &self,
            _t: f64,
            _state: &AttitudeState,
            _epoch: Option<&Epoch>,
        ) -> Vector3<f64> {
            self.torque_val
        }
    }

    fn sample_spacecraft_state() -> SpacecraftState {
        SpacecraftState {
            orbit: State {
                position: Vector3::new(7000.0, 0.0, 0.0),
                velocity: Vector3::new(0.0, 7.5, 0.0),
            },
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    #[test]
    fn force_model_at_com_passthrough_acceleration() {
        let accel = Vector3::new(1e-6, 2e-6, 3e-6);
        let adapter = ForceModelAtCoM(Box::new(MockForce { accel }));
        let state = sample_spacecraft_state();
        let w = adapter.loads(10.0, &state, None);

        assert_eq!(w.acceleration_inertial, accel);
        assert_eq!(w.torque_body, Vector3::zeros());
    }

    #[test]
    fn force_model_at_com_name() {
        let adapter = ForceModelAtCoM(Box::new(MockForce {
            accel: Vector3::zeros(),
        }));
        assert_eq!(adapter.name(), "mock_force");
    }

    #[test]
    fn force_model_at_com_passes_epoch() {
        // Ensure epoch is forwarded (mock doesn't use it, but verify no panic)
        let adapter = ForceModelAtCoM(Box::new(MockForce {
            accel: Vector3::new(1.0, 0.0, 0.0),
        }));
        let state = sample_spacecraft_state();
        let epoch = Epoch::from_jd(2460000.5);
        let w = adapter.loads(0.0, &state, Some(&epoch));
        assert_eq!(w.acceleration_inertial, Vector3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn torque_model_only_passthrough_torque() {
        let torque_val = Vector3::new(0.01, 0.02, 0.03);
        let adapter = TorqueModelOnly(Box::new(MockTorque { torque_val }));
        let state = sample_spacecraft_state();
        let w = adapter.loads(10.0, &state, None);

        assert_eq!(w.torque_body, torque_val);
        assert_eq!(w.acceleration_inertial, Vector3::zeros());
    }

    #[test]
    fn torque_model_only_name() {
        let adapter = TorqueModelOnly(Box::new(MockTorque {
            torque_val: Vector3::zeros(),
        }));
        assert_eq!(adapter.name(), "mock_torque");
    }

    #[test]
    fn torque_model_only_passes_epoch() {
        let adapter = TorqueModelOnly(Box::new(MockTorque {
            torque_val: Vector3::new(0.0, 0.0, 1.0),
        }));
        let state = sample_spacecraft_state();
        let epoch = Epoch::from_jd(2460000.5);
        let w = adapter.loads(0.0, &state, Some(&epoch));
        assert_eq!(w.torque_body, Vector3::new(0.0, 0.0, 1.0));
    }
}
