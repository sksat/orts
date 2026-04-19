//! Sun sensor.
//!
//! Computes the sun direction in the spacecraft body frame from the
//! true spacecraft state and epoch. The sun direction is the
//! satellite→Sun unit vector rotated into the body frame.

use arika::eclipse::{self, SUN_RADIUS_KM, ShadowModel};
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
///
/// When eclipse support is enabled (`shadow_body_radius` is set),
/// the sensor also computes the illumination fraction. During total
/// eclipse (illumination = 0), direction is `None`.
pub struct SunSensor {
    noise: Vec<Box<dyn NoiseModel>>,
    /// Central body radius for eclipse computation \[km\].
    /// `None` disables eclipse (always sunlit, illumination = 1.0).
    shadow_body_radius: Option<f64>,
    /// Shadow model for eclipse computation.
    shadow_model: ShadowModel,
}

impl SunSensor {
    /// Create an ideal sun sensor (no noise, no eclipse).
    pub fn new() -> Self {
        Self {
            noise: Vec::new(),
            shadow_body_radius: None,
            shadow_model: ShadowModel::Conical,
        }
    }

    /// Create a sun sensor for Earth orbit with conical shadow model.
    pub fn for_earth() -> Self {
        Self {
            noise: Vec::new(),
            shadow_body_radius: Some(arika::earth::R),
            shadow_model: ShadowModel::Conical,
        }
    }

    /// Add a noise model. Multiple calls chain in order.
    pub fn with_noise(mut self, noise: impl NoiseModel + 'static) -> Self {
        self.noise.push(Box::new(noise));
        self
    }

    /// Set the shadow body radius for eclipse computation.
    pub fn with_shadow_body(mut self, radius: f64) -> Self {
        self.shadow_body_radius = Some(radius);
        self
    }

    /// Set the shadow model.
    pub fn with_shadow_model(mut self, model: ShadowModel) -> Self {
        self.shadow_model = model;
        self
    }

    /// Measure the sun direction in the body frame (fine sun sensor).
    ///
    /// Returns `SunSensorOutput::Fine` with:
    /// - `direction: Some(...)` when the sun is visible (illumination > 0)
    /// - `direction: None` when in total eclipse (illumination = 0)
    /// - `illumination` in \[0, 1\]: actual eclipse-aware illumination fraction
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

        // Compute illumination if eclipse is enabled
        let illumination = if let Some(body_r) = self.shadow_body_radius {
            eclipse::illumination_central(
                &sc_pos.into_inner(),
                &sun_eci.into_inner(),
                body_r,
                SUN_RADIUS_KM,
                self.shadow_model,
            )
        } else {
            1.0
        };

        // In total eclipse, direction is unmeasurable
        if illumination <= 0.0 {
            return SunSensorOutput::Fine {
                direction: None,
                illumination: 0.0,
            };
        }

        // Rotate to body frame
        let dir_eci_typed = Vec3::<frame::SimpleEci>::from_raw(dir_eci);
        let dir_body = state.attitude.rotation_to_body().transform(&dir_eci_typed);
        let mut d = dir_body.into_inner();

        for n in &mut self.noise {
            d = n.apply(d);
        }

        SunSensorOutput::Fine {
            direction: Some(SunDirectionBody::new(Vec3::<frame::Body>::from_raw(d))),
            illumination,
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
                let dir = direction.expect("should have direction when sunlit");
                let mag = dir.into_inner().magnitude();
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
            SunSensorOutput::Fine { direction, .. } => direction
                .expect("should have direction")
                .into_inner()
                .into_inner(),
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

    #[test]
    fn eclipse_sensor_returns_none_direction_in_shadow() {
        // Place satellite behind Earth where it should be in eclipse
        let mut sensor = SunSensor::for_earth();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        // At equinox, Sun is roughly +X. Place satellite behind Earth at -X.
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(-(6371.0 + 400.0), 0.0, 0.0),
                Vector3::new(0.0, -7.67, 0.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };

        let output = sensor.measure(&state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_none(),
                    "direction should be None in total eclipse"
                );
                assert!(
                    illumination < 0.01,
                    "illumination should be ~0 in shadow, got {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn eclipse_sensor_returns_some_direction_when_sunlit() {
        let mut sensor = SunSensor::for_earth();
        let state = leo_state(); // Sun-side
        let epoch = Epoch::j2000();
        let output = sensor.measure(&state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(direction.is_some(), "direction should be Some when sunlit");
                assert!(
                    (illumination - 1.0).abs() < 0.01,
                    "illumination should be ~1.0, got {illumination}"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }

    #[test]
    fn no_eclipse_sensor_always_sunlit() {
        // Without shadow body, even behind Earth should show illumination = 1
        let mut sensor = SunSensor::new();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(-(6371.0 + 400.0), 0.0, 0.0),
                Vector3::new(0.0, -7.67, 0.0),
            ),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        };

        let output = sensor.measure(&state, &epoch);
        match output {
            SunSensorOutput::Fine {
                direction,
                illumination,
            } => {
                assert!(
                    direction.is_some(),
                    "no eclipse: direction should always be Some"
                );
                assert!(
                    (illumination - 1.0).abs() < 1e-15,
                    "no eclipse: illumination should be 1.0"
                );
            }
            _ => panic!("expected Fine output"),
        }
    }
}
