//! Boundaries a state decides, located inside the step that crosses them.
//!
//! A burn window's edges are known times, so a [`Segments`](crate::Segments)
//! walk can cut the span at them and every step lands on a boundary exactly. A
//! reaction wheel saturating or a tank running dry is not known in advance: the
//! time follows from the state, and the only way to land on it is to look for
//! the crossing while stepping. That is what a [`RootEvent`] describes, and its
//! contract differs from the `event_check` the plain `advance_to` takes:
//!
//! - the boundary is the zero of a signed, continuous function, so a crossing
//!   can be bracketed rather than merely noticed one step late
//! - the event says which direction counts, and whether reaching the boundary
//!   ends the walk
//! - detection reads the raw candidate, before
//!   [`OdeState::project`](crate::OdeState::project) — a projection pulls the
//!   state back onto its constraint surface, and can erase the sign change that
//!   shows the crossing
//!
//! # What a walk with root events guarantees
//!
//! The state and time a stepper holds afterwards are the ones at the boundary,
//! projected. The callback does not see them: a boundary state is not final
//! until the caller has updated the mode it crossed into, so the caller reads
//! that state from the stepper and records it when it is done. States tried
//! during the search reach neither the callback nor the projection.
//! [`RootSet::hits`] lists every event that crossed within the final bracket,
//! so a caller that has to break a tie between two wheels saturating together
//! sees both.
//!
//! # What the caller owes
//!
//! **The right-hand side must not change while a root is being searched for.**
//! Localization re-steps from the last committed state with shorter and shorter
//! widths; if the system switches its discrete mode at the boundary, a method's
//! stages then mix the two modes and the bisection converges on the wrong time.
//! `y' = 1` below `y = 1` and `y' = 0` at or above it, with `g = y - 1`,
//! reaches the boundary at `t = 1`; re-stepping RK4 across the switch converges
//! on a width of `6r/5` for a remaining distance `r`, which reports the arrival
//! `0.2 r` late. Keep the pre-root mode frozen for the whole search and apply
//! the change after landing; a continuous state feedback is fine to evaluate at
//! every stage, it is the discrete mode that has to hold still.
//!
//! **One step may hold at most one change of sign of each event's value**, in
//! either direction — not one crossing in the direction the event counts. Two
//! events may each change sign in the same step; that is what a group of
//! simultaneous roots is. What
//! detection reads is the value at the step's start against the value at a
//! trial end, so a step holding two changes of sign reports nothing at all, and
//! one holding three converges on the last: for
//! `g = (t - 0.2)(t - 0.4)(t - 0.8)` over `[0, 1]` the first trial at `0.5` has
//! the sign the step started with, which discards `0.2` and `0.4` and lands on
//! `0.8`. A step where the value runs out and comes back also reports nothing
//! from its two ends, even though only one of those two changes is in the
//! counted direction. Bound the step size so a step holds one; the search can
//! only read the value at times it picks, so it cannot check this for the
//! caller.
//!
//! One case is covered without that bound: a value that runs out and comes back
//! is still located, when another event's crossing is located **between its two
//! zeros**. The values at a located time are read again there, an event whose
//! sign differs from the step's start is added to the candidates, and the
//! shortened step is searched from its start. [`RootSet::hits`] lists what
//! crosses within the width that search settles on, so the crossings past it
//! are not reported. A crossing located before the value runs out, or after it
//! has come back, adds nothing — which is why the obligation above stands.
//!
//! A walk whose first state sits exactly on a boundary reports a root as soon as
//! the value leaves zero, in whichever direction the event counts. What a
//! resumption after a non-terminal root does instead — where the state is also
//! on the boundary — is settled by [`RootGuard`]: the step that starts at the
//! root's own time does not report that event again.
//!
//! # Storage
//!
//! utsuroi does not allocate, so a [`RootSet`] borrows its storage: one
//! [`RootSlot`] per event, owned by the caller — which is also what lets the
//! event count be whatever that caller's configuration builds, one per wheel or
//! per thruster. A slot carries its event's guard, so the caller holds the slots
//! across resumptions, and that is what stops a non-terminal root from being
//! found again while the state sits on its boundary.

use crate::IntegrationError;

/// Which way across zero counts as reaching the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crossing {
    /// From negative to positive.
    Rising,
    /// From positive to negative.
    Falling,
    /// Either direction.
    Either,
}

impl Crossing {
    /// Whether a move from `before` to `after` is the crossing this counts.
    ///
    /// A value that arrives at exactly zero counts, and one that leaves zero
    /// counts toward the side it goes to: at the endpoints of a step those two
    /// are the same pair of numbers, and whether the state was left there by a
    /// root is what tells them apart. [`RootGuard`] holds that, so this reads
    /// only the two values.
    fn matches(self, before: f64, after: f64) -> bool {
        let rising = before <= 0.0 && after >= 0.0 && (before < 0.0 || after > 0.0);
        let falling = before >= 0.0 && after <= 0.0 && (before > 0.0 || after < 0.0);
        match self {
            Crossing::Rising => rising,
            Crossing::Falling => falling,
            Crossing::Either => rising || falling,
        }
    }
}

/// A boundary the state decides, as the zero of a signed function.
pub trait RootEvent<Y> {
    /// The signed value at `(t, y)`. Continuous in both, and finite wherever
    /// the walk can reach: a non-finite value stops the walk with
    /// [`IntegrationError::NonFiniteRootValue`] rather than being guessed
    /// about, since its sign says nothing about where a crossing is.
    ///
    /// One step may hold at most one change of sign of this value, whichever
    /// direction each change is in — see the module documentation for what the
    /// search reports when a step holds more, and why it cannot detect the case
    /// itself.
    fn value(&self, t: f64, y: &Y) -> f64;

    /// Which direction across zero counts. [`Crossing::Either`], unless
    /// overridden.
    fn crossing(&self) -> Crossing {
        Crossing::Either
    }

    /// Whether reaching this boundary ends the walk. Terminal, unless
    /// overridden: a non-terminal root hands control back at the boundary and
    /// the caller resumes from there.
    fn terminal(&self) -> bool {
        true
    }

    /// Order among events that cross within the same bracket. Lower is
    /// reported first, and events of equal priority keep the order they were
    /// registered in.
    fn priority(&self) -> i32 {
        0
    }

    /// Width of `|value|` within which the state still counts as being on this
    /// boundary, in the value's own units.
    ///
    /// Zero, unless overridden, and it has to be finite and not negative:
    /// [`RootSet::new`] refuses the rest, since a negative width re-arms the
    /// guard at once and a non-finite one never re-arms it.
    ///
    /// What this settles is a constraint the state moves *along* — a wheel held
    /// at its saturation torque — where `value` stays at a jitter around zero
    /// for many steps rather than at zero, and each change of sign in that
    /// jitter would otherwise be a fresh crossing. The step that leaves a root
    /// behind is a separate matter, suppressed exactly, by time; see
    /// [`RootGuard`]. The width cannot be derived from
    /// [`RootSearch::t_tolerance`], which is a time.
    fn boundary_tolerance(&self) -> f64 {
        0.0
    }
}

/// What one event knows between steps, and across resumptions.
///
/// Two things keep a root from being found again, because the state a root
/// leaves behind is not exactly on the boundary — the search stops on the far
/// side of a bracket, so the value there is a small non-zero rather than zero:
///
/// - the time the root was reported, together with the value the root left
///   behind. A step that starts at that time with the value still on that side
///   is the one leaving the root, and it does not report the same event again.
///   Both comparisons are exact and need no tolerance. A caller that moves the
///   value to a different side from the one left there — or to either side,
///   where the root landed exactly on zero — and clear of the event's
///   [`boundary_tolerance`](RootEvent::boundary_tolerance), has moved the state
///   off that root, so the step it then takes reports a crossing as any other
///   would.
/// - whether the value is still within the event's
///   [`boundary_tolerance`](RootEvent::boundary_tolerance), for a state that
///   goes on moving along the boundary over many steps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RootGuard {
    /// Time of the root this event last reported, while the walk is still
    /// standing on it.
    at: Option<f64>,
    /// The value at that root, whose side is what tells a departure from a
    /// fresh arrival.
    left: f64,
    /// Set while the committed state is one a root of this event left behind,
    /// and cleared once the value leaves the event's boundary tolerance.
    on_boundary: bool,
}

/// Whether two values are on the same side of zero.
///
/// Zero is on no side, so it matches only zero: a root that landed exactly on
/// the boundary left the value with no side, and any non-zero value the caller
/// puts there has moved it off.
fn same_side(a: f64, b: f64) -> bool {
    if a == 0.0 || b == 0.0 {
        a == b
    } else {
        (a > 0.0) == (b > 0.0)
    }
}

impl RootGuard {
    /// A guard for an event no root has fired on yet.
    pub const fn new() -> Self {
        Self {
            at: None,
            left: 0.0,
            on_boundary: false,
        }
    }

    /// The time of the root this event last reported, if the walk is still
    /// standing on it.
    pub fn at(&self) -> Option<f64> {
        self.at
    }

    /// Whether this event is suppressed because the state is on its boundary.
    pub fn is_on_boundary(&self) -> bool {
        self.on_boundary
    }
}

/// One event's crossing, located.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootHit {
    /// Index of the event in the set, which is the order it was registered in.
    pub event: usize,
    /// Whether this event asked to end the walk.
    pub terminal: bool,
    /// The event's priority, as it reported it.
    pub priority: i32,
}

/// How a walk with root events ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RootOutcome<B> {
    /// The target time was reached with no root in the way.
    Reached,
    /// The caller's termination check broke on an ordinary step, with this
    /// reason. Boundaries are not run past it: a caller that stops the walk
    /// stops it whether or not an event was about to cross.
    Event {
        /// What the check reported.
        reason: B,
    },
    /// One or more events crossed. [`RootSet::hits`] lists them, ordered by
    /// priority and then by registration order.
    Roots {
        /// The time the walk stopped at, which is where the state now is. Every
        /// hit shares it.
        t: f64,
        /// Width of the bracket the search ended with, in seconds. The time is
        /// uncertain by about this much numerically; how far it is from the
        /// true crossing also depends on the state error and on how flat the
        /// value is there.
        bracket: f64,
        /// Whether any of the hits asked to end the walk. A caller that resumes
        /// anyway takes its next step from the boundary.
        terminal: bool,
    },
}

/// How hard to look for a crossing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootSearch {
    /// Bracket width to stop at, in seconds. The search ends once the interval
    /// holding the crossing is this narrow, or once halving it no longer
    /// changes the bracket in f64.
    pub t_tolerance: f64,
    /// Cap on bisection iterations, counted per pass. A value that behaves
    /// unlike a continuous function cannot make the search spin: the walk fails
    /// with [`IntegrationError::RootNotLocalized`] instead.
    ///
    /// A step whose located end brackets a further event is searched again from
    /// the step's start, and each of those passes gets this many iterations:
    /// one narrowing does not spend the budget of the next, and a cap chosen
    /// for a step's width keeps holding for the shorter widths that follow.
    pub max_iterations: u32,
}

impl Default for RootSearch {
    fn default() -> Self {
        Self {
            // A millisecond is finer than any cadence orts samples at, and 60
            // halvings take a day-wide step well below it.
            t_tolerance: 1e-3,
            max_iterations: 60,
        }
    }
}

impl RootSearch {
    fn validate(&self) -> Result<(), IntegrationError> {
        if self.t_tolerance.is_finite() && self.t_tolerance > 0.0 && self.max_iterations > 0 {
            Ok(())
        } else {
            Err(IntegrationError::InvalidRootSearch {
                t_tolerance: self.t_tolerance,
                max_iterations: self.max_iterations,
            })
        }
    }
}

/// The search state of one root event, owned by the caller.
///
/// A set borrows a slice of these, so the number of events is a runtime
/// quantity: the caller — which knows how many wheels or thrusters its model
/// has — owns the storage, and this crate allocates nothing. A slot pairs with
/// the event at the same index for the life of the walk, since it carries that
/// event's guard across steps and resumptions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootSlot {
    guard: RootGuard,
    /// The event's boundary tolerance, read and checked once when the set is
    /// built, so the width a guard re-arms on cannot change under the walk.
    boundary: f64,
    /// The value at the state the stepper has committed: the "before" of the
    /// next step. Read afresh at the start of every walk, because the caller is
    /// free to change the state between them.
    start: f64,
    /// The value at the far end of the bracket the search is narrowing.
    hi: f64,
    /// The value at whichever state was evaluated last: a trial of the search,
    /// or the state the stepper is about to commit.
    trial: f64,
    /// Whether this event changed sign over the step being examined. The search
    /// narrows on these and leaves the rest alone.
    candidate: bool,
    /// The raw value at the located root, for an event that fired. A projection
    /// can move the state back across the boundary, so the value at the
    /// committed state says nothing about which side the crossing went to.
    located: f64,
    /// What this event reported at the located root.
    hit: Option<RootHit>,
    /// Where this event comes in the order the caller handles that bracket's
    /// roots, counting from zero.
    rank: Option<usize>,
    /// Whether the search looks at this event at all. A mode that makes an
    /// event meaningless — the release of a constraint that is not held —
    /// switches it off, rather than feeding the search a stand-in value.
    active: bool,
}

impl RootSlot {
    /// A slot for an event no step has examined yet, switched on.
    pub const fn new() -> Self {
        Self {
            guard: RootGuard::new(),
            boundary: 0.0,
            start: 0.0,
            hi: 0.0,
            trial: 0.0,
            candidate: false,
            located: 0.0,
            hit: None,
            rank: None,
            active: true,
        }
    }
}

impl Default for RootSlot {
    fn default() -> Self {
        Self::new()
    }
}

/// The root events of a walk, with the state each one carries between steps.
///
/// Built once and handed to `advance_to_roots` for every target, so the guards
/// survive a resumption: a non-terminal root leaves the state on its boundary,
/// and the guard is what keeps the next step from reporting the departure as a
/// new crossing.
pub struct RootSet<'a, Y> {
    events: &'a [&'a dyn RootEvent<Y>],
    slots: &'a mut [RootSlot],
    search: RootSearch,
}

impl<'a, Y> RootSet<'a, Y> {
    /// Pair events with the slots that carry their search state.
    ///
    /// The slots start as [`RootSlot::new`] and stay with the caller, which is
    /// what lets the number of events be whatever the caller's model has. Each
    /// event's boundary tolerance is read here and not again.
    pub fn new(
        events: &'a [&'a dyn RootEvent<Y>],
        slots: &'a mut [RootSlot],
        search: RootSearch,
    ) -> Result<Self, IntegrationError> {
        search.validate()?;
        if events.len() != slots.len() {
            return Err(IntegrationError::RootSlotCount {
                events: events.len(),
                slots: slots.len(),
            });
        }
        for (index, (event, slot)) in events.iter().zip(slots.iter_mut()).enumerate() {
            let tolerance = event.boundary_tolerance();
            if !(tolerance.is_finite() && tolerance >= 0.0) {
                return Err(IntegrationError::InvalidBoundaryTolerance {
                    event: index,
                    tolerance,
                });
            }
            slot.boundary = tolerance;
        }
        Ok(Self {
            events,
            slots,
            search,
        })
    }

    /// The events that crossed within the bracket the last walk ended on, in
    /// the order they should be handled: by priority, then by registration.
    ///
    /// Empty after a walk that reached its target. The iterator borrows the
    /// set, so a caller that switches events off while handling the roots reads
    /// them one at a time with [`hit_at`](Self::hit_at) instead.
    pub fn hits(&self) -> impl Iterator<Item = RootHit> + '_ {
        (0..self.hit_count()).filter_map(|rank| self.hit_at(rank))
    }

    /// How many events crossed within that bracket.
    pub fn hit_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.hit.is_some()).count()
    }

    /// The root to handle `rank`th, counting from the one to handle first.
    ///
    /// Switching events on or off leaves these alone: they describe a boundary
    /// the walk has already committed to.
    pub fn hit_at(&self, rank: usize) -> Option<RootHit> {
        self.slots
            .iter()
            .find(|slot| slot.rank == Some(rank))
            .and_then(|slot| slot.hit)
    }

    /// One event's guard, for a caller checking whether the state is sitting on
    /// that boundary.
    pub fn guard(&self, event: usize) -> RootGuard {
        self.slots[event].guard
    }

    /// How many events are in the set, switched on or off.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the set has no events, in which case a walk over it is the plain
    /// one.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Whether the search looks at this event.
    pub fn is_active(&self, event: usize) -> bool {
        self.slots[event].active
    }

    /// Start looking at this event again, with a fresh guard.
    ///
    /// The guard is re-armed because the value has been unobserved: the side it
    /// was last seen on says nothing about where it is now. The next walk reads
    /// the value at its own start, and a crossing that happened while the event
    /// was switched off is not reported. An event that is already on is left
    /// alone, guard included.
    ///
    /// Switch events while the walk is not running — between the calls to
    /// `advance_to_roots` — so that the value each event is compared against is
    /// the one read at a step's start.
    pub fn activate(&mut self, event: usize) {
        let slot = &mut self.slots[event];
        if slot.active {
            return;
        }
        slot.active = true;
        slot.guard = RootGuard::new();
        slot.candidate = false;
    }

    /// Stop looking at this event.
    ///
    /// Its value is not evaluated while it is off, so an event whose meaning
    /// depends on a mode — the release of a constraint that is not held — costs
    /// nothing in the modes where it says nothing.
    pub fn deactivate(&mut self, event: usize) {
        let slot = &mut self.slots[event];
        slot.active = false;
        slot.candidate = false;
    }

    /// Whether any event is switched on.
    fn any_active(&self) -> bool {
        self.slots.iter().any(|slot| slot.active)
    }

    /// Read every switched-on event's value at a state into its slot.
    fn eval(&mut self, t: f64, y: &Y) -> Result<(), IntegrationError> {
        for index in 0..self.events.len() {
            if !self.slots[index].active {
                continue;
            }
            let value = self.events[index].value(t, y);
            if !value.is_finite() {
                return Err(IntegrationError::NonFiniteRootValue { t, event: index });
            }
            self.slots[index].trial = value;
        }
        Ok(())
    }

    /// Keep the values just evaluated as the far end of the bracket.
    fn keep_trial(&mut self) {
        for slot in self.slots.iter_mut() {
            if slot.active {
                slot.hi = slot.trial;
            }
        }
    }

    /// Read the values at the state a walk starts from, and re-arm the sliding
    /// guard of every event whose value has left its boundary.
    ///
    /// The values are read again rather than carried over from the last walk:
    /// after a non-terminal root the caller changes the state, so the ones from
    /// before that change describe a different trajectory. A guard whose root
    /// time is this walk's start keeps it — that is the resumption the guard
    /// exists for; one from an earlier time is dropped, since the walk has
    /// moved on and the accessor would otherwise name a root it has left.
    ///
    /// The hits of the previous walk are dropped here, and every event is
    /// evaluated before any guard is touched: a walk that cannot start leaves
    /// the guards as they were rather than re-arming the first few.
    pub(crate) fn begin(&mut self, t: f64, y: &Y) -> Result<(), IntegrationError> {
        self.discard_hits();
        self.eval(t, y)?;
        for slot in self.slots.iter_mut() {
            if !slot.active {
                continue;
            }
            slot.start = slot.trial;
            if slot.guard.at != Some(t) {
                slot.guard.at = None;
            }
            if slot.trial.abs() > slot.boundary {
                slot.guard.on_boundary = false;
            }
        }
        Ok(())
    }

    /// Whether the event at `index` counts a move from `before` to `after` over
    /// a step starting at `t_start`, with its guard taken into account.
    fn crossed(&self, index: usize, t_start: f64, before: f64, after: f64) -> bool {
        let slot = &self.slots[index];
        if slot.guard.at == Some(t_start) && same_side(before, slot.guard.left) {
            // This step starts on a root of this very event, with the value
            // still on the side that root left it. The search stops on the far
            // side of a bracket, so that value is a small non-zero rather than
            // zero, and the change of sign this step sees is the one already
            // reported.
            return false;
        }
        if slot.guard.on_boundary && before.abs() <= slot.boundary {
            // The state has been moving along the boundary since a root of this
            // event, within the width the event calls "still on it".
            return false;
        }
        self.events[index].crossing().matches(before, after)
    }

    /// Whether any event crosses over the step whose far end is in the slots,
    /// recording which ones so the search can ignore the rest.
    fn mark_candidates(&mut self, t_start: f64) -> bool {
        let mut any = false;
        for index in 0..self.slots.len() {
            let slot = self.slots[index];
            let crossed = slot.active && self.crossed(index, t_start, slot.start, slot.hi);
            self.slots[index].candidate = crossed;
            any |= crossed;
        }
        any
    }

    /// Mark as candidates the events the located end of a search now brackets,
    /// and answer whether any of them is new.
    ///
    /// Within one search the candidates only grow: an event that crosses over
    /// the width the search has settled on is one the bisection has to keep
    /// narrowing for, and one that crossed over a wider width still does.
    /// Counting them would not do — a shorter width can bracket one event while
    /// no longer bracketing another, which leaves the count where it was and
    /// the set changed.
    fn add_candidates(&mut self, t_start: f64) -> bool {
        let mut added = false;
        for index in 0..self.slots.len() {
            let slot = self.slots[index];
            if !slot.active || slot.candidate {
                continue;
            }
            if self.crossed(index, t_start, slot.start, slot.hi) {
                self.slots[index].candidate = true;
                added = true;
            }
        }
        added
    }

    /// Whether any of the events already marked as candidates crosses over the
    /// shorter step just evaluated.
    fn any_candidate_crosses(&self, t_start: f64) -> bool {
        (0..self.slots.len()).any(|index| {
            let slot = &self.slots[index];
            slot.candidate && self.crossed(index, t_start, slot.start, slot.trial)
        })
    }

    /// Record the events that cross over the located step, ordered by priority
    /// and then by registration, and report whether any is terminal.
    fn record_hits(&mut self, t_start: f64) -> bool {
        for slot in self.slots.iter_mut() {
            slot.hit = None;
            slot.rank = None;
            slot.located = slot.hi;
        }
        for index in 0..self.slots.len() {
            let slot = self.slots[index];
            if !(slot.candidate && self.crossed(index, t_start, slot.start, slot.hi)) {
                continue;
            }
            let event = self.events[index];
            self.slots[index].hit = Some(RootHit {
                event: index,
                terminal: event.terminal(),
                priority: event.priority(),
            });
        }
        // Rank by priority, keeping registration order among equals: a
        // selection pass, since the set is small and no allocation is
        // available.
        let total = self.hit_count();
        let mut terminal = false;
        for rank in 0..total {
            let mut best: Option<(usize, i32)> = None;
            for index in 0..self.slots.len() {
                let slot = &self.slots[index];
                let Some(hit) = slot.hit else { continue };
                if slot.rank.is_some() {
                    continue;
                }
                if best.is_none_or(|(_, priority)| hit.priority < priority) {
                    best = Some((index, hit.priority));
                }
            }
            let (index, _) = best.expect("an unranked hit for every rank below the count");
            self.slots[index].rank = Some(rank);
            terminal |= self.slots[index]
                .hit
                .expect("the slot just ranked holds a hit")
                .terminal;
        }
        terminal
    }

    /// Read the values at a state the stepper is about to commit.
    ///
    /// Separate from [`apply`](Self::apply) so a non-finite value here is
    /// reported while the stepper still holds its previous state: a projection
    /// can produce one that the raw candidate did not have, and an error must
    /// not leave the walk standing on a state it also refused. A failure here
    /// also drops the hits the search had recorded, so [`hits`](Self::hits)
    /// describes a root the walk committed rather than one it gave up on.
    pub(crate) fn check(&mut self, t: f64, y: &Y) -> Result<(), IntegrationError> {
        if let Err(e) = self.eval(t, y) {
            self.discard_hits();
            return Err(e);
        }
        Ok(())
    }

    /// Drop the hits the search recorded, for a stepper that refuses the state
    /// they belong to before it asks for their values.
    pub(crate) fn discard_hits(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.hit = None;
            slot.rank = None;
        }
    }

    /// Record where the walk now is: the events that just fired are on their
    /// boundary at `t`, and the rest have moved on.
    ///
    /// The values are the ones [`check`](Self::check) read for the same state,
    /// so this cannot fail and the stepper can update itself first.
    pub(crate) fn apply(&mut self, t: f64) {
        for slot in self.slots.iter_mut() {
            if !slot.active {
                continue;
            }
            if slot.hit.is_some() {
                slot.guard.at = Some(t);
                // The raw value at the crossing, not the committed one: a
                // projection can put the state back on the side the walk came
                // from, and resuming from there is not a departure from this
                // root.
                slot.guard.left = slot.located;
                slot.guard.on_boundary = true;
            } else {
                // The walk has taken a step that did not end on this event's
                // boundary, so it is no longer standing on the root it reported.
                slot.guard.at = None;
                if slot.trial.abs() > slot.boundary {
                    slot.guard.on_boundary = false;
                }
            }
            slot.start = slot.trial;
        }
    }
}

/// What examining one committed step for roots found.
pub(crate) enum StepRoots<Y> {
    /// No event crossed; the step's own candidate is what to commit.
    None,
    /// A root was located. `state` is the raw state at `t`, which the stepper
    /// projects and commits in place of the step's candidate.
    Found {
        t: f64,
        state: Y,
        bracket: f64,
        terminal: bool,
    },
}

impl<Y: Clone> RootSet<'_, Y> {
    /// Examine one step, from the committed `(t0, y0)` to the raw candidate
    /// `y_end` of width `h`, and locate the earliest crossing in it.
    ///
    /// `raw_step(width)` must re-step from `(t0, y0)` by `width` with the same
    /// solver and the same right-hand side, and return the raw candidate:
    /// unprojected, and without touching any step-size control or cached stage
    /// derivative. Bisection is the only localizer available, since neither
    /// adaptive solver carries a dense output.
    pub(crate) fn scan_step<R>(
        &mut self,
        t0: f64,
        h: f64,
        y_end: &Y,
        mut raw_step: R,
    ) -> Result<StepRoots<Y>, IntegrationError>
    where
        R: FnMut(f64) -> Result<Y, IntegrationError>,
    {
        if !self.any_active() {
            return Ok(StepRoots::None);
        }
        self.eval(t0 + h, y_end)?;
        self.keep_trial();
        if !self.mark_candidates(t0) {
            return Ok(StepRoots::None);
        }

        // Invariant: a candidate crosses within `(t0, t0 + hi]`, and none
        // within `(t0, t0 + lo]`. Both ends re-step from `(t0, y0)`, so the
        // step-size control and the mode of the system stay where they were.
        let mut lo = 0.0_f64;
        let mut hi = h;
        let mut y_hi = y_end.clone();
        // One pass per search. A pass ends on a located time, and the values
        // there can bracket an event the whole step did not: the width the
        // search settles on is a step the caller never asked about. Each
        // further pass therefore starts from `lo = 0` with a candidate more
        // than the last, which is what bounds their number: pass `k` runs with
        // at least `k` candidates, so the last pass runs with every event a
        // candidate and adds none. The loop never ends on an addition it has
        // not searched for.
        for _ in 0..self.slots.len() {
            let mut iterations = 0_u32;
            while hi - lo > self.search.t_tolerance {
                if iterations >= self.search.max_iterations {
                    return Err(IntegrationError::RootNotLocalized {
                        t: t0,
                        bracket: hi - lo,
                    });
                }
                iterations += 1;
                let mid = lo + (hi - lo) / 2.0;
                if mid <= lo || mid >= hi {
                    // f64 has no width left between the two ends: the bracket is as
                    // tight as the clock can express, which is tighter than asked.
                    break;
                }
                // The widths can still be distinct where the times they name are
                // not: at `t0 = 1e15` the spacing is `0.125`, so `t0 + 0.03125` is
                // `t0`. Narrowing past that point would commit a state at a time
                // the clock never left.
                if t0 + mid == t0 + lo || t0 + mid == t0 + hi {
                    break;
                }
                let y_mid = raw_step(mid)?;
                self.eval(t0 + mid, &y_mid)?;
                if self.any_candidate_crosses(t0) {
                    hi = mid;
                    y_hi = y_mid;
                    self.keep_trial();
                } else {
                    lo = mid;
                }
            }
            // `keep_trial` stored every active event's value at `t0 + hi`, so
            // this reads the located end without stepping again.
            if !self.add_candidates(t0) {
                break;
            }
            lo = 0.0;
        }

        // The located time has to be one the clock actually reached. Where the
        // spacing of f64 at `t0` is wider than the bracket, it is not, and
        // publishing it would hand the caller a state at a time the walk
        // never advanced to.
        if t0 + hi == t0 {
            return Err(IntegrationError::TimeStagnated { t: t0, dt: hi });
        }
        let terminal = self.record_hits(t0);
        Ok(StepRoots::Found {
            t: t0 + hi,
            state: y_hi,
            bracket: hi - lo,
            terminal,
        })
    }
}

/// Build a set over the given events, declaring the storage it borrows.
///
/// The slots belong to the caller, so a test that wants a set has to hold them
/// somewhere: this declares both in the caller's scope and names the set.
#[cfg(test)]
macro_rules! root_set {
    ($name:ident, $search:expr, $($event:expr),+ $(,)?) => {
        let events = [$($event),+];
        let mut storage = [$crate::RootSlot::new(); 8];
        let slots = &mut storage[..events.len()];
        let mut $name = $crate::RootSet::new(&events, slots, $search).expect("valid search");
    };
}
#[cfg(test)]
pub(crate) use root_set;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rising_event_ignores_a_fall_and_the_other_way_round() {
        assert!(Crossing::Rising.matches(-1.0, 1.0));
        assert!(!Crossing::Rising.matches(1.0, -1.0));
        assert!(Crossing::Falling.matches(1.0, -1.0));
        assert!(!Crossing::Falling.matches(-1.0, 1.0));
        assert!(Crossing::Either.matches(-1.0, 1.0));
        assert!(Crossing::Either.matches(1.0, -1.0));
    }

    /// Zero counts toward whichever side the value is on at the other end.
    ///
    /// Arriving at zero and leaving zero are the same pair of numbers at a
    /// step's endpoints, so this cannot tell them apart and does not try. What
    /// separates them is whether a root left the state there, which
    /// [`RootGuard`] holds — see
    /// `the_boundary_a_root_left_is_not_reported_again_from_the_state_it_left`.
    #[test]
    fn zero_counts_toward_the_side_the_value_reaches() {
        // Arriving.
        assert!(Crossing::Rising.matches(-1.0, 0.0));
        assert!(Crossing::Falling.matches(1.0, 0.0));
        // Leaving.
        assert!(Crossing::Rising.matches(0.0, 1.0));
        assert!(Crossing::Falling.matches(0.0, -1.0));
        // Each direction still ignores the other.
        assert!(!Crossing::Rising.matches(0.0, -1.0));
        assert!(!Crossing::Falling.matches(0.0, 1.0));
        // Staying put is no crossing at all.
        assert!(!Crossing::Rising.matches(0.0, 0.0));
        assert!(!Crossing::Falling.matches(0.0, 0.0));
        assert!(!Crossing::Either.matches(0.0, 0.0));
    }

    /// A step that starts on the boundary and crosses later in that step still
    /// reports the later crossing.
    ///
    /// `g(t) = t (0.5 - t)` over `[0, 1]` starts at zero, rises, and falls back
    /// through zero at `t = 0.5` — one change of sign, which the contract
    /// allows. Reading the endpoints alone gives `(0, -0.5)`; treating a
    /// departure from zero as no crossing would discard the step and lose the
    /// event.
    #[test]
    fn a_step_starting_on_the_boundary_still_reports_a_later_crossing() {
        struct Arch;
        impl RootEvent<f64> for Arch {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                t * (0.5 - t)
            }
        }
        let event = Arch;
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &event as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!(
                    (t - 0.5).abs() <= 1e-9,
                    "located {t}, the crossing is at 0.5"
                );
            }
            StepRoots::None => panic!("the value falls back through zero at 0.5"),
        }
    }

    /// A value that runs out and comes back within one step is still located,
    /// when another event's crossing splits that step.
    ///
    /// `dip(t) = (t - 0.2) (t - 0.6)` is `+0.12` at `t = 0` and `+0.16` at
    /// `t = 1`: the two ends alone say it never ran out. `split(t) = 0.4 - t`
    /// crosses at `t = 0.4`, between the dip's two zeros, and the ends of the
    /// step up to there do show the dip. The earliest crossing in the step is
    /// the dip's, at `t = 0.2`.
    #[test]
    fn a_value_that_runs_out_and_comes_back_is_located_once_a_root_splits_the_step() {
        struct Dip;
        impl RootEvent<f64> for Dip {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                (t - 0.2) * (t - 0.6)
            }
            fn crossing(&self) -> Crossing {
                Crossing::Falling
            }
            fn terminal(&self) -> bool {
                false
            }
        }
        struct Split;
        impl RootEvent<f64> for Split {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                0.4 - t
            }
            fn crossing(&self) -> Crossing {
                Crossing::Falling
            }
            fn terminal(&self) -> bool {
                false
            }
        }
        let dip = Dip;
        let split = Split;
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &dip as &dyn RootEvent<f64>,
            &split as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite values");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!(
                    (t - 0.2).abs() <= 1e-8,
                    "located {t}, the dip runs out at 0.2"
                );
                let hits: Vec<usize> = set.hits().map(|hit| hit.event).collect();
                assert_eq!(
                    hits,
                    vec![0],
                    "the split's crossing is past the located time"
                );
            }
            StepRoots::None => panic!("the dip runs out at 0.2, inside the step"),
        }
    }

    /// Each pass can bracket a further event, so the search keeps the
    /// candidates it has and looks for what the located end adds.
    ///
    /// Over `[0, 1]`, all three falling: `a(t) = 0.8 - t`,
    /// `b(t) = (t - 0.2) (t - 0.9)`, `c(t) = (t - 0.1) (t - 0.3)`. Only `a`
    /// crosses over the whole step. Locating it at `0.8` brackets `b`, and
    /// locating `b` at `0.2` brackets `c` while `a` no longer crosses — the
    /// number of events bracketed stays at two, so a search that stopped when
    /// that number stopped growing would commit `0.2` and lose `c`'s crossing
    /// at `0.1`.
    #[test]
    fn a_search_that_brackets_a_further_event_each_pass_reaches_the_earliest_root() {
        struct Poly(fn(f64) -> f64);
        impl RootEvent<f64> for Poly {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                (self.0)(t)
            }
            fn crossing(&self) -> Crossing {
                Crossing::Falling
            }
            fn terminal(&self) -> bool {
                false
            }
        }
        let a = Poly(|t| 0.8 - t);
        let b = Poly(|t| (t - 0.2) * (t - 0.9));
        let c = Poly(|t| (t - 0.1) * (t - 0.3));
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &a as &dyn RootEvent<f64>,
            &b as &dyn RootEvent<f64>,
            &c as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite values");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!((t - 0.1).abs() <= 1e-8, "located {t}, c runs out at 0.1");
                let hits: Vec<usize> = set.hits().map(|hit| hit.event).collect();
                assert_eq!(hits, vec![2], "a and b cross past the located time");
            }
            StepRoots::None => panic!("c runs out at 0.1, inside the step"),
        }
    }

    /// A located end that brackets nothing new costs no second narrowing.
    ///
    /// `0.4 - t` over `[0, 1]` crosses once, so the step is narrowed once:
    /// `log2(1 / 1e-9)` is 30 halvings, and each of them re-steps. A second
    /// pass would re-step about 29 times more.
    #[test]
    fn a_step_with_nothing_new_at_its_located_end_is_narrowed_once() {
        struct Fall;
        impl RootEvent<f64> for Fall {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                0.4 - t
            }
            fn crossing(&self) -> Crossing {
                Crossing::Falling
            }
            fn terminal(&self) -> bool {
                false
            }
        }
        let fall = Fall;
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &fall as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        let mut steps = 0_u32;
        let counted = |width: f64| {
            steps += 1;
            ramp(width)
        };
        set.scan_step(0.0, 1.0, &1.0, counted).expect("located");
        assert!(steps <= 32, "re-stepped {steps} times for one narrowing");
    }

    struct Level {
        level: f64,
        crossing: Crossing,
        terminal: bool,
        priority: i32,
        boundary: f64,
    }

    impl Level {
        fn at(level: f64) -> Self {
            Self {
                level,
                crossing: Crossing::Either,
                terminal: true,
                priority: 0,
                boundary: 0.0,
            }
        }
    }

    impl RootEvent<f64> for Level {
        fn value(&self, _t: f64, y: &f64) -> f64 {
            y - self.level
        }
        fn crossing(&self) -> Crossing {
            self.crossing
        }
        fn terminal(&self) -> bool {
            self.terminal
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        fn boundary_tolerance(&self) -> f64 {
            self.boundary
        }
    }

    /// `y' = 1` from `y = 0`, so the raw state after a width is the width
    /// itself. The solution is exact, which makes the located time comparable
    /// against the analytic crossing.
    fn ramp(width: f64) -> Result<f64, IntegrationError> {
        Ok(width)
    }

    /// What a stepper does after it has projected and stored a state: read the
    /// values, then record where the walk is.
    fn commit(set: &mut RootSet<'_, f64>, t: f64, y: &f64) {
        set.check(t, y).expect("finite value");
        set.apply(t);
    }

    #[test]
    fn the_located_time_is_the_analytic_crossing_within_the_tolerance() {
        let level = Level::at(0.25);
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");

        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found {
                t,
                state,
                bracket,
                terminal,
            } => {
                assert!((t - 0.25).abs() <= 1e-9, "located t = {t}");
                assert!((state - 0.25).abs() <= 1e-9, "state = {state}");
                assert!(bracket <= 1e-9, "bracket = {bracket}");
                assert!(terminal);
                assert_eq!(
                    set.hits().collect::<Vec<_>>(),
                    &[RootHit {
                        event: 0,
                        terminal: true,
                        priority: 0
                    }]
                );
            }
            StepRoots::None => panic!("the ramp crosses 0.25 inside the step"),
        }
    }

    #[test]
    fn a_level_the_step_does_not_reach_is_not_a_root() {
        let level = Level::at(2.0);
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp).expect("no error"),
            StepRoots::None
        ));
        assert_eq!(set.hit_count(), 0);
    }

    /// Two levels inside one step: the search converges on the earlier one, and
    /// the later one is left for the next step.
    #[test]
    fn the_earlier_of_two_crossings_is_the_one_located() {
        let early = Level::at(0.25);
        let late = Level::at(0.75);
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &early as &dyn RootEvent<f64>,
            &late
        );
        set.begin(0.0, &0.0).expect("finite value");

        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!((t - 0.25).abs() <= 1e-9, "located t = {t}");
                assert_eq!(
                    set.hit_count(),
                    1,
                    "hits: {:?}",
                    set.hits().collect::<Vec<_>>()
                );
                assert_eq!(set.hit_at(0).expect("one hit").event, 0);
            }
            StepRoots::None => panic!("both levels are inside the step"),
        }
    }

    /// Two events on the same level cross together, so both are reported, and
    /// priority decides the order rather than registration.
    #[test]
    fn events_crossing_together_are_reported_as_a_group_in_priority_order() {
        let first = Level {
            priority: 5,
            ..Level::at(0.5)
        };
        let second = Level {
            priority: -1,
            terminal: false,
            ..Level::at(0.5)
        };
        root_set!(
            set,
            RootSearch::default(),
            &first as &dyn RootEvent<f64>,
            &second
        );
        set.begin(0.0, &0.0).expect("finite value");

        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { terminal, .. } => {
                assert!(terminal, "one of the two asked to stop");
                assert_eq!(
                    set.hits().collect::<Vec<_>>(),
                    &[
                        RootHit {
                            event: 1,
                            terminal: false,
                            priority: -1
                        },
                        RootHit {
                            event: 0,
                            terminal: true,
                            priority: 5
                        },
                    ]
                );
            }
            StepRoots::None => panic!("both events are on the level the step crosses"),
        }
    }

    #[test]
    fn a_falling_event_ignores_the_rising_step_that_crosses_its_level() {
        let level = Level {
            crossing: Crossing::Falling,
            ..Level::at(0.5)
        };
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp).expect("no error"),
            StepRoots::None
        ));
    }

    /// After a non-terminal root the state sits on the boundary. The guard is
    /// what keeps the resumed walk from reporting the departure as a crossing,
    /// and it re-arms once the value is clear of the boundary again.
    #[test]
    fn the_walk_resumed_from_a_boundary_does_not_report_it_again() {
        let level = Level {
            terminal: false,
            ..Level::at(0.5)
        };
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        let t_root = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                assert!(set.guard(0).is_on_boundary());
                assert_eq!(set.guard(0).at(), Some(t));
                t
            }
            StepRoots::None => panic!("the ramp crosses 0.5"),
        };

        // Resume from the boundary: the value leaves zero upward, which is a
        // departure, not an arrival.
        set.begin(t_root, &0.5).expect("finite value");
        assert!(matches!(
            set.scan_step(t_root, 1.0, &1.5, |w| Ok(0.5 + w))
                .expect("no error"),
            StepRoots::None
        ));
    }

    /// A value that only jitters around zero — the state moving along the
    /// boundary — is not a stream of fresh crossings, which is what the event's
    /// own boundary tolerance settles.
    #[test]
    fn jitter_within_the_boundary_tolerance_is_not_a_new_crossing() {
        let sliding = Level {
            terminal: false,
            boundary: 1e-6,
            ..Level::at(0.0)
        };
        root_set!(set, RootSearch::default(), &sliding as &dyn RootEvent<f64>);
        // Reach the boundary from below and commit there, as a non-terminal root
        // leaves the state.
        set.begin(0.0, &-1.0).expect("finite value");
        match set
            .scan_step(0.0, 1.0, &0.0, |w| Ok(-1.0 + w))
            .expect("no error")
        {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
            }
            StepRoots::None => panic!("the walk reaches zero from below"),
        }
        assert!(set.guard(0).is_on_boundary());

        // Jitter to the other side of zero, well inside the tolerance.
        set.begin(1.0, &-1e-9).expect("finite value");
        assert!(
            set.guard(0).is_on_boundary(),
            "a value inside the tolerance leaves the guard set"
        );
        assert!(matches!(
            set.scan_step(1.0, 1.0, &1e-9, |w| Ok(-1e-9 + 2e-9 * w))
                .expect("no error"),
            StepRoots::None
        ));

        // Once the value is clear of the boundary, the guard re-arms and a
        // return to zero is a crossing again.
        set.begin(2.0, &1.0).expect("finite value");
        assert!(!set.guard(0).is_on_boundary());
        assert!(matches!(
            set.scan_step(2.0, 2.0, &-1.0, |w| Ok(1.0 - w))
                .expect("no error"),
            StepRoots::Found { .. }
        ));
    }

    #[test]
    fn a_non_finite_value_stops_the_walk_rather_than_being_bisected_on() {
        struct Blows;
        impl RootEvent<f64> for Blows {
            fn value(&self, _t: f64, y: &f64) -> f64 {
                if *y > 0.5 { f64::NAN } else { y - 0.75 }
            }
        }
        let event = Blows;
        root_set!(set, RootSearch::default(), &event as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("the start is finite");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp),
            Err(IntegrationError::NonFiniteRootValue { t, event: 0 }) if t == 1.0
        ));
    }

    /// A value whose sign depends on something other than where the state is —
    /// here on whether the trial reaches the end of the step — has no crossing
    /// for the bisection to narrow. The search reports that rather than a time
    /// it did not localize.
    #[test]
    fn a_search_that_does_not_converge_is_reported_rather_than_rounded() {
        struct Endpoint;
        impl RootEvent<f64> for Endpoint {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                if t >= 1.0 { 1.0 } else { -1.0 }
            }
        }
        let event = Endpoint;
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 4,
            },
            &event as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp),
            Err(IntegrationError::RootNotLocalized { t, .. }) if t == 0.0
        ));
    }

    #[test]
    fn a_search_that_cannot_narrow_a_bracket_is_refused_before_the_walk() {
        let level = Level::at(1.0);
        for search in [
            RootSearch {
                t_tolerance: 0.0,
                max_iterations: 60,
            },
            RootSearch {
                t_tolerance: -1e-3,
                max_iterations: 60,
            },
            RootSearch {
                t_tolerance: f64::NAN,
                max_iterations: 60,
            },
            RootSearch {
                t_tolerance: f64::INFINITY,
                max_iterations: 60,
            },
            RootSearch {
                t_tolerance: 1e-3,
                max_iterations: 0,
            },
        ] {
            assert!(
                RootSet::new(
                    &[&level as &dyn RootEvent<f64>],
                    &mut [RootSlot::new()],
                    search
                )
                .is_err(),
                "{search:?} was accepted"
            );
        }
    }

    /// A walk resumed from the state the search actually committed does not
    /// report the same boundary again.
    ///
    /// The search stops on the far side of a bracket, so the value there is a
    /// small non-zero rather than zero — the level is a third, which no bracket
    /// end lands on exactly. What suppresses the report is the time: this step
    /// starts on the root, so whichever way the caller sends the state next,
    /// the sign change it sees belongs to the root already reported.
    #[test]
    fn the_boundary_a_root_left_is_not_reported_again_from_the_state_it_left() {
        let level = Level {
            terminal: false,
            ..Level::at(1.0 / 3.0)
        };
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        let (t_root, y_root) = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                (t, state)
            }
            StepRoots::None => panic!("the ramp crosses a third"),
        };
        assert_ne!(
            y_root - 1.0 / 3.0,
            0.0,
            "the committed state is not exactly on the boundary, which is the case \
             a guard reading only the value would miss"
        );

        // Resume from that state with the motion reversed, so the value crosses
        // back through the boundary at once.
        set.begin(t_root, &y_root).expect("finite value");
        assert!(matches!(
            set.scan_step(t_root, 1.0, &(y_root - 1.0), |w| Ok(y_root - w))
                .expect("no error"),
            StepRoots::None
        ));

        // A step that lands away from the boundary clears the guard, and the
        // next return through it is a crossing again.
        let away = y_root + 1.0;
        commit(&mut set, t_root + 1.0, &away);
        assert_eq!(set.guard(0).at(), None);
        set.begin(t_root + 1.0, &away).expect("finite value");
        assert!(matches!(
            set.scan_step(t_root + 1.0, 2.0, &(away - 2.0), |w| Ok(away - w))
                .expect("no error"),
            StepRoots::Found { .. }
        ));
    }

    /// A caller that moves the value across zero before resuming has taken the
    /// state off the root, so the step it then takes reports a crossing.
    ///
    /// The guard suppresses the step leaving a root, and what identifies that
    /// step is the time together with the side the value is on. Suppressing on
    /// the time alone would lose this crossing.
    #[test]
    fn a_value_the_caller_moved_across_zero_crosses_again_from_the_same_time() {
        let level = Level {
            terminal: false,
            crossing: Crossing::Rising,
            ..Level::at(1.0 / 3.0)
        };
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        let t_root = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                t
            }
            StepRoots::None => panic!("the ramp crosses a third"),
        };
        assert!(set.guard(0).at() == Some(t_root));

        // The caller puts the state well below the level, at the same time, and
        // resumes. Rising through the level from there is a crossing.
        let below = 0.0;
        set.begin(t_root, &below).expect("finite value");
        assert_eq!(
            set.guard(0).at(),
            Some(t_root),
            "the walk is still standing at that time"
        );
        assert!(matches!(
            set.scan_step(t_root, 1.0, &(below + 1.0), |w| Ok(below + w))
                .expect("no error"),
            StepRoots::Found { .. }
        ));
    }

    /// A root that landed exactly on zero left the value with no side, so a
    /// caller that moves it anywhere non-zero has moved the state off that root.
    ///
    /// `Crossing::Falling` reaching the level from above lands on exactly zero
    /// when the level is a grid time of the walk. Treating zero as the positive
    /// side would then read a return to positive as "still where the root left
    /// it" and suppress the fall that follows.
    #[test]
    fn a_root_that_landed_on_zero_does_not_claim_a_side() {
        let level = Level {
            terminal: false,
            crossing: Crossing::Falling,
            ..Level::at(0.5)
        };
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );

        // Fall from 1.0 and land on the level exactly: the value at the root is
        // exactly zero.
        set.begin(0.0, &1.0).expect("finite value");
        let t_root = match set
            .scan_step(0.0, 0.5, &0.5, |w| Ok(1.0 - w))
            .expect("located")
        {
            StepRoots::Found { t, state, .. } => {
                assert_eq!(state, 0.5, "the step lands on the level exactly");
                commit(&mut set, t, &state);
                t
            }
            StepRoots::None => panic!("the fall reaches 0.5"),
        };
        assert_eq!(set.guard(0).at(), Some(t_root));

        // The caller puts the value back above the level, at the same time, and
        // it falls through again. That is a crossing.
        set.begin(t_root, &1.0).expect("finite value");
        assert!(matches!(
            set.scan_step(t_root, 1.0, &0.0, |w| Ok(1.0 - w))
                .expect("no error"),
            StepRoots::Found { .. }
        ));
    }

    /// A projection that puts the committed state back on the side the walk came
    /// from does not make the resumed step a departure.
    ///
    /// The guard has to remember the side the *crossing* went to, which is the
    /// raw value the search read. Here the projection pulls the state back below
    /// the level, so the value at the committed state has the sign it had before
    /// the root; reading that as "the side the root left it" would suppress the
    /// next genuine crossing for a whole step.
    #[test]
    fn a_projection_back_across_the_boundary_does_not_suppress_the_next_crossing() {
        let level = Level {
            terminal: false,
            crossing: Crossing::Rising,
            ..Level::at(0.5)
        };
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        let t_root = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                // The stepper projects the located state back under the level,
                // the way a clamping constraint would, and commits that.
                let projected = 0.25;
                commit(&mut set, t, &projected);
                assert!(
                    set.guard(0).is_on_boundary(),
                    "the event fired, so its guard is set"
                );
                t
            }
            StepRoots::None => panic!("the ramp crosses 0.5"),
        };

        // Resuming from under the level and rising through it again is a
        // crossing: the root went to the far side, and the state is no longer
        // there.
        set.begin(t_root, &0.25).expect("finite value");
        assert!(matches!(
            set.scan_step(t_root, 1.0, &1.25, |w| Ok(0.25 + w))
                .expect("no error"),
            StepRoots::Found { .. }
        ));
    }

    /// A bracket the clock cannot express is refused rather than committed at a
    /// time the walk never reached.
    ///
    /// At `t0 = 1e15` the spacing of f64 is `0.125`, so every width the search
    /// would narrow to names the same instant as `t0`. Publishing one would hand
    /// the caller a state at a time the walk never advanced to.
    #[test]
    fn a_root_the_clock_cannot_place_is_refused() {
        const T0: f64 = 1e15;
        let level = Level::at(0.03);
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(T0, &0.0).expect("finite value");
        assert!(
            T0 + 0.03125 == T0,
            "the premise: widths this small name no new instant at {T0}"
        );
        assert!(matches!(
            set.scan_step(T0, 0.0625, &0.0625, ramp),
            Err(IntegrationError::TimeStagnated { .. })
        ));
    }

    /// A step whose midpoints name no new instant reports the root at the step's
    /// end, which is the only time in it the clock can express.
    ///
    /// At `t0 = 1e15` the spacing of f64 is `0.125`, so a step of exactly that
    /// width does advance the clock while its first trial at `0.0625` does not.
    /// The search stops narrowing there and keeps the end it has. Without that
    /// stop it would halve its way down to a width the clock cannot take and
    /// fail instead.
    #[test]
    fn a_step_the_clock_can_only_express_whole_reports_its_end() {
        const T0: f64 = 1e15;
        const H: f64 = 0.125;
        assert!(
            T0 + H > T0,
            "the premise: the whole step advances the clock"
        );
        assert!(
            T0 + H / 2.0 == T0,
            "the premise: its first trial names no new instant"
        );

        let level = Level::at(0.06);
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 200,
            },
            &level as &dyn RootEvent<f64>
        );
        set.begin(T0, &0.0).expect("finite value");
        match set.scan_step(T0, H, &H, ramp).expect("located") {
            StepRoots::Found { t, .. } => assert_eq!(t, T0 + H),
            StepRoots::None => panic!("the ramp crosses 0.06 inside the step"),
        }
    }

    /// A walk that starts somewhere other than the last root no longer reports
    /// standing on it.
    #[test]
    fn a_walk_starting_away_from_a_root_forgets_it() {
        let level = Level {
            terminal: false,
            ..Level::at(0.5)
        };
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        let t_root = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                t
            }
            StepRoots::None => panic!("the ramp crosses 0.5"),
        };
        assert_eq!(set.guard(0).at(), Some(t_root));

        set.begin(t_root + 1.0, &1.5).expect("finite value");
        assert_eq!(set.guard(0).at(), None);
    }

    /// A root the walk gave up on is not left in `hits`.
    ///
    /// The stepper reads the values at the projected state before it moves, and
    /// a non-finite one there fails the walk. The hits the search recorded
    /// belong to a state that was never committed.
    #[test]
    fn hits_are_dropped_when_the_committed_state_has_no_finite_value() {
        /// A pole at a third, which no bisection trial lands on exactly — the
        /// trials are dyadic fractions of the step.
        const POLE: f64 = 1.0 / 3.0;

        struct Breaks;
        impl RootEvent<f64> for Breaks {
            fn value(&self, _t: f64, y: &f64) -> f64 {
                1.0 / (POLE - y)
            }
        }
        let event = Breaks;
        root_set!(set, RootSearch::default(), &event as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        // A step from 0 to 1 passes the pole, so the value changes sign and the
        // search locates it.
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { .. } => {
                assert_eq!(set.hit_count(), 1, "the search recorded a hit");
            }
            StepRoots::None => panic!("the value changes sign across the pole"),
        }
        // A projection that put the state exactly on the pole is what the
        // stepper asks about before it moves.
        assert!(matches!(
            set.check(POLE, &POLE),
            Err(IntegrationError::NonFiniteRootValue { .. })
        ));
        assert!(
            set.hit_count() == 0,
            "a hit the walk gave up on stays out of hits(): {:?}",
            set.hits().collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_event_whose_boundary_tolerance_cannot_re_arm_a_guard_is_refused() {
        for bad in [-1e-9, f64::NAN, f64::INFINITY] {
            let event = Level {
                boundary: bad,
                ..Level::at(1.0)
            };
            assert!(
                matches!(
                    RootSet::new(
                        &[&event as &dyn RootEvent<f64>],
                        &mut [RootSlot::new()],
                        RootSearch::default()
                    ),
                    Err(IntegrationError::InvalidBoundaryTolerance { event: 0, .. })
                ),
                "a boundary tolerance of {bad} was accepted"
            );
        }
    }

    /// What the search reports when a step holds three crossings of one event,
    /// which the contract asks the caller to prevent by bounding the step.
    ///
    /// The first trial is at the middle of the step, where `g` has the sign it
    /// started with, so the first two crossings are discarded and the bisection
    /// converges on the third. The test states the outcome rather than calling
    /// it correct: a caller who needs the first of three has to give the search
    /// a step that holds one.
    #[test]
    fn three_crossings_in_one_step_converge_on_the_last() {
        struct Cubic;
        impl RootEvent<f64> for Cubic {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                (t - 0.2) * (t - 0.4) * (t - 0.8)
            }
        }
        let event = Cubic;
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &event as &dyn RootEvent<f64>
        );
        set.begin(0.0, &0.0).expect("finite value");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!((t - 0.8).abs() <= 1e-9, "located {t}");
            }
            StepRoots::None => panic!("the value changes sign over the step"),
        }
    }

    /// Two crossings of one event inside a step cancel, so the step reports
    /// nothing at all.
    #[test]
    fn two_crossings_in_one_step_are_not_seen() {
        struct Dip;
        impl RootEvent<f64> for Dip {
            fn value(&self, t: f64, _y: &f64) -> f64 {
                (t - 0.3) * (t - 0.7)
            }
        }
        let event = Dip;
        root_set!(set, RootSearch::default(), &event as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp).expect("no error"),
            StepRoots::None
        ));
    }

    /// Events of equal priority are reported in the order they were registered.
    #[test]
    fn events_of_equal_priority_keep_their_registration_order() {
        let first = Level {
            terminal: false,
            ..Level::at(0.5)
        };
        let second = Level {
            terminal: false,
            ..Level::at(0.5)
        };
        let third = Level {
            terminal: false,
            ..Level::at(0.5)
        };
        root_set!(
            set,
            RootSearch::default(),
            &first as &dyn RootEvent<f64>,
            &second as &dyn RootEvent<f64>,
            &third as &dyn RootEvent<f64>,
        );
        set.begin(0.0, &0.0).expect("finite value");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { .. } => {
                let order: Vec<usize> = set.hits().map(|hit| hit.event).collect();
                assert_eq!(order, vec![0, 1, 2]);
            }
            StepRoots::None => panic!("all three are on the level the step crosses"),
        }
    }

    /// An empty set makes the walk the plain one: no value evaluation, no trial
    /// step.
    #[test]
    fn a_set_with_no_events_finds_nothing() {
        let events: [&dyn RootEvent<f64>; 0] = [];
        let mut set = RootSet::new(&events, &mut [], RootSearch::default()).expect("valid");
        assert!(set.is_empty());
        set.begin(0.0, &0.0).expect("nothing to evaluate");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, |_| panic!("no event, so no trial step"))
                .expect("no error"),
            StepRoots::None
        ));
    }

    /// A set pairs one slot with one event, so a caller that brings the wrong
    /// number of slots is refused rather than leaving an event unwatched.
    #[test]
    fn a_set_needs_one_slot_per_event() {
        let level = Level::at(0.5);
        let events = [&level as &dyn RootEvent<f64>];
        assert!(matches!(
            RootSet::new(&events, &mut [], RootSearch::default()),
            Err(IntegrationError::RootSlotCount {
                events: 1,
                slots: 0
            })
        ));
    }

    /// An event switched off does not fire, and the search locates the next one
    /// that is on.
    #[test]
    fn a_switched_off_event_does_not_fire() {
        let early = Level::at(0.25);
        let late = Level::at(0.75);
        root_set!(
            set,
            RootSearch {
                t_tolerance: 1e-9,
                max_iterations: 60,
            },
            &early as &dyn RootEvent<f64>,
            &late
        );
        set.deactivate(0);
        set.begin(0.0, &0.0).expect("finite value");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => {
                assert!(
                    (t - 0.75).abs() < 1e-6,
                    "the search passes the switched-off boundary: {t}"
                );
                assert_eq!(set.hit_count(), 1);
                assert_eq!(set.hit_at(0).expect("one hit").event, 1);
            }
            _ => panic!("expected the later boundary to be located"),
        }
    }

    /// A switched-off event is never asked for its value.
    ///
    /// That is the point of switching one off: the release of a constraint that
    /// is not held has no meaning to evaluate, and a stand-in value would be
    /// read as a crossing when the event comes back.
    #[test]
    fn a_switched_off_event_is_not_evaluated() {
        use core::cell::Cell;

        struct Counted<'a>(&'a Cell<usize>);
        impl RootEvent<f64> for Counted<'_> {
            fn value(&self, _t: f64, y: &f64) -> f64 {
                self.0.set(self.0.get() + 1);
                y - 0.5
            }
        }

        let calls = Cell::new(0);
        let event = Counted(&calls);
        root_set!(set, RootSearch::default(), &event as &dyn RootEvent<f64>);
        set.deactivate(0);
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp).expect("no error"),
            StepRoots::None
        ));
        assert_eq!(calls.get(), 0, "a switched-off event is left unevaluated");
    }

    /// Switching an event back on does not report the crossing it missed.
    ///
    /// Its value is read afresh at the start of the next walk, so the side it
    /// is on then is what the next step is compared against. A caller that
    /// needs the boundary honoured at the moment it switches an event on acts
    /// on it there, rather than waiting for a crossing that has been passed.
    #[test]
    fn a_reactivated_event_does_not_report_the_crossing_it_missed() {
        let level = Level::at(0.5);
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.deactivate(0);

        // The walk passes 0.5 with the event switched off.
        set.begin(0.0, &0.0).expect("finite value");
        commit(&mut set, 1.0, &1.0);

        set.activate(0);
        set.begin(1.0, &1.0).expect("finite value");
        assert!(
            matches!(
                set.scan_step(1.0, 1.0, &2.0, |width| Ok(1.0 + width))
                    .expect("no error"),
                StepRoots::None
            ),
            "the crossing at 0.5 is behind the state the event came back on"
        );
        assert_eq!(set.hit_count(), 0);
    }

    /// Switching on an event that is already on leaves its guard alone.
    ///
    /// The guard is what keeps the step that leaves a root from reporting it
    /// again, so re-arming it on every mode change would report a boundary
    /// twice.
    #[test]
    fn switching_on_an_event_that_is_on_keeps_its_guard() {
        let level = Level::at(0.5);
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        let located = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                t
            }
            _ => panic!("expected a located root"),
        };
        assert_eq!(set.guard(0).at(), Some(located));
        assert!(set.guard(0).is_on_boundary());

        set.activate(0);
        assert_eq!(
            set.guard(0).at(),
            Some(located),
            "the event was already on, so it still stands on the root it reported"
        );
        assert!(set.guard(0).is_on_boundary());
    }

    /// An event switched off after a walk failed does not fire from the values
    /// that walk left behind.
    ///
    /// A failed walk keeps the values it was comparing — the crossing pair the
    /// search had found — and the caller is free to switch the event off before
    /// resuming. Switching one off has to stop the search reading them, since
    /// nothing else refreshes them while the event is off.
    #[test]
    fn a_switched_off_event_does_not_fire_from_a_failed_walks_values() {
        /// Finite everywhere the walk starts and steps to, except at the state
        /// the stepper would commit.
        struct Brittle;
        impl RootEvent<f64> for Brittle {
            fn value(&self, _t: f64, y: &f64) -> f64 {
                if *y == 0.6 { f64::NAN } else { y - 0.5 }
            }
        }

        // A second event, switched on throughout and never crossing, so the
        // search still runs once the first one is switched off.
        let event = Brittle;
        let far = Level::at(1.5);
        root_set!(
            set,
            RootSearch::default(),
            &event as &dyn RootEvent<f64>,
            &far
        );
        set.begin(0.0, &0.0).expect("finite value");
        let located = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, .. } => t,
            _ => panic!("expected a located root"),
        };
        assert!(
            set.check(located, &0.6).is_err(),
            "the state the stepper would commit has no finite value"
        );
        assert_eq!(set.hit_count(), 0, "a walk that failed reports no roots");

        set.deactivate(0);
        set.begin(0.0, &0.0).expect("finite value");
        assert!(matches!(
            set.scan_step(0.0, 1.0, &1.0, ramp).expect("no error"),
            StepRoots::None
        ));
        assert_eq!(
            set.hit_count(),
            0,
            "a switched-off event does not fire from the values it kept"
        );
    }

    /// Switching an event off and on again re-arms its guard, so the boundary
    /// the walk is standing on can be reported again.
    ///
    /// While the event is off its value goes unread, and the side a root left
    /// it on says nothing about where it is when it comes back. The step that
    /// the guard would have suppressed — "this is the root already reported" —
    /// is a crossing again.
    #[test]
    fn switching_an_event_off_and_on_re_arms_its_guard() {
        let level = Level::at(0.5);
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        let (root_t, root_y) = match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => {
                commit(&mut set, t, &state);
                (t, state)
            }
            _ => panic!("expected a located root"),
        };
        assert_eq!(set.guard(0).at(), Some(root_t));
        assert!(set.guard(0).is_on_boundary());

        // A step back across the boundary from the root's own time is the root
        // already reported, so the guard suppresses it.
        let back_down = |width: f64| Ok(root_y - width * 0.2);
        let end = root_y - 0.2;
        set.begin(root_t, &root_y).expect("finite value");
        assert!(matches!(
            set.scan_step(root_t, 1.0, &end, back_down)
                .expect("no error"),
            StepRoots::None
        ));

        set.deactivate(0);
        set.activate(0);
        assert_eq!(set.guard(0).at(), None, "coming back re-arms the guard");
        assert!(!set.guard(0).is_on_boundary());

        set.begin(root_t, &root_y).expect("finite value");
        assert!(
            matches!(
                set.scan_step(root_t, 1.0, &end, back_down)
                    .expect("located"),
                StepRoots::Found { .. }
            ),
            "the same step is a crossing for an event that has just come back"
        );
    }

    /// The roots of the bracket the walk committed to stay readable while the
    /// caller switches events, which is what it does while handling them.
    #[test]
    fn switching_events_leaves_the_located_roots_readable() {
        let level = Level::at(0.5);
        root_set!(set, RootSearch::default(), &level as &dyn RootEvent<f64>);
        set.begin(0.0, &0.0).expect("finite value");
        match set.scan_step(0.0, 1.0, &1.0, ramp).expect("located") {
            StepRoots::Found { t, state, .. } => commit(&mut set, t, &state),
            _ => panic!("expected a located root"),
        }
        assert_eq!(set.hit_count(), 1);

        set.deactivate(0);
        assert_eq!(
            set.hit_count(),
            1,
            "switching the event off does not erase the root it just reported"
        );
        assert_eq!(set.hit_at(0).expect("one hit").event, 0);
    }
}
