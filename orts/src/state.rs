use kaname::Eci;
use nalgebra::Vector3;
use utsuroi::{OdeState, State, Tolerances};

use crate::model::HasOrbit;

/// Orbital state: position and velocity in 3D space.
///
/// A newtype around [`State<3, 2>`] providing domain-specific accessors
/// (`position`, `velocity`) for orbital mechanics.
#[derive(Debug, Clone, PartialEq)]
pub struct OrbitalState(pub State<3, 2>);

impl OrbitalState {
    /// Create a new orbital state from position and velocity vectors.
    pub fn new(position: Vector3<f64>, velocity: Vector3<f64>) -> Self {
        OrbitalState(State::new(position, velocity))
    }

    /// Position vector (km).
    pub fn position(&self) -> &Vector3<f64> {
        self.0.y()
    }

    /// Position as an ECI coordinate (km).
    pub fn position_eci(&self) -> Eci {
        Eci(*self.0.y())
    }

    /// Velocity vector (km/s).
    pub fn velocity(&self) -> &Vector3<f64> {
        self.0.dy()
    }

    /// Mutable access to the position vector.
    pub fn position_mut(&mut self) -> &mut Vector3<f64> {
        self.0.y_mut()
    }

    /// Mutable access to the velocity vector.
    pub fn velocity_mut(&mut self) -> &mut Vector3<f64> {
        self.0.dy_mut()
    }

    /// Create an OrbitalState representing a derivative (velocity, acceleration).
    ///
    /// In the ODE formulation y = (position, velocity), the derivative
    /// dy/dt = (velocity, acceleration):
    /// - `position()` returns velocity (d(position)/dt)
    /// - `velocity()` returns acceleration (d(velocity)/dt)
    pub fn from_derivative(velocity: Vector3<f64>, acceleration: Vector3<f64>) -> Self {
        OrbitalState(State::from_derivative(velocity, acceleration))
    }
}

impl HasOrbit for OrbitalState {
    fn orbit(&self) -> &OrbitalState {
        self
    }
}

impl OdeState for OrbitalState {
    fn zero_like(&self) -> Self {
        OrbitalState(self.0.zero_like())
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        OrbitalState(self.0.axpy(scale, &other.0))
    }

    fn scale(&self, factor: f64) -> Self {
        OrbitalState(self.0.scale(factor))
    }

    fn is_finite(&self) -> bool {
        self.0.is_finite()
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        self.0.error_norm(&y_next.0, &error.0, tol)
    }

    fn project(&mut self, t: f64) {
        self.0.project(t);
    }
}
