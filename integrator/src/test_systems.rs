//! Shared dynamical systems for testing.

use nalgebra::{Vector3, vector};

use crate::{DynamicalSystem, State, StateDerivative};

/// Uniform motion: dx/dt = constant velocity, dv/dt = 0.
pub(crate) struct UniformMotion {
    pub constant_velocity: Vector3<f64>,
}

impl DynamicalSystem for UniformMotion {
    fn derivatives(&self, _t: f64, _state: &State) -> StateDerivative {
        StateDerivative {
            velocity: self.constant_velocity,
            acceleration: vector![0.0, 0.0, 0.0],
        }
    }
}

/// Constant acceleration: dx/dt = v, dv/dt = constant acceleration.
pub(crate) struct ConstantAcceleration {
    pub acceleration: Vector3<f64>,
}

impl DynamicalSystem for ConstantAcceleration {
    fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
        StateDerivative {
            velocity: state.velocity,
            acceleration: self.acceleration,
        }
    }
}

/// Simple harmonic oscillator: dv/dt = -x (ω = 1).
pub(crate) struct HarmonicOscillator;

impl DynamicalSystem for HarmonicOscillator {
    fn derivatives(&self, _t: f64, state: &State) -> StateDerivative {
        StateDerivative {
            velocity: state.velocity,
            acceleration: -state.position,
        }
    }
}
