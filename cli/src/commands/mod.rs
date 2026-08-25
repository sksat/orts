pub mod config;
pub mod convert;
pub mod replay;
pub mod run;
pub mod serve;

/// A command failure, carrying the process exit code it should produce.
///
/// Commands return this instead of calling `std::process::exit` themselves, so
/// `main` is the only place that decides how the process ends. That keeps the
/// failure paths unit-testable: the message can be asserted without spawning a
/// subprocess.
///
/// The code distinguishes a usage error (2) from a runtime failure (1); both
/// were previously spelled as direct `exit` calls at the point of detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdError {
    pub message: String,
    pub code: i32,
}

impl CmdError {
    /// A runtime failure: valid invocation, but the work could not complete
    /// (bad config values, I/O error, integration failure).
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    /// A usage error: the flags themselves are contradictory, so no amount of
    /// retrying the same invocation can succeed.
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CmdError {}

impl From<String> for CmdError {
    fn from(message: String) -> Self {
        Self::failure(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_and_usage_carry_distinct_codes() {
        assert_eq!(CmdError::failure("x").code, 1);
        assert_eq!(CmdError::usage("x").code, 2);
    }

    #[test]
    fn display_is_the_bare_message() {
        // `main` prefixes "Error: ", so the message must not repeat it.
        assert_eq!(
            CmdError::failure("dt must be positive").to_string(),
            "dt must be positive"
        );
    }

    #[test]
    fn string_converts_to_a_runtime_failure() {
        let e: CmdError = "bad config".to_string().into();
        assert_eq!(e, CmdError::failure("bad config"));
    }
}
