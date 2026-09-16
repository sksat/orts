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
    RootEvent, RootOutcome, RootSearch, RootSet, RootSlot,
};

use crate::effector::{ConstraintMode, EffectorBoundary};

/// One boundary a system's effectors declared, with where that effector's own
/// state sits in the augmented vectors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclaredBoundary {
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

    /// Whether the search should look at this boundary, given every mode.
    pub fn is_active(&self, modes: &[ConstraintMode]) -> bool {
        let end = (self.mode_offset + self.mode_dim).min(modes.len());
        self.boundary
            .kind
            .is_active(&modes[self.mode_offset.min(end)..end])
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
    /// Only asked about boundaries this system declared.
    fn boundary_value(&self, _declared: &DeclaredBoundary, _t: f64, _state: &Self::State) -> f64 {
        0.0
    }

    /// Put the state exactly on a boundary it reached, move the mode, and give
    /// back whatever the overshoot took from a conserved total.
    ///
    /// Only asked about boundaries this system declared.
    fn settle_boundary(&self, _declared: &DeclaredBoundary, _state: &mut Self::State) {}

    /// The discrete modes of a state, which say which boundaries mean
    /// anything.
    fn modes<'s>(&self, _state: &'s Self::State) -> &'s [ConstraintMode] {
        &[]
    }
}

/// One declared boundary, read as a root event.
pub struct BoundaryEvent<'a, Sys> {
    system: &'a Sys,
    declared: DeclaredBoundary,
}

impl<'a, Sys: HasBoundaries> BoundaryEvent<'a, Sys> {
    /// Read `declared` through `system`, which knows where its effector's state
    /// is and how to evaluate it.
    pub fn new(system: &'a Sys, declared: DeclaredBoundary) -> Self {
        Self { system, declared }
    }
}

impl<Sys: HasBoundaries> RootEvent<Sys::State> for BoundaryEvent<'_, Sys> {
    fn value(&self, t: f64, y: &Sys::State) -> f64 {
        self.system.boundary_value(&self.declared, t, y)
    }

    fn crossing(&self) -> Crossing {
        self.declared.boundary.crossing
    }

    fn terminal(&self) -> bool {
        // A boundary hands control back so the caller can move the mode; it
        // does not ask for the walk to end.
        false
    }

    fn boundary_tolerance(&self) -> f64 {
        self.declared.boundary.boundary_tolerance
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
#[allow(clippy::too_many_arguments)]
pub fn walk_to_target<Sys, Mk, W, F, E, B>(
    system: &Sys,
    boundaries: &[DeclaredBoundary],
    slots: &mut [RootSlot],
    search: RootSearch,
    state: Sys::State,
    t: f64,
    t_target: f64,
    start_is_checked: bool,
    mut make: Mk,
    observer: &mut F,
    check: &E,
) -> Result<(BoundaryWalk<B>, f64, Sys::State), IntegrationError>
where
    Sys: HasBoundaries,
    Mk: FnMut(Sys::State, f64, bool) -> W,
    W: RootWalk,
    W::Sys: DynamicalSystem<State = Sys::State>,
    F: FnMut(f64, &Sys::State),
    E: Fn(f64, &Sys::State) -> ControlFlow<B>,
{
    let events: Vec<BoundaryEvent<'_, Sys>> = boundaries
        .iter()
        .map(|declared| BoundaryEvent::new(system, *declared))
        .collect();
    let refs: Vec<&dyn RootEvent<Sys::State>> = events
        .iter()
        .map(|e| e as &dyn RootEvent<Sys::State>)
        .collect();
    let mut roots = RootSet::new(&refs, slots, search)?;

    let mut state = state;
    let mut t = t;
    // The caller says whether the state it handed over has been through the
    // check; every state this walk hands back has, since it offers each
    // boundary to the check before resuming.
    let mut checked = start_is_checked;
    loop {
        // A boundary can already be crossed at the state the walk starts from:
        // a command applied between walks can turn a held wheel loose, and a
        // crossing that has already happened is not one a search will find.
        settle_what_is_already_past(system, boundaries, &mut state, t);

        // Only the boundaries that mean something in the modes the state now
        // has. Switched outside the walk, which is where a set allows it.
        for (index, declared) in boundaries.iter().enumerate() {
            if declared.is_active(system.modes(&state)) {
                roots.activate(index);
            } else {
                roots.deactivate(index);
            }
        }

        let mut stepper = make(state, t, checked);
        let outcome =
            stepper.advance_to_roots(t_target, |t, s| observer(t, s), check, &mut roots)?;
        match outcome {
            RootOutcome::Reached => {
                return Ok((BoundaryWalk::Reached, stepper.t(), stepper.into_state()));
            }
            RootOutcome::Event { reason } => {
                return Ok((
                    BoundaryWalk::Stopped(reason),
                    stepper.t(),
                    stepper.into_state(),
                ));
            }
            RootOutcome::Roots { t: t_root, .. } => {
                t = t_root;
                let mut settled = stepper.into_state();
                for hit in roots.hits() {
                    system.settle_boundary(&boundaries[hit.event], &mut settled);
                }
                observer(t, &settled);
                if let ControlFlow::Break(reason) = check(t, &settled) {
                    return Ok((BoundaryWalk::Stopped(reason), t, settled));
                }
                state = settled;
                checked = true;
            }
        }
    }
}

/// Settle every boundary the state is already past, until none is.
///
/// One pass can open another — a wheel let loose by a command can be past the
/// other bound — so this repeats, bounded by the number of boundaries there
/// are, which is how many modes could still move.
fn settle_what_is_already_past<Sys: HasBoundaries>(
    system: &Sys,
    boundaries: &[DeclaredBoundary],
    state: &mut Sys::State,
    t: f64,
) {
    for _ in 0..=boundaries.len() {
        let mut moved = false;
        for declared in boundaries {
            if !declared.is_active(system.modes(state)) {
                continue;
            }
            // The value is a margin: at or below zero is at or past the
            // boundary.
            if system.boundary_value(declared, t, state) <= 0.0 {
                system.settle_boundary(declared, state);
                moved = true;
            }
        }
        if !moved {
            return;
        }
    }
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
