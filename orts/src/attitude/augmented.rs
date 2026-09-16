//! Augmented attitude dynamics system with StateEffector support.
//!
//! Extends [`DecoupledAttitudeSystem`] to support [`StateEffector`]s —
//! components with internal state (e.g., reaction wheels) that are
//! integrated alongside the attitude state.

use arika::epoch::Epoch;
use nalgebra::{Matrix3, Vector3};
use utsuroi::{DynamicalSystem, SegmentContext};

use crate::OrbitalState;
use crate::attitude::DecoupledContext;
use crate::attitude::state::AttitudeState;
use crate::boundary::DeclaredBoundary;
use crate::effector::{AugmentedState, AuxRegistry, ConstraintMode, EffectorInput, StateEffector};
use crate::model::ExternalLoads;
use crate::model::{EvalSegment, Model, eval_maybe_in_segment};

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
        self.registry
            .register(effector.name(), dim, effector.mode_dim());
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
            modes: vec![ConstraintMode::default(); self.registry.total_modes()],
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

impl AugmentedAttitudeSystem {
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
        state: &AugmentedState<AttitudeState>,
    ) -> AugmentedState<AttitudeState> {
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));

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
            total += eval_maybe_in_segment(m, segment, t, &context, epoch.as_ref());
        }

        // 3. Evaluate state effectors
        let mut aux_rates = vec![0.0; self.registry.total_dim()];
        for (i, eff) in self.effectors.iter().enumerate() {
            let entry = &self.registry.entries()[i];
            let rates_slice = &mut aux_rates[entry.offset..entry.offset + entry.dim];
            total += eff.derivatives(
                EffectorInput {
                    t,
                    state: &context,
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
        let alpha = self.inertia_inv
            * (total.torque_body.into_inner() - state.plant.angular_velocity.cross(&iw));

        AugmentedState {
            plant: AttitudeState::from_derivative(q_dot, alpha),
            aux: aux_rates,
            aux_bounds: state.aux_bounds.clone(),
            modes: state.modes.clone(),
        }
    }
}

impl crate::boundary::HasBoundaries for AugmentedAttitudeSystem {
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
        // The same context the derivatives are taken in: the orbit and mass
        // this system prescribes at `t`, around the attitude being examined.
        let context = DecoupledContext {
            attitude: state.plant.clone(),
            orbit: (self.orbit_fn)(t),
            mass: (self.mass_fn)(t),
        };
        let epoch = self.epoch_0.map(|e| e.add_si_seconds(t));
        let segment_epoch = segment.and_then(|s| self.epoch_0.map(|e| e.add_si_seconds(s.start)));
        let eval_segment = segment.map(|s| EvalSegment::new(s, segment_epoch.as_ref()));
        self.effectors[declared.effector].boundary_value(
            declared.boundary.kind,
            EffectorInput {
                t,
                state: &context,
                aux: &state.aux[declared.aux_offset..declared.aux_offset + declared.aux_dim],
                modes: state
                    .modes
                    .get(declared.mode_offset..declared.mode_offset + declared.mode_dim)
                    .unwrap_or(&[]),
                epoch: epoch.as_ref(),
                // The same segment the derivatives were taken in; see
                // [`HasBoundaries::boundary_value`].
                segment: eval_segment.as_ref(),
            },
        )
    }

    fn settle_boundary(&self, declared: &DeclaredBoundary, state: &mut Self::State) {
        let aux = &mut state.aux[declared.aux_offset..declared.aux_offset + declared.aux_dim];
        if let Some(exchange) =
            self.effectors[declared.effector].settle_boundary(declared.boundary.kind, aux)
        {
            state.plant.angular_velocity += self.inertia_inv * exchange.angular_momentum_body;
        }
        state.modes[declared.mode_index()] = declared.boundary.kind.mode_after();
    }

    fn boundary_is_active(&self, declared: &DeclaredBoundary, state: &Self::State) -> bool {
        declared.is_active(&state.modes)
    }
}

impl DynamicalSystem for AugmentedAttitudeSystem {
    type State = AugmentedState<AttitudeState>;

    /// The earliest boundary any model or effector reports.
    ///
    /// A model or effector that switches on a schedule reports when; a system
    /// holding one has to pass that on, or a propagation loop stepping the
    /// system would never see it.
    ///
    /// `orbit_fn` and `mass_fn` are the caller's own functions, so their
    /// breakpoints are not reported here — a caller who passes a piecewise one
    /// holds its schedule already, and hands it to the propagation loop the same
    /// way it hands over the span.
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
        state: &AugmentedState<AttitudeState>,
    ) -> AugmentedState<AttitudeState> {
        self.derivatives_for(None, t, state)
    }

    fn derivatives_in_segment(
        &self,
        segment: &SegmentContext,
        t: f64,
        state: &AugmentedState<AttitudeState>,
    ) -> AugmentedState<AttitudeState> {
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
    use nalgebra::Vector4;

    fn symmetric_inertia(i: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(i, i, i))
    }

    /// This system answers for its effectors' boundaries itself — it flattens
    /// its own declarations and gives a wheel's overshoot back through its own
    /// inertia — and its state carries no position, so no group can propagate
    /// it. `walk_to_target` is the path such a caller takes, and this walks a
    /// wheel to its limit through it: the wheel is held on the bound, its mode
    /// says so, and the body keeps the momentum the wheel stopped taking.
    #[test]
    fn a_wheel_saturating_under_this_system_keeps_the_bodys_momentum() {
        use crate::boundary::{Boundaries, HasBoundaries, Span, walk_to_target};
        use crate::effector::ConstraintMode;
        use crate::spacecraft::{ReactionWheelAssembly, RwCommand};
        use core::ops::ControlFlow;
        use utsuroi::{Integrator, Rk4, RootSearch, RootSlot};

        const BODY_INERTIA: f64 = 10.0;
        const MAX_MOMENTUM: f64 = 0.53;
        const MAX_TORQUE: f64 = 0.1;
        // The z wheel takes a torque about z alone and reaches its limit at
        // t = 5.3 s, between the ticks of this grid.
        const DT: f64 = 0.25;

        let mut rw = ReactionWheelAssembly::three_axis(0.01, MAX_MOMENTUM, MAX_TORQUE);
        rw.command = RwCommand::Torques(rw.core().allocate(&Vector3::new(0.0, 0.0, MAX_TORQUE)));
        let system = AugmentedAttitudeSystem::circular_orbit(
            symmetric_inertia(BODY_INERTIA),
            398600.4418,
            7000.0,
            100.0,
        )
        .with_effector(rw);

        let initial = system.initial_augmented_state(AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        });
        // Isotropic inertia with one wheel axis driven, so the body-frame total
        // is constant rather than merely constant in magnitude.
        let total = |state: &AugmentedState<AttitudeState>| {
            BODY_INERTIA * state.plant.angular_velocity
                + Vector3::new(state.aux[0], state.aux[1], state.aux[2])
        };
        let started_with = total(&initial);

        let boundaries = system.boundaries();
        let mut slots = vec![RootSlot::new(); boundaries.len()];
        let (_, _, ended) = walk_to_target(
            Boundaries {
                system: &system,
                declared: &boundaries,
                slots: &mut slots,
                search: RootSearch::default(),
                segment: None,
            },
            Span {
                from: 0.0,
                to: 20.0,
                start_is_checked: false,
            },
            initial,
            |state, t, _checked| Rk4.stepper(&system, state, t, DT),
            &mut |_: f64, _: &AugmentedState<AttitudeState>| {},
            &|_: f64, _: &AugmentedState<AttitudeState>| -> ControlFlow<()> {
                ControlFlow::Continue(())
            },
        )
        .expect("the walk succeeds");

        assert!(
            (ended.aux[2] + MAX_MOMENTUM).abs() < 1e-6,
            "the z wheel ends held at its lower bound, not at {}",
            ended.aux[2]
        );
        assert_eq!(
            ended.modes[2],
            ConstraintMode::Lower,
            "and its mode says which bound holds it"
        );
        let lost = (total(&ended) - started_with).magnitude();
        assert!(
            lost < 1e-9,
            "{lost:.3e} N·m·s of the body-frame total went missing"
        );
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
            modes: vec![],
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
