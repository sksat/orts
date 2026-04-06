//! Per-tick input handed to a plugin controller.
//!
//! [`TickInput`] bundles everything a guest controller needs to
//! compute a [`super::Command`]: simulation time, epoch, sensor
//! readings, and (optionally) the true spacecraft state for
//! debugging.
//!
//! Using a single struct instead of multiple positional arguments
//! makes it cheap to add new fields later without breaking every
//! backend.

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use crate::SpacecraftState;

/// Sensor readings evaluated at the current tick instant.
///
/// Contains measurements from the host-side sensor models
/// (see [`crate::sensor`]). Each field is `Option`-wrapped because
/// a spacecraft may not carry every sensor type — `None` means
/// the sensor is not configured (or not available this tick).
///
/// ## True state vs sensor readings
///
/// [`TickInput`] provides both the **true state** (via
/// `spacecraft.attitude.angular_velocity`, etc.) and **sensor
/// readings** (via these `Sensors` fields). For ideal sensors
/// these are identical; when noise models are added (future phase),
/// they will diverge. Controllers that want physical realism should
/// use the sensor readings; controllers that need ground-truth for
/// debugging can use the true state directly.
#[derive(Debug, Clone, Default)]
pub struct Sensors {
    /// Magnetic field in the body frame \[T\], as measured by the
    /// magnetometer. `None` if no magnetometer is configured.
    ///
    /// This is the "fast path" for guests that need B_body: it is
    /// pre-evaluated once per tick with no host-call overhead. The
    /// `host-env.magnetic-field-eci` WIT import remains available as
    /// an "escape hatch" for guests that need B at arbitrary
    /// positions/epochs (e.g. for prediction/planning).
    pub magnetic_field_body: Option<Vector3<f64>>,

    /// Angular velocity in the body frame \[rad/s\], as measured by
    /// the rate gyroscope. `None` if no gyroscope is configured.
    pub angular_velocity_body: Option<Vector3<f64>>,
}

impl Sensors {
    /// Construct an empty set of readings (no sensors configured).
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Per-tick input handed to a plugin controller's `update` call.
///
/// Borrowed references keep this zero-copy in the native path. The
/// WASM backend serializes the matching shape via WIT Canonical ABI
/// across the guest boundary.
#[derive(Debug, Clone, Copy)]
pub struct TickInput<'a> {
    /// Current simulation time \[s\] (seconds from the controller's
    /// reference t=0, not wall clock).
    pub t: f64,
    /// Absolute epoch, if the simulation is bound to a wall-clock time
    /// base (e.g. for ephemeris / magnetic-field models).
    pub epoch: Option<&'a Epoch>,
    /// Sensor readings evaluated at this tick. May contain noise in
    /// future phases; use `spacecraft` for ground-truth.
    pub sensors: &'a Sensors,
    /// True spacecraft state: orbit + attitude + mass. This is the
    /// simulation ground-truth, not a sensor measurement. Useful for
    /// debugging and for controllers that don't need sensor realism.
    pub spacecraft: &'a SpacecraftState,
}
