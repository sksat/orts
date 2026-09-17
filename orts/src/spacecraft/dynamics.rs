use std::marker::PhantomData;

use crate::boundary::{DeclaredBoundary, HasBoundaries};
use crate::effector::{AugmentedState, AuxRegistry, ConstraintMode, EffectorInput, StateEffector};
use crate::model::{EvalSegment, Model, eval_maybe_in_segment};
use crate::orbital::gravity::GravityField;
use arika::epoch::Epoch;
use arika::frame::{Eci, SimpleEci};
use nalgebra::Matrix3;
use utsuroi::{DynamicalSystem, SegmentContext};

use super::{ExternalLoads, PropellantPool, SpacecraftState};

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
    /// Models that draw on the propellant, kept apart from the rest so that
    /// running dry stops them and nothing else. Which models those are is the
    /// caller's to say — guessing from a negative mass rate or from a name
    /// would stop drag or a wheel too.
    propulsion: Vec<Box<dyn Model<SpacecraftState<F>>>>,
    /// The propellant every one of them draws from, with the floor under the
    /// spacecraft's mass, and which effector it was registered as — its mode
    /// is what says whether there is anything left to burn. `None` for a
    /// spacecraft that carries none, which is every spacecraft with no
    /// propulsion registered.
    pool: Option<(PropellantPool, usize)>,
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
            propulsion: Vec::new(),
            pool: None,
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

    /// The propellant this spacecraft carries, as a floor under its mass.
    ///
    /// One pool per spacecraft: every model registered with
    /// [`with_propulsion`](Self::with_propulsion) draws from it, and what is
    /// left is the mass the state carries above the floor.
    ///
    /// The pool is registered as an effector, which is how running dry becomes
    /// a boundary the propagation locates: it carries no auxiliary state of its
    /// own — the mass is the plant's — but it carries the mode that says
    /// whether there is anything left to burn, and it declares the floor as a
    /// boundary.
    ///
    /// # Panics
    ///
    /// Panics if a pool is already registered. Two pools would be two floors,
    /// which is what this exists to prevent.
    pub fn with_propellant(self, pool: PropellantPool) -> Self {
        self.with_effector(pool)
    }

    /// Add a model that burns propellant (builder pattern).
    ///
    /// Its loads are evaluated exactly as [`with_model`](Self::with_model)'s
    /// are, until the pool is empty — then this one is not asked at all, and
    /// neither is any other that draws on the same propellant. A propulsion
    /// model registered through `with_model` would keep thrusting on an empty
    /// tank.
    ///
    /// # What "empty" needs
    ///
    /// The pool's mode is what stops the burn, and only a propagation that
    /// handles boundaries moves it: a group, the CLI's controlled path, or
    /// [`walk_to_target`](crate::boundary::walk_to_target) directly. Stepping
    /// this system through [`Integrator`](utsuroi::Integrator) yourself runs no
    /// search and settles nothing, so the mode stays
    /// [`Free`](crate::effector::ConstraintMode::Free) and the burn continues
    /// below the floor — the same limitation the momentum limit of
    /// [`RwAssembly`](crate::spacecraft::RwAssembly) has.
    ///
    /// # Panics
    ///
    /// Panics without a pool to draw from: the floor is what says when the
    /// spacecraft is empty, and thrust with no floor has no end.
    pub fn with_propulsion(mut self, model: impl Model<SpacecraftState<F>> + 'static) -> Self {
        assert!(
            self.pool.is_some(),
            "a propulsion model needs the propellant it burns: call \
             `with_propellant` first"
        );
        self.propulsion.push(Box::new(model));
        self
    }

    /// The propellant pool, if this spacecraft carries any.
    pub fn pool(&self) -> Option<PropellantPool> {
        self.pool.map(|(pool, _)| pool)
    }

    /// Whether the state's modes say there is propellant left to burn.
    ///
    /// The mode, not the mass: a comparison against the floor flips between
    /// the stages of a re-stepped interval, and the search that re-steps it
    /// then reports the crossing late. The mode only moves where the
    /// propagation settled a boundary.
    ///
    /// True for a spacecraft with no pool at all: nothing to run out of.
    fn has_propellant(&self, state: &AugmentedState<SpacecraftState<F>>) -> bool {
        let Some((_, index)) = self.pool else {
            return true;
        };
        let Some(entry) = self.registry.entries().get(index) else {
            return true;
        };
        state
            .modes
            .get(entry.mode_offset)
            .is_none_or(|mode| *mode == ConstraintMode::Free)
    }

    /// Add a state effector (builder pattern).
    ///
    /// A [`PropellantPool`] registered here is registered as the pool, exactly
    /// as [`with_propellant`](Self::with_propellant) would: it declares a
    /// boundary and carries the mode the propulsion is gated on, and a pool
    /// the system did not know about would declare a floor that stops nothing.
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
        let mode_dim = effector.mode_dim();
        crate::effector::check_declared_modes(effector.name(), &effector.boundaries(), mode_dim);
        self.registry.register(effector.name(), dim, mode_dim);
        let index = self.effectors.len();
        let boxed: Box<dyn StateEffector<SpacecraftState<F>>> = Box::new(effector);
        // A pool is the pool wherever it was registered. Recognised here so
        // that the generic path cannot leave one the system does not know
        // about: its floor would be a boundary that stops no thruster, and no
        // state would carry the mode it needs.
        if let Some(pool) = pool_that(boxed.as_ref()) {
            assert!(
                self.pool.is_none(),
                "a spacecraft has one propellant pool, and this one already has one"
            );
            self.pool = Some((pool, index));
        }
        self.effectors.push(boxed);
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
        let mut modes = vec![ConstraintMode::default(); self.registry.total_modes()];
        // A spacecraft can start with an empty tank, and no search would find
        // that: a margin of exactly zero has not been crossed. Its mode says
        // so from the first step. Starting *below* the floor is refused here —
        // and here only: `AugmentedState`'s fields are public, so a state
        // assembled by hand can carry a mass the pool would have rejected, and
        // the walk then settles it onto the floor, which adds the mass the
        // input was missing. Issue #523 is where that check belongs.
        if let Some((pool, index)) = self.pool
            && let Some(entry) = self.registry.entries().get(index)
        {
            modes[entry.mode_offset] = pool.initial_mode(plant.mass);
        }
        AugmentedState {
            plant,
            aux: vec![0.0; self.registry.total_dim()],
            aux_bounds: bounds,
            modes,
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
    ///
    /// A propulsion model is replaced where it was registered, so a rebuilt
    /// thruster assembly stays propulsion and the pool it draws from is
    /// untouched: the propellant is the spacecraft's, and a command does not
    /// refill it.
    pub fn replace_model(
        &mut self,
        name: &str,
        new_model: Box<dyn Model<SpacecraftState<F>>>,
    ) -> Option<Box<dyn Model<SpacecraftState<F>>>> {
        let slot = self
            .models
            .iter_mut()
            .chain(self.propulsion.iter_mut())
            .find(|m| m.name() == name);
        slot.map(|slot| std::mem::replace(slot, new_model))
    }

    /// Names of the registered models, propulsion last.
    ///
    /// Every one of them, whether it acts at a given state or not: this takes
    /// no state, so it cannot say whether a tank still has propellant. The
    /// breakdowns do — they skip *evaluating* a propulsion model the pool has
    /// switched off, and keep its entry as a zero, so a record has one column
    /// per name here whatever the tank holds.
    pub fn model_names(&self) -> Vec<&str> {
        self.models
            .iter()
            .chain(self.propulsion.iter())
            .map(|m| m.name())
            .collect()
    }

    /// Check that a state carries the modes this system's effectors registered.
    ///
    /// Every path that reads a mode goes through here first. A state assembled
    /// by hand can carry none — the fields are public — and a mode that is not
    /// there reads as [`ConstraintMode::Free`], which would say "there is
    /// propellant left" about a tank that has run dry.
    ///
    /// # Panics
    ///
    /// If the state's mode vector is not the length the registry says.
    fn check_modes(&self, state: &AugmentedState<SpacecraftState<F>>) {
        assert_eq!(
            state.modes.len(),
            self.registry.total_modes(),
            "mode vector length ({}) does not match registry ({})",
            state.modes.len(),
            self.registry.total_modes()
        );
    }

    /// Per-model load breakdown at the given state.
    ///
    /// The augmented state rather than the plant alone, because whether the
    /// propulsion burns is the pool's mode — the same value the right-hand
    /// side gates on. Reading the mass here instead would make a record
    /// disagree with the trajectory on a path that settles no boundaries: the
    /// mode stays `Free` and the thrust keeps being integrated, while a
    /// mass-based comparison would report none.
    ///
    /// A state whose mass is not positive gets the same treatment it gets in
    /// the right-hand side, for the same reason: the acceleration a model
    /// reports there is an infinity, and a record of an infinity says nothing
    /// the trajectory did.
    ///
    /// Every registered model gets an entry, in the order
    /// [`model_names`](Self::model_names) lists them. A propulsion model the
    /// pool has switched off is not evaluated — that is the point of the mode —
    /// but its entry is there, a zero: the telemetry that reads this builds one
    /// column per model, and a row that drops an entry after depletion leaves
    /// that column sparse where a reader wants to see the thrust go to zero.
    ///
    /// # Panics
    ///
    /// If the state's mode vector is not the length the registry says (see
    /// [`check_modes`](Self::check_modes)).
    pub fn model_breakdown(
        &self,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> Vec<(&str, ExternalLoads<F>)> {
        self.check_modes(state);
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));
        let in_the_domain = mass_is_positive(state.plant.mass);
        let burning = self.has_propellant(state);
        let mut breakdown: Vec<(&str, ExternalLoads<F>)> = self
            .models_to_evaluate(burning)
            .map(|m| {
                let loads = m.eval(t, &state.plant, epoch.as_ref());
                (m.name(), keep_what_is_finite(loads, in_the_domain))
            })
            .collect();
        if !burning {
            breakdown.extend(
                self.propulsion
                    .iter()
                    .map(|m| (m.name(), ExternalLoads::zeros())),
            );
        }
        breakdown
    }

    /// The models that act on a spacecraft in this state: all of them, and the
    /// propulsion too while there is propellant left to burn.
    ///
    /// See [`keep_what_is_finite`] for what happens to what they return at a
    /// state whose mass is not positive.
    fn models_to_evaluate(
        &self,
        burning: bool,
    ) -> impl Iterator<Item = &Box<dyn Model<SpacecraftState<F>>>> {
        let propulsion = burning.then_some(&self.propulsion);
        self.models.iter().chain(propulsion.into_iter().flatten())
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
        state: &AugmentedState<SpacecraftState<F>>,
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
    pub fn load_breakdown(
        &self,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> LoadBreakdown<'_> {
        let grav = self
            .gravity
            .acceleration(self.mu, state.plant.orbit.position())
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
    pub fn acceleration_breakdown(
        &self,
        t: f64,
        state: &AugmentedState<SpacecraftState<F>>,
    ) -> Vec<(&str, f64)> {
        // Projected here rather than through
        // [`load_breakdown`](Self::load_breakdown), which would build a vector
        // of every model's torque for this caller to drop.
        let grav = self
            .gravity
            .acceleration(self.mu, state.plant.orbit.position())
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
        self.check_modes(state);

        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));

        // Gravitational acceleration
        let grav_accel = self
            .gravity
            .acceleration(self.mu, state.plant.orbit.position());

        // Accumulate external loads from models.
        //
        // Only half of it where the state has no mass. Everything that turns a
        // force into an acceleration divides by the mass — a thruster, a
        // panel's drag or SRP, and a translational
        // [`StateEffector`](crate::effector::StateEffector) — so `F/m` is an
        // infinity there and the walk fails on a state it was going to discard
        // anyway: a boundary search re-steps an interval under the mode that
        // held before the crossing, so it steps past the propellant floor on
        // purpose, and a wide enough step takes a stage below zero mass. The
        // acceleration such a stage reports is dropped.
        //
        // The mass rate is kept, because it is not singular and it is what the
        // search reads the crossing from: a hundred-second step burning 1 kg/s
        // through 99 kg of propellant has its last stage at zero mass, and
        // suppressing that stage's mass rate leaves both ends of the step above
        // the floor — the crossing inside it then goes unreported until a later
        // step, which is the defect this whole path exists to fix.
        //
        // The policy lives here because it is the state that is outside the
        // domain, not any one model.
        let mut total = ExternalLoads::<F>::zeros();
        let in_the_domain = mass_is_positive(state.plant.mass);
        for model in self.models_to_evaluate(self.has_propellant(state)) {
            let loads = eval_maybe_in_segment(model, segment, t, &state.plant, epoch.as_ref());
            total += keep_what_is_finite(loads, in_the_domain);
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
            let loads = eff.derivatives(
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
            total += keep_what_is_finite(loads, in_the_domain);
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
            // And where a boundary fixes the mass — a propellant floor — the
            // state belongs on it. The propellant burned below it is gone with
            // the exhaust, so the impulse it gave stays in the velocity.
            if let Some(mass) = exchange.mass {
                state.plant.mass = mass;
            }
        }
        state.modes[declared.mode_index()] = declared.boundary.kind.mode_after();
    }

    fn boundary_is_active(&self, declared: &DeclaredBoundary, state: &Self::State) -> bool {
        declared.is_active(&state.modes)
    }

    /// The lengths first, then each effector about its own part.
    ///
    /// The lengths are what makes the slices below well defined, and every
    /// other path reads the same offsets: a state whose vectors do not match
    /// the registry would index out of an effector's slice or silently leave
    /// part of it unprojected.
    ///
    /// The effectors answer in their own terms rather than by the sign of a
    /// boundary value, since a quantity resting on a bound in the mode that
    /// holds it there is a legal start.
    fn validate_boundary_walk_start(
        &self,
        state: &Self::State,
    ) -> Result<(), crate::boundary::StartStateError> {
        let (modes, aux) = (state.modes.len(), state.aux.len());
        if modes != self.registry.total_modes() {
            return Err(crate::boundary::StartStateError::new(format!(
                "the state carries {modes} modes where the registered effectors declared {}",
                self.registry.total_modes()
            )));
        }
        if aux != self.registry.total_dim() {
            return Err(crate::boundary::StartStateError::new(format!(
                "the state carries {aux} auxiliary values where the registered effectors \
                 declared {}",
                self.registry.total_dim()
            )));
        }
        if !state.aux_bounds.is_empty() && state.aux_bounds.len() != aux {
            return Err(crate::boundary::StartStateError::new(format!(
                "the state carries {} bounds for {aux} auxiliary values",
                state.aux_bounds.len()
            )));
        }
        for (effector, entry) in self.effectors.iter().zip(self.registry.entries()) {
            effector
                .validate_state(
                    &state.plant,
                    &state.aux[entry.offset..entry.offset + entry.dim],
                    &state.modes[entry.mode_offset..entry.mode_offset + entry.mode_dim],
                )
                .map_err(|reason| {
                    crate::boundary::StartStateError::new(format!("{}: {reason}", effector.name()))
                })?;
        }
        Ok(())
    }
}

/// The propellant pool this effector is, if it is one.
///
/// [`StateEffector`] has [`Any`](std::any::Any) as a supertrait, so a
/// registered effector can be asked what concrete type it is. This is the one
/// place that asks, and what it is for: a pool registered through the generic
/// [`with_effector`](SpacecraftDynamics::with_effector) is still the
/// spacecraft's pool, and a system that did not recognise it would carry a
/// floor that stops no thruster.
fn pool_that<S: crate::model::HasFrame>(effector: &dyn StateEffector<S>) -> Option<PropellantPool> {
    (effector as &dyn std::any::Any)
        .downcast_ref::<PropellantPool>()
        .copied()
}

/// Whether a mass is in the domain of `F/m`.
///
/// `partial_cmp`, so that a mass that is no number is covered too: a NaN is
/// comparable to nothing, and what it means here is the same as no mass.
fn mass_is_positive(mass: f64) -> bool {
    matches!(mass.partial_cmp(&0.0), Some(core::cmp::Ordering::Greater))
}

/// What is left of a model's loads at a state outside the domain of `F/m`.
///
/// A boundary search steps past a propellant floor on purpose — it re-steps an
/// interval under the mode that held before the crossing — so a wide enough
/// step takes a stage below zero mass, and everything that turns a force into
/// an acceleration divides by the mass there. That half is dropped: nothing
/// about such a state is physical, and what the search needs back from it is a
/// finite number.
///
/// The mass rate is kept, because it is not singular in the mass and it is
/// what the search reads the crossing from. Measured: 99 kg of propellant at
/// 1 kg/s in a hundred-second RK4 step has its last stage at zero mass, and
/// with that stage's flow suppressed the step ends at 16.67 kg — both ends
/// above the 1 kg floor, no change of sign, and the crossing at 99 s is not
/// located until a later step.
fn keep_what_is_finite<F: Eci>(loads: ExternalLoads<F>, in_the_domain: bool) -> ExternalLoads<F> {
    if in_the_domain {
        loads
    } else {
        ExternalLoads {
            acceleration_inertial: arika::frame::Vec3::zeros(),
            ..loads
        }
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
        // Propulsion among them: a scheduled burn's start and end are the
        // switches this exists to report, and they are no less switches for
        // the model being registered apart from the others.
        let from_models = self
            .models
            .iter()
            .chain(self.propulsion.iter())
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

    /// An effector that declares a boundary on a mode it never registered.
    ///
    /// The index selects a mode inside the effector's own block, so one past
    /// `mode_dim` reads a mode that is not there — and a missing mode reads as
    /// `Free`, which is what makes a bound look reachable. Settling it would
    /// then write into the next effector's block, or past the end of the
    /// vector. Registration refuses it, where the declaration is already fixed
    /// and the panic can name the effector.
    #[test]
    #[should_panic(expected = "declared a boundary on mode 1, but registered 1 mode(s)")]
    fn a_boundary_on_a_mode_the_effector_never_registered_is_refused() {
        use crate::effector::{BoundaryKind, EffectorBoundary, EffectorInput, StateEffector};
        use crate::model::{ExternalLoads, HasFrame};

        struct OneModeTwoBoundaries;

        impl<S: HasFrame + Send + Sync> StateEffector<S> for OneModeTwoBoundaries {
            fn name(&self) -> &str {
                "one_mode_two_boundaries"
            }
            fn state_dim(&self) -> usize {
                1
            }
            fn mode_dim(&self) -> usize {
                1
            }
            fn boundaries(&self) -> Vec<EffectorBoundary> {
                vec![
                    EffectorBoundary {
                        kind: BoundaryKind::ReachedUpper { index: 0 },
                        boundary_tolerance: 0.0,
                    },
                    // One past the block it registered.
                    EffectorBoundary {
                        kind: BoundaryKind::ReachedUpper { index: 1 },
                        boundary_tolerance: 0.0,
                    },
                ]
            }
            fn derivatives(
                &self,
                _input: EffectorInput<'_, S>,
                _aux_rates: &mut [f64],
            ) -> ExternalLoads<S::Frame> {
                ExternalLoads::zeros()
            }
        }

        let dynamics: SpacecraftDynamics<PointMass> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let _ = dynamics.with_effector(OneModeTwoBoundaries);
    }

    /// An empty tank silences the thrust, and the record still has a column
    /// for it.
    ///
    /// The telemetry that reads a breakdown builds one column per model (see
    /// `SatSnapshot::torques`, whose doc says every model appears, its entry a
    /// measured zero). Dropping the propulsion entry once the pool is empty
    /// would leave that column sparse from the moment of depletion, where what
    /// a reader wants to see is the thrust going to zero.
    #[test]
    fn a_breakdown_keeps_an_entry_for_propulsion_the_pool_switched_off() {
        use crate::spacecraft::{PropellantPool, Thruster};

        const FLOOR: f64 = 100.0;
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(Vector3::new(1e-6, 0.0, 0.0)))
            .with_propellant(PropellantPool::new(FLOOR))
            .with_propulsion(Thruster::new(10.0, 300.0, Vector3::x()));

        let with_fuel = dynamics.initial_augmented_state(SpacecraftState {
            mass: FLOOR + 1.0,
            ..sample_spacecraft()
        });
        let empty = dynamics.initial_augmented_state(SpacecraftState {
            mass: FLOOR,
            ..sample_spacecraft()
        });

        let names = |state: &AugmentedState<SpacecraftState>| -> Vec<&str> {
            dynamics
                .model_breakdown(0.0, state)
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        assert_eq!(
            names(&with_fuel),
            dynamics.model_names(),
            "with propellant left, one entry per registered model"
        );
        assert_eq!(
            names(&empty),
            dynamics.model_names(),
            "and an empty tank keeps the same entries, in the same order"
        );

        let dry = dynamics.model_breakdown(0.0, &empty);
        let (_, thruster) = dry
            .iter()
            .find(|(name, _)| *name == "thruster")
            .expect("the thruster has an entry");
        assert_eq!(
            thruster.acceleration_inertial,
            arika::frame::Vec3::zeros(),
            "its entry is a zero, not a thrust"
        );
        assert_eq!(thruster.mass_rate, 0.0);
    }

    /// A breakdown reads the pool's mode, so it holds the state to the same
    /// contract the right-hand side does: a hand-built state with no modes
    /// would otherwise read as a full tank and report thrust.
    #[test]
    #[should_panic(expected = "mode vector length")]
    fn a_breakdown_rejects_a_state_without_the_modes_its_effectors_registered() {
        use crate::spacecraft::{PropellantPool, Thruster};

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_propellant(PropellantPool::new(100.0))
            .with_propulsion(Thruster::new(10.0, 300.0, Vector3::x()));
        let state = AugmentedState {
            plant: sample_spacecraft(),
            aux: vec![],
            aux_bounds: vec![],
            modes: vec![],
        };

        let _ = dynamics.model_breakdown(0.0, &state);
    }

    /// One pool, one floor, and the system is what reads them: a thruster
    /// fires whatever it is asked to, so a spacecraft with nothing left to
    /// burn has to not be asked. Both places that used to compare
    /// `mass <= dry_mass` inside the right-hand side are gone — a comparison
    /// there flips between the stages of a re-stepped boundary search.
    ///
    /// Everything that is not propulsion keeps being evaluated: drag and a
    /// wheel do not stop because a tank ran dry.
    #[test]
    fn a_spacecraft_with_nothing_left_to_burn_is_not_asked_for_thrust() {
        use crate::spacecraft::{PropellantPool, Thruster};

        const FLOOR: f64 = 100.0;
        let drag_like = ConstantAcceleration(Vector3::new(1e-6, 0.0, 0.0));
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(drag_like)
            .with_propellant(PropellantPool::new(FLOOR))
            .with_propulsion(Thruster::new(10.0, 300.0, Vector3::x()));

        let with_fuel = SpacecraftState {
            mass: FLOOR + 1.0,
            ..sample_spacecraft()
        };
        let burning =
            dynamics.derivatives(0.0, &dynamics.initial_augmented_state(with_fuel.clone()));
        assert!(
            burning.plant.mass < 0.0,
            "with propellant left the thruster burns it, at {}",
            burning.plant.mass
        );

        // Exactly on the floor is empty: the propellant is what is above it.
        let empty = SpacecraftState {
            mass: FLOOR,
            ..sample_spacecraft()
        };
        // On the floor from the start, so its mode says empty from the first
        // step: a margin of exactly zero is not a crossing a search can find.
        let dry = dynamics.derivatives(0.0, &dynamics.initial_augmented_state(empty.clone()));
        assert_eq!(
            dry.plant.mass, 0.0,
            "nothing left to burn, so nothing burns"
        );

        // The drag-like model is still evaluated, so the acceleration is its
        // own and the thrust is gone rather than everything being gone.
        let others_only = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_model(ConstantAcceleration(Vector3::new(1e-6, 0.0, 0.0)));
        let expected =
            others_only.derivatives(0.0, &others_only.initial_augmented_state(empty.clone()));
        assert!(
            (dry.plant.orbit.velocity() - expected.plant.orbit.velocity()).magnitude() < 1e-15,
            "an empty tank leaves the other models where they were"
        );

        // And the record says the same, so a sample cannot show thrust the
        // trajectory never felt. The entry stays — the telemetry's column for
        // it would otherwise go sparse — and what it carries is a zero, which
        // `a_breakdown_keeps_an_entry_for_propulsion_the_pool_switched_off`
        // pins.
        let dry_record = dynamics.model_breakdown(0.0, &dynamics.initial_augmented_state(empty));
        let (_, thruster) = dry_record
            .iter()
            .find(|(name, _)| *name == "thruster")
            .expect("the thruster keeps its entry");
        assert_eq!(thruster.mass_rate, 0.0, "and reports no thrust in it");

        let wet_record =
            dynamics.model_breakdown(0.0, &dynamics.initial_augmented_state(with_fuel));
        let (_, thruster) = wet_record
            .iter()
            .find(|(name, _)| *name == "thruster")
            .expect("the thruster has an entry");
        assert!(
            thruster.mass_rate < 0.0,
            "while there is propellant the record shows the burn, at {}",
            thruster.mass_rate
        );
    }

    /// A boundary search steps past the propellant floor on purpose — it
    /// re-steps an interval under the mode that held before the crossing — so
    /// a wide enough step takes a stage below zero mass. Every model that
    /// turns a force into an acceleration divides by the mass there, so the
    /// derivative would be an infinity and the walk would fail on a state it
    /// was going to discard.
    ///
    /// The policy is the system's, not each model's: a thruster guarding
    /// itself leaves a panel's drag to produce the infinity instead.
    #[test]
    fn a_state_with_no_mass_gets_finite_derivatives() {
        use crate::model::HasMass;
        use crate::spacecraft::{PropellantPool, Thruster};

        // An effector, rather than a model, that turns a force into an
        // acceleration: `StateEffector` supports translational ones, so the
        // policy has to reach them too.
        struct ForceEffector;

        impl<S: HasFrame + HasMass + Send + Sync> StateEffector<S> for ForceEffector {
            fn name(&self) -> &str {
                "force_effector"
            }
            fn state_dim(&self) -> usize {
                0
            }
            fn derivatives(
                &self,
                input: EffectorInput<'_, S>,
                _aux_rates: &mut [f64],
            ) -> ExternalLoads<S::Frame> {
                // 1 N along x, in km/s² as the loads are.
                ExternalLoads::acceleration(Vector3::new(
                    1.0 / input.state.mass() / 1000.0,
                    0.0,
                    0.0,
                ))
            }
        }

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            // Something that divides by mass and is not propulsion.
            .with_model(ConstantForce(Vector3::new(1.0, 0.0, 0.0)))
            .with_effector(ForceEffector)
            .with_propellant(PropellantPool::new(1.0))
            .with_propulsion(Thruster::new(196.133, 200.0, Vector3::x()));

        for mass in [0.0, -1e-6, -10.0] {
            let plant = SpacecraftState {
                mass,
                ..sample_spacecraft()
            };
            // Built by hand: `initial_augmented_state` refuses a mass below
            // the floor, which is the input error. This is the trial state a
            // search produces, which has to be evaluable.
            let state = AugmentedState {
                plant,
                aux: vec![],
                aux_bounds: vec![],
                // One mode, the pool's: the force effector registers none.
                modes: vec![ConstraintMode::Free],
            };
            let d = dynamics.derivatives(0.0, &state);
            assert!(
                d.plant.orbit.velocity().iter().all(|c| c.is_finite())
                    && d.plant.mass.is_finite()
                    && d.plant
                        .attitude
                        .angular_velocity
                        .iter()
                        .all(|c| c.is_finite()),
                "at a mass of {mass} the derivative is {:?}",
                d.plant.orbit.velocity()
            );
            // The mass rate is the half that is not singular, and the
            // search reads the crossing from it.
            assert!(
                d.plant.mass < 0.0,
                "and the burn keeps its flow there, at {}",
                d.plant.mass
            );

            // The record of such a state says the same thing the trajectory
            // did: a telemetry sample taken at a state a walk is about to
            // discard would otherwise carry an infinity.
            for (name, loads) in dynamics.model_breakdown(0.0, &state) {
                assert!(
                    loads.acceleration_inertial.is_finite(),
                    "{name} reports {:?} at a mass of {mass}",
                    loads.acceleration_inertial
                );
            }
        }
    }

    /// A pool is the pool wherever a caller registered it. `with_effector` is
    /// public and `PropellantPool` is an effector, so the generic path could
    /// otherwise leave a pool the system does not know about: its floor would
    /// declare a boundary that stops no thruster, and no state would carry the
    /// mode it needs.
    #[test]
    fn a_pool_registered_as_a_plain_effector_is_still_the_pool() {
        use crate::spacecraft::{PropellantPool, Thruster};

        const FLOOR: f64 = 100.0;
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_effector(PropellantPool::new(FLOOR))
            .with_propulsion(Thruster::new(10.0, 300.0, Vector3::x()));

        assert_eq!(
            dynamics.pool().map(|p| p.dry_mass()),
            Some(FLOOR),
            "the system knows the pool it was handed"
        );

        // And it gates on it: on the floor from the start, so nothing burns.
        let empty = SpacecraftState {
            mass: FLOOR,
            ..sample_spacecraft()
        };
        let dry = dynamics.derivatives(0.0, &dynamics.initial_augmented_state(empty));
        assert_eq!(dry.plant.mass, 0.0);
    }

    /// Two floors are what one pool exists to prevent, whichever door they
    /// came through.
    #[test]
    #[should_panic(expected = "one propellant pool")]
    fn a_second_pool_is_refused() {
        use crate::spacecraft::PropellantPool;

        let _: SpacecraftDynamics<PointMass> =
            SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
                .with_propellant(PropellantPool::new(100.0))
                .with_effector(PropellantPool::new(200.0));
    }

    /// A propulsion model with no pool behind it would thrust forever: the
    /// floor is what says when the spacecraft is empty.
    #[test]
    #[should_panic(expected = "needs the propellant it burns")]
    fn propulsion_without_a_pool_is_refused() {
        use crate::spacecraft::Thruster;

        SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_propulsion(Thruster::new(10.0, 300.0, Vector3::x()));
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

    /// A force, which is what a real model has: the acceleration it produces
    /// is the force over the mass, and that is the division a state with no
    /// mass cannot survive. `PanelDrag` and `PanelSrp` do the same.
    struct ConstantForce(Vector3<f64>);

    impl Model<SpacecraftState> for ConstantForce {
        fn name(&self) -> &str {
            "const_force_n"
        }
        fn eval(&self, _t: f64, state: &SpacecraftState, _epoch: Option<&Epoch>) -> ExternalLoads {
            // N / kg = m/s², and km/s² is what the loads carry.
            ExternalLoads::acceleration(self.0 / state.mass / 1000.0)
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

        let breakdown = dyn_sc.model_breakdown(0.0, &dyn_sc.initial_augmented_state(sc));
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
        let state = dynamics.initial_augmented_state(sample_spacecraft());

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
        let state = dynamics.initial_augmented_state(sample_spacecraft());

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
        let state = dynamics.initial_augmented_state(sample_spacecraft());

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
