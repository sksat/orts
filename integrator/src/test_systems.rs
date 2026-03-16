//! Shared dynamical systems for testing.

use nalgebra::Vector3;

use crate::{DynamicalSystem, State};

/// Uniform motion: dx/dt = constant velocity, dv/dt = 0.
pub(crate) struct UniformMotion {
    pub constant_velocity: Vector3<f64>,
}

impl DynamicalSystem for UniformMotion {
    type State = State<2>;
    fn derivatives(&self, _t: f64, _state: &State<2>) -> State<2> {
        State::from_derivative(self.constant_velocity, Vector3::zeros())
    }
}

/// Constant acceleration: dx/dt = v, dv/dt = constant acceleration.
pub(crate) struct ConstantAcceleration {
    pub acceleration: Vector3<f64>,
}

impl DynamicalSystem for ConstantAcceleration {
    type State = State<2>;
    fn derivatives(&self, _t: f64, state: &State<2>) -> State<2> {
        State::from_derivative(*state.dy(), self.acceleration)
    }
}

/// Simple harmonic oscillator: dv/dt = -x (ω = 1).
pub(crate) struct HarmonicOscillator;

impl DynamicalSystem for HarmonicOscillator {
    type State = State<2>;
    fn derivatives(&self, _t: f64, state: &State<2>) -> State<2> {
        State::from_derivative(*state.dy(), -*state.y())
    }
}
