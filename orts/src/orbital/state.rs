use std::fmt;
use std::marker::PhantomData;

use arika::frame::{Eci, SimpleEci, Vec3};
use nalgebra::Vector3;
use utsuroi::{OdeState, State, Tolerances};

use crate::model::HasOrbit;

/// Orbital state: position and velocity in an inertial frame.
///
/// Parameterized by frame `F` (default `SimpleEci`). The internal
/// representation is a raw `State<3, 2>` — the frame is phantom.
pub struct OrbitalState<F: Eci = SimpleEci>(pub State<3, 2>, PhantomData<F>);

// Manual impls to avoid requiring F: Debug/Clone/PartialEq.
impl<F: Eci> fmt::Debug for OrbitalState<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OrbitalState").field(&self.0).finish()
    }
}
impl<F: Eci> Clone for OrbitalState<F> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}
impl<F: Eci> PartialEq for OrbitalState<F> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<F: Eci> OrbitalState<F> {
    /// Wrap an existing `State<3, 2>` in the given frame.
    pub fn from_state(state: State<3, 2>) -> Self {
        OrbitalState(state, PhantomData)
    }

    /// Create a new orbital state in a specific frame.
    pub fn new_in_frame(position: Vector3<f64>, velocity: Vector3<f64>) -> Self {
        Self::from_state(State::new(position, velocity))
    }

    /// Position vector (km), raw untyped.
    pub fn position(&self) -> &Vector3<f64> {
        self.0.y()
    }

    /// Position as a frame-typed vector (km).
    pub fn position_vec(&self) -> Vec3<F> {
        Vec3::from_raw(*self.0.y())
    }

    /// Velocity vector (km/s), raw untyped.
    pub fn velocity(&self) -> &Vector3<f64> {
        self.0.dy()
    }

    /// Velocity as a frame-typed vector (km/s).
    pub fn velocity_vec(&self) -> Vec3<F> {
        Vec3::from_raw(*self.0.dy())
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
    pub fn from_derivative_in_frame(velocity: Vector3<f64>, acceleration: Vector3<f64>) -> Self {
        OrbitalState(State::from_derivative(velocity, acceleration), PhantomData)
    }

    /// Apply an impulsive delta-V \[km/s\] in the inertial frame.
    pub fn apply_delta_v(&self, dv: Vector3<f64>) -> Self {
        Self::new_in_frame(*self.position(), *self.velocity() + dv)
    }
}

// Convenience constructors and aliases for SimpleEci (default frame).
impl OrbitalState<SimpleEci> {
    /// Create a new orbital state in the default SimpleEci frame.
    pub fn new(position: Vector3<f64>, velocity: Vector3<f64>) -> Self {
        Self::new_in_frame(position, velocity)
    }

    /// Create a derivative state in the default SimpleEci frame.
    pub fn from_derivative(velocity: Vector3<f64>, acceleration: Vector3<f64>) -> Self {
        Self::from_derivative_in_frame(velocity, acceleration)
    }

    /// Position as SimpleEci (km). Alias for `position_vec()`.
    pub fn position_eci(&self) -> Vec3<SimpleEci> {
        self.position_vec()
    }

    /// Velocity as SimpleEci vector (km/s). Alias for `velocity_vec()`.
    pub fn velocity_eci(&self) -> Vec3<SimpleEci> {
        self.velocity_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    #[test]
    fn apply_delta_v_preserves_position() {
        let state = OrbitalState::new(vector![6778.0, 0.0, 0.0], vector![0.0, 7.67, 0.0]);
        let dv = vector![0.1, 0.2, 0.0];
        let new_state = state.apply_delta_v(dv);
        assert_eq!(new_state.position(), state.position());
    }

    #[test]
    fn apply_delta_v_changes_velocity() {
        let state = OrbitalState::new(vector![6778.0, 0.0, 0.0], vector![0.0, 7.67, 0.0]);
        let dv = vector![0.1, 0.2, 0.0];
        let new_state = state.apply_delta_v(dv);
        let expected = vector![0.1, 7.87, 0.0];
        assert!(
            (new_state.velocity() - expected).magnitude() < 1e-14,
            "velocity should be original + dv"
        );
    }

    #[test]
    fn apply_zero_delta_v_is_identity() {
        let state = OrbitalState::new(vector![6778.0, 0.0, 0.0], vector![0.0, 7.67, 0.0]);
        let new_state = state.apply_delta_v(Vector3::zeros());
        assert_eq!(new_state.position(), state.position());
        assert_eq!(new_state.velocity(), state.velocity());
    }
}

impl HasOrbit for OrbitalState<SimpleEci> {
    type Frame = SimpleEci;

    fn orbit(&self) -> &OrbitalState {
        self
    }
}

impl<F: Eci> OdeState for OrbitalState<F> {
    fn zero_like(&self) -> Self {
        OrbitalState(self.0.zero_like(), PhantomData)
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        OrbitalState(self.0.axpy(scale, &other.0), PhantomData)
    }

    fn scale(&self, factor: f64) -> Self {
        OrbitalState(self.0.scale(factor), PhantomData)
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
