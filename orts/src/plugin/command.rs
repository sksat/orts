//! Logical commands returned by a controller backend.
//!
//! A `Command` is the plugin-layer output. Guests do NOT return raw
//! `ExternalLoads` (acceleration / torque / mass rate); instead they
//! return high-level actuator commands that the host translates into
//! physical loads via `ActuatorBundle`.
//!
//! The field set grows incrementally with each phase:
//! - P0.5: `magnetic_moment` (B-dot detumbling)
//! - P1: `rw_torque` (reaction wheel torque command)
//! - P4: thrust throttle / impulsive delta-v
//! - P5: composite commands for coupled attitude + thrust guest
//!
//! See DESIGN.md Phase P, D2 ("Command enum は最小 variant から始めて
//! phase ごとに拡張する").

use arika::frame::{Body, Vec3};

/// Logical command emitted by a controller backend.
///
/// Each field corresponds to one actuator channel. `Some` means the
/// controller is issuing a command for that actuator; `None` means the
/// controller has nothing to say about it this tick (the actuator
/// retains its previous value via zero-order hold).
///
/// "No command at all" (i.e. the controller has nothing to do this
/// tick) is represented by `Option<Command>` being `None` at the call
/// site, not by an all-`None` `Command` struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// Commanded magnetic dipole moment, expressed in the spacecraft body
    /// frame \[A·m²\]. Consumed by
    /// [`crate::attitude::CommandedMagnetorquer`].
    pub magnetic_moment: Option<Vec3<Body>>,

    /// Commanded torque on the spacecraft body from the reaction wheel
    /// assembly \[N·m\], expressed in the body frame. Consumed by
    /// [`crate::spacecraft::ReactionWheelAssembly`] via its
    /// `commanded_torque` field.
    ///
    /// The host-side `ReactionWheelAssembly` performs axis-projection
    /// torque allocation internally (projects this 3D vector onto each
    /// wheel's spin axis). For orthogonal wheel arrangements this is
    /// exact; non-orthogonal layouts may need a separate torque
    /// allocation layer in a future phase.
    pub rw_torque: Option<Vec3<Body>>,
}

impl Command {
    /// Create a command that only sets the magnetic dipole moment.
    pub fn magnetic_moment(m: Vec3<Body>) -> Self {
        Self {
            magnetic_moment: Some(m),
            rw_torque: None,
        }
    }

    /// Create a command that only sets the reaction wheel torque.
    pub fn rw_torque(t: Vec3<Body>) -> Self {
        Self {
            magnetic_moment: None,
            rw_torque: Some(t),
        }
    }

    /// Returns `true` if every numeric component in the command is
    /// finite (not NaN / +-Inf).
    ///
    /// The host MUST call this before handing a guest-produced command
    /// to the actuator layer; a NaN leak will propagate into the 14-D
    /// spacecraft state through `axpy` on the next ODE step and destroy
    /// the whole trajectory.
    pub fn is_finite(&self) -> bool {
        let mm_ok = self.magnetic_moment.is_none_or(|m| m.is_finite());
        let rw_ok = self.rw_torque.is_none_or(|t| t.is_finite());
        mm_ok && rw_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnetic_moment_finite_detects_nan() {
        let good = Command::magnetic_moment(Vec3::new(1.0, -2.0, 0.0));
        assert!(good.is_finite());

        let nan = Command::magnetic_moment(Vec3::new(1.0, f64::NAN, 0.0));
        assert!(!nan.is_finite());

        let inf = Command::magnetic_moment(Vec3::new(f64::INFINITY, 0.0, 0.0));
        assert!(!inf.is_finite());
    }

    #[test]
    fn rw_torque_finite_detects_nan() {
        let good = Command::rw_torque(Vec3::new(0.01, -0.02, 0.0));
        assert!(good.is_finite());

        let nan = Command::rw_torque(Vec3::new(f64::NAN, 0.0, 0.0));
        assert!(!nan.is_finite());
    }

    #[test]
    fn field_access() {
        let mm = Command::magnetic_moment(Vec3::new(1.0, 2.0, 3.0));
        assert!(mm.magnetic_moment.is_some());
        assert!(mm.rw_torque.is_none());

        let rw = Command::rw_torque(Vec3::new(0.1, 0.2, 0.3));
        assert!(rw.magnetic_moment.is_none());
        assert!(rw.rw_torque.is_some());
    }

    #[test]
    fn both_fields_set() {
        let cmd = Command {
            magnetic_moment: Some(Vec3::new(1.0, 0.0, 0.0)),
            rw_torque: Some(Vec3::new(0.0, 0.1, 0.0)),
        };
        assert!(cmd.is_finite());
        assert!(cmd.magnetic_moment.is_some());
        assert!(cmd.rw_torque.is_some());
    }

    #[test]
    fn both_fields_nan_in_one() {
        let cmd = Command {
            magnetic_moment: Some(Vec3::new(f64::NAN, 0.0, 0.0)),
            rw_torque: Some(Vec3::new(0.0, 0.1, 0.0)),
        };
        assert!(!cmd.is_finite());
    }
}
