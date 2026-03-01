pub mod gravity_gradient;
pub mod state;
pub mod system;

pub use gravity_gradient::GravityGradientTorque;
pub use state::AttitudeState;
pub use system::AttitudeSystem;

use kaname::epoch::Epoch;
use nalgebra::Vector3;

/// A torque model for attitude dynamics (analogous to `ForceModel` for orbits).
pub trait TorqueModel: Send + Sync {
    /// Human-readable name for this torque model (e.g., "gravity_gradient").
    fn name(&self) -> &str;

    /// Compute torque in body frame [N·m] (or consistent units with inertia tensor).
    ///
    /// `epoch` is the absolute time corresponding to integration time `t`,
    /// computed as `epoch_0 + t` by `AttitudeSystem`. It is `None` when no
    /// initial epoch was provided.
    fn torque(&self, t: f64, state: &AttitudeState, epoch: Option<&Epoch>) -> Vector3<f64>;
}
