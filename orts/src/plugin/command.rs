//! Logical commands returned by a controller backend.
//!
//! A `Command` is the plugin-layer output. Guests do NOT return raw
//! `ExternalLoads` (acceleration / torque / mass rate); instead they
//! return high-level actuator commands that the host translates into
//! physical loads via `ActuatorBundle`.
//!
//! Phase P0.5 starts with a single variant (`MagneticMoment`) to exercise
//! the `BdotFiniteDiff` -> plugin layer adapter. Subsequent phases extend
//! the enum as needed:
//! - P3: reaction wheel commands
//! - P4: thrust throttle / impulsive delta-v
//! - P5: composite commands for coupled attitude + thrust guest
//!
//! See DESIGN.md Phase P, D2 ("Command enum は最小 variant から始めて
//! phase ごとに拡張する").

use nalgebra::Vector3;

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
    MagneticMoment(Vector3<f64>),
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
            Self::MagneticMoment(m) => m.iter().all(|x| x.is_finite()),
        }
    }

    /// Extract the commanded magnetic dipole moment \[A·m²\], if this
    /// command is a [`Command::MagneticMoment`].
    ///
    /// This accessor exists so integration tests and host-side
    /// dispatch code can query specific variants without writing
    /// `let Command::MagneticMoment(m) = cmd else { ... };` boilerplate
    /// at every call site. The enum is `#[non_exhaustive]`, so
    /// exhaustive `match` from external crates is not allowed anyway.
    ///
    /// Returns `None` for any future variant that is not
    /// `MagneticMoment`.
    pub fn as_magnetic_moment(&self) -> Option<Vector3<f64>> {
        match self {
            Self::MagneticMoment(m) => Some(*m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnetic_moment_finite_detects_nan() {
        let good = Command::MagneticMoment(Vector3::new(1.0, -2.0, 0.0));
        assert!(good.is_finite());

        let nan = Command::MagneticMoment(Vector3::new(1.0, f64::NAN, 0.0));
        assert!(!nan.is_finite());

        let inf = Command::MagneticMoment(Vector3::new(f64::INFINITY, 0.0, 0.0));
        assert!(!inf.is_finite());
    }
}
