use nalgebra::Vector3;

use crate::Tolerances;

/// Algebraic operations required by generic ODE solvers.
///
/// Types implementing this trait can be used as state vectors in RK4,
/// Dormand-Prince, and other integration methods without the integrator
/// knowing anything about the domain-specific structure.
pub trait OdeState: Clone + Sized {
    /// Create a zero vector with the same shape.
    fn zero_like(&self) -> Self;

    /// Compute `self + scale * other` (AXPY operation).
    fn axpy(&self, scale: f64, other: &Self) -> Self;

    /// Compute `self * factor`.
    fn scale(&self, factor: f64) -> Self;

    /// Check whether all components are finite (not NaN or Inf).
    fn is_finite(&self) -> bool;

    /// Compute the RMS error norm for adaptive step-size control.
    ///
    /// Uses the mixed absolute/relative tolerance formula:
    ///   sc_i = atol + rtol * max(|y_n_i|, |y_{n+1}_i|)
    ///   err = sqrt(1/N * sum((delta_i / sc_i)^2))
    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64;

    /// Post-step projection (e.g., quaternion normalization). Default no-op.
    fn project(&mut self, _t: f64) {}
}

/// State of a dynamical system with position and velocity vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub position: Vector3<f64>,
    pub velocity: Vector3<f64>,
}

impl State {
    /// Create a State representing a derivative (velocity, acceleration).
    ///
    /// In the ODE formulation y = (position, velocity), the derivative
    /// dy/dt = (velocity, acceleration) has the same type:
    /// - `position` field holds velocity (d(position)/dt)
    /// - `velocity` field holds acceleration (d(velocity)/dt)
    pub fn from_derivative(velocity: Vector3<f64>, acceleration: Vector3<f64>) -> Self {
        State {
            position: velocity,
            velocity: acceleration,
        }
    }
}

impl OdeState for State {
    fn zero_like(&self) -> Self {
        State {
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        State {
            position: self.position + scale * other.position,
            velocity: self.velocity + scale * other.velocity,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        State {
            position: factor * self.position,
            velocity: factor * self.velocity,
        }
    }

    fn is_finite(&self) -> bool {
        self.position
            .iter()
            .chain(self.velocity.iter())
            .all(|v| v.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let mut sum_sq = 0.0;
        let n = 6; // 3 position + 3 velocity components

        for i in 0..3 {
            let sc = tol.atol + tol.rtol * self.position[i].abs().max(y_next.position[i].abs());
            let e = error.position[i] / sc;
            sum_sq += e * e;
        }
        for i in 0..3 {
            let sc = tol.atol + tol.rtol * self.velocity[i].abs().max(y_next.velocity[i].abs());
            let e = error.velocity[i] / sc;
            sum_sq += e * e;
        }

        (sum_sq / n as f64).sqrt()
    }
}

/// A dynamical system that can compute state derivatives at a given time.
///
/// The derivative has the same type as the state (standard ODE formulation:
/// for y = [q, q'], dy/dt = [q', q''] is also of type `State`).
pub trait DynamicalSystem {
    /// The state type for this system.
    type State: OdeState;

    fn derivatives(&self, t: f64, state: &Self::State) -> Self::State;
}
