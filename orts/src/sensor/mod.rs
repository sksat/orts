//! Spacecraft sensors.
//!
//! This module provides sensor models that convert the simulator's
//! true spacecraft state into measurement values a real onboard
//! computer would see. Sensor readings are collected by
//! [`SensorBundle`] and handed to plugin controllers via
//! [`crate::plugin::tick_input::Sensors`].
//!
//! ## Current sensors
//!
//! - [`Magnetometer`] — geomagnetic field in the body frame \[T\]
//! - [`Gyroscope`] — angular velocity in the body frame \[rad/s\]
//! - [`StarTracker`] — attitude quaternion body→inertial
//!
//! ## Noise injection
//!
//! Each sensor accepts an optional [`noise::NoiseModel`]. Ideal
//! sensors (no noise) are constructed with `::new()`, noisy sensors
//! with `::with_noise(model)`. See [`noise`] for available models.
//!
//! ## Multi-instance support
//!
//! Each sensor type is stored as a `Vec`, allowing multiple
//! instances with different noise models / accuracy. Index order
//! matches the config definition order and is stable during a
//! simulation run.

mod gyroscope;
mod magnetometer;
pub mod noise;
mod star_tracker;
mod sun_sensor;

use arika::epoch::Epoch;

use crate::SpacecraftState;
use crate::plugin::tick_input::Sensors;

pub use gyroscope::Gyroscope;
pub use magnetometer::Magnetometer;
pub use star_tracker::StarTracker;
pub use sun_sensor::SunSensor;

/// Aggregates all sensor instances for a single spacecraft and
/// produces [`Sensors`] from the current true state.
///
/// This mirrors [`crate::plugin::ActuatorBundle`] on the command
/// side: `SensorBundle` collects sensor readings *into* the
/// tick input, while `ActuatorBundle` collects commands *out of*
/// the tick input.
///
/// Each sensor type is a `Vec` — empty means no sensor of that
/// type is mounted. Multiple instances of the same type are
/// supported (e.g., redundant gyroscopes with different noise).
pub struct SensorBundle {
    pub magnetometers: Vec<Magnetometer>,
    pub gyroscopes: Vec<Gyroscope>,
    pub star_trackers: Vec<StarTracker>,
    pub sun_sensors: Vec<SunSensor>,
}

impl SensorBundle {
    /// Create an empty bundle (no sensors configured).
    pub fn new() -> Self {
        Self {
            magnetometers: Vec::new(),
            gyroscopes: Vec::new(),
            star_trackers: Vec::new(),
            sun_sensors: Vec::new(),
        }
    }

    /// Evaluate all configured sensors at the given state and epoch.
    ///
    /// `&mut self` because noise models mutate their internal RNG.
    pub fn evaluate(&mut self, state: &SpacecraftState, epoch: &Epoch) -> Sensors {
        Sensors {
            magnetometers: self
                .magnetometers
                .iter_mut()
                .map(|m| m.measure(state, epoch))
                .collect(),
            gyroscopes: self
                .gyroscopes
                .iter_mut()
                .map(|g| g.measure(state, epoch))
                .collect(),
            star_trackers: self
                .star_trackers
                .iter_mut()
                .map(|s| s.measure(state, epoch))
                .collect(),
            sun_sensors: self
                .sun_sensors
                .iter_mut()
                .map(|s| s.measure(state, epoch))
                .collect(),
        }
    }
}

impl Default for SensorBundle {
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
    use std::sync::Arc;
    use tobari::magnetic::TiltedDipole;

    fn make_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.05, -0.03),
            },
            mass: 50.0,
        }
    }

    #[test]
    fn empty_bundle_produces_empty_vecs() {
        let mut bundle = SensorBundle::new();
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert!(readings.magnetometers.is_empty());
        assert!(readings.gyroscopes.is_empty());
        assert!(readings.star_trackers.is_empty());
    }

    #[test]
    fn single_sensor_produces_one_reading() {
        let mut bundle = SensorBundle {
            magnetometers: vec![Magnetometer::new(Arc::new(TiltedDipole::earth()))],
            gyroscopes: vec![Gyroscope::new()],
            star_trackers: Vec::new(),
            sun_sensors: Vec::new(),
        };
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert_eq!(readings.magnetometers.len(), 1);
        assert_eq!(readings.gyroscopes.len(), 1);
        assert!(readings.star_trackers.is_empty());
    }

    #[test]
    fn multiple_gyroscopes() {
        let mut bundle = SensorBundle {
            magnetometers: Vec::new(),
            gyroscopes: vec![Gyroscope::new(), Gyroscope::new()],
            star_trackers: Vec::new(),
            sun_sensors: Vec::new(),
        };
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert_eq!(readings.gyroscopes.len(), 2);
        // Both ideal gyros should produce the same reading
        assert_eq!(readings.gyroscopes[0], readings.gyroscopes[1]);
    }
}
