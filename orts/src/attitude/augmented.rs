//! Augmented attitude dynamics system with StateEffector support.
//!
//! Extends [`DecoupledAttitudeSystem`] to support [`StateEffector`]s —
//! components with internal state (e.g., reaction wheels) that are
//! integrated alongside the attitude state.

use kaname::epoch::Epoch;
use nalgebra::{Matrix3, Vector3};
use utsuroi::DynamicalSystem;

use crate::OrbitalState;
use crate::attitude::DecoupledContext;
use crate::attitude::state::AttitudeState;
use crate::effector::{AugmentedState, AuxRegistry, StateEffector};
use crate::model::Model;
use crate::spacecraft::ExternalLoads;

/// Attitude dynamics with prescribed orbit, supporting both pure models
/// and state effectors.
///
/// Like [`DecoupledAttitudeSystem`](super::DecoupledAttitudeSystem), the orbit
/// and mass are prescribed via closures. Additionally, this system manages
/// [`StateEffector`]s whose auxiliary state is integrated alongside the
/// attitude quaternion and angular velocity.
///
/// The integrated state is [`AugmentedState<AttitudeState>`], where
/// `plant` holds the quaternion and angular velocity, and `aux` holds
/// the concatenated auxiliary variables from all registered effectors.
pub struct AugmentedAttitudeSystem {
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    models: Vec<Box<dyn Model<DecoupledContext>>>,
    effectors: Vec<Box<dyn StateEffector<DecoupledContext>>>,
    registry: AuxRegistry,
    orbit_fn: Box<dyn Fn(f64) -> OrbitalState + Send + Sync>,
    mass_fn: Box<dyn Fn(f64) -> f64 + Send + Sync>,
    epoch_0: Option<Epoch>,
}

impl AugmentedAttitudeSystem {
    /// Create a new augmented attitude system with the given inertia tensor,
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
            effectors: Vec::new(),
            registry: AuxRegistry::new(),
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

    /// Add a pure model (builder pattern).
    pub fn with_model(mut self, model: impl Model<DecoupledContext> + 'static) -> Self {
        self.models.push(Box::new(model));
        self
    }

    /// Add a state effector (builder pattern).
    ///
    /// The effector's auxiliary state is registered and will be integrated
    /// alongside the plant state.
    pub fn with_effector(
        mut self,
        effector: impl StateEffector<DecoupledContext> + 'static,
    ) -> Self {
        let dim = effector.state_dim();
        self.registry.register(effector.name(), dim);
        self.effectors.push(Box::new(effector));
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

    /// Create the initial auxiliary state vector (all zeros).
    pub fn initial_aux_state(&self) -> Vec<f64> {
        vec![0.0; self.registry.total_dim()]
    }

    /// Collect the concatenated aux bounds from all registered effectors.
    pub fn initial_aux_bounds(&self) -> Vec<(f64, f64)> {
        let mut bounds = Vec::with_capacity(self.registry.total_dim());
        for eff in &self.effectors {
            bounds.extend(eff.aux_bounds());
        }
        bounds
    }

    /// Create an initial [`AugmentedState`] with the given plant state,
    /// zero auxiliary state, and correct bounds from registered effectors.
    pub fn initial_augmented_state(&self, plant: AttitudeState) -> AugmentedState<AttitudeState> {
        AugmentedState {
            plant,
            aux: self.initial_aux_state(),
            aux_bounds: self.initial_aux_bounds(),
        }
    }

    /// Downcast a state effector to a concrete type for command updates.
    ///
    /// Use this between integration segments to update effector commands
    /// (e.g., `ReactionWheelAssembly::commanded_torque`).
    pub fn effector_mut<T: StateEffector<DecoupledContext> + 'static>(
        &mut self,
        index: usize,
    ) -> Option<&mut T> {
        self.effectors.get_mut(index).and_then(|e| {
            let any = e.as_mut() as &mut dyn std::any::Any;
            any.downcast_mut::<T>()
        })
    }

    /// Get the auxiliary state registry.
    pub fn registry(&self) -> &AuxRegistry {
        &self.registry
    }
}

impl DynamicalSystem for AugmentedAttitudeSystem {
    type State = AugmentedState<AttitudeState>;

    fn derivatives(
        &self,
        t: f64,
        state: &AugmentedState<AttitudeState>,
    ) -> AugmentedState<AttitudeState> {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));

        // 0. Validate auxiliary state length
        assert_eq!(
            state.aux.len(),
            self.registry.total_dim(),
            "Auxiliary state length ({}) does not match registry ({})",
            state.aux.len(),
            self.registry.total_dim()
        );

        // 1. Construct context with prescribed orbit and mass
        let context = DecoupledContext {
            attitude: state.plant.clone(),
            orbit: (self.orbit_fn)(t),
            mass: (self.mass_fn)(t),
        };

        // 2. Evaluate continuous models
        let mut total = ExternalLoads::zeros();
        for m in &self.models {
            total += m.eval(t, &context, epoch.as_ref());
        }

        // 3. Evaluate state effectors
        let mut aux_rates = vec![0.0; self.registry.total_dim()];
        for (i, eff) in self.effectors.iter().enumerate() {
            let entry = &self.registry.entries()[i];
            let aux_slice = &state.aux[entry.offset..entry.offset + entry.dim];
            let rates_slice = &mut aux_rates[entry.offset..entry.offset + entry.dim];
            total += eff.derivatives(t, &context, aux_slice, rates_slice, epoch.as_ref());
        }

        // 4. Warn if models produce translational forces or mass changes (ignored here)
        if total.acceleration_inertial.magnitude() > 1e-15 {
            log::warn!(
                "AugmentedAttitudeSystem ignoring non-zero acceleration_inertial: {:?}",
                total.acceleration_inertial
            );
        }
        if total.mass_rate.abs() > 1e-15 {
            log::warn!(
                "AugmentedAttitudeSystem ignoring non-zero mass_rate: {}",
                total.mass_rate
            );
        }

        // 5. Quaternion kinematics: dq/dt = 0.5 * q ⊗ (0, ω)
        let q_dot = state.plant.q_dot();

        // 5. Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.plant.angular_velocity;
        let alpha =
            self.inertia_inv * (total.torque_body - state.plant.angular_velocity.cross(&iw));

        AugmentedState {
            plant: AttitudeState::from_derivative(q_dot, alpha),
            aux: aux_rates,
            aux_bounds: state.aux_bounds.clone(),
        }
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
    fn torque_free_symmetric_body_zero_acceleration() {
        let system = AugmentedAttitudeSystem::circular_orbit(
            symmetric_inertia(10.0),
            398600.4418,
            7000.0,
            100.0,
        );
        let state = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            aux: vec![],
            aux_bounds: vec![],
        };
        let deriv = system.derivatives(0.0, &state);
        // For symmetric body: ω × (I·ω) = I * (ω × ω) = 0
        assert!(deriv.plant.angular_velocity.magnitude() < 1e-15);
        assert!(deriv.aux.is_empty());
    }

    #[test]
    fn builder_with_epoch() {
        let epoch = Epoch::from_jd(2451545.0);
        let system = AugmentedAttitudeSystem::circular_orbit(symmetric_inertia(1.0), 1.0, 1.0, 1.0)
            .with_epoch(epoch);
        assert!(system.epoch_0.is_some());
    }

    #[test]
    fn initial_aux_state_empty_when_no_effectors() {
        let system = AugmentedAttitudeSystem::circular_orbit(symmetric_inertia(1.0), 1.0, 1.0, 1.0);
        let aux = system.initial_aux_state();
        assert!(aux.is_empty());
    }
}
