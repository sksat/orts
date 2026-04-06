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
//!
//! ## Noise injection
//!
//! Each sensor accepts an optional [`noise::NoiseModel`]. Ideal
//! sensors (no noise) are constructed with `::new()`, noisy sensors
//! with `::with_noise(model)`. See [`noise`] for available models.
//!
//! ## Why no `Sensor` trait?
//!
//! A generic `trait Sensor { type Measurement; fn measure(...) }` is
//! intentionally deferred. With only two concrete sensors that produce
//! the same type (`Vector3<f64>`) and feed into a fixed `Sensors`
//! struct, a trait would add abstraction without value. When the
//! sensor count grows (sun sensor, star tracker, GPS, ...) a trait
//! will become useful for `Vec<Box<dyn Sensor>>` iteration.

mod gyroscope;
mod magnetometer;
pub mod noise;
mod star_tracker;

use kaname::epoch::Epoch;

use crate::SpacecraftState;
use crate::plugin::tick_input::Sensors;

pub use gyroscope::Gyroscope;
pub use magnetometer::Magnetometer;
pub use star_tracker::StarTracker;

/// Aggregates all sensor instances for a single spacecraft and
/// produces [`Sensors`] from the current true state.
///
/// This mirrors [`crate::plugin::ActuatorBundle`] on the command
/// side: `SensorBundle` collects sensor readings *into* the
/// tick input, while `ActuatorBundle` collects commands *out of*
/// the tick input.
///
/// Sensors are `Option`-wrapped because a spacecraft may not carry
/// every sensor type. Missing sensors produce `None` in the
/// corresponding `Sensors` field.
pub struct SensorBundle {
    pub magnetometer: Option<Magnetometer>,
    pub gyroscope: Option<Gyroscope>,
    pub star_tracker: Option<StarTracker>,
}

impl SensorBundle {
    /// Create an empty bundle (no sensors configured).
    pub fn new() -> Self {
        Self {
            magnetometer: None,
            gyroscope: None,
            star_tracker: None,
        }
    }

    /// Evaluate all configured sensors at the given state and epoch.
    ///
    /// `&mut self` because noise models mutate their internal RNG.
    pub fn evaluate(&mut self, state: &SpacecraftState, epoch: &Epoch) -> Sensors {
        Sensors {
            magnetometer: self.magnetometer.as_mut().map(|m| m.measure(state, epoch)),
            gyroscope: self.gyroscope.as_mut().map(|g| g.measure(state, epoch)),
            star_tracker: self.star_tracker.as_mut().map(|s| s.measure(state, epoch)),
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
    fn empty_bundle_produces_none_fields() {
        let mut bundle = SensorBundle::new();
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert!(readings.magnetometer.is_none());
        assert!(readings.gyroscope.is_none());
    }

    #[test]
    fn full_bundle_produces_some_fields() {
        let mut bundle = SensorBundle {
            magnetometer: Some(Magnetometer::new(Arc::new(TiltedDipole::earth()))),
            gyroscope: Some(Gyroscope::new()),
            star_tracker: None,
        };
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert!(readings.magnetometer.is_some());
        assert!(readings.gyroscope.is_some());
    }
}
