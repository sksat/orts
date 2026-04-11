//! Per-tick input handed to a plugin controller.
//!
//! [`TickInput`] bundles everything a guest controller needs to
//! compute a [`super::Command`]: simulation time, epoch, sensor
//! readings, and (optionally) the true spacecraft state for
//! debugging.

use arika::epoch::Epoch;
use arika::frame::{Body, Vec3};
use nalgebra::Vector4;

use crate::SpacecraftState;

// ─── sensor output newtypes ──────────────────────────────────────

/// Magnetic field in the body frame \[T\].
///
/// Newtype wrapper that encodes the physical quantity (magnetic field),
/// coordinate frame (body), and units (Tesla) at the type level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagneticFieldBody(Vec3<Body>);

impl MagneticFieldBody {
    /// Wrap a typed body-frame vector as a body-frame magnetic field.
    pub fn new(v: Vec3<Body>) -> Self {
        Self(v)
    }
    /// Borrow the inner vector.
    pub fn inner(&self) -> &Vec3<Body> {
        &self.0
    }
    /// Consume and return the inner vector.
    pub fn into_inner(self) -> Vec3<Body> {
        self.0
    }
}

/// Angular velocity in the body frame \[rad/s\].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngularVelocityBody(Vec3<Body>);

impl AngularVelocityBody {
    pub fn new(v: Vec3<Body>) -> Self {
        Self(v)
    }
    pub fn inner(&self) -> &Vec3<Body> {
        &self.0
    }
    pub fn into_inner(self) -> Vec3<Body> {
        self.0
    }
}

/// Attitude quaternion representing the rotation from the body frame
/// to the inertial (ECI) frame. Hamilton convention, scalar-first
/// `[w, x, y, z]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeBodyToInertial(Vector4<f64>);

impl AttitudeBodyToInertial {
    pub fn new(v: Vector4<f64>) -> Self {
        Self(v)
    }
    pub fn inner(&self) -> &Vector4<f64> {
        &self.0
    }
    pub fn into_inner(self) -> Vector4<f64> {
        self.0
    }
}

// ─── sensor readings ─────────────────────────────────────────────

/// Sensor readings evaluated at the current tick instant.
///
/// Each field is `Option`-wrapped — `None` means the sensor is not
/// configured or not available this tick.
///
/// Field names are sensor names; types encode the physical quantity,
/// coordinate frame, and units.
#[derive(Debug, Clone, Default)]
pub struct Sensors {
    /// Magnetometer reading. Pre-evaluated once per tick.
    pub magnetometer: Option<MagneticFieldBody>,

    /// Gyroscope reading.
    pub gyroscope: Option<AngularVelocityBody>,

    /// Star tracker reading.
    pub star_tracker: Option<AttitudeBodyToInertial>,
}

impl Sensors {
    /// Construct an empty set of readings (no sensors configured).
    pub fn empty() -> Self {
        Self::default()
    }
}

// ─── actuator state ──────────────────────────────────────────────

/// アクチュエータのテレメトリ（状態フィードバック）。
///
/// Each field is `Option`-wrapped — `None` means the actuator is
/// not present or the host has not populated this tick's telemetry.
#[derive(Debug, Clone, Default)]
pub struct ActuatorState {
    /// RW 各ホイールの角運動量 \[N·m·s\]。
    pub rw_momentum: Option<Vec<f64>>,
    // 将来: pub fuel_mass: Option<f64>,
}

// ─── tick input ──────────────────────────────────────────────────

/// Per-tick input handed to a plugin controller's `update` call.
///
/// Borrowed references keep this zero-copy in the native path. The
/// WASM backend serializes the matching shape via WIT Canonical ABI
/// across the guest boundary.
#[derive(Debug, Clone)]
pub struct TickInput<'a> {
    /// Current simulation time \[s\] (seconds from the controller's
    /// reference t=0, not wall clock).
    pub t: f64,
    /// Absolute epoch, if the simulation is bound to a wall-clock time
    /// base (e.g. for ephemeris / magnetic-field models).
    pub epoch: Option<&'a Epoch>,
    /// Sensor readings evaluated at this tick. May contain noise;
    /// use `spacecraft` for ground-truth.
    pub sensors: &'a Sensors,
    /// Actuator state feedback (e.g. RW momentum) at this tick.
    pub actuators: &'a ActuatorState,
    /// True spacecraft state: orbit + attitude + mass. This is the
    /// simulation ground-truth, not a sensor measurement.
    pub spacecraft: &'a SpacecraftState,
}
