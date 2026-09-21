//! The boundaries a system's effectors declare, and the walk that stops at
//! them.
//!
//! A [`StateEffector`](crate::effector::StateEffector) says which boundaries
//! its state can reach and what value each one lives in; the system that holds
//! the effectors flattens those declarations, knowing where each effector's
//! state sits in the augmented vectors. [`BoundaryEvent`] reads one of them as
//! a [`RootEvent`], and [`walk_to_target`] is the loop every propagation path
//! runs: walk, stop at a boundary, put the state on it, move the mode, resume.

use core::ops::ControlFlow;

use utsuroi::{
    AdaptiveStepper, AdaptiveStepper853, Crossing, DynamicalSystem, FixedStepper, IntegrationError,
    RootEvent, RootOutcome, RootSearch, RootSet, RootSlot, SegmentContext,
};

use crate::effector::{ConstraintMode, EffectorBoundary};

/// One boundary a system's effectors declared, with where that effector's own
/// state sits in the augmented vectors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredBoundary {
    /// Index of the satellite whose system declared it, for a group that
    /// propagates several at once. Zero for a system that is one spacecraft.
    pub satellite: usize,
    /// Index of the effector that declared it.
    pub effector: usize,
    /// What the effector said about the boundary.
    pub boundary: EffectorBoundary,
    /// Start of that effector's auxiliary block.
    pub aux_offset: usize,
    /// Length of that block.
    pub aux_dim: usize,
    /// Start of that effector's mode block.
    pub mode_offset: usize,
    /// Length of that block.
    pub mode_dim: usize,
}

impl DeclaredBoundary {
    /// Where this boundary's mode sits in the whole state's mode vector.
    pub fn mode_index(&self) -> usize {
        self.mode_offset + self.boundary.kind.index()
    }

    /// Whether the search should look at this boundary, given the mode block
    /// of the effector that declared it.
    ///
    /// A state assembled without modes has none to read, and a boundary whose
    /// mode is unknown is not one to search for.
    pub fn is_active(&self, modes: &[ConstraintMode]) -> bool {
        match modes.get(self.mode_offset..self.mode_offset + self.mode_dim) {
            Some(block) => self.boundary.kind.is_active(block),
            None => false,
        }
    }
}

/// A system whose effectors declare boundaries the propagation locates.
///
/// Every method has a default that declares nothing, so a system whose
/// right-hand side has no one-sided constraints — most of them — says so with
/// `impl HasBoundaries for MySystem {}`. The propagation paths ask for this
/// trait so that one walk covers both kinds of system.
pub trait HasBoundaries: DynamicalSystem {
    /// Every boundary its effectors declared, flattened.
    ///
    /// The order is the order of the root events built from it, so an index
    /// into this list names the same boundary as a hit's event index.
    fn boundaries(&self) -> Vec<DeclaredBoundary> {
        Vec::new()
    }

    /// The signed value a boundary lives in, at a state the search is
    /// examining.
    ///
    /// `segment` is the interval the step being examined belongs to, and it is
    /// the same one the derivatives were taken in: a part that holds a value
    /// for the length of a segment — a commanded burn, a held throttle —
    /// answers for the segment's start, and a boundary read without it would
    /// see the value from the other side of a switch that the state being
    /// examined never felt. A root function with a step in it is one the
    /// search can converge on the wrong side of, or fail to localize at all.
    ///
    /// Only asked about boundaries this system declared.
    fn boundary_value(
        &self,
        _declared: &DeclaredBoundary,
        _segment: Option<&SegmentContext>,
        _t: f64,
        _state: &Self::State,
    ) -> f64 {
        0.0
    }

    /// Which way a boundary value that is exactly zero at this state is about
    /// to leave zero, as [`RootEvent::leaves_zero_towards`] asks.
    ///
    /// `None`, unless overridden. Only a boundary whose crossing is
    /// [`Crossing::Reversal`] is asked, and only where its value is zero at the
    /// step's start — the one place the step's two ends cannot say whether
    /// anything turned around.
    ///
    /// Only asked about boundaries this system declared.
    fn boundary_departure(
        &self,
        _declared: &DeclaredBoundary,
        _segment: Option<&SegmentContext>,
        _t: f64,
        _state: &Self::State,
    ) -> Option<f64> {
        None
    }

    /// Put the state exactly on a boundary it reached, move the mode, and give
    /// back whatever the overshoot took from a conserved total.
    ///
    /// Return without touching the state where
    /// [`BoundaryKind::mode_after`](crate::effector::BoundaryKind::mode_after)
    /// is `None`. Such a boundary is there to cut the step at a time the walk
    /// has to stop at, and the state it stops on is the state it resumes with:
    /// moving anything — the mode, an effector's own quantity, a conserved
    /// total — would make the cut a transition the caller never asked for.
    ///
    /// Only asked about boundaries this system declared.
    fn settle_boundary(&self, _declared: &DeclaredBoundary, _state: &mut Self::State) {}

    /// Whether a boundary means anything in the mode this state is in.
    ///
    /// A bound cannot be reached while its constraint is already held against
    /// one, and there is nothing to release while it is free. The system
    /// answers rather than handing out its modes, since a group of satellites
    /// keeps one set per satellite and has no single slice to give.
    ///
    /// Only asked about boundaries this system declared.
    fn boundary_is_active(&self, _declared: &DeclaredBoundary, _state: &Self::State) -> bool {
        false
    }

    /// Whether this is a state the system's own constraints can be propagated
    /// from, with the reason when it is not.
    ///
    /// [`walk_to_target`] asks once at its entry, before it settles anything.
    /// A state that already sits past a one-sided constraint is refused here
    /// rather than corrected: settling a mass below a propellant floor puts it
    /// *on* the floor, which adds the mass the input was missing, and settling
    /// a wheel past its limit turns the body at a rate the caller did not ask
    /// for. Neither is a trajectory of the state that was handed over.
    ///
    /// The question is not "is every boundary value non-negative" — a state
    /// resting on a bound, or one a command has just released, is a legal place
    /// to start. Each constraint answers in its own terms, so this reports the
    /// name of what refused and what it read.
    ///
    /// `t` is where the walk starts, because a system can prescribe part of
    /// what a constraint reads: an attitude system takes its orbit and mass
    /// from functions of time, so a mass valid at one instant is not
    /// necessarily valid at another.
    ///
    /// Not asked during a walk: a boundary search steps past a constraint on
    /// purpose, and its trial states are meant to be evaluable there.
    ///
    /// Asked once more, on the state the walk is about to hand back. A caller
    /// propagates that state further, so one this system refuses is one no
    /// later call can use, and reporting it as
    /// [`BoundaryWalkError::ProducedRejected`] names the span that produced it
    /// rather than leaving the next call to refuse a state it did not make.
    /// A system whose answer here disagrees with what its own propagation
    /// produces will see that span fail — which is the report, not a
    /// contradiction to live with.
    ///
    /// A caller that moves a state between walks, as
    /// [`Scheduler`](crate::group::Scheduler) does with its KDK kicks, is asked
    /// about the state it moved *to*: before the next walk runs, and — since a
    /// kick can be the last thing a run does — before it hands the state
    /// back.
    fn validate_boundary_walk_start(
        &self,
        _t: f64,
        _state: &Self::State,
    ) -> Result<(), StartStateError> {
        Ok(())
    }
}

/// Why a state cannot start a boundary walk.
///
/// The reason is what a reader needs; `satellite` is what a caller acts on. A
/// group walks its satellites as one composite state, so the one that refused
/// has to be named apart from the message — a termination recorded against the
/// wrong spacecraft would have a caller resetting or re-commanding one that was
/// fine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartStateError {
    /// Which satellite of a group refused, where a group was asked. `None`
    /// where the system is one spacecraft.
    pub satellite: Option<usize>,
    /// What refused the state, and what it read.
    pub reason: String,
}

impl StartStateError {
    /// A refusal by a system that is one spacecraft.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            satellite: None,
            reason: reason.into(),
        }
    }

    /// The same refusal, attributed to a satellite's place in its group.
    pub fn from_satellite(mut self, index: usize) -> Self {
        self.satellite = Some(index);
        self
    }
}

impl From<String> for StartStateError {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

impl core::fmt::Display for StartStateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.satellite {
            Some(index) => write!(f, "satellite {index}: {}", self.reason),
            None => write!(f, "{}", self.reason),
        }
    }
}

/// What can stop a [`walk_to_target`] before it reaches its target.
///
/// The solver's own failures, and the one the system reports about the state
/// the walk was handed: those come from the caller's input rather than from the
/// integration, so they are named apart from it.
///
/// `#[non_exhaustive]`, as [`IntegrationError`] is: a walk gaining a way to fail
/// should not break a downstream `match`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BoundaryWalkError {
    /// The solver or the boundary search failed.
    Integration(IntegrationError),
    /// The system refused the state the walk was asked to start from
    /// ([`HasBoundaries::validate_boundary_walk_start`]). Nothing was
    /// integrated: what a caller handed over is what was refused.
    StartRejected {
        /// Time the refused state belongs to.
        t: f64,
        /// Which satellite refused, and why.
        error: StartStateError,
    },
    /// The walk produced a state the system refuses
    /// ([`HasBoundaries::validate_boundary_walk_start`]).
    ///
    /// A walk hands back states a caller propagates further, so a state the
    /// system itself calls invalid is reported where it was produced rather
    /// than at the next call.
    ///
    /// What refused is in `error`, and a system implementing this trait can
    /// refuse for any invariant of its own. Three spans reach it in this crate,
    /// all of them the caller's step size to keep out or a model to gate.
    ///
    /// One is stepped too coarsely for a lag it carries: a rate evaluated
    /// inside the step takes a sign neither end shows, and a constraint that
    /// stops only what would carry a state past its bound lets that sign
    /// through, so the state leaves the bound its mode still claims. A wheel
    /// with a 50 ms torque lag under a 200 ms step does this.
    ///
    /// Another holds a crossing the search cannot report, since a step with two
    /// changes of sign of one boundary's value reports neither.
    ///
    /// The last has a contribution to the right-hand side that no boundary
    /// gates: a thruster registered with `with_model` burns propellant the
    /// pool's floor does not stop, and the mass ends below it.
    ProducedRejected {
        /// Time the refused state belongs to.
        t: f64,
        /// Where the walk started, for the span that produced it.
        from: f64,
        /// Which satellite refused, and why.
        error: StartStateError,
    },
}

impl BoundaryWalkError {
    /// The integration time the failure belongs to, where it has one.
    pub fn time(&self) -> Option<f64> {
        match self {
            Self::Integration(e) => e.time(),
            Self::StartRejected { t, .. } | Self::ProducedRejected { t, .. } => Some(*t),
        }
    }
}

impl From<IntegrationError> for BoundaryWalkError {
    fn from(e: IntegrationError) -> Self {
        Self::Integration(e)
    }
}

impl core::fmt::Display for BoundaryWalkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Integration(e) => write!(f, "{e}"),
            Self::StartRejected { t, error } => {
                write!(
                    f,
                    "the state at t = {t} cannot start a boundary walk: {error}"
                )
            }
            Self::ProducedRejected { t, from, error } => {
                write!(
                    f,
                    "the walk from t = {from} produced a state at t = {t} that the system \
                     refuses: {error}. Three spans that reach this, as examples: one whose \
                     step is too coarse for a lag it carries, where a rate evaluated inside \
                     the step takes a sign neither end shows; one holding a crossing the \
                     search cannot report, since a step with two changes of sign of one \
                     boundary's value reports neither; and one with a contribution to the \
                     right-hand side that no boundary gates"
                )
            }
        }
    }
}

impl std::error::Error for BoundaryWalkError {}

/// One declared boundary, read as a root event.
pub struct BoundaryEvent<'a, Sys> {
    system: &'a Sys,
    declared: DeclaredBoundary,
    segment: Option<&'a SegmentContext>,
}

impl<'a, Sys: HasBoundaries> BoundaryEvent<'a, Sys> {
    /// Read `declared` through `system`, which knows where its effector's state
    /// is and how to evaluate it, in the segment the steps belong to.
    pub fn new(
        system: &'a Sys,
        declared: DeclaredBoundary,
        segment: Option<&'a SegmentContext>,
    ) -> Self {
        Self {
            system,
            declared,
            segment,
        }
    }
}

impl<Sys: HasBoundaries> RootEvent<Sys::State> for BoundaryEvent<'_, Sys> {
    fn value(&self, t: f64, y: &Sys::State) -> f64 {
        self.system
            .boundary_value(&self.declared, self.segment, t, y)
    }

    fn crossing(&self) -> Crossing {
        // A bound and a release are margins, so reaching one is the margin
        // running out; a turn of the rate counts either way. See
        // [`BoundaryKind::crossing`](crate::effector::BoundaryKind::crossing).
        self.declared.boundary.kind.crossing()
    }

    fn terminal(&self) -> bool {
        // A boundary hands control back so the caller can move the mode; it
        // does not ask for the walk to end.
        false
    }

    fn boundary_tolerance(&self) -> f64 {
        self.declared.boundary.boundary_tolerance
    }

    fn leaves_zero_towards(&self, t: f64, y: &Sys::State) -> Option<f64> {
        self.system
            .boundary_departure(&self.declared, self.segment, t, y)
    }
}

/// A stepper that can be walked to a target, stopping at boundaries.
///
/// The three solvers answer the same three questions; this is what lets the
/// walk below be written once.
pub trait RootWalk {
    /// The system being propagated.
    type Sys: DynamicalSystem;

    /// Advance towards `t_target`, stopping at the first boundary in `roots`.
    fn advance_to_roots<F, E, B>(
        &mut self,
        t_target: f64,
        callback: F,
        check: E,
        roots: &mut RootSet<'_, <Self::Sys as DynamicalSystem>::State>,
    ) -> Result<RootOutcome<B>, IntegrationError>
    where
        F: FnMut(f64, &<Self::Sys as DynamicalSystem>::State),
        E: Fn(f64, &<Self::Sys as DynamicalSystem>::State) -> ControlFlow<B>;

    /// The time the stepper holds.
    fn t(&self) -> f64;

    /// The state it holds.
    fn state(&self) -> &<Self::Sys as DynamicalSystem>::State;

    /// Take that state.
    fn into_state(self) -> <Self::Sys as DynamicalSystem>::State;
}

/// How a walk over a span ended.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryWalk<B> {
    /// The target time was reached.
    Reached,
    /// The caller's check stopped the walk, with this reason.
    Stopped(B),
}

/// Walk to `t_target`, settling every boundary the walk stops at.
///
/// `make` builds a stepper from a state and the time it belongs to; the walk
/// rebuilds one after each boundary, which is also what drops the stage
/// derivative an adaptive solver cached on the other side of the mode change.
///
/// The order at a boundary is fixed: the state is put on it, the mode moves,
/// and only then is the state observed and offered to the check. A caller's
/// observer therefore never sees the state as it was before the mode it just
/// crossed into was applied.
/// Everything a walk looks for boundaries with.
///
/// One value rather than a row of arguments, as
/// [`EffectorInput`](crate::effector::EffectorInput) is: the search state and
/// the segment belong to the boundaries, not to the state being propagated.
pub struct Boundaries<'a, Sys: HasBoundaries> {
    /// The system that declared them and answers about them.
    pub system: &'a Sys,
    /// What it declared, in the order the events are built in.
    pub declared: &'a [DeclaredBoundary],
    /// One guard per boundary, living across the walks of a propagation: a
    /// guard is what keeps a boundary reported at one segment's end from being
    /// reported again at the next one's start.
    pub slots: &'a mut [RootSlot],
    /// How closely a crossing's time is located.
    pub search: RootSearch,
    /// The interval the steps belong to, for the boundary values that hold a
    /// value across it. `None` where the caller walks no segments.
    pub segment: Option<&'a SegmentContext>,
}

/// The interval a walk is given: where it runs, and what the caller has
/// already done with the state at its start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Time the state belongs to.
    pub from: f64,
    /// Time to walk to, which a boundary can stop the walk short of.
    pub to: f64,
    /// Whether the caller's check has already seen the state at `from`. True
    /// for a segment continuing from one this walk finished, since every state
    /// a walk hands back has been through the check.
    pub start_is_checked: bool,
}

pub fn walk_to_target<Sys, Mk, W, F, E, B>(
    boundaries: Boundaries<'_, Sys>,
    span: Span,
    state: Sys::State,
    mut make: Mk,
    observer: &mut F,
    check: &E,
) -> Result<(BoundaryWalk<B>, f64, Sys::State), BoundaryWalkError>
where
    Sys: HasBoundaries,
    Mk: FnMut(Sys::State, f64, bool) -> W,
    W: RootWalk,
    W::Sys: DynamicalSystem<State = Sys::State>,
    F: FnMut(f64, &Sys::State),
    E: Fn(f64, &Sys::State) -> ControlFlow<B>,
{
    let Boundaries {
        system,
        declared,
        slots,
        search,
        segment,
    } = boundaries;
    let events: Vec<BoundaryEvent<'_, Sys>> = declared
        .iter()
        .map(|boundary| BoundaryEvent::new(system, *boundary, segment))
        .collect();
    let refs: Vec<&dyn RootEvent<Sys::State>> = events
        .iter()
        .map(|e| e as &dyn RootEvent<Sys::State>)
        .collect();
    let mut roots = RootSet::new(&refs, slots, search)?;

    let Span {
        from,
        to: t_target,
        start_is_checked,
    } = span;
    // Before anything is settled: a state the system's constraints call
    // invalid is refused rather than corrected, and correcting it is exactly
    // what the reconciliation below would do.
    system
        .validate_boundary_walk_start(from, &state)
        .map_err(|error| BoundaryWalkError::StartRejected { t: from, error })?;
    let mut state = state;
    let mut t = from;
    // The caller says whether the state it handed over has been through the
    // check; every state this walk hands back has, since it offers each
    // boundary to the check before resuming.
    let mut checked = start_is_checked;
    // Set by a root, and read at the top of the next pass: the state a root
    // produced is reported once every transition it set off has been applied,
    // rather than between two of them. The only way back to the top of this
    // loop is through a root, so nothing has to clear it.
    let mut resumed_from_a_boundary = false;
    loop {
        // A boundary can already be crossed at the state the walk starts from:
        // a command applied between walks can turn a held wheel loose, and a
        // crossing that has already happened is not one a search will find.
        // Settling one can open another, so this runs until the state is in a
        // mode that agrees with itself.
        let moved = settle_what_is_already_past(system, declared, segment, &mut state, t)?;

        // The state the caller last saw is not this one, so it is reported and
        // offered to the check before the walk carries it any further. A
        // boundary that stops the walk here stops it on the settled state, at
        // the time the transition happened.
        if moved || resumed_from_a_boundary {
            observer(t, &state);
            if let ControlFlow::Break(reason) = check(t, &state) {
                // Through the same check as the other ways out: this hands back
                // a state the settling above produced.
                let state = hand_back(system, from, t, state)?;
                return Ok((BoundaryWalk::Stopped(reason), t, state));
            }
            checked = true;
        }

        // Only the boundaries that mean something in the modes the state now
        // has. Switched outside the walk, which is where a set allows it.
        for (index, boundary) in declared.iter().enumerate() {
            if system.boundary_is_active(boundary, &state) {
                roots.activate(index);
            } else {
                roots.deactivate(index);
            }
        }

        let mut stepper = make(state, t, checked);
        let outcome =
            stepper.advance_to_roots(t_target, |t, s| observer(t, s), check, &mut roots)?;
        let walked = match outcome {
            RootOutcome::Reached => {
                let (t_end, state) = (stepper.t(), stepper.into_state());
                hand_back(system, from, t_end, state)
                    .map(|state| (BoundaryWalk::Reached, t_end, state))?
            }
            RootOutcome::Event { reason } => {
                let (t_end, state) = (stepper.t(), stepper.into_state());
                hand_back(system, from, t_end, state)
                    .map(|state| (BoundaryWalk::Stopped(reason), t_end, state))?
            }
            RootOutcome::Roots { t: t_root, .. } => {
                t = t_root;
                let mut settled = stepper.into_state();
                for hit in roots.hits() {
                    system.settle_boundary(&declared[hit.event], &mut settled);
                }
                // Reported at the top of the loop, after whatever these
                // transitions opened has been settled too.
                state = settled;
                resumed_from_a_boundary = true;
                continue;
            }
        };
        return Ok(walked);
    }
}

/// Check the state a walk is about to hand back, which a caller propagates
/// further.
///
/// The system's own start check is what this asks: a state it refuses is one no
/// later call can use, and reporting it here names the span that produced it
/// rather than leaving the next call to refuse a state it did not make.
fn hand_back<Sys: HasBoundaries>(
    system: &Sys,
    from: f64,
    t: f64,
    state: Sys::State,
) -> Result<Sys::State, BoundaryWalkError> {
    match system.validate_boundary_walk_start(t, &state) {
        Ok(()) => Ok(state),
        Err(error) => Err(BoundaryWalkError::ProducedRejected { t, from, error }),
    }
}

/// Settle every boundary the state is already past, until none is, and answer
/// whether anything moved.
///
/// One pass can open another — a wheel let loose by a command can be past the
/// other bound — so this repeats, bounded by the number of boundaries there
/// are, which is how many modes could still move.
///
/// Settling is what puts the state on a boundary, so that bound is enough for
/// a state to reach a mode that agrees with itself. Where it is not, the state
/// is still past a boundary the search cannot find — a value already on the
/// crossed side has no change of sign left to give — so this reports
/// [`IntegrationError::RootStillCrossed`] rather than walking on with it.
fn settle_what_is_already_past<Sys: HasBoundaries>(
    system: &Sys,
    boundaries: &[DeclaredBoundary],
    segment: Option<&SegmentContext>,
    state: &mut Sys::State,
    t: f64,
) -> Result<bool, IntegrationError> {
    let mut settled_any = false;
    for _ in 0..=boundaries.len() {
        let mut moved = false;
        for declared in boundaries {
            if !system.boundary_is_active(declared, state) {
                continue;
            }
            // A boundary that only splits the step has no side to be past: its
            // value is a rate, and a rate below zero is an ordinary state to
            // start from. Settling it moves nothing either, so reading it here
            // would spend every pass of this loop on a state that never
            // changes and end in `RootStillCrossed`.
            if declared.boundary.kind.mode_after().is_none() {
                continue;
            }
            // The value is a margin, so past the boundary is below zero — the
            // one side there is to be past — and a state *on* the boundary,
            // at exactly zero, is not past it. A wheel resting on its bound
            // with the motor asking for nothing has both its bound's margin
            // and its release margin at zero, and reading either as crossed
            // would move the mode back and forth for as long as this loop
            // runs.
            //
            // No tolerance band here: a margin that is negative at all is on
            // the crossed side, and the search cannot find it — the value only
            // goes further negative from there, so there is no sign change
            // left to report and the state would run on past its bound. What
            // the effector's `boundary_tolerance` suppresses is a second
            // report at a root already located, which is the search's own
            // guard.
            if system.boundary_value(declared, segment, t, state) < 0.0 {
                system.settle_boundary(declared, state);
                moved = true;
                settled_any = true;
            }
        }
        if !moved {
            return Ok(settled_any);
        }
    }

    // Every pass moved something, so the state never came to rest. Name the
    // first boundary it is still past.
    let unsettled = boundaries
        .iter()
        .position(|declared| {
            system.boundary_is_active(declared, state)
                && declared.boundary.kind.mode_after().is_some()
                && system.boundary_value(declared, segment, t, state) < 0.0
        })
        .unwrap_or(0);
    Err(IntegrationError::RootStillCrossed {
        t,
        event: unsettled,
    })
}

// The three solvers, each forwarding the same three questions. Written out
// rather than macro-expanded: there are three of them and each is six lines.
macro_rules! root_walk {
    ($($generic:ident),* ; $stepper:ty, $path:ident) => {
        impl<$($generic,)* S: DynamicalSystem> RootWalk for $stepper
        where
            $($generic: utsuroi::Integrator,)*
        {
            type Sys = S;

            fn advance_to_roots<F, E, B>(
                &mut self,
                t_target: f64,
                callback: F,
                check: E,
                roots: &mut RootSet<'_, S::State>,
            ) -> Result<RootOutcome<B>, IntegrationError>
            where
                F: FnMut(f64, &S::State),
                E: Fn(f64, &S::State) -> ControlFlow<B>,
            {
                $path::advance_to_roots(self, t_target, callback, check, roots)
            }

            fn t(&self) -> f64 {
                $path::t(self)
            }

            fn state(&self) -> &S::State {
                $path::state(self)
            }

            fn into_state(self) -> S::State {
                $path::into_state(self)
            }
        }
    };
}

root_walk!(I ; FixedStepper<'_, I, S>, FixedStepper);
root_walk!( ; AdaptiveStepper<'_, S>, AdaptiveStepper);
root_walk!( ; AdaptiveStepper853<'_, S>, AdaptiveStepper853);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effector::{BoundaryKind, EffectorBoundary};
    use core::cell::RefCell;
    use utsuroi::{Integrator, OdeState, Projection, Rk4, Tolerances};

    const RATE: f64 = -1.0;
    const DT: f64 = 0.25;

    /// A scalar that runs down at [`RATE`] until it is held, carrying the one
    /// mode that says which of the two it is doing.
    #[derive(Clone, Debug, PartialEq)]
    struct Ramp {
        x: f64,
        held: bool,
    }

    impl OdeState for Ramp {
        fn zero_like(&self) -> Self {
            // The mode is carried, as `AugmentedState` carries its own: a
            // derivative belongs to the mode the state is in.
            Ramp {
                x: 0.0,
                held: self.held,
            }
        }

        fn axpy(&self, scale: f64, other: &Self) -> Self {
            Ramp {
                x: self.x + scale * other.x,
                held: self.held,
            }
        }

        fn scale(&self, factor: f64) -> Self {
            Ramp {
                x: self.x * factor,
                held: self.held,
            }
        }

        fn is_finite(&self) -> bool {
            self.x.is_finite()
        }

        fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
            let sc = tol.atol + tol.rtol * self.x.abs().max(y_next.x.abs());
            (error.x / sc).abs()
        }

        fn project(&mut self, _t: f64) -> Projection {
            Projection::Unchanged
        }
    }

    /// Runs a [`Ramp`] down to a floor at zero, and records every boundary
    /// question it is asked together with the mode it was asked in.
    struct Ramped {
        asked: RefCell<Vec<(BoundaryKind, bool)>>,
    }

    impl Ramped {
        fn new() -> Self {
            Ramped {
                asked: RefCell::new(Vec::new()),
            }
        }

        fn declared(kind: BoundaryKind) -> DeclaredBoundary {
            DeclaredBoundary {
                satellite: 0,
                effector: 0,
                boundary: EffectorBoundary {
                    kind,
                    boundary_tolerance: 0.0,
                },
                aux_offset: 0,
                aux_dim: 0,
                mode_offset: 0,
                mode_dim: 1,
            }
        }
    }

    impl DynamicalSystem for Ramped {
        type State = Ramp;

        fn derivatives(&self, _t: f64, state: &Ramp) -> Ramp {
            Ramp {
                x: if state.held { 0.0 } else { RATE },
                held: state.held,
            }
        }
    }

    impl HasBoundaries for Ramped {
        fn boundaries(&self) -> Vec<DeclaredBoundary> {
            vec![
                Self::declared(BoundaryKind::ReachedLower { index: 0 }),
                Self::declared(BoundaryKind::Released { index: 0 }),
            ]
        }

        fn boundary_value(
            &self,
            declared: &DeclaredBoundary,
            _segment: Option<&SegmentContext>,
            _t: f64,
            state: &Ramp,
        ) -> f64 {
            self.asked
                .borrow_mut()
                .push((declared.boundary.kind, state.held));
            match declared.boundary.kind {
                // How far the scalar still has to fall.
                BoundaryKind::ReachedLower { .. } => state.x,
                // Nothing ever lets it go again.
                _ => 1.0,
            }
        }

        fn settle_boundary(&self, declared: &DeclaredBoundary, state: &mut Ramp) {
            if let BoundaryKind::ReachedLower { .. } = declared.boundary.kind {
                state.x = 0.0;
                state.held = true;
            }
        }

        fn boundary_is_active(&self, declared: &DeclaredBoundary, state: &Ramp) -> bool {
            match declared.boundary.kind {
                BoundaryKind::ReachedLower { .. } => !state.held,
                _ => state.held,
            }
        }
    }

    /// Walk `from` to `t_target` through the boundary handling, with a
    /// termination check of the caller's and a record of what it observed.
    ///
    /// `start_is_checked` is true, as it is for a segment continuing from one
    /// the caller already checked: a state the walk has not changed owes the
    /// check nothing.
    fn walk_watching(
        system: &Ramped,
        from: Ramp,
        t_target: f64,
        check: &dyn Fn(f64, &Ramp) -> ControlFlow<&'static str>,
    ) -> (BoundaryWalk<&'static str>, f64, Ramp, Vec<(f64, Ramp)>) {
        let boundaries = system.boundaries();
        let mut slots = vec![RootSlot::new(); boundaries.len()];
        let mut seen: Vec<(f64, Ramp)> = Vec::new();
        let (walked, t, state) = walk_to_target(
            Boundaries {
                system,
                declared: &boundaries,
                slots: &mut slots,
                search: RootSearch::default(),
                segment: None,
            },
            Span {
                from: 0.0,
                to: t_target,
                start_is_checked: true,
            },
            from,
            |state, t, _checked| Rk4.stepper(system, state, t, DT),
            &mut |t: f64, s: &Ramp| seen.push((t, s.clone())),
            &check,
        )
        .expect("the walk succeeds");
        (walked, t, state, seen)
    }

    /// Walk `from` to `t_target` through the boundary handling.
    fn walk(system: &Ramped, from: Ramp, t_target: f64) -> (f64, Ramp) {
        let boundaries = system.boundaries();
        let mut slots = vec![RootSlot::new(); boundaries.len()];
        let (_, t, state) = walk_to_target(
            Boundaries {
                system,
                declared: &boundaries,
                slots: &mut slots,
                search: RootSearch::default(),
                segment: None,
            },
            Span {
                from: 0.0,
                to: t_target,
                start_is_checked: false,
            },
            from,
            |state, t, _checked| Rk4.stepper(system, state, t, DT),
            &mut |_: f64, _: &Ramp| {},
            &|_: f64, _: &Ramp| -> ControlFlow<()> { ControlFlow::Continue(()) },
        )
        .expect("the walk succeeds");
        (t, state)
    }

    /// Settling is what puts the state on a boundary, and a system whose settle
    /// does not — it moves the mode and leaves the value where it was — never
    /// reaches a state that agrees with itself. The walk cannot go on from
    /// there: the search reads a crossing from a change of sign, and a value
    /// already on the crossed side has none left. It reports rather than
    /// walking past the boundary for the whole span.
    #[test]
    fn a_settle_that_never_comes_to_rest_is_an_error() {
        /// Two boundaries that hand the mode back and forth, each leaving the
        /// scalar where it was.
        struct NeverRests;

        impl DynamicalSystem for NeverRests {
            type State = Ramp;
            fn derivatives(&self, _t: f64, state: &Ramp) -> Ramp {
                Ramp {
                    x: RATE,
                    held: state.held,
                }
            }
        }

        impl HasBoundaries for NeverRests {
            fn boundaries(&self) -> Vec<DeclaredBoundary> {
                vec![
                    Ramped::declared(BoundaryKind::ReachedLower { index: 0 }),
                    Ramped::declared(BoundaryKind::Released { index: 0 }),
                ]
            }

            fn boundary_value(
                &self,
                _declared: &DeclaredBoundary,
                _segment: Option<&SegmentContext>,
                _t: f64,
                state: &Ramp,
            ) -> f64 {
                // Past whichever boundary is live, always.
                state.x
            }

            fn settle_boundary(&self, _declared: &DeclaredBoundary, state: &mut Ramp) {
                // Only the mode moves, so the value stays on the crossed side.
                state.held = !state.held;
            }

            fn boundary_is_active(&self, declared: &DeclaredBoundary, state: &Ramp) -> bool {
                match declared.boundary.kind {
                    BoundaryKind::ReachedLower { .. } => !state.held,
                    _ => state.held,
                }
            }
        }

        let system = NeverRests;
        let boundaries = system.boundaries();
        let mut slots = vec![RootSlot::new(); boundaries.len()];
        let outcome = walk_to_target(
            Boundaries {
                system: &system,
                declared: &boundaries,
                slots: &mut slots,
                search: RootSearch::default(),
                segment: None,
            },
            Span {
                from: 0.0,
                to: 1.0,
                start_is_checked: false,
            },
            Ramp {
                x: -0.5,
                held: false,
            },
            |state, t, _checked| Rk4.stepper(&system, state, t, DT),
            &mut |_: f64, _: &Ramp| {},
            &|_: f64, _: &Ramp| -> ControlFlow<()> { ControlFlow::Continue(()) },
        );

        assert!(
            matches!(
                outcome,
                Err(BoundaryWalkError::Integration(
                    IntegrationError::RootStillCrossed { t, event: 0 }
                )) if t == 0.0
            ),
            "the walk reports which boundary it is still past, at the time it gave up"
        );
    }

    /// The mode decides which boundaries the search looks at, and the walk asks
    /// for no others: a bound cannot be reached while it is already held
    /// against one, and an inactive boundary's value is not a number the search
    /// may read (it is what the mode leaves undefined).
    #[test]
    fn only_the_boundaries_the_mode_allows_are_asked_about() {
        let system = Ramped::new();
        let (_, ended) = walk(
            &system,
            Ramp {
                x: 0.6,
                held: false,
            },
            2.0,
        );
        assert!(ended.held, "the scalar reaches its floor within the walk");

        let asked = system.asked.borrow();
        assert!(
            asked
                .iter()
                .any(|(kind, held)| matches!(kind, BoundaryKind::ReachedLower { .. }) && !held),
            "the floor is what the search looks for while the scalar is running"
        );
        for (kind, held) in asked.iter() {
            let expected_release = *held;
            let is_release = matches!(kind, BoundaryKind::Released { .. });
            assert_eq!(
                is_release, expected_release,
                "asked about {kind:?} while held={held}"
            );
        }
    }

    /// Settling a boundary the state was already past changes the state after
    /// the caller last looked at it, so the walk reports it and offers it to
    /// the caller's check before stepping. A caller whose check breaks on what
    /// the transition produced — a mode that ends the run — would otherwise be
    /// told about it a whole step late, or not until the target.
    #[test]
    fn a_state_settled_before_the_first_step_is_reported_and_checked() {
        let system = Ramped::new();
        let (walked, t, state, seen) = walk_watching(
            &system,
            Ramp {
                x: -0.5,
                held: false,
            },
            1.0,
            &|_t, s| {
                if s.held {
                    ControlFlow::Break("held")
                } else {
                    ControlFlow::Continue(())
                }
            },
        );

        assert!(
            matches!(walked, BoundaryWalk::Stopped("held")),
            "the check sees the settled state"
        );
        assert_eq!(t, 0.0, "at the time the transition happened");
        assert_eq!(
            state,
            Ramp { x: 0.0, held: true },
            "and the walk hands back what it settled"
        );
        assert_eq!(
            seen,
            vec![(0.0, Ramp { x: 0.0, held: true })],
            "reported once, after the transition"
        );
    }

    /// A walk can start from a state that is already past a boundary — a
    /// command applied between walks, or a caller handing over a state it built
    /// — and a crossing that has already happened is not one a search can find:
    /// the value never changes sign inside the walk. The walk settles what is
    /// already past before it steps.
    #[test]
    fn a_boundary_already_crossed_at_the_start_is_settled_first() {
        let system = Ramped::new();
        let (_, ended) = walk(
            &system,
            Ramp {
                x: -0.5,
                held: false,
            },
            1.0,
        );
        assert_eq!(
            ended,
            Ramp { x: 0.0, held: true },
            "the state is put on the boundary and the mode moved, at the start"
        );
    }
}
