//! Shared dynamical systems for testing.

use nalgebra::{Vector1, Vector2, Vector3};

use crate::{DynamicalSystem, State};

/// Uniform motion: dx/dt = constant velocity, dv/dt = 0.
pub(crate) struct UniformMotion {
    pub constant_velocity: Vector3<f64>,
}

impl DynamicalSystem for UniformMotion {
    type State = State<3, 2>;
    fn derivatives(&self, _t: f64, _state: &State<3, 2>) -> State<3, 2> {
        State::from_derivative(self.constant_velocity, Vector3::zeros())
    }
}

/// Constant acceleration: dx/dt = v, dv/dt = constant acceleration.
pub(crate) struct ConstantAcceleration {
    pub acceleration: Vector3<f64>,
}

impl DynamicalSystem for ConstantAcceleration {
    type State = State<3, 2>;
    fn derivatives(&self, _t: f64, state: &State<3, 2>) -> State<3, 2> {
        State::from_derivative(*state.dy(), self.acceleration)
    }
}

/// Simple harmonic oscillator: dv/dt = -x (ω = 1).
pub(crate) struct HarmonicOscillator;

impl DynamicalSystem for HarmonicOscillator {
    type State = State<3, 2>;
    fn derivatives(&self, _t: f64, state: &State<3, 2>) -> State<3, 2> {
        State::from_derivative(*state.dy(), -*state.y())
    }
}

// --- 1D test systems ---

/// 1D harmonic oscillator: dv/dt = -x (ω = 1).
pub(crate) struct HarmonicOscillator1D;

impl DynamicalSystem for HarmonicOscillator1D {
    type State = State<1, 2>;
    fn derivatives(&self, _t: f64, state: &State<1, 2>) -> State<1, 2> {
        State::from_derivative(*state.dy(), -*state.y())
    }
}

/// 1D exponential decay: dy/dt = -ky (1st-order ODE).
pub(crate) struct ExponentialDecay {
    pub k: f64,
}

impl DynamicalSystem for ExponentialDecay {
    type State = State<1, 1>;
    fn derivatives(&self, _t: f64, state: &Self::State) -> Self::State {
        State {
            components: [Vector1::new(-self.k * state.components[0][0])],
        }
    }
}

// --- 2D test systems ---

/// 2D harmonic oscillator: dv/dt = -x (ω = 1).
pub(crate) struct HarmonicOscillator2D;

impl DynamicalSystem for HarmonicOscillator2D {
    type State = State<2, 2>;
    fn derivatives(&self, _t: f64, state: &State<2, 2>) -> State<2, 2> {
        State::from_derivative(*state.dy(), -*state.y())
    }
}

/// Lotka-Volterra predator-prey: dx/dt = αx - βxy, dy/dt = δxy - γy.
/// 1st-order 2D ODE (State<2, 1>).
pub(crate) struct LotkaVolterra {
    pub alpha: f64,
    pub beta: f64,
    pub delta: f64,
    pub gamma: f64,
}

impl DynamicalSystem for LotkaVolterra {
    type State = State<2, 1>;
    fn derivatives(&self, _t: f64, state: &Self::State) -> Self::State {
        let x = state.components[0][0]; // prey
        let y = state.components[0][1]; // predator
        State {
            components: [Vector2::new(
                self.alpha * x - self.beta * x * y,
                self.delta * x * y - self.gamma * y,
            )],
        }
    }
}
