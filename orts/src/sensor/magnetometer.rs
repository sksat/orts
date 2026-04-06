//! Ideal magnetometer sensor.
//!
//! Transforms the geomagnetic field from the ECI frame to the
//! spacecraft body frame using the attitude quaternion. The returned
//! vector is what a real three-axis magnetometer would measure in the
//! absence of sensor noise, hard/soft iron distortion, and temperature
//! effects.

use std::sync::Arc;

use kaname::epoch::Epoch;
use nalgebra::Vector3;
use tobari::magnetic::MagneticFieldModel;

use crate::SpacecraftState;

/// Ideal three-axis magnetometer.
///
/// Evaluates the host's geomagnetic field model at the spacecraft's
/// current ECI position and epoch, then rotates the result into the
/// body frame via the attitude quaternion:
///
/// ```text
/// B_body = R_bi · B_eci(r, epoch)
/// ```
///
/// where `R_bi` is the inertial-to-body rotation matrix.
pub struct Magnetometer {
    field_model: Arc<dyn MagneticFieldModel>,
}

impl Magnetometer {
    /// Create an ideal magnetometer backed by the given field model.
    ///
    /// The field model is `Arc`-wrapped so it can be shared with other
    /// subsystems (e.g. `CommandedMagnetorquer`) that also need to
    /// evaluate the geomagnetic field.
    pub fn new(field_model: Arc<dyn MagneticFieldModel>) -> Self {
        Self { field_model }
    }

    /// Measure the magnetic field in the body frame \[T\].
    pub fn measure(&self, state: &SpacecraftState, epoch: &Epoch) -> Vector3<f64> {
        let b_eci = self
            .field_model
            .field_eci(&state.orbit.position_eci(), epoch);
        state.attitude.inertial_to_body() * b_eci
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use nalgebra::Vector4;
    use tobari::magnetic::TiltedDipole;

    #[test]
    fn ideal_magnetometer_returns_finite_nonzero_for_leo() {
        let mag = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        let state = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };
        let epoch = Epoch::j2000();
        let b_body = mag.measure(&state, &epoch);
        assert!(b_body.iter().all(|x| x.is_finite()));
        let magnitude = b_body.magnitude();
        // LEO geomagnetic field: ~20-60 µT (2e-5 to 6e-5 T).
        assert!(
            magnitude > 1e-5 && magnitude < 1e-4,
            "expected LEO-range B, got {magnitude:.3e} T"
        );
    }

    #[test]
    fn identity_quaternion_gives_same_as_eci() {
        let field_model = Arc::new(TiltedDipole::earth());
        let mag = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>);
        let state = SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0), // identity
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };
        let epoch = Epoch::j2000();
        let b_body = mag.measure(&state, &epoch);
        let b_eci = field_model.field_eci(&state.orbit.position_eci(), &epoch);
        // Identity quaternion: body frame == ECI frame.
        assert!((b_body - b_eci).magnitude() < 1e-15);
    }
}
