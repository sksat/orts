//! Ideal rate gyroscope sensor.
//!
//! Returns the spacecraft's true angular velocity in the body frame.
//! A real MEMS or fiber-optic gyroscope would add bias drift, angle
//! random walk, and quantisation noise; this ideal version returns
//! the exact value from the simulation state.

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use crate::SpacecraftState;

/// Ideal three-axis rate gyroscope.
///
/// The measurement is simply `state.attitude.angular_velocity`, which
/// is already expressed in the body frame \[rad/s\].
///
/// The `epoch` parameter is unused in the ideal sensor but is
/// accepted for signature consistency with [`super::Magnetometer`],
/// enabling a future `Sensor` trait with a uniform
/// `measure(&self, state, epoch)` signature.
pub struct Gyroscope;

impl Gyroscope {
    /// Create an ideal gyroscope (zero noise, zero bias).
    pub fn new() -> Self {
        Self
    }

    /// Measure the angular velocity in the body frame \[rad/s\].
    pub fn measure(&self, state: &SpacecraftState, _epoch: &Epoch) -> Vector3<f64> {
        state.attitude.angular_velocity
    }
}

impl Default for Gyroscope {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use nalgebra::Vector4;

    #[test]
    fn ideal_gyroscope_returns_true_angular_velocity() {
        let gyro = Gyroscope::new();
        let omega = Vector3::new(0.1, 0.05, -0.03);
        let state = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: omega,
            },
            mass: 50.0,
        };
        let epoch = Epoch::j2000();
        let measured = gyro.measure(&state, &epoch);
        assert_eq!(measured, omega);
    }
}
