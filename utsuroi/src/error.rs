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

impl Tolerances {
    /// Reject tolerances that would make adaptive error control meaningless.
    ///
    /// `atol == 0 && rtol == 0` makes the scale factor `sc` zero, so a
    /// component whose error is also zero yields `0 / 0 == NaN`. A NaN error
    /// norm compares false against both the accept threshold and the
    /// `dt_min` floor, so the stepper would retry the same step forever.
    pub fn validate(&self) -> Result<(), IntegrationError> {
        let usable = self.atol.is_finite()
            && self.rtol.is_finite()
            && self.atol >= 0.0
            && self.rtol >= 0.0
            && (self.atol > 0.0 || self.rtol > 0.0);
        if usable {
            Ok(())
        } else {
            Err(IntegrationError::InvalidTolerances {
                atol: self.atol,
                rtol: self.rtol,
            })
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
    /// The requested step size is zero, negative, or non-finite, so the
    /// integration could never reach `t_end`.
    InvalidStepSize { dt: f64 },
    /// `t_end` lies before `t0`. Backward integration is not supported; the
    /// loops advance time monotonically upward.
    InvalidTimeSpan { t0: f64, t_end: f64 },
    /// Tolerances cannot drive adaptive error control (see
    /// [`Tolerances::validate`]).
    InvalidTolerances { atol: f64, rtol: f64 },
    /// The error norm evaluated to NaN (an indeterminate `0 / 0`), so the step
    /// could neither be accepted nor usefully retried. `+Inf` is not an error:
    /// it shrinks the step and recovers or ends in [`Self::StepSizeTooSmall`].
    IndeterminateErrorNorm { t: f64 },
    /// `t + dt` rounded back to `t` in f64, so time stopped advancing even
    /// though `dt` itself is positive.
    TimeStagnated { t: f64, dt: f64 },
}

impl IntegrationError {
    /// The simulation time the failure is attributed to, when the variant
    /// carries one.
    ///
    /// `None` for the pre-flight argument checks ([`Self::InvalidStepSize`],
    /// [`Self::InvalidTolerances`]), which reject before any step runs — the
    /// caller's start time is the meaningful timestamp there. Callers that
    /// need a `f64` unconditionally should use
    /// `err.time().unwrap_or(start_t)`, which also keeps them compiling as
    /// new variants are added.
    pub fn time(&self) -> Option<f64> {
        match self {
            Self::NonFiniteState { t }
            | Self::StepSizeTooSmall { t, .. }
            | Self::IndeterminateErrorNorm { t }
            | Self::TimeStagnated { t, .. } => Some(*t),
            Self::InvalidTimeSpan { t0, .. } => Some(*t0),
            Self::InvalidStepSize { .. } | Self::InvalidTolerances { .. } => None,
        }
    }
}

/// Reject step sizes that cannot advance time toward `t_end`.
///
/// `dt == 0` (or negative, or NaN) leaves `t` unchanged in the fixed-step
/// loops, which spin forever instead of terminating.
///
/// Public so downstream fixed-step loops built on [`Integrator::step`]
/// (rather than on `integrate`/`advance_to`, which check internally) can
/// apply the same precondition and report the same error.
///
/// [`Integrator::step`]: crate::Integrator::step
pub fn validate_step_size(dt: f64) -> Result<(), IntegrationError> {
    if dt.is_finite() && dt > 0.0 {
        Ok(())
    } else {
        Err(IntegrationError::InvalidStepSize { dt })
    }
}

/// Reject a time span the monotonically-increasing loops cannot cover.
pub(crate) fn validate_time_span(t0: f64, t_end: f64) -> Result<(), IntegrationError> {
    if t0.is_finite() && t_end.is_finite() && t_end >= t0 {
        Ok(())
    } else {
        Err(IntegrationError::InvalidTimeSpan { t0, t_end })
    }
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
            Self::InvalidStepSize { dt } => {
                write!(f, "step size must be positive and finite, got dt = {dt}")
            }
            Self::InvalidTimeSpan { t0, t_end } => {
                write!(
                    f,
                    "t_end ({t_end}) must be finite and not before t0 ({t0}); \
                     backward integration is not supported"
                )
            }
            Self::InvalidTolerances { atol, rtol } => {
                write!(
                    f,
                    "tolerances must be finite and non-negative with at least one \
                     positive, got atol = {atol}, rtol = {rtol}"
                )
            }
            Self::IndeterminateErrorNorm { t } => {
                write!(
                    f,
                    "error norm was NaN at t = {t} (check for atol = 0 with an \
                     exactly zero state component, which gives 0 / 0)"
                )
            }
            Self::TimeStagnated { t, dt } => {
                write!(
                    f,
                    "time stopped advancing at t = {t}: t + {dt} rounds back to t"
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

    #[test]
    fn default_tolerances_are_valid() {
        assert!(Tolerances::default().validate().is_ok());
    }

    #[test]
    fn one_positive_tolerance_is_enough() {
        assert!(
            Tolerances {
                atol: 0.0,
                rtol: 1e-8,
            }
            .validate()
            .is_ok()
        );
        assert!(
            Tolerances {
                atol: 1e-10,
                rtol: 0.0,
            }
            .validate()
            .is_ok()
        );
    }

    /// Both zero makes `sc == 0`, so a zero-error component yields `0 / 0`.
    #[test]
    fn both_zero_tolerances_rejected() {
        assert_eq!(
            Tolerances {
                atol: 0.0,
                rtol: 0.0,
            }
            .validate(),
            Err(IntegrationError::InvalidTolerances {
                atol: 0.0,
                rtol: 0.0
            })
        );
    }

    #[test]
    fn negative_and_non_finite_tolerances_rejected() {
        for (atol, rtol) in [
            (-1e-10, 1e-8),
            (1e-10, -1e-8),
            (f64::NAN, 1e-8),
            (1e-10, f64::INFINITY),
        ] {
            assert!(
                Tolerances { atol, rtol }.validate().is_err(),
                "atol = {atol}, rtol = {rtol} should be rejected"
            );
        }
    }

    #[test]
    fn step_size_validator() {
        assert!(validate_step_size(1e-3).is_ok());
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(validate_step_size(dt).is_err(), "dt = {dt}");
        }
    }

    #[test]
    fn time_span_validator() {
        assert!(validate_time_span(0.0, 1.0).is_ok());
        assert!(
            validate_time_span(1.0, 1.0).is_ok(),
            "empty span is a no-op"
        );
        assert!(validate_time_span(1.0, 0.0).is_err(), "backward span");
        assert!(validate_time_span(f64::NAN, 1.0).is_err());
        assert!(validate_time_span(0.0, f64::INFINITY).is_err());
    }

    #[test]
    fn new_variants_have_informative_display() {
        for (err, needle) in [
            (IntegrationError::InvalidStepSize { dt: 0.0 }, "positive"),
            (
                IntegrationError::InvalidTimeSpan {
                    t0: 1.0,
                    t_end: 0.0,
                },
                "backward",
            ),
            (
                IntegrationError::InvalidTolerances {
                    atol: 0.0,
                    rtol: 0.0,
                },
                "tolerances",
            ),
            (
                IntegrationError::IndeterminateErrorNorm { t: 3.0 },
                "error norm",
            ),
            (
                IntegrationError::TimeStagnated { t: 4.0, dt: 1e-9 },
                "stopped advancing",
            ),
        ] {
            let msg = err.to_string();
            assert!(msg.contains(needle), "{err:?} display was {msg:?}");
        }
    }
}
