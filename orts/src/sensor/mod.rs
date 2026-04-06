//! Spacecraft sensors (Phase P1).
//!
//! This module provides ideal sensor models that convert the simulator's
//! true spacecraft state into measurement values a real onboard computer
//! would see. Sensor readings are collected by [`SensorBundle`] and
//! handed to plugin controllers via
//! [`crate::plugin::tick_input::Sensors`].
//!
//! ## Current sensors
//!
//! - [`Magnetometer`] — geomagnetic field in the body frame \[T\]
//! - [`Gyroscope`] — angular velocity in the body frame \[rad/s\]
//!
//! ## Noise injection
//!
//! All sensors in this module are **ideal** (zero noise). A `NoiseModel`
//! trait and concrete noise types (Gaussian white noise, bias, etc.)
//! will be added in a follow-up phase. The sensor structs are designed
//! so that noise can be composed in without changing their public API
//! shape.
//!
//! ## Why no `Sensor` trait?
//!
//! A generic `trait Sensor { type Measurement; fn measure(...) }` is
//! intentionally deferred. With only two concrete sensors that produce
//! the same type (`Vector3<f64>`) and feed into a fixed
//! `Sensors` struct, a trait would add abstraction without value.
//! When the sensor count grows (sun sensor, star tracker, GPS, ...) a
//! trait will become useful for `Vec<Box<dyn Sensor>>` iteration; at
//! that point the existing structs can implement it without breaking
//! changes.

mod gyroscope;
mod magnetometer;

use kaname::epoch::Epoch;

use crate::SpacecraftState;
use crate::plugin::tick_input::Sensors;

pub use gyroscope::Gyroscope;
pub use magnetometer::Magnetometer;

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
}

impl SensorBundle {
    /// Create an empty bundle (no sensors configured).
    pub fn new() -> Self {
        Self {
            magnetometer: None,
            gyroscope: None,
        }
    }

    /// Evaluate all configured sensors at the given state and epoch,
    /// producing immutable [`Sensors`] for the current tick.
    pub fn evaluate(&self, state: &SpacecraftState, epoch: &Epoch) -> Sensors {
        Sensors {
            magnetic_field_body: self.magnetometer.as_ref().map(|m| m.measure(state, epoch)),
            angular_velocity_body: self.gyroscope.as_ref().map(|g| g.measure(state, epoch)),
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
        let bundle = SensorBundle::new();
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert!(readings.magnetic_field_body.is_none());
        assert!(readings.angular_velocity_body.is_none());
    }

    #[test]
    fn full_bundle_produces_some_fields() {
        let bundle = SensorBundle {
            magnetometer: Some(Magnetometer::new(Arc::new(TiltedDipole::earth()))),
            gyroscope: Some(Gyroscope::new()),
        };
        let epoch = Epoch::j2000();
        let state = make_state();
        let readings = bundle.evaluate(&state, &epoch);
        assert!(readings.magnetic_field_body.is_some());
        assert!(readings.angular_velocity_body.is_some());
    }
}
