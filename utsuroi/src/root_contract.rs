//! What a walk with root events promises, checked once per solver.
//!
//! Three steppers implement the boundary contract separately — the fixed-step
//! one, and the adaptive ones behind DP5(4) and DOP853 — so they can break
//! separately. Each case here is written once and run against all three.
//!
//! The system is `dy/dt = t` from `y(0) = 0`, so `y(t) = t² / 2` and every
//! solver integrates it exactly up to rounding: a boundary at `y = level` is
//! crossed at `t = sqrt(2 · level)`, which is an analytic oracle rather than a
//! tolerance. The levels below are chosen so the crossing falls inside a step
//! rather than on the fixed grid, since landing on a grid time is the case that
//! passes even without a search.

use core::cell::Cell;

use nalgebra::Vector1;

use crate::root::root_set;
use crate::{
    AdaptiveStepper, AdaptiveStepper853, Crossing, Dop853, DormandPrince, DynamicalSystem,
    FixedStepper, IntegrationError, Integrator, OdeState, Projection, Rk4, RootEvent, RootOutcome,
    RootSearch, RootSet, State, Tolerances,
};

const T0: f64 = 0.0;
const T_END: f64 = 3.0;
const DT: f64 = 0.25;

/// `y = 1.28` is crossed at `t = 1.6`, between the grid times `1.5` and `1.75`.
const LEVEL: f64 = 1.28;
const ROOT_T: f64 = 1.6;

/// Loose enough for the bracket the search stops at (`1e-9` below) plus the
/// state error the adaptive solvers carry, and far tighter than the `0.25` step
/// a walk without a search would report.
const SLACK: f64 = 1e-6;

const SEARCH: RootSearch = RootSearch {
    t_tolerance: 1e-9,
    max_iterations: 200,
};

/// `dy/dt = t`, whose solution `y = y0 + (t² - t0²) / 2` depends on the time
/// rather than on the state, so every solver here is exact on it.
struct Ramp;

impl DynamicalSystem for Ramp {
    type State = State<1, 1>;
    fn derivatives(&self, t: f64, _state: &Self::State) -> Self::State {
        State {
            components: [Vector1::new(t)],
        }
    }
}

static SYSTEM: Ramp = Ramp;
static RK4: Rk4 = Rk4;
static DP45: DormandPrince = DormandPrince;
static DOP853: Dop853 = Dop853;

fn start() -> State<1, 1> {
    State {
        components: [Vector1::new(0.0)],
    }
}

/// `y` reaches `level`, in whichever direction the caller asks for.
struct AtLevel {
    level: f64,
    crossing: Crossing,
    terminal: bool,
    priority: i32,
}

impl AtLevel {
    const fn rising(level: f64) -> Self {
        Self {
            level,
            crossing: Crossing::Rising,
            terminal: true,
            priority: 0,
        }
    }
}

impl RootEvent<State<1, 1>> for AtLevel {
    fn value(&self, _t: f64, y: &State<1, 1>) -> f64 {
        y.components[0][0] - self.level
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
}

/// A state whose projection is observable: it counts its own calls, and clamps
/// at a ceiling the way a constraint surface pulls a candidate back onto it.
///
/// The count lives beside the state rather than in it, so cloning a state —
/// which the solvers do constantly — does not carry a count with it. It is
/// per-thread, because the test harness runs one test per thread and a shared
/// counter would mix in every other case's projections.
#[derive(Clone, Debug, PartialEq)]
struct Clamped {
    y: f64,
    /// Above this the projection pulls `y` back down. `INFINITY` leaves the
    /// projection a no-op that still counts.
    ceiling: f64,
}

thread_local! {
    static PROJECTIONS: Cell<usize> = const { Cell::new(0) };
}

fn projections() -> usize {
    PROJECTIONS.with(Cell::get)
}

impl OdeState for Clamped {
    fn zero_like(&self) -> Self {
        Self { y: 0.0, ..*self }
    }
    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Self {
            y: self.y + scale * other.y,
            ..*self
        }
    }
    fn scale(&self, factor: f64) -> Self {
        Self {
            y: self.y * factor,
            ..*self
        }
    }
    fn is_finite(&self) -> bool {
        self.y.is_finite()
    }
    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let sc = tol.atol + tol.rtol * self.y.abs().max(y_next.y.abs());
        (error.y / sc).abs()
    }
    fn project(&mut self, _t: f64) -> Projection {
        PROJECTIONS.with(|count| count.set(count.get() + 1));
        if self.y > self.ceiling {
            self.y = self.ceiling;
            Projection::Changed
        } else {
            Projection::Unchanged
        }
    }
}

impl Scalar for Clamped {
    fn scalar(&self) -> f64 {
        self.y
    }
}

/// A state whose projection turns non-finite once it is past a level, for the
/// case where the walk has to refuse the state it just projected.
#[derive(Clone, Debug, PartialEq)]
struct Breaking {
    y: f64,
}

impl OdeState for Breaking {
    fn zero_like(&self) -> Self {
        Self { y: 0.0 }
    }
    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Self {
            y: self.y + scale * other.y,
        }
    }
    fn scale(&self, factor: f64) -> Self {
        Self { y: self.y * factor }
    }
    fn is_finite(&self) -> bool {
        self.y.is_finite()
    }
    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let sc = tol.atol + tol.rtol * self.y.abs().max(y_next.y.abs());
        (error.y / sc).abs()
    }
    fn project(&mut self, _t: f64) -> Projection {
        if self.y > CLAMPED_LEVEL {
            self.y = f64::INFINITY;
            Projection::Changed
        } else {
            Projection::Unchanged
        }
    }
}

impl Scalar for Breaking {
    fn scalar(&self) -> f64 {
        self.y
    }
}

/// A state that reports the accepted step as accurate and every root-search
/// trial as over tolerance.
///
/// The branch under test needs a shorter step whose error estimate is worse
/// than the accepted step's, which no smooth system produces: a trial from the
/// same start is a sub-interval, so its local error is the smaller of the two.
/// Rather than build a right-hand side non-smooth enough to invert that, the
/// state forces the answer. `advance_to_roots` asks for the accepted step's
/// error norm once and then one per trial, in that order, so the first call is
/// the step and the rest are trials.
#[derive(Clone, Debug, PartialEq)]
struct Forced {
    y: f64,
}

thread_local! {
    static NORMS_ASKED: Cell<usize> = const { Cell::new(0) };
}

impl OdeState for Forced {
    fn zero_like(&self) -> Self {
        Self { y: 0.0 }
    }
    fn axpy(&self, scale: f64, other: &Self) -> Self {
        Self {
            y: self.y + scale * other.y,
        }
    }
    fn scale(&self, factor: f64) -> Self {
        Self { y: self.y * factor }
    }
    fn is_finite(&self) -> bool {
        self.y.is_finite()
    }
    fn error_norm(&self, _y_next: &Self, _error: &Self, _tol: &Tolerances) -> f64 {
        let asked = NORMS_ASKED.with(|c| {
            let n = c.get();
            c.set(n + 1);
            n
        });
        if asked == 0 { 0.5 } else { 2.0 }
    }
}

impl Scalar for Forced {
    fn scalar(&self) -> f64 {
        self.y
    }
}

/// `dy/dt = 1` on the state above.
struct Forcing;

impl DynamicalSystem for Forcing {
    type State = Forced;
    fn derivatives(&self, _t: f64, _state: &Self::State) -> Self::State {
        Forced { y: 1.0 }
    }
}

impl RootEvent<Forced> for ClampedLevel {
    fn value(&self, _t: f64, y: &Forced) -> f64 {
        y.y - self.level
    }
    fn crossing(&self) -> Crossing {
        Crossing::Rising
    }
}

/// A trial the solver refuses fails the walk where it stands.
///
/// The state is where the walk started, the time has not moved, and the hits the
/// search had not yet recorded stay empty: a trial that failed error control is
/// not a trajectory, and a crossing time read off it would name nothing.
fn case_a_refused_trial_leaves_the_walk_where_it_started<W: Walker<Sys = Forcing> + HasStepSize>(
    label: &str,
    mut walker: W,
) {
    let event = ClampedLevel { level: 0.02 };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<Forced>);
    let mut reported = Vec::new();

    let err = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect_err("every trial is over tolerance");
    assert!(
        matches!(err, IntegrationError::RootTrialRejected { .. }),
        "{label}: {err:?}"
    );
    assert_eq!(walker.t(), T0, "{label}: the walk did not move");
    assert_eq!(
        walker.y(),
        0.0,
        "{label}: the state is the one it started from"
    );
    assert_eq!(
        roots.hit_count(),
        0,
        "{label}: hits {:?}",
        roots.hits().collect::<Vec<_>>()
    );
    assert!(reported.is_empty(), "{label}: nothing was reported");
}

/// `dy/dt = 1` on a state whose projection breaks past the level.
struct Broken;

impl DynamicalSystem for Broken {
    type State = Breaking;
    fn derivatives(&self, _t: f64, _state: &Self::State) -> Self::State {
        Breaking { y: 1.0 }
    }
}

impl RootEvent<Breaking> for ClampedLevel {
    fn value(&self, _t: f64, y: &Breaking) -> f64 {
        y.y - self.level
    }
    fn crossing(&self) -> Crossing {
        Crossing::Rising
    }
}

/// `dy/dt = 1`, so `y = t` and a level is crossed at the level itself.
struct Unit {
    ceiling: f64,
}

impl DynamicalSystem for Unit {
    type State = Clamped;
    fn derivatives(&self, _t: f64, _state: &Self::State) -> Self::State {
        Clamped {
            y: 1.0,
            ceiling: self.ceiling,
        }
    }
}

/// The one number a system's state carries here, so a case can read it without
/// knowing which system it is driving.
trait Scalar: OdeState {
    fn scalar(&self) -> f64;
}

impl Scalar for State<1, 1> {
    fn scalar(&self) -> f64 {
        self.components[0][0]
    }
}

/// One stepper, driven the same way whichever solver is behind it.
///
/// The system is an associated type rather than a parameter, so a case names
/// only the walker and the states it sees follow from it.
trait Walker {
    type Sys: DynamicalSystem;

    fn advance(
        &mut self,
        t_target: f64,
        reported: &mut Vec<(f64, f64)>,
        roots: &mut RootSet<'_, <Self::Sys as DynamicalSystem>::State>,
    ) -> Result<RootOutcome, IntegrationError>;

    fn t(&self) -> f64;
    fn y(&self) -> f64;
}

impl<S> Walker for FixedStepper<'static, Rk4, S>
where
    S: DynamicalSystem,
    S::State: Scalar,
{
    type Sys = S;

    fn advance(
        &mut self,
        t_target: f64,
        reported: &mut Vec<(f64, f64)>,
        roots: &mut RootSet<'_, S::State>,
    ) -> Result<RootOutcome, IntegrationError> {
        self.advance_to_roots(t_target, |t, y| reported.push((t, y.scalar())), roots)
    }
    fn t(&self) -> f64 {
        FixedStepper::t(self)
    }
    fn y(&self) -> f64 {
        self.state().scalar()
    }
}

impl<S> Walker for AdaptiveStepper<'static, S>
where
    S: DynamicalSystem,
    S::State: Scalar,
{
    type Sys = S;

    fn advance(
        &mut self,
        t_target: f64,
        reported: &mut Vec<(f64, f64)>,
        roots: &mut RootSet<'_, S::State>,
    ) -> Result<RootOutcome, IntegrationError> {
        self.advance_to_roots(t_target, |t, y| reported.push((t, y.scalar())), roots)
    }
    fn t(&self) -> f64 {
        AdaptiveStepper::t(self)
    }
    fn y(&self) -> f64 {
        self.state().scalar()
    }
}

impl<S> Walker for AdaptiveStepper853<'static, S>
where
    S: DynamicalSystem,
    S::State: Scalar,
{
    type Sys = S;

    fn advance(
        &mut self,
        t_target: f64,
        reported: &mut Vec<(f64, f64)>,
        roots: &mut RootSet<'_, S::State>,
    ) -> Result<RootOutcome, IntegrationError> {
        self.advance_to_roots(t_target, |t, y| reported.push((t, y.scalar())), roots)
    }
    fn t(&self) -> f64 {
        AdaptiveStepper853::t(self)
    }
    fn y(&self) -> f64 {
        self.state().scalar()
    }
}

fn rk4_walker() -> FixedStepper<'static, Rk4, Ramp> {
    RK4.stepper(&SYSTEM, start(), T0, DT)
}

fn dp45_walker() -> AdaptiveStepper<'static, Ramp> {
    DP45.stepper(&SYSTEM, start(), T0, DT, Tolerances::default())
}

fn dop853_walker() -> AdaptiveStepper853<'static, Ramp> {
    DOP853.stepper(&SYSTEM, start(), T0, DT, Tolerances::default())
}

/// The reported times increase and end where the stepper says it is, so a
/// trial state that leaked into the callback would show up as a time out of
/// order or past the walk's own end.
fn assert_reported_walk(label: &str, reported: &[(f64, f64)], end_t: f64) {
    assert!(
        !reported.is_empty(),
        "{label}: a walk that advanced must report at least one state"
    );
    for pair in reported.windows(2) {
        assert!(
            pair[1].0 > pair[0].0,
            "{label}: reported times must increase: {:?}",
            reported.iter().map(|s| s.0).collect::<Vec<_>>()
        );
    }
    let last = reported.last().expect("checked non-empty above");
    assert_eq!(
        last.0, end_t,
        "{label}: the last reported state must be the one the stepper holds"
    );
    for (t, _) in reported {
        assert!(
            *t <= end_t,
            "{label}: no reported time may be past the end of the walk: {t} > {end_t}"
        );
    }
}

/// The reported times of a walk that stopped at a root: increasing, and every
/// one of them before the boundary, which the callback does not see at all. The
/// state there is the caller's to handle first, so it reads it from the stepper.
///
/// A root inside the first step reports nothing, which is why this does not ask
/// for a state the way `assert_reported_walk` does.
fn assert_reported_up_to_root(label: &str, reported: &[(f64, f64)], root_t: f64) {
    for pair in reported.windows(2) {
        assert!(
            pair[1].0 > pair[0].0,
            "{label}: reported times must increase: {:?}",
            reported.iter().map(|s| s.0).collect::<Vec<_>>()
        );
    }
    for (t, _) in reported {
        assert!(
            *t < root_t,
            "{label}: the boundary at {root_t} is not reported, and neither is \
             anything past it: {t}"
        );
    }
}

fn case_locates_the_analytic_crossing<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let event = AtLevel::rising(LEVEL);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    match outcome {
        RootOutcome::Roots {
            t,
            bracket,
            terminal,
        } => {
            assert!(
                (t - ROOT_T).abs() <= SLACK,
                "{label}: located {t}, analytic crossing is {ROOT_T}"
            );
            assert!(bracket <= SEARCH.t_tolerance, "{label}: bracket {bracket}");
            assert!(terminal, "{label}: the event asked to stop");
            assert_eq!(walker.t(), t, "{label}: the stepper is at the boundary");
            assert!(
                (walker.y() - LEVEL).abs() <= SLACK,
                "{label}: the committed state is on the boundary: y = {}",
                walker.y()
            );
            assert_eq!(
                roots.hit_count(),
                1,
                "{label}: hits {:?}",
                roots.hits().collect::<Vec<_>>()
            );
            assert_eq!(roots.hit_at(0).expect("one hit").event, 0);
            assert_reported_up_to_root(label, &reported, t);
        }
        RootOutcome::Reached => panic!("{label}: the ramp crosses {LEVEL} before {T_END}"),
    }
}

fn case_a_level_out_of_reach_is_not_a_root<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    // `y(3) = 4.5`, so this level is never reached.
    let event = AtLevel::rising(10.0);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    assert_eq!(
        outcome,
        RootOutcome::Reached,
        "{label}: nothing crosses, so the walk reaches its target"
    );
    assert_eq!(
        roots.hit_count(),
        0,
        "{label}: hits {:?}",
        roots.hits().collect::<Vec<_>>()
    );
    assert_eq!(walker.t(), T_END, "{label}: the walk ends at its target");
    assert_reported_walk(label, &reported, T_END);
}

fn case_the_wrong_direction_is_ignored<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let event = AtLevel {
        crossing: Crossing::Falling,
        ..AtLevel::rising(LEVEL)
    };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    assert_eq!(
        outcome,
        RootOutcome::Reached,
        "{label}: `y` only rises, so a falling event never triggers"
    );
    assert_eq!(walker.t(), T_END);
}

/// A non-terminal root hands control back at the boundary, and the walk resumed
/// from there does not report the same crossing again.
fn case_a_resumed_walk_does_not_refind_the_boundary<W: Walker<Sys = Ramp>>(
    label: &str,
    mut walker: W,
) {
    let event = AtLevel {
        terminal: false,
        ..AtLevel::rising(LEVEL)
    };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    let first = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    let t_root = match first {
        RootOutcome::Roots { t, terminal, .. } => {
            assert!(!terminal, "{label}: the event asked to continue");
            t
        }
        RootOutcome::Reached => panic!("{label}: the ramp crosses {LEVEL} before {T_END}"),
    };

    let mut resumed = Vec::new();
    let second = walker
        .advance(T_END, &mut resumed, &mut roots)
        .expect("the resumed walk succeeds");
    assert_eq!(
        second,
        RootOutcome::Reached,
        "{label}: the boundary the walk is sitting on is not a new crossing"
    );
    assert_eq!(walker.t(), T_END, "{label}: the resumed walk reaches T_END");
    assert!(
        roots.hit_count() == 0,
        "{label}: the second walk found nothing: {:?}",
        roots.hits().collect::<Vec<_>>()
    );
    assert!(
        resumed.first().expect("the resumed walk reports states").0 > t_root,
        "{label}: the resumed walk starts after the boundary"
    );
    assert_reported_walk(label, &resumed, T_END);
}

/// Two events on the same level cross within one bracket, so both are reported,
/// and the caller's priority decides the order.
fn case_simultaneous_roots_are_one_group<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let registered_first = AtLevel {
        priority: 5,
        ..AtLevel::rising(LEVEL)
    };
    let registered_second = AtLevel {
        priority: -1,
        terminal: false,
        ..AtLevel::rising(LEVEL)
    };
    root_set!(
        roots,
        SEARCH,
        &registered_first as &dyn RootEvent<State<1, 1>>,
        &registered_second,
    );
    let mut reported = Vec::new();

    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    match outcome {
        RootOutcome::Roots { t, terminal, .. } => {
            assert!((t - ROOT_T).abs() <= SLACK, "{label}: located {t}");
            assert!(terminal, "{label}: one of the two asked to stop");
            let order: Vec<usize> = roots.hits().map(|hit| hit.event).collect();
            assert_eq!(
                order,
                vec![1, 0],
                "{label}: the lower priority is reported first"
            );
        }
        RootOutcome::Reached => panic!("{label}: both events are on a level the ramp crosses"),
    }
}

/// The earlier of two crossings inside one step is the one the walk stops at,
/// and the later one is still there when the walk resumes.
fn case_the_earlier_crossing_wins<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    // `y = 0.02` at `t = 0.2` and `y = 0.03125` at `t = 0.25`: both inside the
    // first step of the fixed grid.
    let early = AtLevel {
        terminal: false,
        ..AtLevel::rising(0.02)
    };
    let late = AtLevel {
        terminal: false,
        ..AtLevel::rising(0.03125)
    };
    root_set!(roots, SEARCH, &early as &dyn RootEvent<State<1, 1>>, &late);
    let mut reported = Vec::new();

    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    match outcome {
        RootOutcome::Roots { t, .. } => {
            assert!((t - 0.2).abs() <= SLACK, "{label}: located {t}, want 0.2");
            assert_eq!(
                roots.hit_count(),
                1,
                "{label}: only the earlier event crossed: {:?}",
                roots.hits().collect::<Vec<_>>()
            );
            assert_eq!(roots.hit_at(0).expect("one hit").event, 0);
        }
        RootOutcome::Reached => panic!("{label}: both levels are below y(T_END)"),
    }

    let mut resumed = Vec::new();
    match walker
        .advance(T_END, &mut resumed, &mut roots)
        .expect("the resumed walk succeeds")
    {
        RootOutcome::Roots { t, .. } => {
            assert!(
                (t - 0.25).abs() <= SLACK,
                "{label}: the later crossing is at 0.25, located {t}"
            );
            assert_eq!(roots.hit_at(0).expect("one hit").event, 1);
        }
        RootOutcome::Reached => panic!("{label}: the later crossing is still ahead"),
    }
}

/// A boundary inside the last step of the walk is found there, not skipped as
/// the walk lands on its target.
///
/// `y = 4.0` is crossed at `t = sqrt(8) = 2.828…`, inside the fixed grid's
/// final step `[2.75, 3.0]`. A boundary the trajectory reaches only *at* the
/// target is a different case, and one the solver's own error decides: DOP853
/// arrives at `y(3) = 4.5 - 1.8e-15`, so for its trajectory that boundary lies
/// outside the span.
fn case_a_crossing_on_the_last_step_is_found<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let event = AtLevel::rising(4.0);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    match walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds")
    {
        RootOutcome::Roots { t, .. } => {
            let analytic = 8.0_f64.sqrt();
            assert!(
                (t - analytic).abs() <= SLACK,
                "{label}: located {t}, analytic crossing is {analytic}"
            );
            assert!(
                t < T_END,
                "{label}: the boundary is before the target, not on it"
            );
            assert_reported_up_to_root(label, &reported, t);
        }
        RootOutcome::Reached => panic!("{label}: y reaches 4.0 inside the last step"),
    }
}

/// A boundary the fixed grid lands on exactly is reported at that step's end,
/// not one step late.
///
/// `y = 2.0` is crossed at `t = 2.0`, which is a grid time for a `0.25` step.
/// The value at the step's end is then exactly zero rather than past it, which
/// is the case a detection asking for a strict sign change would miss.
fn case_a_crossing_on_a_grid_time_is_found<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let event = AtLevel::rising(2.0);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    match walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds")
    {
        RootOutcome::Roots { t, .. } => {
            assert!(
                (t - 2.0).abs() <= SLACK,
                "{label}: located {t}, analytic crossing is 2.0"
            );
            assert_reported_up_to_root(label, &reported, t);
        }
        RootOutcome::Reached => panic!("{label}: y reaches 2.0 at t = 2.0"),
    }
}

/// A boundary inside the first step leaves the callback untouched.
///
/// `y = 0.02` is crossed at `t = 0.2`, inside the first `0.25` step of every
/// solver here, so the walk commits one state and it is the boundary's. The
/// caller reads it from the stepper, which is where a state it still has to
/// handle belongs.
fn case_a_root_in_the_first_step_reports_nothing<W: Walker<Sys = Ramp>>(
    label: &str,
    mut walker: W,
) {
    let event = AtLevel::rising(0.02);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    match walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds")
    {
        RootOutcome::Roots { t, .. } => {
            let analytic = 0.04_f64.sqrt();
            assert!(
                (t - analytic).abs() <= SLACK,
                "{label}: located {t}, analytic crossing is {analytic}"
            );
            assert!(
                reported.is_empty(),
                "{label}: the only state the walk committed is the boundary's: {:?}",
                reported.iter().map(|s| s.0).collect::<Vec<_>>()
            );
            assert_eq!(walker.t(), t, "{label}: the stepper holds the boundary");
        }
        RootOutcome::Reached => panic!("{label}: y reaches 0.02 inside the first step"),
    }
}

/// A walk over an empty set steps as it always would and reports nothing found.
fn case_no_events_walks_the_span<W: Walker<Sys = Ramp>>(label: &str, mut walker: W) {
    let events: [&dyn RootEvent<State<1, 1>>; 0] = [];
    let mut roots = RootSet::new(&events, &mut [], SEARCH).expect("valid search");
    let mut reported = Vec::new();

    assert_eq!(
        walker
            .advance(T_END, &mut reported, &mut roots)
            .expect("the walk succeeds"),
        RootOutcome::Reached,
        "{label}: an empty set finds nothing"
    );
    assert_eq!(walker.t(), T_END);
    assert!(
        (walker.y() - T_END * T_END / 2.0).abs() <= SLACK,
        "{label}: the walk still integrates the system: y = {}",
        walker.y()
    );
    assert_reported_walk(label, &reported, T_END);
}

/// `y` reaches a level, on a state whose projection can move it.
struct ClampedLevel {
    level: f64,
}

impl RootEvent<Clamped> for ClampedLevel {
    fn value(&self, _t: f64, y: &Clamped) -> f64 {
        y.y - self.level
    }
    fn crossing(&self) -> Crossing {
        Crossing::Rising
    }
}

/// The search does not project the states it tries.
///
/// The walk takes several steps and locates one root, so the projection runs
/// once per accepted step plus once on the state committed at the boundary. A
/// search that projected its trials would run it tens of times more, since the
/// bracket is halved until it is narrower than a nanosecond.
fn case_the_search_does_not_project_its_trials<W: Walker<Sys = Unit>>(label: &str, mut walker: W) {
    let event = ClampedLevel {
        level: CLAMPED_LEVEL,
    };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<Clamped>);
    let mut reported = Vec::new();

    let before = projections();
    let outcome = walker
        .advance(1.0, &mut reported, &mut roots)
        .expect("the walk succeeds");
    let spent = projections() - before;

    assert!(
        matches!(outcome, RootOutcome::Roots { .. }),
        "{label}: y reaches 0.6 inside the span"
    );
    assert_eq!(
        spent,
        reported.len() + 1,
        "{label}: the projection runs once per state the walk committed — the reported \
         steps plus the boundary, which the callback does not see — and on nothing else; \
         reported {:?}",
        reported.iter().map(|s| s.0).collect::<Vec<_>>()
    );
}

/// A crossing that only a raw candidate has is still found.
///
/// The projection clamps at `CEILING`, below the level the event looks for, so
/// no state the walk publishes ever reaches the boundary: a detection reading
/// those states would see the value stay negative for the whole walk and report
/// no crossing at all. What crosses is the candidate inside a step, and that is
/// what detection reads.
///
/// The located time is not the level itself, because the clamping is part of
/// the trajectory: each step starts from the ceiling, so where the crossing
/// falls depends on the widths the solver chose. What every solver owes is that
/// the crossing is found, and that nothing it published reached the level.
fn case_a_crossing_only_the_raw_candidate_has<W: Walker<Sys = Unit>>(label: &str, mut walker: W) {
    let event = ClampedLevel {
        level: CLAMPED_LEVEL,
    };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<Clamped>);
    let mut reported = Vec::new();

    match walker
        .advance(1.0, &mut reported, &mut roots)
        .expect("the walk succeeds")
    {
        RootOutcome::Roots { t, .. } => {
            assert!(
                t > T0 && t < 1.0,
                "{label}: the crossing is inside the span, located {t}"
            );
            assert_eq!(
                walker.t(),
                t,
                "{label}: the stepper stopped where the root was located"
            );
            // The state the walk holds went through the projection, so it is
            // back under the ceiling rather than on the boundary.
            assert!(
                (walker.y() - CEILING).abs() <= SLACK,
                "{label}: the committed state is clamped to the ceiling: {}",
                walker.y()
            );
            for (at, y) in &reported {
                assert!(
                    y - CLAMPED_LEVEL < 0.0,
                    "{label}: the state reported at {at} never reached the level: {y}"
                );
            }
        }
        RootOutcome::Reached => {
            panic!("{label}: a candidate crosses the level, so the walk stops there")
        }
    }
}

/// A root value that only the projected state makes non-finite is reported
/// without moving the walk.
///
/// The event reads `1 / (CEILING - y)`, which the projection drives to a
/// division by zero by clamping `y` onto the ceiling. The stepper has to notice
/// before it stores that state: a caller told the walk failed must still hold
/// the last state the walk accepted.
fn case_a_projection_that_breaks_the_value_leaves_the_walk_where_it_was<W: Walker<Sys = Unit>>(
    label: &str,
    mut walker: W,
) {
    struct Reciprocal;
    impl RootEvent<Clamped> for Reciprocal {
        fn value(&self, _t: f64, y: &Clamped) -> f64 {
            1.0 / (CEILING - y.y)
        }
    }

    let event = Reciprocal;
    root_set!(roots, SEARCH, &event as &dyn RootEvent<Clamped>);
    let mut reported = Vec::new();

    let err = walker
        .advance(1.0, &mut reported, &mut roots)
        .expect_err("the projected state has no finite value");
    assert!(
        matches!(err, IntegrationError::NonFiniteRootValue { .. }),
        "{label}: {err:?}"
    );
    let stopped_at = walker.t();
    assert!(
        reported.last().map(|s| s.0).unwrap_or(T0) == stopped_at,
        "{label}: the walk holds the last state it reported, not the one it refused: \
         at {stopped_at}, reported {:?}",
        reported.iter().map(|s| s.0).collect::<Vec<_>>()
    );
    assert!(
        walker.y() < CEILING,
        "{label}: the refused state is the one on the ceiling, so the walk is below it: {}",
        walker.y()
    );
}

/// The step size a root leaves behind is the one the accepted step earned.
///
/// A root cuts the step short, and the controller still moves `dt` from that
/// step's error — the trials never touch it. The level is crossed inside the
/// *first* step (`y = 0.02` at `t = 0.2`, within the configured `0.25`), so no
/// earlier step has moved `dt` and what is left is the root-bearing step's own
/// doing. Checked on the adaptive steppers; the fixed one has no step size to
/// move.
fn case_a_root_still_moves_the_step_size<W: Walker<Sys = Ramp> + HasStepSize>(
    label: &str,
    mut walker: W,
) {
    let event = AtLevel::rising(0.02);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();

    assert_eq!(
        walker.dt(),
        DT,
        "{label}: the walk starts at the configured dt"
    );
    let outcome = walker
        .advance(T_END, &mut reported, &mut roots)
        .expect("the walk succeeds");
    match outcome {
        RootOutcome::Roots { t, .. } => {
            assert!(
                (t - 0.2).abs() <= SLACK,
                "{label}: the crossing is inside the first step, located {t}"
            );
        }
        RootOutcome::Reached => panic!("{label}: the ramp reaches 0.02 in the first step"),
    }
    assert_ne!(
        walker.dt(),
        DT,
        "{label}: the accepted step's error moved the step size, even though a root \
         cut the step short"
    );
}

/// The step size after a root is the one the same step would have left with no
/// event at all.
///
/// `advance_to_roots` documents that the size is grown or shrunk from the
/// accepted step's error exactly as `advance_to` does. Comparing against a walk
/// that takes the same first step without an event pins the factor, not only
/// that something moved.
fn case_the_step_size_after_a_root_matches_a_walk_without_one<
    W: Walker<Sys = Ramp> + HasStepSize,
>(
    label: &str,
    mut with_root: W,
    mut without: W,
) {
    let event = AtLevel::rising(0.02);
    root_set!(roots, SEARCH, &event as &dyn RootEvent<State<1, 1>>);
    let mut reported = Vec::new();
    assert!(
        matches!(
            with_root
                .advance(T_END, &mut reported, &mut roots)
                .expect("the walk succeeds"),
            RootOutcome::Roots { .. }
        ),
        "{label}: the ramp crosses 0.02 in the first step"
    );

    // One step of the same width, with nothing to find: the target is the end of
    // that step, so the stepper takes it and stops.
    let events: [&dyn RootEvent<State<1, 1>>; 0] = [];
    let mut empty = RootSet::new(&events, &mut [], SEARCH).expect("valid search");
    let mut plain = Vec::new();
    assert_eq!(
        without
            .advance(T0 + DT, &mut plain, &mut empty)
            .expect("the walk succeeds"),
        RootOutcome::Reached
    );

    assert_eq!(
        with_root.dt(),
        without.dt(),
        "{label}: the root-bearing step left the same size as the plain one"
    );
}

/// The adaptive steppers' step size, for the case above.
trait HasStepSize {
    fn dt(&self) -> f64;
}

impl HasStepSize for AdaptiveStepper<'static, Ramp> {
    fn dt(&self) -> f64 {
        AdaptiveStepper::dt(self)
    }
}

impl HasStepSize for AdaptiveStepper853<'static, Ramp> {
    fn dt(&self) -> f64 {
        AdaptiveStepper853::dt(self)
    }
}

macro_rules! step_size_contract_for {
    ($solver:ident, $walker:expr) => {
        mod $solver {
            use super::*;

            #[test]
            fn a_root_still_moves_the_step_size() {
                case_a_root_still_moves_the_step_size(stringify!($solver), $walker);
            }

            #[test]
            fn the_step_size_after_a_root_matches_a_walk_without_one() {
                case_the_step_size_after_a_root_matches_a_walk_without_one(
                    stringify!($solver),
                    $walker,
                    $walker,
                );
            }
        }
    };
}

step_size_contract_for!(dp45_step_size, dp45_walker());
step_size_contract_for!(dop853_step_size, dop853_walker());

macro_rules! contract_for {
    ($solver:ident, $walker:expr) => {
        mod $solver {
            use super::*;

            #[test]
            fn locates_the_analytic_crossing() {
                case_locates_the_analytic_crossing(stringify!($solver), $walker);
            }

            #[test]
            fn a_level_out_of_reach_is_not_a_root() {
                case_a_level_out_of_reach_is_not_a_root(stringify!($solver), $walker);
            }

            #[test]
            fn the_wrong_direction_is_ignored() {
                case_the_wrong_direction_is_ignored(stringify!($solver), $walker);
            }

            #[test]
            fn a_resumed_walk_does_not_refind_the_boundary() {
                case_a_resumed_walk_does_not_refind_the_boundary(stringify!($solver), $walker);
            }

            #[test]
            fn simultaneous_roots_are_one_group() {
                case_simultaneous_roots_are_one_group(stringify!($solver), $walker);
            }

            #[test]
            fn the_earlier_crossing_wins() {
                case_the_earlier_crossing_wins(stringify!($solver), $walker);
            }

            #[test]
            fn a_crossing_on_the_last_step_is_found() {
                case_a_crossing_on_the_last_step_is_found(stringify!($solver), $walker);
            }

            #[test]
            fn a_crossing_on_a_grid_time_is_found() {
                case_a_crossing_on_a_grid_time_is_found(stringify!($solver), $walker);
            }

            #[test]
            fn a_root_in_the_first_step_reports_nothing() {
                case_a_root_in_the_first_step_reports_nothing(stringify!($solver), $walker);
            }

            #[test]
            fn no_events_walks_the_span() {
                case_no_events_walks_the_span(stringify!($solver), $walker);
            }
        }
    };
}

contract_for!(rk4, rk4_walker());
contract_for!(dp45, dp45_walker());
contract_for!(dop853, dop853_walker());

/// The cases that need a state whose projection is observable.
/// A projection that makes the state itself non-finite fails the walk without
/// leaving the located root in `hits`.
///
/// The stepper refuses such a state before it asks the events for their values,
/// so the path that drops the hits inside `check` is not the one taken here.
fn case_a_projection_that_breaks_the_state_leaves_no_hits<W: Walker<Sys = Broken>>(
    label: &str,
    mut walker: W,
) {
    let event = ClampedLevel {
        level: CLAMPED_LEVEL,
    };
    root_set!(roots, SEARCH, &event as &dyn RootEvent<Breaking>);
    let mut reported = Vec::new();

    let err = walker
        .advance(1.0, &mut reported, &mut roots)
        .expect_err("the projection produces a state the walk refuses");
    assert!(
        matches!(err, IntegrationError::NonFiniteState { .. }),
        "{label}: {err:?}"
    );
    assert!(
        roots.hit_count() == 0,
        "{label}: the root the walk gave up on stays out of hits(): {:?}",
        roots.hits().collect::<Vec<_>>()
    );
}

macro_rules! projection_contract_for {
    ($solver:ident, $walker:expr) => {
        mod $solver {
            use super::*;

            #[test]
            fn the_search_does_not_project_its_trials() {
                case_the_search_does_not_project_its_trials(stringify!($solver), $walker);
            }

            #[test]
            fn a_crossing_only_the_raw_candidate_has() {
                case_a_crossing_only_the_raw_candidate_has(stringify!($solver), $walker);
            }

            #[test]
            fn a_projection_that_breaks_the_value_leaves_the_walk_where_it_was() {
                case_a_projection_that_breaks_the_value_leaves_the_walk_where_it_was(
                    stringify!($solver),
                    $walker,
                );
            }
        }
    };
}

/// The ceiling sits below the level every event here looks for, so a projected
/// state never reaches the boundary and only the raw candidate crosses it.
///
/// It also sits off the fixed grid (`0.25`, `0.5`, …), which matters for the
/// case whose value blows up *on* the ceiling: a raw candidate landing exactly
/// there would be non-finite before any projection, and the case would stop
/// testing the projected state.
const CEILING: f64 = 0.4;
const CLAMPED_LEVEL: f64 = 0.6;

static CLAMPED: Unit = Unit { ceiling: CEILING };

fn clamped_start() -> Clamped {
    Clamped {
        y: 0.0,
        ceiling: CEILING,
    }
}

fn rk4_clamped() -> FixedStepper<'static, Rk4, Unit> {
    RK4.stepper(&CLAMPED, clamped_start(), T0, DT)
}

fn dp45_clamped() -> AdaptiveStepper<'static, Unit> {
    DP45.stepper(&CLAMPED, clamped_start(), T0, DT, Tolerances::default())
}

fn dop853_clamped() -> AdaptiveStepper853<'static, Unit> {
    DOP853.stepper(&CLAMPED, clamped_start(), T0, DT, Tolerances::default())
}

macro_rules! broken_contract_for {
    ($solver:ident, $walker:expr) => {
        mod $solver {
            use super::*;

            #[test]
            fn a_projection_that_breaks_the_state_leaves_no_hits() {
                case_a_projection_that_breaks_the_state_leaves_no_hits(
                    stringify!($solver),
                    $walker,
                );
            }
        }
    };
}

macro_rules! trial_contract_for {
    ($solver:ident, $walker:expr) => {
        mod $solver {
            use super::*;

            #[test]
            fn a_refused_trial_leaves_the_walk_where_it_started() {
                case_a_refused_trial_leaves_the_walk_where_it_started(stringify!($solver), $walker);
            }
        }
    };
}

static FORCING: Forcing = Forcing;

fn forced_start() -> Forced {
    Forced { y: 0.0 }
}

fn dp45_forced() -> AdaptiveStepper<'static, Forcing> {
    DP45.stepper(&FORCING, forced_start(), T0, DT, Tolerances::default())
}

fn dop853_forced() -> AdaptiveStepper853<'static, Forcing> {
    DOP853.stepper(&FORCING, forced_start(), T0, DT, Tolerances::default())
}

impl HasStepSize for AdaptiveStepper<'static, Forcing> {
    fn dt(&self) -> f64 {
        AdaptiveStepper::dt(self)
    }
}

impl HasStepSize for AdaptiveStepper853<'static, Forcing> {
    fn dt(&self) -> f64 {
        AdaptiveStepper853::dt(self)
    }
}

trial_contract_for!(dp45_refused_trial, dp45_forced());
trial_contract_for!(dop853_refused_trial, dop853_forced());

static BROKEN: Broken = Broken;

fn broken_start() -> Breaking {
    Breaking { y: 0.0 }
}

fn rk4_broken() -> FixedStepper<'static, Rk4, Broken> {
    RK4.stepper(&BROKEN, broken_start(), T0, DT)
}

fn dp45_broken() -> AdaptiveStepper<'static, Broken> {
    DP45.stepper(&BROKEN, broken_start(), T0, DT, Tolerances::default())
}

fn dop853_broken() -> AdaptiveStepper853<'static, Broken> {
    DOP853.stepper(&BROKEN, broken_start(), T0, DT, Tolerances::default())
}

broken_contract_for!(rk4_broken_projection, rk4_broken());
broken_contract_for!(dp45_broken_projection, dp45_broken());
broken_contract_for!(dop853_broken_projection, dop853_broken());

projection_contract_for!(rk4_projected, rk4_clamped());
projection_contract_for!(dp45_projected, dp45_clamped());
projection_contract_for!(dop853_projected, dop853_clamped());

/// What the search converges on when the caller lets the right-hand side switch
/// mode at the boundary, which the contract forbids.
///
/// `y' = 1` below `y = 1` and `y' = 0` at or above it reaches the boundary at
/// `t = 1` and stays. RK4's fourth stage evaluates at `y0 + w k3`, so a trial of
/// exactly the remaining distance `r = 1 - y0` puts that stage on the boundary,
/// its rate is 0, and the step advances only `5w/6`. Bisection therefore
/// converges on `w = 6r/5` and reports the arrival `0.2 r` late.
///
/// The test is here to pin the number the contract is argued from: a search
/// that started re-stepping with the post-root mode would land here even for a
/// caller that does everything right.
#[test]
fn a_mode_that_switches_at_the_boundary_is_located_late() {
    struct Switching;

    impl DynamicalSystem for Switching {
        type State = State<1, 1>;
        fn derivatives(&self, _t: f64, state: &Self::State) -> Self::State {
            let rate = if state.components[0][0] < 1.0 {
                1.0
            } else {
                0.0
            };
            State {
                components: [Vector1::new(rate)],
            }
        }
    }

    struct Reaches;
    impl RootEvent<State<1, 1>> for Reaches {
        fn value(&self, _t: f64, y: &State<1, 1>) -> f64 {
            y.components[0][0] - 1.0
        }
        fn crossing(&self) -> Crossing {
            Crossing::Rising
        }
    }

    static SWITCHING: Switching = Switching;

    // `y = t` before the boundary, so the step that carries the crossing starts
    // at `y0 = t0`, and the distance left to the boundary is `1 - t0`.
    for (dt, remaining) in [(0.05, 0.05), (0.2, 0.2), (0.3, 0.1), (1.0, 1.0)] {
        let mut stepper = RK4.stepper(
            &SWITCHING,
            State {
                components: [Vector1::new(0.0)],
            },
            0.0,
            dt,
        );
        let event = Reaches;
        root_set!(
            roots,
            RootSearch {
                t_tolerance: 1e-12,
                max_iterations: 200,
            },
            &event as &dyn RootEvent<State<1, 1>>
        );

        match stepper
            .advance_to_roots(2.0, |_, _| {}, &mut roots)
            .expect("the walk succeeds")
        {
            RootOutcome::Roots { t, .. } => {
                let expected = 1.0 + 0.2 * remaining;
                assert!(
                    (t - expected).abs() <= 1e-9,
                    "dt = {dt}: located {t}, expected {expected} (= 1 + 0.2 × {remaining})"
                );
            }
            RootOutcome::Reached => panic!("dt = {dt}: y reaches 1 inside the span"),
        }
    }
}
