use nalgebra::SVector;

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

/// N-th order ODE state: `ORDER` vectors of `DIM` components each.
///
/// For a 2nd-order ODE in 3D (e.g., orbital mechanics), `State<3, 2>` holds
/// `[position, velocity]`. For a 1D oscillator, `State<1, 2>` holds `[x, v]`.
/// For a 1st-order ODE, `State<DIM, 1>` holds just `[y]`.
#[derive(Debug, Clone, PartialEq)]
pub struct State<const DIM: usize, const ORDER: usize> {
    pub components: [SVector<f64, DIM>; ORDER],
}

impl<const DIM: usize, const ORDER: usize> OdeState for State<DIM, ORDER> {
    fn zero_like(&self) -> Self {
        State {
            components: [SVector::zeros(); ORDER],
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        let mut components = self.components;
        for (c, (s, o)) in components
            .iter_mut()
            .zip(self.components.iter().zip(other.components.iter()))
        {
            *c = s + scale * o;
        }
        State { components }
    }

    fn scale(&self, factor: f64) -> Self {
        let mut components = self.components;
        for (c, s) in components.iter_mut().zip(self.components.iter()) {
            *c = factor * s;
        }
        State { components }
    }

    fn is_finite(&self) -> bool {
        self.components
            .iter()
            .flat_map(|c| c.iter())
            .all(|v| v.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let mut sum_sq = 0.0;
        let n = DIM * ORDER;

        for i in 0..ORDER {
            for j in 0..DIM {
                let sc = tol.atol
                    + tol.rtol
                        * self.components[i][j]
                            .abs()
                            .max(y_next.components[i][j].abs());
                let e = error.components[i][j] / sc;
                sum_sq += e * e;
            }
        }

        (sum_sq / n as f64).sqrt()
    }
}

/// Convenience methods for 2nd-order ODE states (e.g., position + velocity).
impl<const DIM: usize> State<DIM, 2> {
    /// Create a new 2nd-order state from `y` (0th derivative) and `dy` (1st derivative).
    pub fn new(y: SVector<f64, DIM>, dy: SVector<f64, DIM>) -> Self {
        State {
            components: [y, dy],
        }
    }

    /// The 0th-order component (position-like).
    pub fn y(&self) -> &SVector<f64, DIM> {
        &self.components[0]
    }

    /// The 1st-order component (velocity-like).
    pub fn dy(&self) -> &SVector<f64, DIM> {
        &self.components[1]
    }

    /// Mutable access to the 0th-order component.
    pub fn y_mut(&mut self) -> &mut SVector<f64, DIM> {
        &mut self.components[0]
    }

    /// Mutable access to the 1st-order component.
    pub fn dy_mut(&mut self) -> &mut SVector<f64, DIM> {
        &mut self.components[1]
    }

    /// Create a State representing a derivative (dy, ddy).
    ///
    /// In the ODE formulation y = (q, q'), the derivative
    /// dy/dt = (q', q'') has the same type:
    /// - `components[0]` holds dy (1st derivative)
    /// - `components[1]` holds ddy (2nd derivative)
    pub fn from_derivative(dy: SVector<f64, DIM>, ddy: SVector<f64, DIM>) -> Self {
        State {
            components: [dy, ddy],
        }
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
