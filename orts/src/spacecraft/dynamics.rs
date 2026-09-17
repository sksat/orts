use std::marker::PhantomData;

use crate::boundary::{DeclaredBoundary, HasBoundaries};
use crate::effector::{AugmentedState, AuxRegistry, ConstraintMode, EffectorInput, StateEffector};
use crate::model::{EvalSegment, Model, eval_maybe_in_segment};
use crate::orbital::gravity::GravityField;
use arika::epoch::Epoch;
use arika::frame::{Eci, SimpleEci};
use nalgebra::Matrix3;
use utsuroi::{DynamicalSystem, SegmentContext};

use super::{ExternalLoads, SpacecraftState};

/// Coupled orbit-attitude dynamics for a rigid spacecraft.
///
/// Composes a gravitational field, inertia tensor, external load models,
/// and state effectors (e.g. reaction wheels) into a [`DynamicalSystem`]
/// for the augmented spacecraft state.
///
/// Parameterized by the inertial frame `F` (default `SimpleEci`),
/// matching [`SpacecraftState<F>`].
///
/// The state type is `AugmentedState<SpacecraftState<F>>` — the 14D plant
/// state (orbit 6D + attitude 7D + mass 1D) plus concatenated auxiliary
/// variables from registered [`StateEffector`]s (e.g. RW angular
/// momentum). When no effectors are registered, `aux` is empty and the
/// dynamics are equivalent to the pre-effector version.
///
/// Equations of motion:
/// - Translation: dr/dt = v, dv/dt = a_gravity + Σ a_loads
/// - Rotation: dq/dt = ½ q ⊗ (0,ω), dω/dt = I⁻¹(τ − ω × Iω)
/// - Auxiliary: daux/dt from registered effectors
pub struct SpacecraftDynamics<G: GravityField, F: Eci = SimpleEci> {
    mu: f64,
    gravity: G,
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    models: Vec<Box<dyn Model<SpacecraftState<F>>>>,
    effectors: Vec<Box<dyn StateEffector<SpacecraftState<F>>>>,
    registry: AuxRegistry,
    epoch_0: Option<Epoch>,
    body_radius: Option<f64>,
    _frame: PhantomData<F>,
}

/// What every model is doing to a spacecraft at one state.
///
/// Answered by [`SpacecraftDynamics::load_breakdown`] from a single evaluation
/// of every model, for a caller that wants both halves — telemetry reporting
/// one sample.
pub struct LoadBreakdown<'a> {
    /// Acceleration magnitudes [km/s²], with the gravity field first and then
    /// one entry per model.
    pub accelerations: Vec<(&'a str, f64)>,
    /// Body-frame torques [N·m], one entry per model.
    pub torques: Vec<(&'a str, arika::frame::Vec3<arika::frame::Body>)>,
}

impl<G: GravityField, F: Eci + 'static> SpacecraftDynamics<G, F> {
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
            _frame: PhantomData,
        }
    }

    /// Add an external model (builder pattern).
    pub fn with_model(mut self, model: impl Model<SpacecraftState<F>> + 'static) -> Self {
        self.models.push(Box::new(model));
        self
    }

    /// Add a state effector (builder pattern).
    ///
    /// Effectors have auxiliary state (e.g. RW angular momentum) that
    /// is integrated alongside the plant state. The effector must produce
    /// its loads in this system's inertial frame `F` (its
    /// [`StateEffector::derivatives`] returns [`ExternalLoads<F>`]), so the
    /// dynamics never re-tag coordinates. See issue #103.
    pub fn with_effector(
        mut self,
        effector: impl StateEffector<SpacecraftState<F>> + 'static,
    ) -> Self {
        let dim = effector.state_dim();
        self.registry
            .register(effector.name(), dim, effector.mode_dim());
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
        plant: SpacecraftState<F>,
    ) -> AugmentedState<SpacecraftState<F>> {
        let mut bounds = Vec::with_capacity(self.registry.total_dim());
        for eff in &self.effectors {
            bounds.extend(eff.aux_bounds());
        }
        AugmentedState {
            plant,
            aux: vec![0.0; self.registry.total_dim()],
            aux_bounds: bounds,
            modes: vec![ConstraintMode::default(); self.registry.total_modes()],
        }
    }

    /// Downcast a state effector by index (immutable).
    pub fn effector<T: StateEffector<SpacecraftState<F>> + 'static>(
        &self,
        index: usize,
    ) -> Option<&T> {
        self.effectors
            .get(index)
            .and_then(|e| (e.as_ref() as &dyn std::any::Any).downcast_ref::<T>())
    }

    /// Downcast a state effector by index (mutable).
    pub fn effector_mut<T: StateEffector<SpacecraftState<F>> + 'static>(
        &mut self,
        index: usize,
    ) -> Option<&mut T> {
        self.effectors
            .get_mut(index)
            .and_then(|e| (e.as_mut() as &mut dyn std::any::Any).downcast_mut::<T>())
    }

    /// Find and downcast a state effector by name (immutable).
    pub fn effector_by_name<T: StateEffector<SpacecraftState<F>> + 'static>(
        &self,
        name: &str,
    ) -> Option<&T> {
        let idx = self
            .registry
            .entries()
            .iter()
            .position(|e| e.name == name)?;
        self.effector(idx)
    }

    /// Find and downcast a state effector by name (mutable).
    pub fn effector_by_name_mut<T: StateEffector<SpacecraftState<F>> + 'static>(
        &mut self,
        name: &str,
    ) -> Option<&mut T> {
        let idx = self
            .registry
            .entries()
            .iter()
            .position(|e| e.name == name)?;
        self.effector_mut(idx)
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

    /// Replace a model by name, returning the old one (if found).
    ///
    /// This is used to swap in a model with updated commanded state
    /// between integration segments (e.g., `MtqAssembly` with a new
    /// `command`).
    pub fn replace_model(
        &mut self,
        name: &str,
        new_model: Box<dyn Model<SpacecraftState<F>>>,
    ) -> Option<Box<dyn Model<SpacecraftState<F>>>> {
        if let Some(slot) = self.models.iter_mut().find(|m| m.name() == name) {
            Some(std::mem::replace(slot, new_model))
        } else {
            None
        }
    }

    /// Names of active models.
    pub fn model_names(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.name()).collect()
    }

    /// Per-model load breakdown at the given state.
    pub fn model_breakdown(
        &self,
        t: f64,
        state: &SpacecraftState<F>,
    ) -> Vec<(&str, ExternalLoads<F>)> {
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));
        self.models
            .iter()
            .map(|m| (m.name(), m.eval(t, state, epoch.as_ref())))
            .collect()
    }

    /// Per-model disturbance torque in the body frame [N·m], for telemetry.
    ///
    /// The vector rather than a magnitude, because a magnitude carries neither
    /// the sign nor the axis, and those are what a torque is read for: an
    /// attitude disturbance that turns the wrong way looks the same size as one
    /// that turns the right way.
    ///
    /// The gravity field is absent. It acts on the centre of mass, so it exerts
    /// no torque about it; a gravity-gradient torque is a
    /// [`Model`](crate::model::Model) and answers under its own name. This is
    /// why the list is one entry per model, where
    /// [`acceleration_breakdown`](Self::acceleration_breakdown) leads with
    /// `"gravity"`.
    ///
    /// Every model answers, not only the environmental ones. A magnetorquer and
    /// a thruster assembly are `Model`s and are installed as such, so their
    /// commanded torques appear here under their own names. A reaction wheel is
    /// a `StateEffector` — it carries its own state — and is absent; its
    /// momentum is read from the effector.
    pub fn torque_breakdown(
        &self,
        t: f64,
        state: &SpacecraftState<F>,
    ) -> Vec<(&str, arika::frame::Vec3<arika::frame::Body>)> {
        // Straight from the models, without the gravity field
        // [`load_breakdown`](Self::load_breakdown) needs: a torque-only caller
        // — every `orts run` output sample is one — has no use for it, and a
        // substituted `GravityField` can be expensive.
        self.model_breakdown(t, state)
            .into_iter()
            .map(|(name, loads)| (name, loads.torque_body))
            .collect()
    }

    /// Both breakdowns from one evaluation of every model.
    ///
    /// `ExternalLoads` carries the acceleration and the torque together, so a
    /// caller that wants both — telemetry reporting one sample — has no reason
    /// to evaluate every model twice. The panel models make that visible: the
    /// shadow geometry is most of their cost, and a 22-panel spacecraft is
    /// 50 µs an evaluation.
    ///
    /// The two halves are what
    /// [`acceleration_breakdown`](Self::acceleration_breakdown) and
    /// [`torque_breakdown`](Self::torque_breakdown) answer. Each of those
    /// projects from `model_breakdown` on its own, so a caller wanting one half
    /// does not build the other; this exists for the caller wanting both.
    pub fn load_breakdown(&self, t: f64, state: &SpacecraftState<F>) -> LoadBreakdown<'_> {
        let grav = self
            .gravity
            .acceleration(self.mu, state.orbit.position())
            .magnitude();
        let breakdown = self.model_breakdown(t, state);
        let mut accelerations = Vec::with_capacity(breakdown.len() + 1);
        accelerations.push(("gravity", grav));
        let mut torques = Vec::with_capacity(breakdown.len());
        for (name, loads) in breakdown {
            accelerations.push((name, loads.acceleration_inertial.magnitude()));
            torques.push((name, loads.torque_body));
        }
        LoadBreakdown {
            accelerations,
            torques,
        }
    }

    /// Acceleration breakdown for telemetry.
    pub fn acceleration_breakdown(&self, t: f64, state: &SpacecraftState<F>) -> Vec<(&str, f64)> {
        // Projected here rather than through
        // [`load_breakdown`](Self::load_breakdown), which would build a vector
        // of every model's torque for this caller to drop.
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

impl<G: GravityField, F: Eci + 'static> SpacecraftDynamics<G, F> {
    /// Shared body of [`derivatives`](DynamicalSystem::derivatives) and
    /// [`derivatives_in_segment`](DynamicalSystem::derivatives_in_segment).
    ///
    /// `segment` is `Some` only on the segment path, where it carries the
    /// interval's start in both time bases so a model holding a schedule can
    /// answer for it.
    fn derivatives_for(
        &self,
        segment: Option<&EvalSegment<'_>>,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> AugmentedState<SpacecraftState<F>> {
        // A state whose mode vector does not match what the effectors
        // registered cannot say which constraints are held, and every boundary
        // declared for them would read as inactive: the propagation would walk
        // past a wheel's limit with the walk running and nothing to stop it.
        // Rejected here, on the first evaluation, rather than silently opting
        // out of the constraints. `initial_augmented_state` builds the vector
        // this expects.
        assert_eq!(
            state.modes.len(),
            self.registry.total_modes(),
            "mode vector length ({}) does not match registry ({})",
            state.modes.len(),
            self.registry.total_modes()
        );

        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));

        // Gravitational acceleration
        let grav_accel = self
            .gravity
            .acceleration(self.mu, state.plant.orbit.position());

        // Accumulate external loads from models
        let mut total = ExternalLoads::<F>::zeros();
        for model in &self.models {
            total += eval_maybe_in_segment(model, segment, t, &state.plant, epoch.as_ref());
        }

        // Evaluate state effectors.
        //
        // INVARIANT: a `StateEffector<S>` returns `ExternalLoads<S::Frame>` —
        // already expressed in the frame this system propagates in — so loads
        // accumulate directly with no coordinate re-tag. Torque-only effectors
        // (reaction wheels) write `impl<S: HasFrame + HasAttitude>
        // StateEffector<S>` and leave the acceleration zero, since body-frame
        // torque names no inertial frame; a translational effector must rotate
        // its body-frame vector through the state's own attitude. The loads
        // frame is not a separate parameter that could disagree with the
        // state's, which is what makes the SimpleEci mislabel from issue #103
        // unrepresentable.
        let mut aux_rates = vec![0.0; self.registry.total_dim()];
        for (i, eff) in self.effectors.iter().enumerate() {
            let entry = &self.registry.entries()[i];
            let rates_slice = &mut aux_rates[entry.offset..entry.offset + entry.dim];
            total += eff.derivatives(
                EffectorInput {
                    t,
                    state: &state.plant,
                    aux: &state.aux[entry.offset..entry.offset + entry.dim],
                    // A state assembled by hand carries no modes; an
                    // effector reading none falls back to judging its own
                    // constraint, which is what a walk with no boundary
                    // handling has always done.
                    modes: state
                        .modes
                        .get(entry.mode_offset..entry.mode_offset + entry.mode_dim)
                        .unwrap_or(&[]),
                    epoch: epoch.as_ref(),
                    segment,
                },
                rates_slice,
            );
        }

        // Total translational acceleration
        let total_accel = grav_accel + total.acceleration_inertial.into_inner();

        // Quaternion kinematics: dq/dt = ½ q ⊗ (0, ω)
        let q_dot = state.plant.attitude.q_dot();

        // Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.plant.attitude.angular_velocity;
        let alpha = self.inertia_inv
            * (total.torque_body.into_inner() - state.plant.attitude.angular_velocity.cross(&iw));

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
            modes: state.modes.clone(),
        }
    }
}

impl<G: GravityField, F: Eci + 'static> HasBoundaries for SpacecraftDynamics<G, F> {
    fn boundaries(&self) -> Vec<DeclaredBoundary> {
        self.effectors
            .iter()
            .enumerate()
            .flat_map(|(index, eff)| {
                let entry = &self.registry.entries()[index];
                eff.boundaries()
                    .into_iter()
                    .map(move |boundary| DeclaredBoundary {
                        satellite: 0,
                        effector: index,
                        boundary,
                        aux_offset: entry.offset,
                        aux_dim: entry.dim,
                        mode_offset: entry.mode_offset,
                        mode_dim: entry.mode_dim,
                    })
            })
            .collect()
    }

    fn boundary_value(
        &self,
        declared: &DeclaredBoundary,
        segment: Option<&SegmentContext>,
        t: f64,
        state: &Self::State,
    ) -> f64 {
        let effector = &self.effectors[declared.effector];
        let aux = &state.aux[declared.aux_offset..declared.aux_offset + declared.aux_dim];
        let modes = state
            .modes
            .get(declared.mode_offset..declared.mode_offset + declared.mode_dim)
            .unwrap_or(&[]);
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));
        let segment_epoch = segment.and_then(|s| self.epoch_0.map(|e| e.add_si_seconds(s.start)));
        let eval_segment = segment.map(|s| EvalSegment::new(s, segment_epoch.as_ref()));
        effector.boundary_value(
            declared.boundary.kind,
            EffectorInput {
                t,
                state: &state.plant,
                aux,
                modes,
                epoch: epoch.as_ref(),
                // The same segment the derivatives were taken in, with the
                // absolute time at its start, so an effector holding a value
                // for the segment answers the boundary with the one the state
                // was integrated under.
                segment: eval_segment.as_ref(),
            },
        )
    }

    fn settle_boundary(&self, declared: &DeclaredBoundary, state: &mut Self::State) {
        let effector = &self.effectors[declared.effector];
        let aux = &mut state.aux[declared.aux_offset..declared.aux_offset + declared.aux_dim];
        if let Some(exchange) = effector.settle_boundary(declared.boundary.kind, aux) {
            // What the effector gave up goes back to the body, through the
            // inertia that turns angular momentum into a rate.
            state.plant.attitude.angular_velocity +=
                self.inertia_inv * exchange.angular_momentum_body.into_inner();
        }
        state.modes[declared.mode_index()] = declared.boundary.kind.mode_after();
    }

    fn boundary_is_active(&self, declared: &DeclaredBoundary, state: &Self::State) -> bool {
        declared.is_active(&state.modes)
    }
}

impl<G: GravityField, F: Eci + 'static> DynamicalSystem for SpacecraftDynamics<G, F> {
    type State = AugmentedState<SpacecraftState<F>>;

    /// The earliest boundary any model or effector reports.
    ///
    /// Models that report the same time collapse to one, which is what a
    /// propagation loop wants: two thrusters whose windows share an edge, or a
    /// window whose end abuts the next one's start, are one place to end a step.
    fn next_discontinuity_after(&self, t: f64) -> Option<f64> {
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));
        let from_models = self
            .models
            .iter()
            .filter_map(|m| m.next_discontinuity_after(t, epoch.as_ref()));
        let from_effectors = self
            .effectors
            .iter()
            .filter_map(|e| e.next_discontinuity_after(t, epoch.as_ref()));
        from_models
            .chain(from_effectors)
            .filter(|next| *next > t && next.is_finite())
            .min_by(f64::total_cmp)
    }

    fn derivatives(
        &self,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> AugmentedState<SpacecraftState<F>> {
        self.derivatives_for(None, t, state)
    }

    fn derivatives_in_segment(
        &self,
        segment: &SegmentContext,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> AugmentedState<SpacecraftState<F>> {
        let start_epoch = self.epoch_0.map(|e| e.add_si_seconds(segment.start));
        self.derivatives_for(
            Some(&EvalSegment::new(segment, start_epoch.as_ref())),
            t,
            state,
        )
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
    use arika::earth::MU as MU_EARTH;
    use nalgebra::{Vector3, Vector4};
    use utsuroi::{Integrator, OdeState, Rk4};

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

    /// A wheel's two bounds and its release are not all live at once: the
    /// search looks for a bound to be reached only while the wheel is running
    /// free, and for a release only while it is held against one. The
    /// propagation switches the set from what this answers, so a boundary that
    /// stayed live would be searched for in a mode where its value says
    /// nothing.
    #[test]
    fn a_held_wheels_bound_is_no_longer_one_to_search_for() {
        use crate::effector::{BoundaryKind, ConstraintMode};
        use crate::spacecraft::ReactionWheelAssembly;

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_effector(ReactionWheelAssembly::three_axis(0.01, 0.5, 0.1));
        let boundaries = dynamics.boundaries();
        let mut state = dynamics.initial_augmented_state(sample_spacecraft());

        let find = |kind: BoundaryKind| {
            *boundaries
                .iter()
                .find(|d| d.boundary.kind == kind)
                .expect("the wheel declared it")
        };
        let upper = find(BoundaryKind::ReachedUpper { index: 0 });
        let lower = find(BoundaryKind::ReachedLower { index: 0 });
        let release = find(BoundaryKind::Released { index: 0 });

        // A wheel that is running: either bound is still ahead of it, and
        // there is nothing to release.
        assert!(dynamics.boundary_is_active(&upper, &state));
        assert!(dynamics.boundary_is_active(&lower, &state));
        assert!(!dynamics.boundary_is_active(&release, &state));

        state.modes[0] = ConstraintMode::Upper;

        // Held against the upper bound: that bound is where it already is, the
        // other one is not reachable without coming off this one first, and
        // the release is what the search now looks for.
        assert!(!dynamics.boundary_is_active(&upper, &state));
        assert!(!dynamics.boundary_is_active(&lower, &state));
        assert!(dynamics.boundary_is_active(&release, &state));

        // The other wheels are untouched by the first one's mode.
        assert!(
            dynamics.boundary_is_active(&find(BoundaryKind::ReachedUpper { index: 1 }), &state)
        );
    }

    /// A hand-built state can carry auxiliary values without the modes that
    /// say which constraints hold, and `AugmentedState`'s fields are public, so
    /// nothing stops one from reaching a group. Every boundary declared for
    /// those effectors would then read as inactive and the propagation would
    /// walk a wheel past its limit with the walk running — so the first
    /// evaluation rejects it instead.
    #[test]
    #[should_panic(expected = "mode vector length")]
    fn a_state_without_the_modes_its_effectors_registered_is_rejected() {
        use crate::spacecraft::ReactionWheelAssembly;

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_effector(ReactionWheelAssembly::three_axis(0.01, 0.5, 0.1));
        let state = AugmentedState {
            plant: sample_spacecraft(),
            aux: vec![0.0; 3],
            aux_bounds: vec![],
            modes: vec![],
        };

        dynamics.derivatives(0.0, &state);
    }

    /// Wrap a plant state as an augmented state with no effectors.
    fn augment(plant: SpacecraftState) -> AugmentedState<SpacecraftState> {
        AugmentedState {
            plant,
            aux: vec![],
            aux_bounds: vec![],
            modes: vec![],
        }
    }

    // Mock models

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

    /// A torque under a name the test chooses, so several can stand together:
    /// `Model::name` is what the breakdown keys on.
    struct NamedTorqueModel(&'static str, Vector3<f64>);

    impl Model<SpacecraftState> for NamedTorqueModel {
        fn name(&self) -> &str {
            self.0
        }
        fn eval(&self, _t: f64, _state: &SpacecraftState, _epoch: Option<&Epoch>) -> ExternalLoads {
            ExternalLoads::torque(self.1)
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
                    acceleration_inertial: arika::frame::Vec3::new(e.jd() * 1e-10, 0.0, 0.0),
                    torque_body: arika::frame::Vec3::zeros(),
                    mass_rate: 0.0,
                },
                None => ExternalLoads::zeros(),
            }
        }
    }

    // Step 1: Gravity only (no LoadModel)

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

    // Step 2: Euler equation

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

    // Step 3: Model integration

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

    // Step 4: Builder + telemetry

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
        assert_eq!(
            breakdown[0].1.acceleration_inertial,
            arika::frame::Vec3::from_raw(accel)
        );
        assert_eq!(breakdown[0].1.torque_body, arika::frame::Vec3::zeros());
        assert_eq!(breakdown[1].0, "const_torque");
        assert_eq!(
            breakdown[1].1.acceleration_inertial,
            arika::frame::Vec3::zeros()
        );
        assert_eq!(
            breakdown[1].1.torque_body,
            arika::frame::Vec3::from_raw(torque)
        );
    }

    // Step 5: Epoch + integration + edge cases

    #[test]
    fn epoch_forwarded_to_loads() {
        let epoch = Epoch::from_jd(2460000.5);
        let t = 100.0;
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(EpochSensitiveLoad)
            .with_epoch(epoch);

        let d = dyn_sc.derivatives(t, &augment(sc.clone()));

        let expected_epoch = epoch.add_si_seconds(t);
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
        let _dyn_sc: SpacecraftDynamics<PointMass> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, Matrix3::zeros());
    }

    // Step 6: Derivative-level conservation laws

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

    // Step 7: StateEffector integration

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
        let mut dyn_sc: SpacecraftDynamics<PointMass> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);

        let rw_ref = dyn_sc
            .effector_mut::<ReactionWheelAssembly>(0)
            .expect("should downcast");
        rw_ref.command = crate::plugin::command::RwCommand::Torques(vec![0.1, 0.0, 0.0]);
    }

    #[test]
    fn rw_effector_integrates_with_spacecraft() {
        use crate::spacecraft::ReactionWheelAssembly;
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
        let mut dyn_sc =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);

        // Command a small torque on the x-axis (per-wheel).
        dyn_sc
            .effector_mut::<ReactionWheelAssembly>(0)
            .unwrap()
            .command = crate::plugin::command::RwCommand::Torques(vec![0.01, 0.0, 0.0]);

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

    // Step 8: frame-generic StateEffector (issue #103 fix)
    //
    // A `StateEffector<S>` returns `ExternalLoads<F>` — already in the
    // host inertial frame — so the dynamics accumulate effector loads with
    // no coordinate re-tag. Torque-only effectors work on any `F`; a
    // translational effector produces its inertial acceleration in `F`
    // itself (e.g. by rotating a body-frame thrust via
    // `attitude_to_inertial()`, whose `F` comes from the state's
    // `HasFrame::Frame`). This makes the old SimpleEci→`F` mislabel
    // unrepresentable.

    use crate::model::{HasAttitude, HasFrame};
    use arika::frame::{Body, Gcrs, Vec3 as FrameVec3};

    /// Translational mock: contributes a constant acceleration already
    /// expressed in the frame the state is propagated in.
    struct ConstAccelEffector {
        accel: Vector3<f64>,
    }

    impl<S: HasFrame> StateEffector<S> for ConstAccelEffector {
        fn name(&self) -> &str {
            "const_accel"
        }
        fn state_dim(&self) -> usize {
            0
        }
        fn derivatives(
            &self,
            _input: EffectorInput<'_, S>,
            _aux_rates: &mut [f64],
        ) -> ExternalLoads<S::Frame> {
            ExternalLoads::<S::Frame>::acceleration(self.accel)
        }
    }

    /// Realistic translational mock: a body-frame thrust rotated into the
    /// host inertial frame `F` — the correct pattern for a thruster effector
    /// on any frame, with no silent re-tag.
    struct BodyThrustEffector {
        accel_body: Vector3<f64>,
    }

    impl<S: HasFrame<Frame = F> + HasAttitude, F: Eci> StateEffector<S> for BodyThrustEffector {
        fn name(&self) -> &str {
            "body_thrust"
        }
        fn state_dim(&self) -> usize {
            0
        }
        fn derivatives(
            &self,
            input: EffectorInput<'_, S>,
            _aux_rates: &mut [f64],
        ) -> ExternalLoads<F> {
            let state = input.state;
            let a_body = FrameVec3::<Body>::from_raw(self.accel_body);
            let a_inertial = state.attitude_to_inertial().transform(&a_body);
            ExternalLoads {
                acceleration_inertial: a_inertial,
                torque_body: FrameVec3::zeros(),
                mass_rate: 0.0,
            }
        }
    }

    fn gcrs_spacecraft(attitude: AttitudeState) -> SpacecraftState<Gcrs> {
        SpacecraftState {
            orbit: crate::OrbitalState::new_in_frame(
                Vector3::new(7000.0, 0.0, 0.0),
                Vector3::new(0.0, 7.5, 0.0),
            ),
            attitude,
            mass: 500.0,
        }
    }

    #[test]
    fn torque_only_effector_works_on_gcrs() {
        // A torque-only effector (RW) registers and integrates on a
        // non-SimpleEci system: RW is `StateEffector<S>` for any state, and its
        // loads are body-frame torque with zero inertial acceleration, so the
        // state's frame never enters.
        use crate::spacecraft::ReactionWheelAssembly;
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.5);
        let mut dyn_sc: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(rw);
        dyn_sc
            .effector_mut::<ReactionWheelAssembly>(0)
            .unwrap()
            .command = crate::plugin::command::RwCommand::Torques(vec![0.01, 0.0, 0.0]);

        let state = dyn_sc.initial_augmented_state(gcrs_spacecraft(AttitudeState::identity()));
        let d = dyn_sc.derivatives(0.0, &state);

        assert_eq!(d.aux.len(), 3);
        // RW reaction torque produces a body angular acceleration, ...
        assert!(d.plant.attitude.angular_velocity.magnitude() > 1e-9);
        // ... and contributes zero inertial acceleration (gravity only).
        let dyn_grav: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(
            0.0,
            &dyn_grav.initial_augmented_state(gcrs_spacecraft(AttitudeState::identity())),
        );
        assert!((d.plant.orbit.velocity() - d_grav.plant.orbit.velocity()).magnitude() < 1e-15);
    }

    #[test]
    fn translational_effector_acceleration_applied_in_gcrs() {
        // The effector's ExternalLoads<Gcrs> acceleration reaches the Gcrs
        // translational EOM directly — no coordinate re-tag.
        let accel = Vector3::new(1e-6, 2e-6, 3e-6);
        let dyn_sc: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
                .with_effector(ConstAccelEffector { accel });
        let plant = gcrs_spacecraft(AttitudeState::identity());
        let d = dyn_sc.derivatives(0.0, &dyn_sc.initial_augmented_state(plant.clone()));

        let dyn_grav: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &dyn_grav.initial_augmented_state(plant));

        let diff = d.plant.orbit.velocity() - d_grav.plant.orbit.velocity();
        assert!((diff - accel).magnitude() < 1e-15);
    }

    #[test]
    fn body_thrust_effector_rotates_into_gcrs() {
        // Regression guard for #103: a body-frame thrust is rotated into the
        // host frame `F = Gcrs` via `attitude_to_inertial()`, not re-tagged.
        // 90° about +Z: body +X thrust → inertial +Y acceleration.
        let half = std::f64::consts::FRAC_PI_2 / 2.0;
        let attitude = AttitudeState {
            quaternion: Vector4::new(half.cos(), 0.0, 0.0, half.sin()),
            angular_velocity: Vector3::zeros(),
        };
        let dyn_sc: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0)).with_effector(
                BodyThrustEffector {
                    accel_body: Vector3::new(1e-6, 0.0, 0.0),
                },
            );
        let plant = gcrs_spacecraft(attitude);
        let d = dyn_sc.derivatives(0.0, &dyn_sc.initial_augmented_state(plant.clone()));

        let dyn_grav: SpacecraftDynamics<PointMass, Gcrs> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &dyn_grav.initial_augmented_state(plant));

        let inertial_accel = d.plant.orbit.velocity() - d_grav.plant.orbit.velocity();
        assert!(
            inertial_accel[0].abs() < 1e-12,
            "x ~0, got {}",
            inertial_accel[0]
        );
        assert!(
            (inertial_accel[1] - 1e-6).abs() < 1e-12,
            "y = 1e-6, got {}",
            inertial_accel[1]
        );
        assert!(inertial_accel[2].abs() < 1e-15);
    }

    #[test]
    fn translational_effector_on_simple_eci_still_works() {
        // Backward compatibility: the default `F = SimpleEci` path is unchanged.
        let accel = Vector3::new(1e-6, 2e-6, 3e-6);
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_effector(ConstAccelEffector { accel });
        let d = dyn_sc.derivatives(0.0, &dyn_sc.initial_augmented_state(sample_spacecraft()));

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &augment(sample_spacecraft()));

        let diff = d.plant.orbit.velocity() - d_grav.plant.orbit.velocity();
        assert!((diff - accel).magnitude() < 1e-15);
    }

    /// The breakdown telemetry reads has to carry the torque's direction.
    ///
    /// `acceleration_breakdown` answers with a magnitude, which cannot carry a
    /// sign or an axis: a disturbance turning the spacecraft the wrong way
    /// reads the same as one turning it the right way. The two models here have
    /// perpendicular torques, so a breakdown that summed or normalised them
    /// could not produce these components.
    #[test]
    fn torque_breakdown_reports_each_model_as_a_body_frame_vector() {
        let about_x = Vector3::new(0.4, 0.0, 0.0);
        let about_z = Vector3::new(0.0, 0.0, -0.25);
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(NamedTorqueModel("first", about_x))
            .with_model(NamedTorqueModel("second", about_z));
        let state = sample_spacecraft();

        let breakdown = dynamics.torque_breakdown(0.0, &state);

        assert_eq!(
            breakdown.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            vec!["first", "second"],
            "the models keep their order and their names"
        );
        assert_eq!(breakdown[0].1.into_inner(), about_x);
        assert_eq!(breakdown[1].1.into_inner(), about_z);

        // What the acceleration breakdown can say about the same two models:
        // nothing, because neither produces one.
        for (name, magnitude) in dynamics.acceleration_breakdown(0.0, &state) {
            if name == "first" || name == "second" {
                assert_eq!(magnitude, 0.0, "{name} produces no acceleration");
            }
        }
    }

    /// The gravity field appears in the acceleration breakdown and must not
    /// appear in the torque one: it acts on the centre of mass, so it exerts no
    /// torque about it. A gravity-gradient torque is a model of its own and
    /// answers under its own name.
    #[test]
    fn torque_breakdown_leaves_out_the_gravity_field() {
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(NamedTorqueModel("only_model", Vector3::new(0.0, 1.0, 0.0)));
        let state = sample_spacecraft();

        let accel = dynamics.acceleration_breakdown(0.0, &state);
        assert!(
            accel
                .iter()
                .any(|(name, value)| *name == "gravity" && *value > 0.0),
            "the acceleration breakdown leads with gravity: {accel:?}"
        );

        let torque = dynamics.torque_breakdown(0.0, &state);
        assert_eq!(
            torque.len(),
            1,
            "one entry per model and nothing else: {:?}",
            torque.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        assert_eq!(torque[0].0, "only_model");
    }

    /// Both breakdowns come from one evaluation of every model, which is what
    /// telemetry reporting a sample needs: the panel models' shadow geometry is
    /// most of their cost, and evaluating them twice for one sample doubles it.
    ///
    /// Counted rather than argued: the model records how often it is asked.
    #[test]
    fn one_evaluation_answers_both_breakdowns() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counting {
            calls: Arc<AtomicUsize>,
        }

        impl Model<SpacecraftState> for Counting {
            fn name(&self) -> &str {
                "counting"
            }
            fn eval(
                &self,
                _t: f64,
                _state: &SpacecraftState,
                _epoch: Option<&Epoch>,
            ) -> ExternalLoads {
                self.calls.fetch_add(1, Ordering::Relaxed);
                ExternalLoads::torque(Vector3::new(0.0, 1.0, 0.0))
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(Counting {
                calls: Arc::clone(&calls),
            });
        let state = sample_spacecraft();

        let LoadBreakdown {
            accelerations,
            torques,
        } = dynamics.load_breakdown(0.0, &state);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "one evaluation for both halves"
        );
        assert_eq!(accelerations.len(), 2, "gravity leads, then the model");
        assert_eq!(torques.len(), 1);
        assert_eq!(torques[0].1.into_inner(), Vector3::new(0.0, 1.0, 0.0));

        // And the two single-sided accessors give the same answers, each for
        // one evaluation of its own — and without building the half its caller
        // did not ask for.
        calls.store(0, Ordering::Relaxed);
        let separate = dynamics.acceleration_breakdown(0.0, &state);
        assert_eq!(separate, accelerations);
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        calls.store(0, Ordering::Relaxed);
        let separate = dynamics.torque_breakdown(0.0, &state);
        assert_eq!(separate.len(), 1);
        assert_eq!(separate[0].1.into_inner(), Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
