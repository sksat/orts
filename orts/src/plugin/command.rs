//! Logical commands returned by a controller backend.
//!
//! A `Command` is the plugin-layer output. Guests do NOT return raw
//! `ExternalLoads` (acceleration / torque / mass rate); instead they
//! return per-device actuator commands that the host translates into
//! physical loads via `ActuatorBundle`.
//!
//! The field set grows incrementally with each phase:
//! - P1: `mtq_moments` (per-MTQ magnetic moment) + `rw` (per-wheel speed or torque)
//! - P4: thrust throttle / impulsive delta-v
//! - P5: composite commands for coupled attitude + thrust guest
//!
//! See DESIGN.md Phase P, D2 ("Command は per-device 論理指令").

/// Per-wheel RW command.
///
/// The variant selects the command mode: target speeds (the host motor
/// model converts to torque) or direct motor torques (applied after
/// rate/saturation clamping).
#[derive(Debug, Clone, PartialEq)]
pub enum RwCommand {
    /// Target speeds [rad/s]. Host motor model generates torque.
    Speeds(Vec<f64>),
    /// Direct motor torques [N·m]. Applied after clamp.
    Torques(Vec<f64>),
}

impl RwCommand {
    /// Returns `true` if every element in the command is finite.
    pub fn is_finite(&self) -> bool {
        match self {
            RwCommand::Speeds(v) | RwCommand::Torques(v) => v.iter().all(|x| x.is_finite()),
        }
    }
}

/// Logical command emitted by a controller backend.
///
/// Each field corresponds to one actuator type. `Some` means the
/// controller is issuing a command for that actuator; `None` means the
/// controller has nothing to say about it this tick (the actuator
/// retains its previous value via zero-order hold).
///
/// "No command at all" (i.e. the controller has nothing to do this
/// tick) is represented by `Option<Command>` being `None` at the call
/// site, not by an all-`None` `Command` struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// Per-MTQ commanded magnetic dipole moment \[A·m²\].
    /// Length must match the number of MTQs in the assembly.
    pub mtq_moments: Option<Vec<f64>>,

    /// Per-wheel RW command (speed or torque).
    /// Length must match the number of wheels in the assembly.
    /// Sign convention: positive value → wheel absorbs positive angular momentum.
    pub rw: Option<RwCommand>,
}

impl Command {
    /// Create a command that only sets the MTQ moments.
    pub fn mtq(moments: Vec<f64>) -> Self {
        Self {
            mtq_moments: Some(moments),
            rw: None,
        }
    }

    /// Create a command that only sets the RW command.
    pub fn rw_cmd(cmd: RwCommand) -> Self {
        Self {
            mtq_moments: None,
            rw: Some(cmd),
        }
    }

    /// Create a command that only sets the RW torques (convenience shorthand).
    pub fn rw_torques(torques: Vec<f64>) -> Self {
        Self::rw_cmd(RwCommand::Torques(torques))
    }

    /// Create a command that only sets the RW speeds (convenience shorthand).
    pub fn rw_speeds(speeds: Vec<f64>) -> Self {
        Self::rw_cmd(RwCommand::Speeds(speeds))
    }

    /// Returns `true` if every numeric component in the command is
    /// finite (not NaN / +-Inf).
    ///
    /// The host MUST call this before handing a guest-produced command
    /// to the actuator layer; a NaN leak will propagate into the
    /// spacecraft state through `axpy` on the next ODE step and destroy
    /// the whole trajectory.
    pub fn is_finite(&self) -> bool {
        let mtq_ok = self
            .mtq_moments
            .as_ref()
            .is_none_or(|v| v.iter().all(|x| x.is_finite()));
        let rw_ok = self.rw.as_ref().is_none_or(|cmd| cmd.is_finite());
        mtq_ok && rw_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtq_finite_detects_nan() {
        let good = Command::mtq(vec![1.0, -2.0, 0.0]);
        assert!(good.is_finite());

        let nan = Command::mtq(vec![1.0, f64::NAN, 0.0]);
        assert!(!nan.is_finite());

        let inf = Command::mtq(vec![f64::INFINITY, 0.0, 0.0]);
        assert!(!inf.is_finite());
    }

    #[test]
    fn rw_finite_detects_nan() {
        let good = Command::rw_torques(vec![0.01, -0.02, 0.0]);
        assert!(good.is_finite());

        let nan = Command::rw_torques(vec![f64::NAN, 0.0, 0.0]);
        assert!(!nan.is_finite());
    }

    #[test]
    fn rw_speeds_finite_detects_nan() {
        let good = Command::rw_speeds(vec![10.0, -5.0, 0.0]);
        assert!(good.is_finite());

        let nan = Command::rw_speeds(vec![f64::NAN, 0.0, 0.0]);
        assert!(!nan.is_finite());
    }

    #[test]
    fn field_access() {
        let mm = Command::mtq(vec![1.0, 2.0, 3.0]);
        assert!(mm.mtq_moments.is_some());
        assert!(mm.rw.is_none());

        let rw = Command::rw_torques(vec![0.1, 0.2, 0.3]);
        assert!(rw.mtq_moments.is_none());
        assert!(rw.rw.is_some());
    }

    #[test]
    fn both_fields_set() {
        let cmd = Command {
            mtq_moments: Some(vec![1.0, 0.0, 0.0]),
            rw: Some(RwCommand::Torques(vec![0.0, 0.1, 0.0])),
        };
        assert!(cmd.is_finite());
        assert!(cmd.mtq_moments.is_some());
        assert!(cmd.rw.is_some());
    }

    #[test]
    fn both_fields_nan_in_one() {
        let cmd = Command {
            mtq_moments: Some(vec![f64::NAN, 0.0, 0.0]),
            rw: Some(RwCommand::Torques(vec![0.0, 0.1, 0.0])),
        };
        assert!(!cmd.is_finite());
    }

    #[test]
    fn empty_vec_is_finite() {
        let cmd = Command {
            mtq_moments: Some(vec![]),
            rw: Some(RwCommand::Torques(vec![])),
        };
        assert!(cmd.is_finite());
    }
}
