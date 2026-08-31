//! Discrete-time controller trait for simulation with zero-order hold.

use arika::epoch::Epoch;
use arika::frame;

use crate::OrbitalState;
use crate::attitude::AttitudeState;

/// A discrete-time controller that runs at fixed sample intervals.
///
/// Controllers have internal state (`&mut self`) and produce commands
/// that are held constant between sample times (zero-order hold).
///
/// The type parameter `F` is the frame of the observed orbital state (default
/// `SimpleEci`). A controller that evaluates an environment model at the
/// spacecraft's position — the geomagnetic field, say — needs that frame to
/// resolve it, and implements this trait once per frame it supports.
///
/// `attitude` arrives as a bare [`AttitudeState`], so a controller that rotates
/// an inertial quantity into the body frame names `F` itself via
/// [`AttitudeState::rotation_tagged_as`]. Unlike [`Model`](crate::model::Model),
/// whose loads frame comes from the state's own
/// [`HasFrame::Frame`](crate::model::HasFrame::Frame), nothing here ties the two
/// together; the caller passes an attitude and an orbit that belong to the same
/// state.
pub trait DiscreteController<F: frame::Eci = frame::SimpleEci>: Send {
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
        orbit: &OrbitalState<F>,
        epoch: Option<&Epoch>,
    ) -> Self::Command;
}
