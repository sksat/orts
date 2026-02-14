use nalgebra::Vector3;
use orts_integrator::State;

/// A non-gravitational perturbation force (e.g., atmospheric drag, SRP, third-body gravity).
pub trait ForceModel: Send + Sync {
    /// Compute perturbation acceleration [km/s²].
    fn acceleration(&self, t: f64, state: &State) -> Vector3<f64>;
}
