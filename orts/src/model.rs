//! Capability-based model trait for dynamics systems.
//!
//! Models declare their state requirements via capability bounds (`HasAttitude`,
//! `HasOrbit`, `HasMass`), and each dynamics system accepts models compatible
//! with its concrete state type:
//!
//! - `OrbitalSystem`: `dyn Model<OrbitalState>` — accepts `S: HasOrbit` models
//! - `AttitudeSystem`: `dyn Model<AttitudeState>` — accepts `S: HasAttitude` models
//! - `SpacecraftDynamics`: `dyn Model<SpacecraftState>` — accepts all models
//!
//! A model implemented as `impl<S: HasAttitude> Model<S> for PdController`
//! automatically works with both `AttitudeSystem` and `SpacecraftDynamics`.

use std::ops::{Add, AddAssign};

use arika::epoch::Epoch;
#[cfg(test)]
use arika::frame::SimpleEci;
use arika::frame::{self, Body, Rotation, Vec3};
use nalgebra::Vector3;

use crate::OrbitalState;
use crate::attitude::AttitudeState;

// Capability traits

/// The inertial frame a state is propagated in.
///
/// Both the orbit and the attitude of a given state are expressed against this
/// one frame, so it is declared once here rather than separately on
/// [`HasOrbit`] and [`HasAttitude`] — a state cannot claim its orbit is in
/// `Gcrs` while its attitude is in `SimpleEci`.
///
/// This is what drives frame-generic model dispatch: a model is written as
/// `impl<F: Cap, S: HasFrame<Frame = F> + HasOrbit> Model<S>`, bounded on the
/// minimal capability `Cap` it needs from the frame (e.g.
/// `EarthRotationPole`, `EarthFixedTransform`, `EphemerisFrameBridge`), so a
/// frame that cannot supply it is a compile error rather than a silent mislabel.
/// The frame the state is read in and the frame the model emits loads in are the
/// same type parameter, which is what keeps a model from reading a quaternion in
/// one frame and reporting the result in another (`SimpleEci` and `Gcrs` differ
/// by ~484 arcsec at 2024).
///
/// TODO: `Eci` bound は地心慣性系に限定している。月周回や深宇宙では
/// より general な `Frame` bound が必要になる (別 milestone)。
pub trait HasFrame {
    /// Inertial frame of this state.
    type Frame: arika::frame::Eci;
}

/// State type that provides attitude information (quaternion + angular velocity).
pub trait HasAttitude: HasFrame {
    fn attitude(&self) -> &AttitudeState;

    /// Rotation body → [`HasFrame::Frame`].
    ///
    /// Prefer this over [`AttitudeState::rotation_tagged_as`]: the frame comes
    /// from the state's own type rather than from a caller-supplied parameter.
    fn attitude_to_inertial(&self) -> Rotation<Body, Self::Frame> {
        self.attitude().rotation_tagged_as()
    }

    /// Rotation [`HasFrame::Frame`] → body.
    fn attitude_from_inertial(&self) -> Rotation<Self::Frame, Body> {
        self.attitude()
            .rotation_tagged_as::<Self::Frame>()
            .inverse()
    }
}

/// State type that provides orbital information (position + velocity).
pub trait HasOrbit: HasFrame {
    fn orbit(&self) -> &OrbitalState<Self::Frame>;
}

/// State type that provides translational mass.
///
/// Inertia tensor is held by the dynamics system, not by the state.
pub trait HasMass {
    fn mass(&self) -> f64;
}

// ExternalLoads

/// Acceleration (inertial frame) and torque (body frame) pair.
///
/// Parameterized by the inertial frame `F` (default `SimpleEci`).
/// - acceleration: inertial frame [km/s²] (for translational EOM)
/// - torque: body frame [N·m] (for rotational EOM)
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalLoads<F: frame::Eci = frame::SimpleEci> {
    /// Translational acceleration in inertial frame [km/s²].
    pub acceleration_inertial: Vec3<F>,
    /// Torque in body frame [N·m].
    pub torque_body: Vec3<Body>,
    /// Mass rate [kg/s] (negative for depletion, e.g. propellant consumption).
    pub mass_rate: f64,
}

impl<F: frame::Eci> ExternalLoads<F> {
    pub fn zeros() -> Self {
        Self {
            acceleration_inertial: Vec3::zeros(),
            torque_body: Vec3::zeros(),
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only torque (body frame) [N·m].
    pub fn torque(t: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: Vec3::zeros(),
            torque_body: Vec3::from_raw(t),
            mass_rate: 0.0,
        }
    }

    /// Create an ExternalLoads with only translational acceleration (inertial frame) [km/s²].
    pub fn acceleration(a: Vector3<f64>) -> Self {
        Self {
            acceleration_inertial: Vec3::from_raw(a),
            torque_body: Vec3::zeros(),
            mass_rate: 0.0,
        }
    }
}

impl<F: frame::Eci> Add for ExternalLoads<F> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            acceleration_inertial: self.acceleration_inertial + rhs.acceleration_inertial,
            torque_body: self.torque_body + rhs.torque_body,
            mass_rate: self.mass_rate + rhs.mass_rate,
        }
    }
}

impl<F: frame::Eci> AddAssign for ExternalLoads<F> {
    fn add_assign(&mut self, rhs: Self) {
        self.acceleration_inertial += rhs.acceleration_inertial;
        self.torque_body += rhs.torque_body;
        self.mass_rate += rhs.mass_rate;
    }
}

// Unified Model trait

/// A physical model that evaluates external loads on a spacecraft.
///
/// The loads come back in the frame the state is propagated in
/// ([`HasFrame::Frame`]) — there is no separate frame parameter to disagree
/// with it, so a model cannot read a state in one frame and report the result
/// in another.
///
/// Models declare their state requirements via generic bounds:
/// - `impl<S: HasFrame + HasAttitude> Model<S>` — attitude-only (e.g., PD controller)
/// - `impl<S: HasFrame + HasOrbit> Model<S>` — orbit-only (e.g., constant thrust)
/// - `impl<S: HasFrame + HasAttitude + HasOrbit + HasMass> Model<S>` — full state (e.g., thruster)
///
/// A model that needs a *capability* of the frame binds its own `F` to the
/// state's: `impl<F: EarthFixedTransform, S: HasFrame<Frame = F> + HasOrbit>
/// Model<S> for AtmosphericDrag<F>`. That is the only place an equality bound
/// is needed, and there it says something — the model and the state agree.
///
/// `eval` must be a pure function with no side effects.
/// All models are evaluated against the same immutable state snapshot;
/// evaluation order must not affect results.
pub trait Model<S: HasFrame>: Send + Sync {
    /// Human-readable name for this model (e.g., "drag", "gravity_gradient").
    fn name(&self) -> &str;

    /// Evaluate the model at the given state and return external loads.
    ///
    /// `epoch` is the absolute time corresponding to integration time `t`.
    /// It is `None` when no initial epoch was provided.
    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<S::Frame>;
}

// Blanket impl so Box<dyn Model<S>> also satisfies Model<S>.
// This allows with_model() to accept both concrete types and boxed trait objects.
impl<S: HasFrame> Model<S> for Box<dyn Model<S>> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<S::Frame> {
        (**self).eval(t, state, epoch)
    }
}

#[cfg(test)]
mod external_loads_tests {
    use super::*;

    fn loads(ax: f64, ay: f64, az: f64, tx: f64, ty: f64, tz: f64, mr: f64) -> ExternalLoads {
        ExternalLoads {
            acceleration_inertial: Vec3::<SimpleEci>::new(ax, ay, az),
            torque_body: Vec3::<Body>::new(tx, ty, tz),
            mass_rate: mr,
        }
    }

    #[test]
    fn add_component_wise() {
        let a = loads(1.0, 2.0, 3.0, 0.1, 0.2, 0.3, -0.5);
        let b = loads(10.0, 20.0, 30.0, 1.0, 2.0, 3.0, -0.3);
        let sum = a + b;
        assert_eq!(
            sum.acceleration_inertial,
            Vec3::<SimpleEci>::new(11.0, 22.0, 33.0)
        );
        assert_eq!(sum.torque_body, Vec3::<Body>::new(1.1, 2.2, 3.3));
        assert!((sum.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_assign_component_wise() {
        let mut a = loads(1.0, 2.0, 3.0, 0.1, 0.2, 0.3, -0.5);
        let b = loads(10.0, 20.0, 30.0, 1.0, 2.0, 3.0, -0.3);
        a += b;
        assert_eq!(
            a.acceleration_inertial,
            Vec3::<SimpleEci>::new(11.0, 22.0, 33.0)
        );
        assert_eq!(a.torque_body, Vec3::<Body>::new(1.1, 2.2, 3.3));
        assert!((a.mass_rate - (-0.8)).abs() < 1e-15);
    }

    #[test]
    fn add_zeros_identity() {
        let w = loads(1.0, 2.0, 3.0, 0.1, 0.2, 0.3, -0.1);
        let sum = w.clone() + ExternalLoads::zeros();
        assert_eq!(sum, w);
    }
}
