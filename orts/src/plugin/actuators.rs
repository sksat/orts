//! Command -> actuator bridge.
//!
//! `ActuatorBundle` holds the most recent command emitted by a
//! controller backend, decomposed into per-actuator fields. The host
//! uses this struct to translate the plugin-layer `Command` struct into
//! concrete physical actuator models (`CommandedMagnetorquer`,
//! `DynamicThrottle` + `Thruster`, `ReactionWheelAssembly`) when
//! assembling the ODE system for the next zero-order-hold segment.
//!
//! The bundle does NOT own the actuator model instances themselves
//! (e.g. a magnetorquer with its own `MagneticFieldModel`). The caller
//! rebuilds the model at each segment boundary using the applied
//! commanded state stored here. This keeps the bundle free of generic
//! parameters on environment-model types and lets different backends /
//! tests pick their own field model independently.
//!
//! ## Multi-command semantics
//!
//! Each `apply()` call updates the actuators for which the `Command`
//! has `Some` fields. Other actuators retain their last value
//! (zero-order hold). If a guest sets both `magnetic_moment` and
//! `rw_torque` in a single `Command`, both actuators are updated
//! simultaneously.

use arika::frame::{Body, Vec3};

use super::command::Command;
use super::error::PluginError;

/// Per-actuator applied command state.
#[derive(Debug, Clone, Default)]
pub struct ActuatorBundle {
    /// Magnetorquer commanded dipole moment, body frame \[A·m²\].
    /// `None` until `apply()` receives a `Command` with `magnetic_moment`.
    commanded_magnetic_moment: Option<Vec3<Body>>,

    /// Reaction wheel assembly commanded torque, body frame \[N·m\].
    /// `None` until `apply()` receives a `Command` with `rw_torque`.
    commanded_rw_torque: Option<Vec3<Body>>,
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
        if let Some(m) = cmd.magnetic_moment {
            self.commanded_magnetic_moment = Some(m);
        }
        if let Some(t) = cmd.rw_torque {
            self.commanded_rw_torque = Some(t);
        }
        Ok(())
    }

    /// Returns the currently-commanded magnetic moment, if any was ever
    /// applied. Defaults to `Vec3::zeros()` when no command has been
    /// observed yet (i.e. the magnetorquer produces zero torque).
    pub fn magnetic_moment(&self) -> Vec3<Body> {
        self.commanded_magnetic_moment.unwrap_or_else(Vec3::zeros)
    }

    /// Returns `true` if a magnetic moment command has been applied
    /// at least once. Useful for oracle tests that want to distinguish
    /// "default zero" from "controller has spoken".
    pub fn has_magnetic_moment_command(&self) -> bool {
        self.commanded_magnetic_moment.is_some()
    }

    /// Returns the currently-commanded reaction wheel torque \[N·m\],
    /// body frame. Defaults to `Vec3::zeros()`.
    pub fn rw_torque(&self) -> Vec3<Body> {
        self.commanded_rw_torque.unwrap_or_else(Vec3::zeros)
    }

    /// Returns `true` if a reaction wheel torque command has been applied
    /// at least once.
    pub fn has_rw_torque_command(&self) -> bool {
        self.commanded_rw_torque.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_stores_magnetic_moment() {
        let mut bundle = ActuatorBundle::new();
        assert!(!bundle.has_magnetic_moment_command());
        assert_eq!(bundle.magnetic_moment(), Vec3::zeros());

        let m = Vec3::new(1.0, -2.0, 3.0);
        bundle.apply(&Command::magnetic_moment(m)).unwrap();
        assert!(bundle.has_magnetic_moment_command());
        assert_eq!(bundle.magnetic_moment(), m);
    }

    #[test]
    fn apply_stores_rw_torque() {
        let mut bundle = ActuatorBundle::new();
        assert!(!bundle.has_rw_torque_command());
        assert_eq!(bundle.rw_torque(), Vec3::zeros());

        let t = Vec3::new(0.01, -0.02, 0.03);
        bundle.apply(&Command::rw_torque(t)).unwrap();
        assert!(bundle.has_rw_torque_command());
        assert_eq!(bundle.rw_torque(), t);
    }

    #[test]
    fn apply_rejects_nan() {
        let mut bundle = ActuatorBundle::new();
        let bad = Command::magnetic_moment(Vec3::new(1.0, f64::NAN, 0.0));
        let err = bundle.apply(&bad).unwrap_err();
        match err {
            PluginError::BadCommand(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
        // State must remain untouched after a rejected command.
        assert!(!bundle.has_magnetic_moment_command());
    }

    #[test]
    fn apply_rw_rejects_nan() {
        let mut bundle = ActuatorBundle::new();
        let bad = Command::rw_torque(Vec3::new(0.0, f64::INFINITY, 0.0));
        assert!(bundle.apply(&bad).is_err());
        assert!(!bundle.has_rw_torque_command());
    }

    #[test]
    fn multi_command_retains_both() {
        let mut bundle = ActuatorBundle::new();
        bundle
            .apply(&Command::magnetic_moment(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        bundle
            .apply(&Command::rw_torque(Vec3::new(0.0, 0.1, 0.0)))
            .unwrap();
        // Both actuators should retain their values.
        assert_eq!(bundle.magnetic_moment(), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(bundle.rw_torque(), Vec3::new(0.0, 0.1, 0.0));
    }

    #[test]
    fn single_command_with_both_fields() {
        let mut bundle = ActuatorBundle::new();
        let cmd = Command {
            magnetic_moment: Some(Vec3::new(1.0, 0.0, 0.0)),
            rw_torque: Some(Vec3::new(0.0, 0.1, 0.0)),
        };
        bundle.apply(&cmd).unwrap();
        assert_eq!(bundle.magnetic_moment(), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(bundle.rw_torque(), Vec3::new(0.0, 0.1, 0.0));
    }
}
