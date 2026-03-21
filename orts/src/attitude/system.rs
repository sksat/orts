use kaname::epoch::Epoch;
use nalgebra::{Matrix3, Vector3};
use utsuroi::DynamicalSystem;

use crate::model::Model;

use super::state::AttitudeState;

/// Attitude dynamics system composing Euler's rotation equation with torque models.
///
/// Implements [`DynamicalSystem`] for use with any ODE integrator (RK4, DP45, etc.).
///
/// Equations of motion:
/// - Kinematics: dq/dt = 0.5 * q ⊗ (0, ω)
/// - Dynamics:   dω/dt = I⁻¹ (τ_total − ω × (I·ω))
pub struct AttitudeSystem {
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    models: Vec<Box<dyn Model<AttitudeState>>>,
    epoch_0: Option<Epoch>,
}

impl AttitudeSystem {
    /// Create a new attitude system with the given inertia tensor.
    ///
    /// The inertia tensor must be symmetric positive-definite.
    /// Units should be consistent with the torque models (e.g., kg·km² if using km).
    pub fn new(inertia: Matrix3<f64>) -> Self {
        let inertia_inv = inertia
            .try_inverse()
            .expect("Inertia tensor must be invertible");
        Self {
            inertia,
            inertia_inv,
            models: Vec::new(),
            epoch_0: None,
        }
    }

    /// Add a model (builder pattern).
    pub fn with_model(mut self, model: impl Model<AttitudeState> + 'static) -> Self {
        self.models.push(Box::new(model));
        self
    }

    /// Set the initial epoch for time-dependent torque models.
    pub fn with_epoch(mut self, epoch: Epoch) -> Self {
        self.epoch_0 = Some(epoch);
        self
    }

    /// Get the inertia tensor.
    pub fn inertia(&self) -> &Matrix3<f64> {
        &self.inertia
    }

    /// Get the names of all active models.
    pub fn model_names(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.name()).collect()
    }
}

impl DynamicalSystem for AttitudeSystem {
    type State = AttitudeState;

    fn derivatives(&self, t: f64, state: &AttitudeState) -> AttitudeState {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));

        // 1. Quaternion kinematics: dq/dt = 0.5 * q ⊗ (0, ω)
        let q_dot = state.q_dot();

        // 2. Total torque from all models
        let mut tau = Vector3::zeros();
        for m in &self.models {
            let loads = m.eval(t, state, epoch.as_ref());
            tau += loads.torque_body;
        }

        // 3. Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.angular_velocity;
        let alpha = self.inertia_inv * (tau - state.angular_velocity.cross(&iw));

        AttitudeState::from_derivative(q_dot, alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector4;

    fn symmetric_inertia(i: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(i, i, i))
    }

    #[test]
    fn torque_free_symmetric_body_zero_acceleration() {
        let system = AttitudeSystem::new(symmetric_inertia(10.0));
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.1, 0.2, 0.3),
        };
        let deriv = system.derivatives(0.0, &state);
        // For symmetric body: ω × (I·ω) = I * (ω × ω) = 0
        // So dω/dt = I⁻¹(0 - 0) = 0
        assert!(deriv.angular_velocity.magnitude() < 1e-15);
    }

    #[test]
    fn torque_free_zero_omega_no_change() {
        let system = AttitudeSystem::new(symmetric_inertia(10.0));
        let state = AttitudeState::identity();
        let deriv = system.derivatives(0.0, &state);
        assert!(deriv.quaternion.magnitude() < 1e-15);
        assert!(deriv.angular_velocity.magnitude() < 1e-15);
    }

    #[test]
    fn model_names_empty() {
        let system = AttitudeSystem::new(symmetric_inertia(1.0));
        assert!(system.model_names().is_empty());
    }

    #[test]
    fn builder_with_epoch() {
        let epoch = Epoch::from_jd(2451545.0);
        let system = AttitudeSystem::new(symmetric_inertia(1.0)).with_epoch(epoch);
        assert!(system.epoch_0.is_some());
    }
}
