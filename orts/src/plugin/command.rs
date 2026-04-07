//! Logical commands returned by a controller backend.
//!
//! A `Command` is the plugin-layer output. Guests do NOT return raw
//! `ExternalLoads` (acceleration / torque / mass rate); instead they
//! return high-level actuator commands that the host translates into
//! physical loads via `ActuatorBundle`.
//!
//! The variant set grows incrementally with each phase:
//! - P0.5: `MagneticMoment` (B-dot detumbling)
//! - P1: `RwTorque` (reaction wheel torque command)
//! - P4: thrust throttle / impulsive delta-v
//! - P5: composite commands for coupled attitude + thrust guest
//!
//! See DESIGN.md Phase P, D2 ("Command enum は最小 variant から始めて
//! phase ごとに拡張する").

use kaname::frame::{Body, Vec3};

/// Logical command emitted by a controller backend.
///
/// The variants intentionally start minimal and will grow across phases.
/// The representation is `#[non_exhaustive]` so adding a variant later is
/// not a breaking change for downstream code that matches on `Command`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// Commanded magnetic dipole moment, expressed in the spacecraft body
    /// frame \[A·m²\]. Consumed by
    /// [`crate::attitude::CommandedMagnetorquer`].
    MagneticMoment(Vec3<Body>),

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
    RwTorque(Vec3<Body>),
}

impl Command {
    /// Returns `true` if every numeric component in the command is
    /// finite (not NaN / +-Inf).
    ///
    /// The host MUST call this before handing a guest-produced command
    /// to the actuator layer; a NaN leak will propagate into the 14-D
    /// spacecraft state through `axpy` on the next ODE step and destroy
    /// the whole trajectory.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::MagneticMoment(m) => m.is_finite(),
            Self::RwTorque(t) => t.is_finite(),
        }
    }

    /// Extract the commanded magnetic dipole moment \[A·m²\], if this
    /// command is a [`Command::MagneticMoment`].
    pub fn as_magnetic_moment(&self) -> Option<Vec3<Body>> {
        match self {
            Self::MagneticMoment(m) => Some(*m),
            _ => None,
        }
    }

    /// Extract the commanded reaction wheel torque \[N·m\], if this
    /// command is a [`Command::RwTorque`].
    pub fn as_rw_torque(&self) -> Option<Vec3<Body>> {
        match self {
            Self::RwTorque(t) => Some(*t),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnetic_moment_finite_detects_nan() {
        let good = Command::MagneticMoment(Vec3::new(1.0, -2.0, 0.0));
        assert!(good.is_finite());

        let nan = Command::MagneticMoment(Vec3::new(1.0, f64::NAN, 0.0));
        assert!(!nan.is_finite());

        let inf = Command::MagneticMoment(Vec3::new(f64::INFINITY, 0.0, 0.0));
        assert!(!inf.is_finite());
    }

    #[test]
    fn rw_torque_finite_detects_nan() {
        let good = Command::RwTorque(Vec3::new(0.01, -0.02, 0.0));
        assert!(good.is_finite());

        let nan = Command::RwTorque(Vec3::new(f64::NAN, 0.0, 0.0));
        assert!(!nan.is_finite());
    }

    #[test]
    fn as_accessors() {
        let mm = Command::MagneticMoment(Vec3::new(1.0, 2.0, 3.0));
        assert!(mm.as_magnetic_moment().is_some());
        assert!(mm.as_rw_torque().is_none());

        let rw = Command::RwTorque(Vec3::new(0.1, 0.2, 0.3));
        assert!(rw.as_magnetic_moment().is_none());
        assert!(rw.as_rw_torque().is_some());
    }
}
