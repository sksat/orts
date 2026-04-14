//! Logical commands returned by a controller backend.
//!
//! A `Command` is the plugin-layer output. Guests do NOT return raw
//! `ExternalLoads` (acceleration / torque / mass rate); instead they
//! return per-device actuator commands that the host translates into
//! physical loads via `ActuatorBundle`.
//!
//! The field set grows incrementally with each phase:
//! - P1: `mtq_moments` (per-MTQ magnetic moment) + `rw_torques` (per-wheel torque)
//! - P4: thrust throttle / impulsive delta-v
//! - P5: composite commands for coupled attitude + thrust guest
//!
//! See DESIGN.md Phase P, D2 ("Command は per-device 論理指令").

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

    /// Per-wheel commanded torque \[N·m\].
    /// Length must match the number of wheels in the assembly.
    /// Sign convention: positive value → wheel absorbs positive angular momentum.
    pub rw_torques: Option<Vec<f64>>,
}

impl Command {
    /// Create a command that only sets the MTQ moments.
    pub fn mtq(moments: Vec<f64>) -> Self {
        Self {
            mtq_moments: Some(moments),
            rw_torques: None,
        }
    }

    /// Create a command that only sets the RW torques.
    pub fn rw(torques: Vec<f64>) -> Self {
        Self {
            mtq_moments: None,
            rw_torques: Some(torques),
        }
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
        let rw_ok = self
            .rw_torques
            .as_ref()
            .is_none_or(|v| v.iter().all(|x| x.is_finite()));
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
        let good = Command::rw(vec![0.01, -0.02, 0.0]);
        assert!(good.is_finite());

        let nan = Command::rw(vec![f64::NAN, 0.0, 0.0]);
        assert!(!nan.is_finite());
    }

    #[test]
    fn field_access() {
        let mm = Command::mtq(vec![1.0, 2.0, 3.0]);
        assert!(mm.mtq_moments.is_some());
        assert!(mm.rw_torques.is_none());

        let rw = Command::rw(vec![0.1, 0.2, 0.3]);
        assert!(rw.mtq_moments.is_none());
        assert!(rw.rw_torques.is_some());
    }

    #[test]
    fn both_fields_set() {
        let cmd = Command {
            mtq_moments: Some(vec![1.0, 0.0, 0.0]),
            rw_torques: Some(vec![0.0, 0.1, 0.0]),
        };
        assert!(cmd.is_finite());
        assert!(cmd.mtq_moments.is_some());
        assert!(cmd.rw_torques.is_some());
    }

    #[test]
    fn both_fields_nan_in_one() {
        let cmd = Command {
            mtq_moments: Some(vec![f64::NAN, 0.0, 0.0]),
            rw_torques: Some(vec![0.0, 0.1, 0.0]),
        };
        assert!(!cmd.is_finite());
    }

    #[test]
    fn empty_vec_is_finite() {
        let cmd = Command {
            mtq_moments: Some(vec![]),
            rw_torques: Some(vec![]),
        };
        assert!(cmd.is_finite());
    }
}
