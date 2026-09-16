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
    /// Only asked about boundaries this system declared.
    fn boundary_value(&self, _declared: &DeclaredBoundary, _t: f64, _state: &Self::State) -> f64 {
        0.0
    }

    /// Put the state exactly on a boundary it reached, move the mode, and give
    /// back whatever the overshoot took from a conserved total.
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
            if system.boundary_is_active(declared, &state) {
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
            if !system.boundary_is_active(declared, state) {
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
                    crossing: Crossing::Falling,
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

        fn boundary_value(&self, declared: &DeclaredBoundary, _t: f64, state: &Ramp) -> f64 {
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

    /// Walk `from` to `t_target` through the boundary handling.
    fn walk(system: &Ramped, from: Ramp, t_target: f64) -> (f64, Ramp) {
        let boundaries = system.boundaries();
        let mut slots = vec![RootSlot::new(); boundaries.len()];
        let (_, t, state) = walk_to_target(
            system,
            &boundaries,
            &mut slots,
            RootSearch::default(),
            from,
            0.0,
            t_target,
            false,
            |state, t, _checked| Rk4.stepper(system, state, t, DT),
            &mut |_: f64, _: &Ramp| {},
            &|_: f64, _: &Ramp| -> ControlFlow<()> { ControlFlow::Continue(()) },
        )
        .expect("the walk succeeds");
        (t, state)
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
