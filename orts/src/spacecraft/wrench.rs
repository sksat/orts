use std::ops::{Add, AddAssign};

use nalgebra::Vector3;

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
    /// Mass rate [kg/s] (negative for depletion, e.g. propellant consumption).
    pub mass_rate: f64,
}

impl ExternalLoads {
    pub fn zeros() -> Self {
        Self {
            acceleration_inertial: Vector3::zeros(),
            torque_body: Vector3::zeros(),
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only torque (body frame) [N·m].
    pub fn torque(t: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: Vector3::zeros(),
            torque_body: t,
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only translational acceleration (inertial frame) [km/s²].
    pub fn acceleration(a: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: a,
            torque_body: Vector3::zeros(),
            mass_rate: 0.0,
        }
    }
}

impl Add for ExternalLoads {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            acceleration_inertial: self.acceleration_inertial + rhs.acceleration_inertial,
            torque_body: self.torque_body + rhs.torque_body,
            mass_rate: self.mass_rate + rhs.mass_rate,
        }
    }
}

impl AddAssign for ExternalLoads {
    fn add_assign(&mut self, rhs: Self) {
        self.acceleration_inertial += rhs.acceleration_inertial;
        self.torque_body += rhs.torque_body;
        self.mass_rate += rhs.mass_rate;
    }
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
            mass_rate: -0.5,
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
            mass_rate: -0.3,
        };
        let sum = a + b;
        assert_eq!(sum.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(sum.torque_body, Vector3::new(1.1, 2.2, 3.3));
        assert!((sum.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_assign_component_wise() {
        let mut a = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
            mass_rate: -0.5,
        };
        let b = ExternalLoads {
            acceleration_inertial: Vector3::new(10.0, 20.0, 30.0),
            torque_body: Vector3::new(1.0, 2.0, 3.0),
            mass_rate: -0.3,
        };
        a += b;
        assert_eq!(a.acceleration_inertial, Vector3::new(11.0, 22.0, 33.0));
        assert_eq!(a.torque_body, Vector3::new(1.1, 2.2, 3.3));
        assert!((a.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_zeros_identity() {
        let w = ExternalLoads {
            acceleration_inertial: Vector3::new(1.0, 2.0, 3.0),
            torque_body: Vector3::new(0.1, 0.2, 0.3),
            mass_rate: -0.1,
        };
        let sum = w.clone() + ExternalLoads::zeros();
        assert_eq!(sum, w);
    }

    #[test]
    fn zeros_has_zero_mass_rate() {
        assert_eq!(ExternalLoads::zeros().mass_rate, 0.0);
    }
}
