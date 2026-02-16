use crate::State;

/// Tolerance configuration for adaptive step-size integrators.
#[derive(Debug, Clone)]
pub struct Tolerances {
    /// Absolute tolerance (applied uniformly to all state components).
    pub atol: f64,
    /// Relative tolerance (applied uniformly to all state components).
    pub rtol: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            atol: 1e-10,
            rtol: 1e-8,
        }
    }
}

/// Reason the integration was stopped by the integrator itself.
#[derive(Debug, Clone, PartialEq)]
pub enum IntegrationError {
    /// A NaN or Inf was detected in the state after a step.
    NonFiniteState { t: f64 },
    /// Step size became smaller than minimum threshold.
    StepSizeTooSmall { t: f64, dt: f64 },
}

/// Outcome of an integration with event detection.
#[derive(Debug, Clone)]
pub enum IntegrationOutcome<B> {
    /// Integration completed normally (reached t_end).
    Completed(State),
    /// Integration was terminated early by the event checker.
    Terminated { state: State, t: f64, reason: B },
    /// Integration was aborted due to a numerical error.
    Error(IntegrationError),
}
