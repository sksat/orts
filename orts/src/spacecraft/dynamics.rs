use crate::effector::{AugmentedState, AuxRegistry, StateEffector};
use crate::model::Model;
use crate::orbital::gravity::GravityField;
use kaname::epoch::Epoch;
use nalgebra::Matrix3;
use utsuroi::DynamicalSystem;

use super::{ExternalLoads, SpacecraftState};

/// Coupled orbit-attitude dynamics for a rigid spacecraft.
///
/// Composes a gravitational field, inertia tensor, external load models,
/// and state effectors (e.g. reaction wheels) into a [`DynamicalSystem`]
/// for the augmented spacecraft state.
///
/// The state type is `AugmentedState<SpacecraftState>` — the 14D plant
/// state (orbit 6D + attitude 7D + mass 1D) plus concatenated auxiliary
/// variables from registered [`StateEffector`]s (e.g. RW angular
/// momentum). When no effectors are registered, `aux` is empty and the
/// dynamics are equivalent to the pre-effector version.
///
/// Equations of motion:
/// - Translation: dr/dt = v, dv/dt = a_gravity + Σ a_loads
/// - Rotation: dq/dt = ½ q ⊗ (0,ω), dω/dt = I⁻¹(τ − ω × Iω)
/// - Auxiliary: daux/dt from registered effectors
pub struct SpacecraftDynamics<G: GravityField> {
    mu: f64,
    gravity: G,
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    models: Vec<Box<dyn Model<SpacecraftState>>>,
    effectors: Vec<Box<dyn StateEffector<SpacecraftState>>>,
    registry: AuxRegistry,
    epoch_0: Option<Epoch>,
    body_radius: Option<f64>,
}

impl<G: GravityField> SpacecraftDynamics<G> {
    /// Create with gravitational parameter, gravity model, and inertia tensor.
    ///
    /// # Panics
    /// Panics if `inertia` is singular (not invertible).
    pub fn new(mu: f64, gravity: G, inertia: Matrix3<f64>) -> Self {
        let inertia_inv = inertia
            .try_inverse()
            .expect("Inertia tensor must be invertible");
        Self {
            mu,
            gravity,
            inertia,
            inertia_inv,
            models: Vec::new(),
            effectors: Vec::new(),
            registry: AuxRegistry::new(),
            epoch_0: None,
            body_radius: None,
        }
    }

    /// Add an external model (builder pattern).
    pub fn with_model(mut self, model: impl Model<SpacecraftState> + 'static) -> Self {
        self.models.push(Box::new(model));
        self
    }

    /// Add a state effector (builder pattern).
    ///
    /// Effectors have auxiliary state (e.g. RW angular momentum) that
    /// is integrated alongside the plant state.
    pub fn with_effector(
        mut self,
        effector: impl StateEffector<SpacecraftState> + 'static,
    ) -> Self {
        let dim = effector.state_dim();
        self.registry.register(effector.name(), dim);
        self.effectors.push(Box::new(effector));
        self
    }

    /// Set the initial epoch corresponding to integration time t = 0.
    pub fn with_epoch(mut self, epoch: Epoch) -> Self {
        self.epoch_0 = Some(epoch);
        self
    }

    /// Set the central body radius for event detection.
    pub fn with_body_radius(mut self, radius: f64) -> Self {
        self.body_radius = Some(radius);
        self
    }

    /// Create an initial augmented state with the given plant state.
    ///
    /// Auxiliary state is initialized to zeros; bounds are collected
    /// from all registered effectors.
    pub fn initial_augmented_state(
        &self,
        plant: SpacecraftState,
    ) -> AugmentedState<SpacecraftState> {
        let mut bounds = Vec::with_capacity(self.registry.total_dim());
        for eff in &self.effectors {
            bounds.extend(eff.aux_bounds());
        }
        AugmentedState {
            plant,
            aux: vec![0.0; self.registry.total_dim()],
            aux_bounds: bounds,
        }
    }

    /// Downcast a state effector to a concrete type for mutation between
    /// integration segments (e.g. updating `commanded_torque` on a
    /// `ReactionWheelAssembly`).
    pub fn effector_mut<T: StateEffector<SpacecraftState> + 'static>(
        &mut self,
        index: usize,
    ) -> Option<&mut T> {
        self.effectors
            .get_mut(index)
            .and_then(|e| (e.as_mut() as &mut dyn std::any::Any).downcast_mut::<T>())
    }

    /// Get the auxiliary state registry.
    pub fn registry(&self) -> &AuxRegistry {
        &self.registry
    }

    /// Get the inertia tensor.
    pub fn inertia(&self) -> &Matrix3<f64> {
        &self.inertia
    }

    /// Get the central body radius (if set).
    pub fn body_radius(&self) -> Option<f64> {
        self.body_radius
    }

    /// Names of active models.
    pub fn model_names(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.name()).collect()
    }

    /// Per-model load breakdown at the given state.
    pub fn model_breakdown(&self, t: f64, state: &SpacecraftState) -> Vec<(&str, ExternalLoads)> {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));
        self.models
            .iter()
            .map(|m| (m.name(), m.eval(t, state, epoch.as_ref())))
            .collect()
    }

    /// Acceleration breakdown for telemetry.
    pub fn acceleration_breakdown(&self, t: f64, state: &SpacecraftState) -> Vec<(&str, f64)> {
        let grav = self
            .gravity
            .acceleration(self.mu, state.orbit.position())
            .magnitude();
        let mut result = vec![("gravity", grav)];
        for (name, loads) in self.model_breakdown(t, state) {
            result.push((name, loads.acceleration_inertial.magnitude()));
        }
        result
    }
}

impl<G: GravityField> DynamicalSystem for SpacecraftDynamics<G> {
    type State = AugmentedState<SpacecraftState>;

    fn derivatives(
        &self,
        t: f64,
        state: &AugmentedState<SpacecraftState>,
    ) -> AugmentedState<SpacecraftState> {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));

        // Gravitational acceleration
        let grav_accel = self
            .gravity
            .acceleration(self.mu, state.plant.orbit.position());

        // Accumulate external loads from models
        let mut total = ExternalLoads::zeros();
        for model in &self.models {
            total += model.eval(t, &state.plant, epoch.as_ref());
        }

        // Evaluate state effectors
        let mut aux_rates = vec![0.0; self.registry.total_dim()];
        for (i, eff) in self.effectors.iter().enumerate() {
            let entry = &self.registry.entries()[i];
            let aux_slice = &state.aux[entry.offset..entry.offset + entry.dim];
            let rates_slice = &mut aux_rates[entry.offset..entry.offset + entry.dim];
            total += eff.derivatives(t, &state.plant, aux_slice, rates_slice, epoch.as_ref());
        }

        // Total translational acceleration
        let total_accel = grav_accel + total.acceleration_inertial;

        // Quaternion kinematics: dq/dt = ½ q ⊗ (0, ω)
        let q_dot = state.plant.attitude.q_dot();

        // Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.plant.attitude.angular_velocity;
        let alpha = self.inertia_inv
            * (total.torque_body - state.plant.attitude.angular_velocity.cross(&iw));

        AugmentedState {
            plant: SpacecraftState::from_derivative(
                *state.plant.orbit.velocity(),
                total_accel,
                q_dot,
                alpha,
                total.mass_rate,
            ),
            aux: aux_rates,
            aux_bounds: state.aux_bounds.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use crate::model::Model;
    use crate::orbital::OrbitalSystem;
    use crate::orbital::gravity::PointMass;
    use kaname::constants::MU_EARTH;
    use nalgebra::{Vector3, Vector4};
    use utsuroi::{Integrator, OdeState, Rk4};

    // --- Helpers ---

    fn symmetric_inertia(i: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(i, i, i))
    }

    fn sample_orbit() -> OrbitalState {
        OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0))
    }

    fn sample_spacecraft() -> SpacecraftState {
        SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    /// Wrap a plant state as an augmented state with no effectors.
    fn augment(plant: SpacecraftState) -> AugmentedState<SpacecraftState> {
        AugmentedState {
            plant,
            aux: vec![],
            aux_bounds: vec![],
        }
    }

    // --- Mock models ---

    struct ConstantAcceleration(Vector3<f64>);

    impl Model<SpacecraftState> for ConstantAcceleration {
        fn name(&self) -> &str {
            "const_force"
        }
        fn eval(&self, _t: f64, _state: &SpacecraftState, _epoch: Option<&Epoch>) -> ExternalLoads {
            ExternalLoads::acceleration(self.0)
        }
    }

    struct ConstantTorqueModel(Vector3<f64>);

    impl Model<SpacecraftState> for ConstantTorqueModel {
        fn name(&self) -> &str {
            "const_torque"
        }
        fn eval(&self, _t: f64, _state: &SpacecraftState, _epoch: Option<&Epoch>) -> ExternalLoads {
            ExternalLoads::torque(self.0)
        }
    }

    struct EpochSensitiveLoad;

    impl Model<SpacecraftState> for EpochSensitiveLoad {
        fn name(&self) -> &str {
            "epoch_sensitive"
        }
        fn eval(&self, _t: f64, _state: &SpacecraftState, epoch: Option<&Epoch>) -> ExternalLoads {
            match epoch {
                Some(e) => ExternalLoads {
                    acceleration_inertial: Vector3::new(e.jd() * 1e-10, 0.0, 0.0),
                    torque_body: Vector3::zeros(),
                    mass_rate: 0.0,
                },
                None => ExternalLoads::zeros(),
            }
        }
    }

    // ======== Step 1: Gravity only (no LoadModel) ========

    #[test]
    fn gravity_only_matches_orbital_system() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let dyn_orb = OrbitalSystem::new(MU_EARTH, Box::new(PointMass));

        let d_sc = dyn_sc.derivatives(0.0, &augment(sc.clone()));
        let d_orb = dyn_orb.derivatives(0.0, &sc.orbit);

        assert!((d_sc.plant.orbit.velocity() - d_orb.velocity()).magnitude() < 1e-15);
    }

    #[test]
    fn gravity_only_velocity_derivative() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        assert_eq!(*d.plant.orbit.position(), *sc.orbit.velocity());
    }

    #[test]
    fn torque_free_symmetric_inertia() {
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            mass: 500.0,
        };
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc));

        assert!(d.plant.attitude.angular_velocity.magnitude() < 1e-15);
    }

    #[test]
    fn mass_rate_always_zero() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc));
        assert_eq!(d.plant.mass, 0.0);
    }

    // ======== Step 2: Euler equation ========

    #[test]
    fn euler_diagonal_inertia_known_torque() {
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let torque = Vector3::new(1.0, 2.0, 3.0);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_model(ConstantTorqueModel(torque));

        let d = dyn_sc.derivatives(0.0, &augment(sc));

        let expected_alpha = Vector3::new(0.1, 0.1, 0.1);
        assert!((d.plant.attitude.angular_velocity - expected_alpha).magnitude() < 1e-14);
    }

    #[test]
    fn euler_gyroscopic_term() {
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 1.0, 0.0),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &augment(sc));

        let expected_alpha = Vector3::new(0.0, 0.0, -1.0 / 3.0);
        assert!(
            (d.plant.attitude.angular_velocity - expected_alpha).magnitude() < 1e-14,
            "Expected α = {expected_alpha:?}, got {:?}",
            d.plant.attitude.angular_velocity
        );
    }

    #[test]
    fn euler_non_diagonal_inertia() {
        let inertia = Matrix3::new(4.0, 1.0, 0.0, 1.0, 4.0, 0.0, 0.0, 0.0, 6.0);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 0.0, 1.0),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &augment(sc));

        let expected_alpha = Vector3::new(2.0 / 15.0, 7.0 / 15.0, -1.0 / 6.0);
        assert!(
            (d.plant.attitude.angular_velocity - expected_alpha).magnitude() < 1e-13,
            "Expected α = {expected_alpha:?}, got {:?}",
            d.plant.attitude.angular_velocity
        );
    }

    // ======== Step 3: Model integration ========

    #[test]
    fn model_adds_acceleration() {
        let accel = Vector3::new(1e-6, 2e-6, 3e-6);
        let sc = sample_spacecraft();

        let dyn_with = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(accel));
        let d_with = dyn_with.derivatives(0.0, &augment(sc.clone()));

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &augment(sc));

        let diff = d_with.plant.orbit.velocity() - d_grav.plant.orbit.velocity();
        assert!((diff - accel).magnitude() < 1e-15);
    }

    #[test]
    fn model_adds_torque() {
        let torque = Vector3::new(0.01, 0.02, 0.03);
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantTorqueModel(torque));

        let d = dyn_sc.derivatives(0.0, &augment(sc));

        let expected_alpha = torque / 10.0;
        assert!((d.plant.attitude.angular_velocity - expected_alpha).magnitude() < 1e-15);
    }

    #[test]
    fn multiple_models_accumulate() {
        let accel = Vector3::new(1e-6, 0.0, 0.0);
        let torque = Vector3::new(0.0, 0.01, 0.0);
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(accel))
            .with_model(ConstantTorqueModel(torque));
        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &augment(sc));

        let accel_diff = d.plant.orbit.velocity() - d_grav.plant.orbit.velocity();
        assert!((accel_diff - accel).magnitude() < 1e-15);
        assert!((d.plant.attitude.angular_velocity - torque / 10.0).magnitude() < 1e-15);
    }

    // ======== Step 4: Builder + telemetry ========

    #[test]
    fn builder_with_model_epoch_body_radius() {
        let epoch = Epoch::from_jd(2460000.5);
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(Vector3::zeros()))
            .with_epoch(epoch)
            .with_body_radius(6378.137);

        assert_eq!(dyn_sc.models.len(), 1);
        assert_eq!(dyn_sc.epoch_0, Some(epoch));
        assert_eq!(dyn_sc.body_radius, Some(6378.137));
    }

    #[test]
    fn model_names_returns_all() {
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(Vector3::zeros()))
            .with_model(ConstantTorqueModel(Vector3::zeros()));

        let names = dyn_sc.model_names();
        assert_eq!(names, vec!["const_force", "const_torque"]);
    }

    #[test]
    fn model_breakdown_per_model() {
        let accel = Vector3::new(1e-6, 0.0, 0.0);
        let torque = Vector3::new(0.0, 0.01, 0.0);
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(accel))
            .with_model(ConstantTorqueModel(torque));

        let breakdown = dyn_sc.model_breakdown(0.0, &sc);
        assert_eq!(breakdown.len(), 2);
        assert_eq!(breakdown[0].0, "const_force");
        assert_eq!(breakdown[0].1.acceleration_inertial, accel);
        assert_eq!(breakdown[0].1.torque_body, Vector3::zeros());
        assert_eq!(breakdown[1].0, "const_torque");
        assert_eq!(breakdown[1].1.acceleration_inertial, Vector3::zeros());
        assert_eq!(breakdown[1].1.torque_body, torque);
    }

    // ======== Step 5: Epoch + integration + edge cases ========

    #[test]
    fn epoch_forwarded_to_loads() {
        let epoch = Epoch::from_jd(2460000.5);
        let t = 100.0;
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(EpochSensitiveLoad)
            .with_epoch(epoch);

        let d = dyn_sc.derivatives(t, &augment(sc.clone()));

        let expected_epoch = epoch.add_seconds(t);
        let expected_accel_x = expected_epoch.jd() * 1e-10;

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(t, &augment(sc));
        let diff_x = d.plant.orbit.velocity()[0] - d_grav.plant.orbit.velocity()[0];

        let rel_err = (diff_x - expected_accel_x).abs() / expected_accel_x.abs();
        assert!(
            rel_err < 1e-14,
            "Epoch not forwarded correctly: diff_x={diff_x}, expected={expected_accel_x}, rel_err={rel_err:.3e}"
        );
    }

    #[test]
    fn epoch_none_no_panic() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(EpochSensitiveLoad);

        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &augment(sc));

        assert!((d.plant.orbit.velocity() - d_grav.plant.orbit.velocity()).magnitude() < 1e-15);
    }

    #[test]
    fn integrable_with_rk4() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let result = Rk4.integrate(&dyn_sc, augment(sc), 0.0, 60.0, 10.0, |_, _| {});

        assert!(result.plant.orbit.position().magnitude() > 0.0);
        assert!(result.is_finite());
    }

    #[test]
    #[should_panic(expected = "Inertia tensor must be invertible")]
    fn singular_inertia_panics() {
        let _dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, Matrix3::zeros());
    }

    // ======== Step 6: Derivative-level conservation laws ========

    #[test]
    fn derivative_preserves_two_body_energy() {
        let sc = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(7000.0, 1000.0, 500.0),
                Vector3::new(-1.0, 7.0, 0.5),
            ),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        let r = sc.orbit.position();
        let v = sc.orbit.velocity();
        let a = d.plant.orbit.velocity();
        let r_mag = r.magnitude();

        let de_dt = v.dot(a) + MU_EARTH / (r_mag.powi(3)) * r.dot(v);
        assert!(de_dt.abs() < 1e-12, "dE/dt should be ≈ 0, got {de_dt:.3e}");
    }

    #[test]
    fn derivative_preserves_angular_momentum() {
        let sc = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(7000.0, 1000.0, 500.0),
                Vector3::new(-1.0, 7.0, 0.5),
            ),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        let r = sc.orbit.position();
        let a = d.plant.orbit.velocity();
        let dl_dt = r.cross(a);

        assert!(
            dl_dt.magnitude() < 1e-12,
            "dL/dt should be ≈ 0, got magnitude {:.3e}",
            dl_dt.magnitude()
        );
    }

    #[test]
    fn derivative_preserves_rotational_energy() {
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let omega = Vector3::new(0.1, 0.2, 0.3);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: omega,
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &augment(sc));

        let alpha = &d.plant.attitude.angular_velocity;
        let dt_rot = omega.dot(&(inertia * alpha));

        assert!(
            dt_rot.abs() < 1e-14,
            "dT_rot/dt should be ≈ 0, got {dt_rot:.3e}"
        );
    }

    #[test]
    fn derivative_preserves_quaternion_norm() {
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &augment(sc.clone()));

        let q = &sc.attitude.quaternion;
        let q_dot = &d.plant.attitude.quaternion;
        let d_norm_sq = 2.0 * q.dot(q_dot);

        assert!(
            d_norm_sq.abs() < 1e-15,
            "d/dt(|q|²) should be ≈ 0, got {d_norm_sq:.3e}"
        );
    }

    // ======== Step 7: StateEffector integration ========

    #[test]
    fn with_effector_registers_aux() {
        use crate::spacecraft::ReactionWheelAssembly;
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
        let dyn_sc =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);

        assert_eq!(dyn_sc.registry().total_dim(), 3);
        let state = dyn_sc.initial_augmented_state(sample_spacecraft());
        assert_eq!(state.aux.len(), 3);
        assert_eq!(state.aux, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn effector_mut_downcasts() {
        use crate::spacecraft::ReactionWheelAssembly;
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
        let mut dyn_sc =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);

        let rw_ref = dyn_sc
            .effector_mut::<ReactionWheelAssembly>(0)
            .expect("should downcast");
        rw_ref.commanded_torque = Vector3::new(0.1, 0.0, 0.0);
    }

    #[test]
    fn rw_effector_integrates_with_spacecraft() {
        use crate::spacecraft::ReactionWheelAssembly;
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
        let mut dyn_sc =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);

        // Command a small torque on the x-axis.
        dyn_sc
            .effector_mut::<ReactionWheelAssembly>(0)
            .unwrap()
            .commanded_torque = Vector3::new(0.01, 0.0, 0.0);

        let state = dyn_sc.initial_augmented_state(sample_spacecraft());
        let result = Rk4.integrate(&dyn_sc, state, 0.0, 10.0, 0.1, |_, _| {});

        // RW x-axis wheel should have accumulated momentum.
        assert!(result.aux[0].abs() > 0.01, "RW momentum should change");
        // Spacecraft should have reacted (angular velocity change).
        assert!(
            result.plant.attitude.angular_velocity.magnitude() > 1e-6,
            "spacecraft should react to RW torque"
        );
        assert!(result.is_finite());
    }
}
