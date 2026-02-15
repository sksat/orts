use nalgebra::Vector3;
use orts_kaname::epoch::Epoch;
use orts_integrator::State;

/// A non-gravitational perturbation force (e.g., atmospheric drag, SRP, third-body gravity).
pub trait ForceModel: Send + Sync {
    /// Compute perturbation acceleration [km/s²].
    ///
    /// `epoch` is the absolute time corresponding to integration time `t`,
    /// computed as `epoch_0 + t` by OrbitalSystem. It is `None` when no
    /// initial epoch was provided (e.g., for abstract test cases).
    fn acceleration(&self, t: f64, state: &State, epoch: Option<&Epoch>) -> Vector3<f64>;
}
