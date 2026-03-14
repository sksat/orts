mod drag;
mod srp;
mod third_body;

pub use drag::{AtmosphericDrag, DEFAULT_BALLISTIC_COEFF};
pub use kaname::constants::OMEGA_EARTH;
pub use srp::{SolarRadiationPressure, DEFAULT_CR, DEFAULT_AREA_TO_MASS};
pub use third_body::ThirdBodyGravity;

use nalgebra::Vector3;
use kaname::epoch::Epoch;
use orts_integrator::State;

/// A non-gravitational perturbation force (e.g., atmospheric drag, SRP, third-body gravity).
pub trait ForceModel: Send + Sync {
    /// Human-readable name for this force model (e.g., "drag", "srp", "third_body_sun").
    fn name(&self) -> &str;

    /// Compute perturbation acceleration [km/s²].
    ///
    /// `epoch` is the absolute time corresponding to integration time `t`,
    /// computed as `epoch_0 + t` by OrbitalSystem. It is `None` when no
    /// initial epoch was provided (e.g., for abstract test cases).
    fn acceleration(&self, t: f64, state: &State, epoch: Option<&Epoch>) -> Vector3<f64>;
}
