use nalgebra::Vector3;

/// State of a dynamical system with position and velocity vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub position: Vector3<f64>,
    pub velocity: Vector3<f64>,
}

/// Time derivative of a state: velocity and acceleration vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct StateDerivative {
    pub velocity: Vector3<f64>,
    pub acceleration: Vector3<f64>,
}

/// A dynamical system that can compute state derivatives at a given time.
pub trait DynamicalSystem {
    fn derivatives(&self, t: f64, state: &State) -> StateDerivative;
}
