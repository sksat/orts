//! Command -> actuator bridge.
//!
//! `ActuatorBundle` holds the most recent command emitted by a
//! controller backend, decomposed into per-actuator fields. The host
//! uses this struct to translate the plugin-layer `Command` struct into
//! concrete physical actuator models (`MtqAssembly`, `RwAssembly`,
//! `DynamicThrottle` + `Thruster`) when assembling the ODE system
//! for the next zero-order-hold segment.
//!
//! The bundle does NOT own the actuator model instances themselves.
//! The caller rebuilds the model at each segment boundary using the
//! applied commanded state stored here. This keeps the bundle free of
//! generic parameters on environment-model types and lets different
//! backends / tests pick their own field model independently.
//!
//! ## Multi-command semantics
//!
//! Each `apply()` call updates the actuators for which the `Command`
//! has `Some` fields. Other actuators retain their last value
//! (zero-order hold). If a guest sets both `mtq_moments` and
//! `rw` in a single `Command`, both actuators are updated
//! simultaneously.

use super::command::{Command, RwCommand};
use super::error::PluginError;

/// Per-actuator applied command state.
#[derive(Debug, Clone, Default)]
pub struct ActuatorBundle {
    /// Per-MTQ commanded dipole moment \[A·m²\].
    /// `None` until `apply()` receives a `Command` with `mtq_moments`.
    commanded_mtq_moments: Option<Vec<f64>>,

    /// Per-wheel commanded RW command (speed or torque).
    /// `None` until `apply()` receives a `Command` with `rw`.
    commanded_rw: Option<RwCommand>,
}

impl ActuatorBundle {
    /// Create an empty bundle with no actuators armed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a command, updating the corresponding actuator(s)' state.
    ///
    /// Rejects non-finite commands before they can poison downstream
    /// actuator models (NaN guard, see DESIGN.md Phase P 落とし穴リスト).
    ///
    /// Only actuators for which the `Command` has `Some` fields are
    /// updated; other actuators retain their previous value (zero-order
    /// hold).
    pub fn apply(&mut self, cmd: &Command) -> Result<(), PluginError> {
        if !cmd.is_finite() {
            return Err(PluginError::BadCommand(format!("{cmd:?}")));
        }
        if let Some(m) = &cmd.mtq_moments {
            self.commanded_mtq_moments = Some(m.clone());
        }
        if let Some(rw) = &cmd.rw {
            self.commanded_rw = Some(rw.clone());
        }
        Ok(())
    }

    /// Returns the currently-commanded per-MTQ moments, if any was ever
    /// applied. Returns empty slice when no command has been observed yet.
    pub fn mtq_moments(&self) -> &[f64] {
        self.commanded_mtq_moments.as_deref().unwrap_or(&[])
    }

    /// Returns `true` if an MTQ command has been applied at least once.
    pub fn has_mtq_command(&self) -> bool {
        self.commanded_mtq_moments.is_some()
    }

    /// Returns the currently-commanded RW command.
    pub fn rw_command(&self) -> Option<&RwCommand> {
        self.commanded_rw.as_ref()
    }

    /// Returns `true` if an RW command has been applied at least once.
    pub fn has_rw_command(&self) -> bool {
        self.commanded_rw.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_stores_mtq_moments() {
        let mut bundle = ActuatorBundle::new();
        assert!(!bundle.has_mtq_command());
        assert!(bundle.mtq_moments().is_empty());

        bundle.apply(&Command::mtq(vec![1.0, -2.0, 3.0])).unwrap();
        assert!(bundle.has_mtq_command());
        assert_eq!(bundle.mtq_moments(), &[1.0, -2.0, 3.0]);
    }

    #[test]
    fn apply_stores_rw_torques() {
        let mut bundle = ActuatorBundle::new();
        assert!(!bundle.has_rw_command());

        bundle
            .apply(&Command::rw_torques(vec![0.01, -0.02, 0.03]))
            .unwrap();
        assert!(bundle.has_rw_command());
        assert_eq!(
            bundle.rw_command(),
            Some(&RwCommand::Torques(vec![0.01, -0.02, 0.03]))
        );
    }

    #[test]
    fn apply_stores_rw_speeds() {
        let mut bundle = ActuatorBundle::new();
        bundle
            .apply(&Command::rw_speeds(vec![10.0, -5.0, 0.0]))
            .unwrap();
        assert!(bundle.has_rw_command());
        assert_eq!(
            bundle.rw_command(),
            Some(&RwCommand::Speeds(vec![10.0, -5.0, 0.0]))
        );
    }

    #[test]
    fn apply_rejects_nan() {
        let mut bundle = ActuatorBundle::new();
        let bad = Command::mtq(vec![1.0, f64::NAN, 0.0]);
        let err = bundle.apply(&bad).unwrap_err();
        match err {
            PluginError::BadCommand(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!bundle.has_mtq_command());
    }

    #[test]
    fn apply_rw_rejects_nan() {
        let mut bundle = ActuatorBundle::new();
        let bad = Command::rw_torques(vec![0.0, f64::INFINITY, 0.0]);
        assert!(bundle.apply(&bad).is_err());
        assert!(!bundle.has_rw_command());
    }

    #[test]
    fn multi_command_retains_both() {
        let mut bundle = ActuatorBundle::new();
        bundle.apply(&Command::mtq(vec![1.0, 0.0, 0.0])).unwrap();
        bundle
            .apply(&Command::rw_torques(vec![0.0, 0.1, 0.0]))
            .unwrap();
        assert_eq!(bundle.mtq_moments(), &[1.0, 0.0, 0.0]);
        assert_eq!(
            bundle.rw_command(),
            Some(&RwCommand::Torques(vec![0.0, 0.1, 0.0]))
        );
    }

    #[test]
    fn single_command_with_both_fields() {
        let mut bundle = ActuatorBundle::new();
        let cmd = Command {
            mtq_moments: Some(vec![1.0, 0.0, 0.0]),
            rw: Some(RwCommand::Torques(vec![0.0, 0.1, 0.0])),
        };
        bundle.apply(&cmd).unwrap();
        assert_eq!(bundle.mtq_moments(), &[1.0, 0.0, 0.0]);
        assert_eq!(
            bundle.rw_command(),
            Some(&RwCommand::Torques(vec![0.0, 0.1, 0.0]))
        );
    }
}
