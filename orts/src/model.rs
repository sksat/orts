//! Capability-based model trait for dynamics systems.
//!
//! Models declare their state requirements via capability bounds (`HasAttitude`,
//! `HasOrbit`, `HasMass`), and each dynamics system accepts models compatible
//! with its concrete state type:
//!
//! - `OrbitalSystem`: `dyn Model<OrbitalState>` — accepts `S: HasOrbit` models
//! - `AttitudeSystem`: `dyn Model<AttitudeState>` — accepts `S: HasAttitude` models
//! - `SpacecraftDynamics`: `dyn Model<SpacecraftState>` — accepts all models
//!
//! A model implemented as `impl<S: HasAttitude> Model<S> for PdController`
//! automatically works with both `AttitudeSystem` and `SpacecraftDynamics`.

use kaname::epoch::Epoch;

use crate::OrbitalState;
use crate::attitude::AttitudeState;
use crate::spacecraft::ExternalLoads;

// ---------------------------------------------------------------------------
// Capability traits
// ---------------------------------------------------------------------------

/// State type that provides attitude information (quaternion + angular velocity).
pub trait HasAttitude {
    fn attitude(&self) -> &AttitudeState;
}

/// State type that provides orbital information (position + velocity).
pub trait HasOrbit {
    fn orbit(&self) -> &OrbitalState;
}

/// State type that provides translational mass.
///
/// Inertia tensor is held by the dynamics system, not by the state.
pub trait HasMass {
    fn mass(&self) -> f64;
}

// ---------------------------------------------------------------------------
// Unified Model trait
// ---------------------------------------------------------------------------

/// A physical model that evaluates external loads on a spacecraft.
///
/// Models declare their state requirements via generic bounds:
/// - `impl<S: HasAttitude> Model<S>` — attitude-only (e.g., PD controller)
/// - `impl<S: HasOrbit> Model<S>` — orbit-only (e.g., atmospheric drag)
/// - `impl<S: HasAttitude + HasOrbit + HasMass> Model<S>` — full state (e.g., thruster)
///
/// `eval` must be a pure function with no side effects.
/// All models are evaluated against the same immutable state snapshot;
/// evaluation order must not affect results.
pub trait Model<S>: Send + Sync {
    /// Human-readable name for this model (e.g., "drag", "gravity_gradient").
    fn name(&self) -> &str;

    /// Evaluate the model at the given state and return external loads.
    ///
    /// `epoch` is the absolute time corresponding to integration time `t`.
    /// It is `None` when no initial epoch was provided.
    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads;
}

// Blanket impl so Box<dyn Model<S>> also satisfies Model<S>.
// This allows with_model() to accept both concrete types and boxed trait objects.
impl<S> Model<S> for Box<dyn Model<S>> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        (**self).eval(t, state, epoch)
    }
}
