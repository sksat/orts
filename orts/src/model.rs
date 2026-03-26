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

use std::ops::{Add, AddAssign};

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use crate::OrbitalState;
use crate::attitude::AttitudeState;

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
// ExternalLoads
// ---------------------------------------------------------------------------

/// Acceleration (inertial frame) and torque (body frame) pair.
///
/// Each field is in the frame used by its respective equation of motion:
/// - acceleration: inertial frame [km/s²] (for translational EOM)
/// - torque: body frame [N·m] (for rotational EOM)
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLoads {
    /// Translational acceleration in inertial frame [km/s²].
    pub acceleration_inertial: Vector3<f64>,
    /// Torque in body frame [N·m].
    pub torque_body: Vector3<f64>,
    /// Mass rate [kg/s] (negative for depletion, e.g. propellant consumption).
    pub mass_rate: f64,
}

impl ExternalLoads {
    pub fn zeros() -> Self {
        Self {
            acceleration_inertial: Vector3::zeros(),
            torque_body: Vector3::zeros(),
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only torque (body frame) [N·m].
    pub fn torque(t: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: Vector3::zeros(),
            torque_body: t,
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only translational acceleration (inertial frame) [km/s²].
    pub fn acceleration(a: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: a,
            torque_body: Vector3::zeros(),
            mass_rate: 0.0,
        }
    }
}

impl Add for ExternalLoads {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            acceleration_inertial: self.acceleration_inertial + rhs.acceleration_inertial,
            torque_body: self.torque_body + rhs.torque_body,
            mass_rate: self.mass_rate + rhs.mass_rate,
        }
    }
}

impl AddAssign for ExternalLoads {
    fn add_assign(&mut self, rhs: Self) {
        self.acceleration_inertial += rhs.acceleration_inertial;
        self.torque_body += rhs.torque_body;
        self.mass_rate += rhs.mass_rate;
    }
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

#[cfg(test)]
mod external_loads_tests {
    use super::*;

    #[test]
    fn zeros() {
        let w = ExternalLoads::zeros();
        assert_eq!(w.acceleration_inertial, Vector3::zeros());
        assert_eq!(w.torque_body, Vector3::zeros());
    }

    #[test]
    fn add_component_wise() {
        let a = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
            mass_rate: -0.5,
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
            mass_rate: -0.3,
        };
        let sum = a + b;
        assert_eq!(sum.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(sum.torque_body, Vector3::new(1.1, 2.2, 3.3));
        assert!((sum.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_assign_component_wise() {
        let mut a = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
            mass_rate: -0.5,
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
            mass_rate: -0.3,
        };
        a += b;
        assert_eq!(a.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(a.torque_body, Vector3::new(1.1, 2.2, 3.3));
        assert!((a.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_zeros_identity() {
        let w = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
            mass_rate: -0.1,
        };
        let sum = w.clone() + ExternalLoads::zeros();
        assert_eq!(sum, w);
    }

    #[test]
    fn zeros_has_zero_mass_rate() {
        assert_eq!(ExternalLoads::zeros().mass_rate, 0.0);
    }
}
