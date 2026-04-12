use arika::epoch::Epoch;
use nalgebra::{Matrix3, Vector3};
use utsuroi::DynamicalSystem;

use crate::OrbitalState;
use crate::attitude::AttitudeState;
use crate::model::ExternalLoads;
use crate::model::{HasAttitude, HasMass, HasOrbit, Model};

/// Combined state providing attitude, orbit, and mass for decoupled models.
///
/// In decoupled attitude propagation, the orbit is prescribed (not integrated),
/// so models that need orbit information can access it through this context.
pub struct DecoupledContext {
    pub attitude: AttitudeState,
    pub orbit: OrbitalState,
    pub mass: f64,
}

impl HasAttitude for DecoupledContext {
    fn attitude(&self) -> &AttitudeState {
        &self.attitude
    }
}

impl HasOrbit for DecoupledContext {
    type Frame = arika::frame::SimpleEci;

    fn orbit(&self) -> &OrbitalState<arika::frame::SimpleEci> {
        &self.orbit
    }
}

impl HasMass for DecoupledContext {
    fn mass(&self) -> f64 {
        self.mass
    }
}

/// Attitude dynamics system with prescribed orbit for decoupled propagation.
///
/// Unlike [`AttitudeSystem`](super::AttitudeSystem), this system provides orbit
/// and mass information to models via [`DecoupledContext`], enabling models that
/// require `HasOrbit` or `HasMass` (e.g., tracking PD controllers, aerodynamic
/// torques) to be used in attitude-only propagation.
///
/// The orbit and mass are prescribed via closures rather than being integrated.
pub struct DecoupledAttitudeSystem {
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    models: Vec<Box<dyn Model<DecoupledContext>>>,
    orbit_fn: Box<dyn Fn(f64) -> OrbitalState + Send + Sync>,
    mass_fn: Box<dyn Fn(f64) -> f64 + Send + Sync>,
    epoch_0: Option<Epoch>,
}

impl DecoupledAttitudeSystem {
    /// Create a new decoupled attitude system with the given inertia tensor,
    /// orbit function, and mass function.
    pub fn new(
        inertia: Matrix3<f64>,
        orbit_fn: impl Fn(f64) -> OrbitalState + Send + Sync + 'static,
        mass_fn: impl Fn(f64) -> f64 + Send + Sync + 'static,
    ) -> Self {
        let inertia_inv = inertia
            .try_inverse()
            .expect("Inertia tensor must be invertible");
        Self {
            inertia,
            inertia_inv,
            models: Vec::new(),
            orbit_fn: Box::new(orbit_fn),
            mass_fn: Box::new(mass_fn),
            epoch_0: None,
        }
    }

    /// Create a system for a circular orbit in the x-y plane with constant mass.
    ///
    /// Convenience constructor that generates the orbit function from
    /// gravitational parameter `mu` and orbit `radius`.
    pub fn circular_orbit(inertia: Matrix3<f64>, mu: f64, radius: f64, mass: f64) -> Self {
        let n = (mu / radius.powi(3)).sqrt(); // mean motion
        let v = (mu / radius).sqrt(); // circular velocity
        Self::new(
            inertia,
            move |t| {
                let theta = n * t;
                OrbitalState::new(
                    Vector3::new(radius * theta.cos(), radius * theta.sin(), 0.0),
                    Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0),
                )
            },
            move |_| mass,
        )
    }

    /// Add a model (builder pattern).
    pub fn with_model(mut self, model: impl Model<DecoupledContext> + 'static) -> Self {
        self.models.push(Box::new(model));
        self
    }

    /// Set the initial epoch for time-dependent models.
    pub fn with_epoch(mut self, epoch: Epoch) -> Self {
        self.epoch_0 = Some(epoch);
        self
    }

    /// Get the inertia tensor.
    pub fn inertia(&self) -> &Matrix3<f64> {
        &self.inertia
    }

    /// Get the names of all active models.
    pub fn model_names(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.name()).collect()
    }
}

impl DynamicalSystem for DecoupledAttitudeSystem {
    type State = AttitudeState;

    fn derivatives(&self, t: f64, state: &AttitudeState) -> AttitudeState {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));

        // 1. Construct context with prescribed orbit and mass
        let context = DecoupledContext {
            attitude: state.clone(),
            orbit: (self.orbit_fn)(t),
            mass: (self.mass_fn)(t),
        };

        // 2. Quaternion kinematics: dq/dt = 0.5 * q ⊗ (0, ω)
        let q_dot = state.q_dot();

        // 3. Total loads from all models
        let mut total = ExternalLoads::zeros();
        for m in &self.models {
            total += m.eval(t, &context, epoch.as_ref());
        }

        // 4. Warn if models produce translational forces or mass changes (ignored here)
        if total.acceleration_inertial.magnitude() > 1e-15 {
            log::warn!(
                "DecoupledAttitudeSystem ignoring non-zero acceleration_inertial: {:?}",
                total.acceleration_inertial
            );
        }
        if total.mass_rate.abs() > 1e-15 {
            log::warn!(
                "DecoupledAttitudeSystem ignoring non-zero mass_rate: {}",
                total.mass_rate
            );
        }

        // 5. Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.angular_velocity;
        let alpha =
            self.inertia_inv * (total.torque_body.into_inner() - state.angular_velocity.cross(&iw));

        AttitudeState::from_derivative(q_dot, alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector4;

    fn symmetric_inertia(i: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(i, i, i))
    }

    #[test]
    fn decoupled_context_has_attitude() {
        let ctx = DecoupledContext {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            mass: 100.0,
        };
        assert_eq!(ctx.attitude().angular_velocity, Vector3::zeros());
    }

    #[test]
    fn decoupled_context_has_orbit() {
        let ctx = DecoupledContext {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            mass: 100.0,
        };
        assert_eq!(*ctx.orbit().position(), Vector3::new(7000.0, 0.0, 0.0));
    }

    #[test]
    fn decoupled_context_has_mass() {
        let ctx = DecoupledContext {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            mass: 42.0,
        };
        assert!((ctx.mass() - 42.0).abs() < 1e-15);
    }

    #[test]
    fn torque_free_symmetric_body_zero_acceleration() {
        let system = DecoupledAttitudeSystem::circular_orbit(
            symmetric_inertia(10.0),
            398600.4418,
            7000.0,
            100.0,
        );
        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.1, 0.2, 0.3),
        };
        let deriv = system.derivatives(0.0, &state);
        // For symmetric body: ω × (I·ω) = I * (ω × ω) = 0
        assert!(deriv.angular_velocity.magnitude() < 1e-15);
    }

    #[test]
    fn circular_orbit_position_at_t0() {
        let mu = 398600.4418;
        let r = 7000.0;
        let system = DecoupledAttitudeSystem::circular_orbit(symmetric_inertia(10.0), mu, r, 100.0);

        // At t=0, orbit_fn should return (r, 0, 0)
        let orbit = (system.orbit_fn)(0.0);
        assert!((orbit.position()[0] - r).abs() < 1e-10);
        assert!(orbit.position()[1].abs() < 1e-10);
        assert!(orbit.position()[2].abs() < 1e-10);
    }

    #[test]
    fn model_names_empty() {
        let system = DecoupledAttitudeSystem::circular_orbit(symmetric_inertia(1.0), 1.0, 1.0, 1.0);
        assert!(system.model_names().is_empty());
    }

    #[test]
    fn builder_with_epoch() {
        let epoch = Epoch::from_jd(2451545.0);
        let system = DecoupledAttitudeSystem::circular_orbit(symmetric_inertia(1.0), 1.0, 1.0, 1.0)
            .with_epoch(epoch);
        assert!(system.epoch_0.is_some());
    }
}
