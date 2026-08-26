//! Per-tick input handed to a plugin controller.
//!
//! [`TickInput`] bundles everything a guest controller needs to
//! compute a [`super::Command`]: simulation time, epoch, sensor
//! readings, and (optionally) the true spacecraft state for
//! debugging.

use core::fmt;

use arika::epoch::Epoch;
use arika::frame::{Body, Eci, Rotation, SimpleEci, Vec3};

use crate::SpacecraftState;

// sensor output newtypes

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

/// Attitude: the rotation from the body frame to the inertial frame `F`.
///
/// The frame is part of the type. A quaternion's components depend on the
/// frame it is expressed against — the same physical attitude has different
/// components in `SimpleEci` and in `Gcrs` (the two differ by
/// precession/nutation, ~0.1° at the pole by 2024) — so an untagged
/// `Vector4` cannot say what it means. `F` defaults to
/// [`SimpleEci`](arika::frame::SimpleEci), the frame the plugin path
/// propagates in.
pub struct AttitudeBodyToInertial<F: Eci = SimpleEci>(Rotation<Body, F>);

impl<F: Eci> AttitudeBodyToInertial<F> {
    /// Wrap a body→`F` rotation as an attitude reading in `F`.
    pub fn new(rotation: Rotation<Body, F>) -> Self {
        Self(rotation)
    }
    /// Borrow the inner rotation.
    pub fn inner(&self) -> &Rotation<Body, F> {
        &self.0
    }
    /// Consume and return the inner rotation.
    pub fn into_inner(self) -> Rotation<Body, F> {
        self.0
    }
}

impl AttitudeBodyToInertial<SimpleEci> {
    /// Drop the frame tag for the **v0 plugin WIT boundary**: the components
    /// `(w, x, y, z)` of the body→simple-ECI quaternion, Hamilton convention.
    ///
    /// The v0 `attitude-body-to-inertial` record in `orts/wit/v0/orts.wit`
    /// carries four bare floats whose frame is *defined* by the contract to be
    /// simple-ECI. This is the named crossing for that boundary, and it exists
    /// only for `SimpleEci`: shipping a `Gcrs` attitude to a guest is a compile
    /// error rather than silent nonsense. Widening the contract means a new WIT
    /// version that names the frame in the payload.
    ///
    /// `inner()` still hands out the rotation in any frame — the guarantee is
    /// that the *boundary* operation names its frame, not that the components
    /// are unreachable.
    pub fn to_wit_v0_simple_eci_quat(&self) -> (f64, f64, f64, f64) {
        let q = self.0.inner();
        (q.w, q.i, q.j, q.k)
    }
}

// Manual impls to avoid requiring `F: Debug/Clone/Copy/PartialEq`
// (the frame is a phantom tag, never a value).
impl<F: Eci> fmt::Debug for AttitudeBodyToInertial<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AttitudeBodyToInertial")
            .field(&self.0)
            .finish()
    }
}
impl<F: Eci> Clone for AttitudeBodyToInertial<F> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<F: Eci> Copy for AttitudeBodyToInertial<F> {}
impl<F: Eci> PartialEq for AttitudeBodyToInertial<F> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// sensor readings

/// Sensor readings evaluated at the current tick instant.
///
/// Each field is a `Vec` — empty means no sensor of that type is
/// configured. Index order is stable (config definition order) and
/// does not change during a simulation run.
///
/// `F` is the inertial frame the state was propagated in, carried by the
/// attitude readings ([`AttitudeBodyToInertial`]), whose components depend on
/// it. The body-frame readings are unaffected to the precision that matters —
/// the gyro's ω is the rate relative to `F`, but two inertial frames differ
/// only by their relative rotation rate (~8e-12 rad/s for
/// `SimpleEci` ↔ `Gcrs`). `F` defaults to
/// [`SimpleEci`](arika::frame::SimpleEci), which is what [`TickInput`] accepts:
/// a bundle evaluated in another frame cannot be handed to a plugin, because
/// the v0 plugin contract is defined in simple-ECI.
pub struct Sensors<F: Eci = SimpleEci> {
    /// Magnetometer readings. Pre-evaluated once per tick.
    pub magnetometers: Vec<MagneticFieldBody>,

    /// Gyroscope readings.
    pub gyroscopes: Vec<AngularVelocityBody>,

    /// Star tracker readings, expressed against `F`.
    pub star_trackers: Vec<AttitudeBodyToInertial<F>>,

    /// Sun sensor outputs.
    pub sun_sensors: Vec<SunSensorOutput>,
}

impl<F: Eci> Sensors<F> {
    /// Construct an empty set of readings (no sensors configured).
    pub fn empty() -> Self {
        Self::default()
    }
}

// Manual impls to avoid requiring `F: Debug/Clone/Default`.
impl<F: Eci> fmt::Debug for Sensors<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sensors")
            .field("magnetometers", &self.magnetometers)
            .field("gyroscopes", &self.gyroscopes)
            .field("star_trackers", &self.star_trackers)
            .field("sun_sensors", &self.sun_sensors)
            .finish()
    }
}
impl<F: Eci> Clone for Sensors<F> {
    fn clone(&self) -> Self {
        Self {
            magnetometers: self.magnetometers.clone(),
            gyroscopes: self.gyroscopes.clone(),
            star_trackers: self.star_trackers.clone(),
            sun_sensors: self.sun_sensors.clone(),
        }
    }
}
impl<F: Eci> Default for Sensors<F> {
    fn default() -> Self {
        Self {
            magnetometers: Vec::new(),
            gyroscopes: Vec::new(),
            star_trackers: Vec::new(),
            sun_sensors: Vec::new(),
        }
    }
}

// sun sensor output types

/// Sun direction in the body frame (unit vector, satellite→Sun).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunDirectionBody(Vec3<Body>);

impl SunDirectionBody {
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

/// Sun sensor output. Sensor type determines the output variant.
#[derive(Debug, Clone, PartialEq)]
pub enum SunSensorOutput {
    /// Fine sun sensor: direction unit vector + illumination.
    ///
    /// `direction` is `None` during total eclipse (illumination = 0)
    /// because the sun sensor cannot measure a direction when no sunlight
    /// is received. During partial eclipse (penumbra), the direction is
    /// still `Some` — the sun center direction — but illumination < 1.
    ///
    /// Note: directional bias from the visible solar disk centroid shift
    /// during partial eclipse is not modeled.
    Fine {
        direction: Option<SunDirectionBody>,
        /// Illumination [0, 1]. 1 = full sun, 0 = full eclipse.
        illumination: f64,
    },
    /// Coarse sun sensor (CSS): cos(incidence) × illumination scalar.
    // TODO: coarse 実装は将来追加予定。
    Coarse(f64),
}

// actuator telemetry

/// Per-wheel RW telemetry (observable internal state).
///
/// All fields are always populated when the RW is mounted.
/// `realized_torques` is `Some` only when motor lag is enabled.
#[derive(Debug, Clone)]
pub struct RwTelemetry {
    /// Per-wheel angular momentum \[N·m·s\].
    pub momentum: Vec<f64>,
    /// Per-wheel spin speed \[rad/s\].
    pub speeds: Vec<f64>,
    /// Per-wheel realized torque \[N·m\] (motor lag model only).
    pub realized_torques: Option<Vec<f64>>,
}

/// Actuator telemetry (per-device structured feedback).
///
/// Each field wraps a per-device telemetry struct. `None` means
/// the actuator type is not mounted on this spacecraft.
#[derive(Debug, Clone, Default)]
pub struct ActuatorTelemetry {
    /// RW telemetry. `None` if no RW is mounted.
    pub rw: Option<RwTelemetry>,
}

// tick input

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
    ///
    /// Simple-ECI, like `spacecraft`: that is what the v0 plugin contract
    /// defines, so a bundle evaluated in another inertial frame does not fit
    /// here (see [`Sensors`]).
    pub sensors: &'a Sensors,
    /// Actuator telemetry (e.g. RW momentum/speed) at this tick.
    pub actuators: &'a ActuatorTelemetry,
    /// True spacecraft state: orbit + attitude + mass. This is the
    /// simulation ground-truth, not a sensor measurement.
    pub spacecraft: &'a SpacecraftState,
}
