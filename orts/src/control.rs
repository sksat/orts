//! Discrete-time controller trait for simulation with zero-order hold.

use kaname::epoch::Epoch;

use crate::OrbitalState;
use crate::attitude::AttitudeState;

/// A discrete-time controller that runs at fixed sample intervals.
///
/// Controllers have internal state (`&mut self`) and produce commands
/// that are held constant between sample times (zero-order hold).
pub trait DiscreteController: Send {
    /// Command output type.
    type Command: Clone + Send;

    /// Sample period \[s\].
    fn sample_period(&self) -> f64;

    /// Initial command before first update.
    fn initial_command(&self) -> Self::Command;

    /// Compute new command from current observation.
    ///
    /// Internal state (previous values, integrators, etc.) is updated.
    fn update(
        &mut self,
        t: f64,
        attitude: &AttitudeState,
        orbit: &OrbitalState,
        epoch: Option<&Epoch>,
    ) -> Self::Command;
}
