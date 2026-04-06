//! Command -> actuator bridge.
//!
//! `ActuatorBundle` holds the most recent command emitted by a
//! controller backend, decomposed into per-actuator fields. The host
//! uses this struct to translate the plugin-layer `Command` enum into
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
//! Each `apply()` call updates exactly the actuator that matches the
//! command variant. Other actuators retain their last value (zero-order
//! hold). If a guest alternates between `MagneticMoment` and
//! `RwTorque` across ticks, both actuators stay armed with their most
//! recent command — the host applies them simultaneously during ODE
//! integration.

use nalgebra::Vector3;

use super::command::Command;
use super::error::PluginError;

/// Per-actuator applied command state.
#[derive(Debug, Clone, Default)]
pub struct ActuatorBundle {
    /// Magnetorquer commanded dipole moment, body frame \[A·m²\].
    /// `None` until `apply()` receives a `Command::MagneticMoment`.
    commanded_magnetic_moment: Option<Vector3<f64>>,

    /// Reaction wheel assembly commanded torque, body frame \[N·m\].
    /// `None` until `apply()` receives a `Command::RwTorque`.
    commanded_rw_torque: Option<Vector3<f64>>,
}

impl ActuatorBundle {
    /// Create an empty bundle with no actuators armed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a command, updating the corresponding actuator's state.
    ///
    /// Rejects non-finite commands before they can poison downstream
    /// actuator models (NaN guard, see DESIGN.md Phase P 落とし穴リスト).
    ///
    /// Only the actuator matching the command variant is updated; other
    /// actuators retain their previous value (zero-order hold).
    pub fn apply(&mut self, cmd: &Command) -> Result<(), PluginError> {
        if !cmd.is_finite() {
            return Err(PluginError::BadCommand(format!("{cmd:?}")));
        }
        match cmd {
            Command::MagneticMoment(m) => {
                self.commanded_magnetic_moment = Some(*m);
            }
            Command::RwTorque(t) => {
                self.commanded_rw_torque = Some(*t);
            }
        }
        Ok(())
    }

    /// Returns the currently-commanded magnetic moment, if any was ever
    /// applied. Defaults to `Vector3::zeros()` when no command has been
    /// observed yet (i.e. the magnetorquer produces zero torque).
    pub fn magnetic_moment(&self) -> Vector3<f64> {
        self.commanded_magnetic_moment
            .unwrap_or_else(Vector3::zeros)
    }

    /// Returns `true` if a `Command::MagneticMoment` has been applied
    /// at least once. Useful for oracle tests that want to distinguish
    /// "default zero" from "controller has spoken".
    pub fn has_magnetic_moment_command(&self) -> bool {
        self.commanded_magnetic_moment.is_some()
    }

    /// Returns the currently-commanded reaction wheel torque \[N·m\],
    /// body frame. Defaults to `Vector3::zeros()`.
    pub fn rw_torque(&self) -> Vector3<f64> {
        self.commanded_rw_torque.unwrap_or_else(Vector3::zeros)
    }

    /// Returns `true` if a `Command::RwTorque` has been applied
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
        assert_eq!(bundle.magnetic_moment(), Vector3::zeros());

        let m = Vector3::new(1.0, -2.0, 3.0);
        bundle.apply(&Command::MagneticMoment(m)).unwrap();
        assert!(bundle.has_magnetic_moment_command());
        assert_eq!(bundle.magnetic_moment(), m);
    }

    #[test]
    fn apply_stores_rw_torque() {
        let mut bundle = ActuatorBundle::new();
        assert!(!bundle.has_rw_torque_command());
        assert_eq!(bundle.rw_torque(), Vector3::zeros());

        let t = Vector3::new(0.01, -0.02, 0.03);
        bundle.apply(&Command::RwTorque(t)).unwrap();
        assert!(bundle.has_rw_torque_command());
        assert_eq!(bundle.rw_torque(), t);
    }

    #[test]
    fn apply_rejects_nan() {
        let mut bundle = ActuatorBundle::new();
        let bad = Command::MagneticMoment(Vector3::new(1.0, f64::NAN, 0.0));
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
        let bad = Command::RwTorque(Vector3::new(0.0, f64::INFINITY, 0.0));
        assert!(bundle.apply(&bad).is_err());
        assert!(!bundle.has_rw_torque_command());
    }

    #[test]
    fn multi_command_retains_both() {
        let mut bundle = ActuatorBundle::new();
        bundle
            .apply(&Command::MagneticMoment(Vector3::new(1.0, 0.0, 0.0)))
            .unwrap();
        bundle
            .apply(&Command::RwTorque(Vector3::new(0.0, 0.1, 0.0)))
            .unwrap();
        // Both actuators should retain their values.
        assert_eq!(bundle.magnetic_moment(), Vector3::new(1.0, 0.0, 0.0));
        assert_eq!(bundle.rw_torque(), Vector3::new(0.0, 0.1, 0.0));
    }
}
