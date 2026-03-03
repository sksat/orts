use std::ops::{Add, AddAssign};

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use super::SpacecraftState;

/// Acceleration (inertial frame) and torque (body frame) pair.
///
/// This is NOT a classical wrench (which would be in a single frame).
/// Each field is in the frame used by its respective equation of motion:
/// - acceleration: inertial frame [km/s²] (for translational EOM)
/// - torque: body frame [N·m] (for rotational EOM)
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLoads {
    /// Translational acceleration in inertial frame [km/s²].
    pub acceleration_inertial: Vector3<f64>,
    /// Torque in body frame [N·m].
    pub torque_body: Vector3<f64>,
}

impl ExternalLoads {
    pub fn zeros() -> Self {
        Self {
            acceleration_inertial: Vector3::zeros(),
            torque_body: Vector3::zeros(),
        }
    }
}

impl Add for ExternalLoads {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            acceleration_inertial: self.acceleration_inertial + rhs.acceleration_inertial,
            torque_body: self.torque_body + rhs.torque_body,
        }
    }
}

impl AddAssign for ExternalLoads {
    fn add_assign(&mut self, rhs: Self) {
        self.acceleration_inertial += rhs.acceleration_inertial;
        self.torque_body += rhs.torque_body;
    }
}

/// A model that computes both acceleration and torque on a spacecraft.
///
/// Unlike `ForceModel` (acceleration only) or `TorqueModel` (torque only),
/// `LoadModel` has access to the full `SpacecraftState` and can produce
/// coupled effects (e.g., gravity gradient torque depends on orbital position).
pub trait LoadModel: Send + Sync {
    fn name(&self) -> &str;
    fn loads(&self, t: f64, state: &SpacecraftState, epoch: Option<&Epoch>) -> ExternalLoads;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeros() {
        let w = ExternalLoads::zeros();
        assert_eq!(w.acceleration_inertial, Vector3::zeros());
        assert_eq!(w.torque_body, Vector3::zeros());
    }

    #[test]
    fn add_component_wise() {
        let a = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
        };
        let sum = a + b;
        assert_eq!(sum.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(sum.torque_body, Vector3::new(1.1, 2.2, 3.3));
    }

    #[test]
    fn add_assign_component_wise() {
        let mut a = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
        };
        a += b;
        assert_eq!(a.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(a.torque_body, Vector3::new(1.1, 2.2, 3.3));
    }

    #[test]
    fn add_zeros_identity() {
        let w = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
        };
        let sum = w.clone() + ExternalLoads::zeros();
        assert_eq!(sum, w);
    }
}
