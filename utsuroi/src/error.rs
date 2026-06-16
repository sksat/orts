use crate::OdeState;

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

impl core::fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFiniteState { t } => {
                write!(f, "non-finite state (NaN or Inf) detected at t = {t}")
            }
            Self::StepSizeTooSmall { t, dt } => {
                write!(
                    f,
                    "step size {dt} fell below the minimum threshold at t = {t}"
                )
            }
        }
    }
}

// `core::error::Error` (re-exported as `std::error::Error` since Rust 1.81) so the
// error participates in `?` chains. No inner cause, so `source()` stays the default
// `None`. Implemented by hand rather than via `thiserror` to keep utsuroi free of
// proc-macro dependencies (it pulls in only nalgebra + libm).
impl core::error::Error for IntegrationError {}

/// Outcome of an integration with event detection.
#[derive(Debug, Clone)]
pub enum IntegrationOutcome<Y: OdeState, B> {
    /// Integration completed normally (reached t_end).
    Completed(Y),
    /// Integration was terminated early by the event checker.
    Terminated { state: Y, t: f64, reason: B },
    /// Integration was aborted due to a numerical error.
    Error(IntegrationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time proof that the bound holds in no_std too: we assert against
    // `core::error::Error`, which is what `std::error::Error` re-exports since 1.81.
    fn assert_error<E: core::error::Error>() {}

    #[test]
    fn implements_error_trait() {
        assert_error::<IntegrationError>();
    }

    #[test]
    fn display_mentions_cause_and_time() {
        let msg = IntegrationError::NonFiniteState { t: 1.5 }.to_string();
        assert!(msg.contains("1.5"), "display should report t: {msg}");
        assert!(
            msg.to_lowercase().contains("non-finite") || msg.to_lowercase().contains("nan"),
            "display should explain the cause: {msg}"
        );

        let msg = IntegrationError::StepSizeTooSmall { t: 2.0, dt: 1e-15 }.to_string();
        assert!(msg.contains('2'), "display should report t: {msg}");
        assert!(
            msg.to_lowercase().contains("step"),
            "display should explain the cause: {msg}"
        );
    }

    #[test]
    fn no_inner_source() {
        use std::error::Error;
        assert!(
            IntegrationError::NonFiniteState { t: 0.0 }
                .source()
                .is_none()
        );
    }

    #[test]
    fn propagates_with_question_mark() {
        fn fallible() -> Result<(), Box<dyn std::error::Error>> {
            Err(IntegrationError::StepSizeTooSmall { t: 0.0, dt: 0.0 })?;
            Ok(())
        }
        assert!(fallible().is_err());
    }
}
