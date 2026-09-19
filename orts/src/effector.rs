//! StateEffector trait, AugmentedState, and AuxRegistry for components
//! with internal state that couples to spacecraft dynamics.
//!
//! Unlike [`Model<S>`](crate::model::Model) which is a pure function,
//! a [`StateEffector`] has auxiliary state variables (e.g., reaction wheel
//! angular momentum) that are integrated by the ODE solver alongside the
//! plant state.

use arika::epoch::Epoch;
use arika::frame::{Body, Vec3};
use utsuroi::{Crossing, OdeState, Projection, Tolerances};

use crate::model::{ExternalLoads, HasFrame};

// StateEffector trait

/// A physical component with internal state that couples to spacecraft dynamics.
///
/// Unlike `Model<S>` (pure function), StateEffector has auxiliary state
/// that is integrated by the ODE solver alongside the plant state.
/// Examples: reaction wheels (angular momentum), gimbals, fuel slosh.
///
/// The `derivatives` method writes aux_rates into a caller-owned buffer
/// to avoid allocation in the ODE hot path.
///
/// The loads come back in the frame the state is propagated in
/// ([`HasFrame::Frame`]), mirroring [`Model<S>`]: an effector contributes
/// directly in that frame, so the host never re-tags coordinates. A torque-only
/// effector such as a reaction wheel writes `impl<S: HasFrame + HasAttitude>
/// StateEffector<S>` and leaves the acceleration zero, since body-frame torque
/// names no inertial frame. A translational effector must produce its inertial
/// acceleration in `S::Frame` — by rotating a body-frame vector through the
/// state's own attitude, not by tagging raw numbers (see issue #103).
///
/// [`Model<S>`]: crate::model::Model
pub trait StateEffector<S: HasFrame>: Send + Sync + std::any::Any {
    /// Human-readable name for this effector (e.g., "reaction_wheels").
    fn name(&self) -> &str;

    /// The next time after `t` at which this effector's contribution to the
    /// right-hand side changes discontinuously, if it knows one in advance.
    ///
    /// An effector is part of the right-hand side alongside the models, and
    /// [`derivatives`](Self::derivatives) receives `t` and `epoch`, so one
    /// driven by a schedule can switch. The contract is the same as
    /// [`Model::next_discontinuity_after`](crate::model::Model::next_discontinuity_after):
    /// integration time, strictly after `t`, finite, and only for switches whose
    /// time is known without integrating.
    ///
    /// A reaction wheel reaching its momentum limit is not one of those — when
    /// that happens follows from the trajectory. Issue #446 covers those as
    /// state events.
    fn next_discontinuity_after(&self, _t: f64, _epoch: Option<&Epoch>) -> Option<f64> {
        None
    }

    /// Number of scalar state variables this effector contributes.
    fn state_dim(&self) -> usize;

    /// The boundaries this effector's state can reach.
    ///
    /// One per side of each one-sided constraint, plus one for its release: a
    /// reaction wheel assembly declares three per wheel, and a fourth — the
    /// turning point of the momentum — for each wheel whose motor lags. Empty,
    /// unless overridden.
    ///
    /// The propagation turns these into root events, so the modes change only
    /// where a walk stopped — which is what keeps the right-hand side fixed
    /// for the search that found the stopping point.
    ///
    /// Declaring a boundary binds nothing by itself: the constraint it stands
    /// for holds on the paths that run
    /// [`walk_to_target`](crate::boundary::walk_to_target) — the groups and the
    /// CLI's controlled path — and a plain `Integrator::integrate` steps
    /// straight past it with every mode left as it was.
    fn boundaries(&self) -> Vec<EffectorBoundary> {
        Vec::new()
    }

    /// The signed value one declared boundary is found in, at this stage.
    ///
    /// Zero is the boundary, and the sign says which side of it the state is
    /// on. For a bound this is the margin left — the quantity against its
    /// limit — and for a release it is the rate that has to turn around, so
    /// that both are crossings of zero from the side the mode was entered on.
    ///
    /// Only asked about boundaries this effector declared, and only in a mode
    /// where [`BoundaryKind::is_active`] holds.
    fn boundary_value(&self, _kind: BoundaryKind, _input: EffectorInput<'_, S>) -> f64 {
        0.0
    }

    /// Put this effector's state exactly on a boundary it just reached, and
    /// report what the overshoot gives back to the plant.
    ///
    /// The quantity is a little past its bound — by the rate times the width of
    /// the bracket the search ended on — and the plant has already integrated
    /// the exchange that carried it there. Moving the quantity back without
    /// returning that much would lose it from a total that is conserved.
    ///
    /// `None` for a boundary that moves nothing, such as a release.
    fn settle_boundary(&self, _kind: BoundaryKind, _aux: &mut [f64]) -> Option<BoundaryExchange> {
        None
    }

    /// Number of discrete modes this effector carries.
    ///
    /// One per one-sided constraint it can be held against — a reaction wheel
    /// assembly has one per wheel. Zero, unless overridden, for an effector
    /// whose right-hand side is the same whatever the state is.
    fn mode_dim(&self) -> usize {
        0
    }

    /// Whether this effector's part of a state is one a propagation can start
    /// from, with the reason when it is not.
    ///
    /// The slices are this effector's own (lengths `state_dim()` and
    /// `mode_dim()`), and `plant` is the state they belong to — a constraint on
    /// a plant quantity, such as a propellant floor under the mass, reads it
    /// there.
    ///
    /// Answer for what the constraint means, not for the sign of the boundary
    /// value: a quantity resting exactly on its bound in the mode that holds
    /// it there is a legal start. Only a state no trajectory of this effector
    /// could be at belongs in an `Err`, since the propagation refuses it
    /// instead of moving it ([`HasBoundaries::validate_boundary_walk_start`](crate::boundary::HasBoundaries::validate_boundary_walk_start)).
    fn validate_state(
        &self,
        _plant: &S,
        _aux: &[f64],
        _modes: &[ConstraintMode],
    ) -> Result<(), String> {
        Ok(())
    }

    /// Loads on the spacecraft and derivatives of this effector's auxiliary
    /// state, at the stage [`EffectorInput`] describes.
    ///
    /// `aux_rates` is the output buffer (length = `state_dim()`). The returned
    /// [`ExternalLoads`] are already in the frame the state is propagated in
    /// ([`HasFrame::Frame`]).
    ///
    /// # The state can be outside the physical domain
    ///
    /// As for [`Model::eval`](crate::model::Model::eval): a boundary search
    /// steps past a constraint on purpose, so a stage can arrive with a mass
    /// of zero or less. Write finite rates and return loads there rather than
    /// panicking; `SpacecraftDynamics` drops the acceleration and keeps the
    /// rest.
    fn derivatives(
        &self,
        input: EffectorInput<'_, S>,
        aux_rates: &mut [f64],
    ) -> ExternalLoads<S::Frame>;

    /// Per-element (min, max) bounds for auxiliary state projection.
    ///
    /// By default returns unbounded `(-INF, +INF)` for each element. Override
    /// for a quantity whose bound can be imposed by moving that element alone
    /// — a reaction wheel's realized motor torque, which tracks a command the
    /// driver already limits.
    ///
    /// A bound that a conserved total is shared across does not belong here:
    /// clamping one side of an exchange the rest of the state has already
    /// integrated destroys that much of the total. Such a bound is a boundary
    /// the propagation locates, through
    /// [`boundaries`](Self::boundaries) and
    /// [`settle_boundary`](Self::settle_boundary), which is where a wheel's
    /// momentum limit is kept.
    fn aux_bounds(&self) -> Vec<(f64, f64)> {
        vec![(f64::NEG_INFINITY, f64::INFINITY); self.state_dim()]
    }
}

/// Everything a [`StateEffector`] is evaluated at.
///
/// One value rather than a row of arguments, so that an input the effectors
/// did not have before — the discrete modes — reaches every implementation
/// without touching the ones that ignore it.
pub struct EffectorInput<'a, S> {
    /// Integration time of the stage.
    pub t: f64,
    /// The plant state at that stage.
    pub state: &'a S,
    /// This effector's slice of the auxiliary state (length = `state_dim()`).
    pub aux: &'a [f64],
    /// This effector's slice of the discrete modes (length = `mode_dim()`).
    ///
    /// Read-only here: a mode changes where a walk stops, never inside a step.
    pub modes: &'a [ConstraintMode],
    /// The epoch of the stage, for a schedule written in epochs.
    pub epoch: Option<&'a Epoch>,
    /// The segment the solver is stepping through, when there is one.
    ///
    /// An effector that reports a boundary through
    /// [`next_discontinuity_after`](StateEffector::next_discontinuity_after)
    /// has its switch turned into the end of a segment, and the stage that
    /// lands there belongs to the step integrating the segment before it.
    /// Answering for [`segment.start`](crate::model::EvalSegment::start) — and
    /// [`segment.start_epoch`](crate::model::EvalSegment::start_epoch) for a
    /// schedule written in epochs — keeps that stage on the inside of a
    /// half-open interval ending there.
    ///
    /// `t`, `state`, `aux` and `epoch` still describe the stage, so an effector
    /// whose contribution varies continuously — a reaction wheel following its
    /// own momentum — keeps using them. Only a switch in time is held.
    pub segment: Option<&'a crate::model::EvalSegment<'a>>,
}

// Written out rather than derived: the state is behind a reference, so neither
// copying nor cloning an input asks anything of the state's own type.
impl<S> Clone for EffectorInput<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for EffectorInput<'_, S> {}

// Boundaries

/// Which boundary of a one-sided constraint an effector declared.
///
/// Reaching a bound and releasing from it are separate boundaries, because they
/// are found in different quantities: the first in the constrained quantity
/// itself, the second in the rate whatever pushes against the bound is asking
/// for. Only one of them means anything in a given mode, which is what
/// [`is_active`](Self::is_active) answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    /// The constrained quantity `index` reaches its upper bound.
    ReachedUpper {
        /// Which of this effector's constrained quantities.
        index: usize,
    },
    /// The constrained quantity `index` reaches its lower bound.
    ReachedLower {
        /// Which of this effector's constrained quantities.
        index: usize,
    },
    /// The constraint held at `index` releases: what pushed it against the
    /// bound has turned around.
    Released {
        /// Which of this effector's constrained quantities.
        index: usize,
    },
    /// The rate carrying the free quantity `index` passes through zero, so the
    /// quantity turns around there.
    ///
    /// Nothing is held and nothing is settled: this boundary is there to give
    /// the search a root inside the step. A quantity that runs past its bound
    /// and comes back within one step shows the same sign of margin at the
    /// step's two ends, and those two ends are all the detection reads.
    ///
    /// What recovers the bound is the narrowing, not the turn itself:
    /// localizing this root takes [`RootSet`](utsuroi::RootSet) through trial
    /// widths that end inside the excursion, where the margin does differ in
    /// sign from the step's start, and the bound is added to the candidates and
    /// localized from the step's start. The bound is therefore reported at the
    /// time it was reached, ahead of the turn — committing the turn first would
    /// put the state past the bound before anything noticed.
    TurningPoint {
        /// Which of this effector's constrained quantities.
        index: usize,
    },
}

impl BoundaryKind {
    /// Which of this effector's constrained quantities this boundary belongs
    /// to.
    pub fn index(self) -> usize {
        match self {
            Self::ReachedUpper { index }
            | Self::ReachedLower { index }
            | Self::Released { index }
            | Self::TurningPoint { index } => index,
        }
    }

    /// Whether this boundary is one the search should look at, in the mode the
    /// state is in.
    ///
    /// A bound cannot be reached while the constraint is already held against
    /// one, and there is nothing to release while it is free. A turn of the
    /// rate matters in the same mode a bound does: while the quantity is held,
    /// the rate turning around is what [`Released`](Self::Released) reads.
    pub fn is_active(self, modes: &[ConstraintMode]) -> bool {
        let mode = modes.get(self.index()).copied().unwrap_or_default();
        match self {
            Self::ReachedUpper { .. } | Self::ReachedLower { .. } | Self::TurningPoint { .. } => {
                mode == ConstraintMode::Free
            }
            Self::Released { .. } => mode != ConstraintMode::Free,
        }
    }

    /// The mode the constraint is in once this boundary has been handled, or
    /// `None` for one that leaves both the state and the mode where they are.
    ///
    /// `None` is what makes a boundary a split and nothing else: the walk stops
    /// at it, so the search starts again from there, and the state it resumes
    /// with is the one it stopped on.
    pub fn mode_after(self) -> Option<ConstraintMode> {
        match self {
            Self::ReachedUpper { .. } => Some(ConstraintMode::Upper),
            Self::ReachedLower { .. } => Some(ConstraintMode::Lower),
            Self::Released { .. } => Some(ConstraintMode::Free),
            Self::TurningPoint { .. } => None,
        }
    }

    /// Which way across zero counts as reaching this boundary.
    ///
    /// A bound and a release are read as margins running out, so a fall
    /// through zero is the crossing. A turn of the rate counts in either
    /// direction, and only from a rate that was not already zero: a motor
    /// whose realized torque starts at zero with a command to follow is
    /// starting to move, not turning around.
    pub fn crossing(self) -> Crossing {
        match self {
            Self::ReachedUpper { .. } | Self::ReachedLower { .. } | Self::Released { .. } => {
                Crossing::Falling
            }
            Self::TurningPoint { .. } => Crossing::Reversal,
        }
    }
}

/// What the plant takes back when an effector's state is put on a boundary.
///
/// The search stops on the far side of a bracket, so the quantity is a little
/// past its bound and the plant has already felt the exchange that carried it
/// there. Putting the quantity back on the bound without this would destroy
/// that much of a conserved total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundaryExchange {
    /// Angular momentum to return to the plant [N·m·s].
    ///
    /// The body equations are what consume it, through the inertia that turns
    /// angular momentum into a rate, so the frame is part of the type: a
    /// correction expressed in any other frame cannot be built here.
    pub angular_momentum_body: Vec3<Body>,
    /// The mass the plant is left with, where a boundary fixes it [kg].
    ///
    /// This is not the same kind of exchange the angular momentum is: a
    /// spacecraft that burned past its propellant floor cannot be given the
    /// propellant back, because the exhaust carried it — and its momentum —
    /// out of the system. What this corrects is the state: the mass belongs on
    /// the floor, and the impulse for the propellant it spent below the floor
    /// stays in the velocity, as the error of locating the time.
    pub mass: Option<f64>,
}

impl Default for BoundaryExchange {
    /// Nothing to give back: the effector moved its own state and the plant
    /// keeps what it has.
    fn default() -> Self {
        Self {
            angular_momentum_body: Vec3::zeros(),
            mass: None,
        }
    }
}

/// Check that every boundary an effector declares names a mode it registered.
///
/// [`BoundaryKind::index`] selects the mode inside the effector's own block, so
/// an index past its `mode_dim` reads a mode that is not there — the boundary
/// then looks active, because a missing mode reads as
/// [`ConstraintMode::Free`], and settling it writes into the *next* effector's
/// block or past the end of the vector. Registration is where this is caught:
/// `boundaries` takes only `&self`, so what an effector declares cannot change
/// afterwards.
///
/// # Panics
///
/// If any declared boundary's index is not below `mode_dim`.
pub(crate) fn check_declared_modes(name: &str, boundaries: &[EffectorBoundary], mode_dim: usize) {
    for boundary in boundaries {
        let index = boundary.kind.index();
        assert!(
            index < mode_dim,
            "effector {name} declared a boundary on mode {index}, but registered \
             {mode_dim} mode(s): a boundary can only name a mode of its own effector"
        );
    }
}

/// A boundary an effector's state can reach, for the propagation to stop at.
///
/// The value it is found in is the effector's to compute
/// ([`StateEffector::boundary_value`]), and for a boundary that moves a mode it
/// is a *margin*: positive while the boundary is still ahead, zero on it,
/// negative past it. Reaching such a boundary is therefore one direction — the
/// margin running out — and a bound whose value would rise into it is written
/// with its sign flipped. That is what lets the propagation recognise a state
/// that is *already* past a boundary, which no search can find: there is one
/// side to be past.
///
/// A boundary whose [`BoundaryKind::mode_after`] is `None` is read differently.
/// It cuts the step at a time the walk has to stop at, and its value is the
/// rate whose zero is that time: both signs are ordinary states to be at, in
/// either order, and neither is "past" anything. The propagation leaves these
/// out of the reconciliation it does at a walk's start for that reason, and
/// their crossing counts in either direction
/// ([`BoundaryKind::crossing`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectorBoundary {
    /// Which boundary of which constrained quantity.
    pub kind: BoundaryKind,
    /// Width of the value within which the state still counts as being on the
    /// boundary, in the value's own units.
    ///
    /// A constraint the state moves *along* — a wheel held at its bound — has
    /// its value jittering around zero for many steps, and each change of sign
    /// in that jitter would otherwise be a fresh crossing.
    pub boundary_tolerance: f64,
}

// ConstraintMode

/// Which side of a one-sided constraint an effector is held against.
///
/// A reaction wheel at its momentum bound cannot be driven further out, and a
/// tank that has run dry cannot deliver propellant: the right-hand side changes
/// when the boundary is reached, and changes back when the thing pushing
/// against it turns around. That switch is discrete, so it belongs to the
/// state rather than to a comparison inside the right-hand side — a comparison
/// would flip mid-step, and a root search re-stepping across the flip converges
/// on the wrong time (see the root-event contract in DESIGN.md).
///
/// Nothing in the integration writes a mode. It changes where the walk stops:
/// at a boundary a [`RootEvent`](utsuroi::RootEvent) located, or at a time the
/// caller reconciles the modes with a command it just applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConstraintMode {
    /// Inside the bounds: the unconstrained right-hand side holds.
    #[default]
    Free,
    /// Held at the upper bound.
    Upper,
    /// Held at the lower bound.
    Lower,
}

// AugmentedState

/// Plant state augmented with auxiliary effector state.
///
/// The ODE solver integrates this composite state, where `plant` is the
/// primary dynamics state (e.g., `AttitudeState`) and `aux` holds the
/// concatenated auxiliary variables from all registered [`StateEffector`]s.
///
/// `aux_bounds` provides per-element `(min, max)` bounds that are enforced
/// during [`OdeState::project`] (e.g., reaction wheel momentum saturation).
#[derive(Debug, Clone, PartialEq)]
pub struct AugmentedState<S: OdeState> {
    /// Primary dynamics state (e.g., attitude quaternion + angular velocity).
    pub plant: S,
    /// Concatenated auxiliary state from all registered effectors.
    pub aux: Vec<f64>,
    /// Per-element (min, max) bounds for auxiliary state projection.
    /// Empty means no bounds (unconstrained).
    pub aux_bounds: Vec<(f64, f64)>,
    /// Concatenated discrete modes from all registered effectors.
    ///
    /// Carried through every state operation as it is: a mode is not a quantity
    /// to scale or add, and a trajectory that mixed two of them would be a
    /// trajectory of neither.
    pub modes: Vec<ConstraintMode>,
}

impl<S: OdeState> From<S> for AugmentedState<S> {
    /// Wrap a plant state as augmented with no effectors (empty aux).
    fn from(plant: S) -> Self {
        Self {
            plant,
            aux: vec![],
            aux_bounds: vec![],
            modes: vec![],
        }
    }
}

impl<S: OdeState> OdeState for AugmentedState<S> {
    fn zero_like(&self) -> Self {
        Self {
            plant: self.plant.zero_like(),
            aux: vec![0.0; self.aux.len()],
            aux_bounds: self.aux_bounds.clone(),
            modes: self.modes.clone(),
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        let mut aux = self.aux.clone();
        for (a, o) in aux.iter_mut().zip(other.aux.iter()) {
            *a += scale * o;
        }
        Self {
            plant: self.plant.axpy(scale, &other.plant),
            aux,
            aux_bounds: self.aux_bounds.clone(),
            modes: self.modes.clone(),
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Self {
            plant: self.plant.scale(factor),
            aux: self.aux.iter().map(|v| v * factor).collect(),
            aux_bounds: self.aux_bounds.clone(),
            modes: self.modes.clone(),
        }
    }

    fn is_finite(&self) -> bool {
        self.plant.is_finite() && self.aux.iter().all(|v| v.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let plant_norm = self.plant.error_norm(&y_next.plant, &error.plant, tol);

        // Aux error norm: RMS of scaled errors
        if self.aux.is_empty() {
            return plant_norm;
        }

        let mut sum_sq = 0.0;
        for i in 0..self.aux.len() {
            let sc = tol.atol + tol.rtol * self.aux[i].abs().max(y_next.aux[i].abs());
            let e = error.aux[i] / sc;
            sum_sq += e * e;
        }
        let aux_norm = (sum_sq / self.aux.len() as f64).sqrt();
        plant_norm.max(aux_norm)
    }

    fn project(&mut self, t: f64) -> Projection {
        let mut projection = self.plant.project(t);
        // Clamp auxiliary state to bounds (e.g., reaction wheel momentum limits)
        for (i, &(lo, hi)) in self.aux_bounds.iter().enumerate() {
            let clamped = self.aux[i].clamp(lo, hi);
            if clamped != self.aux[i] {
                self.aux[i] = clamped;
                projection = projection.or(Projection::Changed);
            }
        }
        projection
    }
}

// AuxRegistry

/// Metadata for a registered auxiliary state block.
#[derive(Debug, Clone)]
pub struct AuxEntry {
    /// Human-readable name of the effector owning this block.
    pub name: String,
    /// Starting index within the concatenated aux vector.
    pub offset: usize,
    /// Number of scalar variables in this block.
    pub dim: usize,
    /// Starting index within the concatenated mode vector.
    pub mode_offset: usize,
    /// Number of discrete modes in this block.
    pub mode_dim: usize,
}

/// Registry mapping [`StateEffector`]s to their auxiliary state slices.
///
/// Each effector is assigned a contiguous block of the aux vector.
/// The registry tracks the offset and dimension of each block.
#[derive(Debug, Default)]
pub struct AuxRegistry {
    entries: Vec<AuxEntry>,
    total_dim: usize,
    total_modes: usize,
}

impl AuxRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Default::default()
    }

    /// Register a new effector and return its offset into the aux vector.
    pub fn register(&mut self, name: &str, dim: usize, mode_dim: usize) -> usize {
        let offset = self.total_dim;
        self.entries.push(AuxEntry {
            name: name.to_string(),
            offset,
            dim,
            mode_offset: self.total_modes,
            mode_dim,
        });
        self.total_dim += dim;
        self.total_modes += mode_dim;
        offset
    }

    /// Total number of auxiliary state variables across all effectors.
    pub fn total_dim(&self) -> usize {
        self.total_dim
    }

    /// Total number of discrete modes across all effectors.
    pub fn total_modes(&self) -> usize {
        self.total_modes
    }

    /// All registered entries.
    pub fn entries(&self) -> &[AuxEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use nalgebra::{Vector3, Vector4};

    // AuxRegistry tests

    #[test]
    fn registry_single_effector() {
        let mut reg = AuxRegistry::new();
        let offset = reg.register("rw", 3, 0);
        assert_eq!(offset, 0);
        assert_eq!(reg.total_dim(), 3);
        assert_eq!(reg.entries().len(), 1);
        assert_eq!(reg.entries()[0].name, "rw");
        assert_eq!(reg.entries()[0].offset, 0);
        assert_eq!(reg.entries()[0].dim, 3);
    }

    #[test]
    fn registry_multiple_effectors() {
        let mut reg = AuxRegistry::new();
        let o1 = reg.register("rw", 3, 0);
        let o2 = reg.register("gimbal", 2, 0);
        assert_eq!(o1, 0);
        assert_eq!(o2, 3);
        assert_eq!(reg.total_dim(), 5);
        assert_eq!(reg.entries().len(), 2);
    }

    // AugmentedState OdeState tests

    fn sample_augmented() -> AugmentedState<AttitudeState> {
        AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            aux: vec![1.0, 2.0, 3.0],
            aux_bounds: vec![],
            modes: vec![],
        }
    }

    #[test]
    fn zero_like() {
        let s = sample_augmented();
        let z = s.zero_like();
        assert_eq!(z.plant.quaternion, Vector4::zeros());
        assert_eq!(z.plant.angular_velocity, Vector3::zeros());
        assert_eq!(z.aux, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn axpy_identity() {
        let s = sample_augmented();
        let other = s.zero_like();
        let result = s.axpy(0.0, &other);
        assert_eq!(result.aux, s.aux);
    }

    #[test]
    fn axpy_adds() {
        let s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![1.0, 2.0],
            aux_bounds: vec![],
            modes: vec![],
        };
        let other = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![10.0, 20.0],
            aux_bounds: vec![],
            modes: vec![],
        };
        let result = s.axpy(0.5, &other);
        assert!((result.aux[0] - 6.0).abs() < 1e-15);
        assert!((result.aux[1] - 12.0).abs() < 1e-15);
    }

    #[test]
    fn scale_multiplies() {
        let s = sample_augmented();
        let scaled = s.scale(2.0);
        assert!((scaled.aux[0] - 2.0).abs() < 1e-15);
        assert!((scaled.aux[1] - 4.0).abs() < 1e-15);
        assert!((scaled.aux[2] - 6.0).abs() < 1e-15);
    }

    #[test]
    fn is_finite_true() {
        let s = sample_augmented();
        assert!(s.is_finite());
    }

    #[test]
    fn is_finite_false_nan_aux() {
        let mut s = sample_augmented();
        s.aux[1] = f64::NAN;
        assert!(!s.is_finite());
    }

    #[test]
    fn is_finite_false_inf_aux() {
        let mut s = sample_augmented();
        s.aux[0] = f64::INFINITY;
        assert!(!s.is_finite());
    }

    /// A mode is carried through every state operation as it is.
    ///
    /// A solver combines stages with `axpy` and `scale`, and a mode is not a
    /// quantity to add or scale: a state holding the modes of two different
    /// right-hand sides would describe neither. `axpy` keeps the modes of the
    /// state being stepped from, since the increment is a derivative and
    /// carries the modes it was taken at. `project` leaves them alone too —
    /// which side a constraint is held against is not something to clamp.
    #[test]
    fn a_mode_is_carried_through_every_state_operation() {
        let mut s = sample_augmented();
        s.modes = vec![ConstraintMode::Upper, ConstraintMode::Free];
        let mut other = sample_augmented();
        other.modes = vec![ConstraintMode::Lower, ConstraintMode::Lower];

        assert_eq!(s.zero_like().modes, s.modes);
        assert_eq!(s.scale(0.5).modes, s.modes);
        assert_eq!(
            s.axpy(2.0, &other).modes,
            s.modes,
            "the modes of the state being stepped from, not the increment's"
        );

        let before = s.modes.clone();
        let _ = s.project(0.0);
        assert_eq!(s.modes, before, "the projection does not touch a mode");
    }

    #[test]
    fn project_normalizes_quaternion() {
        let mut s = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(2.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            aux: vec![5.0, 10.0],
            aux_bounds: vec![],
            modes: vec![],
        };
        assert_eq!(s.project(0.0), Projection::Changed);
        let norm = s.plant.quaternion.magnitude();
        assert!((norm - 1.0).abs() < 1e-15);
        // Aux should be unchanged (no bounds set)
        assert_eq!(s.aux, vec![5.0, 10.0]);
    }

    #[test]
    fn project_clamps_aux_to_bounds() {
        let mut s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![15.0, -5.0, 3.0],
            aux_bounds: vec![(-10.0, 10.0), (-2.0, 2.0), (0.0, 100.0)],
            modes: vec![],
        };
        assert_eq!(s.project(0.0), Projection::Changed);
        assert!((s.aux[0] - 10.0).abs() < 1e-15); // clamped from 15 to 10
        assert!((s.aux[1] - (-2.0)).abs() < 1e-15); // clamped from -5 to -2
        assert!((s.aux[2] - 3.0).abs() < 1e-15); // within bounds, unchanged
    }

    #[test]
    fn project_reports_unchanged_when_aux_within_bounds() {
        let mut s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![3.0, -1.0],
            aux_bounds: vec![(-10.0, 10.0), (-2.0, 2.0)],
            modes: vec![],
        };
        assert_eq!(s.project(0.0), Projection::Unchanged);
        assert_eq!(s.aux, vec![3.0, -1.0]);
    }

    #[test]
    fn error_norm_empty_aux() {
        let s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![],
            aux_bounds: vec![],
            modes: vec![],
        };
        let y_next = s.clone();
        let error = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1e-8, 1e-8, 1e-8, 1e-8),
                angular_velocity: Vector3::new(1e-8, 1e-8, 1e-8),
            },
            aux: vec![],
            aux_bounds: vec![],
            modes: vec![],
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = s.error_norm(&y_next, &error, &tol);
        assert!(norm > 0.0);
        assert!(norm.is_finite());
    }

    #[test]
    fn error_norm_with_aux() {
        let s = sample_augmented();
        let y_next = s.clone();
        let error = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1e-8, 1e-8, 1e-8, 1e-8),
                angular_velocity: Vector3::new(1e-8, 1e-8, 1e-8),
            },
            aux: vec![1e-8, 1e-8, 1e-8],
            aux_bounds: vec![],
            modes: vec![],
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = s.error_norm(&y_next, &error, &tol);
        assert!(norm > 0.0);
        assert!(norm.is_finite());
    }
}
