//! Sun sensor.
//!
//! Computes the sun direction in the spacecraft body frame from the
//! true spacecraft state and epoch. The sun direction is the
//! satellite→Sun unit vector rotated into the body frame.

use arika::epoch::Epoch;
use arika::frame::{self, Vec3};
use arika::sun::sun_position_eci;

use super::noise::NoiseModel;
use crate::SpacecraftState;
use crate::plugin::tick_input::{SunDirectionBody, SunSensorOutput};

/// Sun sensor that measures the sun direction in the body frame.
///
/// Computes the satellite→Sun unit vector and rotates it into the
/// body frame via the attitude quaternion:
///
/// ```text
/// d_eci = normalize(sun_pos_eci - sc_pos_eci)
/// d_body = noise(R_bi · d_eci)
/// ```
pub struct SunSensor {
    noise: Vec<Box<dyn NoiseModel>>,
}

impl SunSensor {
    /// Create an ideal sun sensor (no noise).
    pub fn new() -> Self {
        Self { noise: Vec::new() }
    }

    /// Add a noise model. Multiple calls chain in order.
    pub fn with_noise(mut self, noise: impl NoiseModel + 'static) -> Self {
        self.noise.push(Box::new(noise));
        self
    }

    /// Measure the sun direction in the body frame (fine sun sensor).
    ///
    /// Returns `SunSensorOutput::Fine` with illumination = 1.0.
    // TODO: eclipse 対応 — shadow_function で illumination を計算し、
    // eclipse 中は direction = None にする。
    pub fn measure(&mut self, state: &SpacecraftState, epoch: &Epoch) -> SunSensorOutput {
        // Satellite-to-Sun vector in ECI
        let sun_eci = sun_position_eci(epoch);
        let sc_pos = state.orbit.position_eci();
        let sat_to_sun = sun_eci.into_inner() - sc_pos.into_inner();
        let norm = sat_to_sun.magnitude();
        let dir_eci = if norm > 1e-15 {
            sat_to_sun / norm
        } else {
            sat_to_sun
        };

        // Rotate to body frame
        let dir_eci_typed = Vec3::<frame::SimpleEci>::from_raw(dir_eci);
        let dir_body = state.attitude.rotation_to_body().transform(&dir_eci_typed);
        let mut d = dir_body.into_inner();

        for n in &mut self.noise {
            d = n.apply(d);
        }

        SunSensorOutput::Fine {
            direction: SunDirectionBody::new(Vec3::<frame::Body>::from_raw(d)),
            illumination: 1.0, // TODO: eclipse
        }
    }
}

impl Default for SunSensor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use nalgebra::{Vector3, Vector4};

    fn leo_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        }
    }

    #[test]
    fn ideal_sun_sensor_returns_fine_with_unit_vector() {
        let mut sensor = SunSensor::new();
        let state = leo_state();
        let epoch = Epoch::j2000();
        let output = sensor.measure(&state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                let mag = direction.into_inner().magnitude();
                assert!(
                    (mag - 1.0).abs() < 1e-10,
                    "expected unit vector, got magnitude {mag}"
                );
                assert!((illumination - 1.0).abs() < 1e-15);
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn identity_attitude_preserves_eci_direction() {
        let mut sensor = SunSensor::new();
        let state = leo_state();
        let epoch = Epoch::j2000();
        let output = sensor.measure(&state, &epoch);
        let dir_body = match output {
            SunSensorOutput::Fine { direction, .. } => direction.into_inner().into_inner(),
            _ => panic!("expected Fine output"),
        };

        // With identity quaternion, body == ECI
        use arika::sun::sun_position_eci;
        let sun_eci = sun_position_eci(&epoch).into_inner();
        let sc_pos = state.orbit.position_eci().into_inner();
        let expected = (sun_eci - sc_pos).normalize();
        assert!(
            (dir_body - expected).magnitude() < 1e-10,
            "body should match ECI for identity attitude"
        );
    }
}
