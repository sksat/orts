//! Fixed-step integration: the times it steps through, and the stepper that
//! takes them.
//!
//! Walking a span by adding `dt` to a running clock drifts, and the drift
//! grows with the walk: nine steps of `0.1` from zero reach
//! `0.8999999999999999`, so the remainder is `0.10000000000000009` — a hair
//! over `dt` — and `[0, 1]` takes eleven steps, the last of them an ulp wide.
//! Over a thousand steps the sum is several ulps from the exact grid.
//!
//! Counting from the span's start instead — `t0 + n * dt`, with the last step
//! assigned `t_end` — keeps each step time within an ulp of the exact grid
//! however long the walk, spends no step on a drift-sized remainder, and lands
//! the last one on `t_end` by assignment rather than by arithmetic. A caller
//! that ends a step at a switch of the right-hand side needs the state at the
//! switch, and its grid times to be the ones it asked for.
//!
//! The accumulated walk did reach `t_end` in the example above: its eleventh
//! step is `t_end - t` wide, which is exact, so the callback there was already
//! on `1.0`. What changes is the extra step and the ten grid times before it.
//!
//! [`FixedStepper`] is what a caller drives: it holds the state and the time it
//! belongs to and takes one target after another, so a propagation loop that
//! stops where it likes never handles the grid itself. Repeated targets anchor
//! a new grid at each call's own start — splitting a span decides where the
//! solver lands, and does not reproduce an unsplit trajectory in pieces.

use core::ops::ControlFlow;

use crate::error::{IntegrationError, validate_step_size, validate_time_span};
#[cfg(not(feature = "std"))]
use crate::math::F64Ext;
use crate::{AdvanceOutcome, DynamicalSystem, Integrator, OdeState};

/// The step times of a fixed-step walk from `t0` to `t_end`.
///
/// Yields each step's start, width and end, counting from `t0` so the grid does
/// not drift, and lands the last step on `t_end`. Driven by
/// [`FixedStepper::advance_to`], which is how a caller reaches this grid: the
/// times are the stepper's own machinery, and every caller that used to walk
/// them itself wanted the steps taken, not the times.
pub(crate) struct FixedSteps {
    t0: f64,
    t_end: f64,
    dt: f64,
    /// Grid index of the time the next step starts from: the walk is at
    /// `t0 + index * dt`.
    index: u64,
    /// Whether the walk has already reported that it cannot advance. One
    /// report ends it, rather than repeating forever.
    stagnated: bool,
}

/// One step of a [`FixedSteps`] walk: where it starts, how wide it is, and
/// where it lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Step {
    /// Time the step starts at.
    pub(crate) t: f64,
    /// Width of the step.
    pub(crate) h: f64,
    /// Time the step lands on. The last step of a walk lands on the span's
    /// end exactly rather than on `t + h`.
    pub(crate) next_t: f64,
}

impl FixedSteps {
    /// Walk `[t0, t_end]` in steps of `dt`.
    ///
    /// Rejects what cannot produce a terminating walk: a step size that is not
    /// positive and finite, a non-finite start or end of the span, or an end
    /// before the start. Each of those would otherwise yield forever — a `dt`
    /// of zero never leaves `t0`, a negative one walks backwards, and every
    /// comparison against a NaN is false.
    ///
    /// An empty span (`t0 == t_end`) is valid and yields nothing.
    ///
    /// A `dt` that passes here can still fail to move the clock further into
    /// the span, where the spacing of f64 is wider than `dt` itself: the walk
    /// reports [`IntegrationError::TimeStagnated`] at the step it happens on,
    /// not before. A caller whose event fires earlier never reaches it — an
    /// event at `t = 1` in a span ending at `1e16` is the case that rules out
    /// judging the whole span up front.
    pub(crate) fn new(t0: f64, t_end: f64, dt: f64) -> Result<Self, IntegrationError> {
        validate_step_size(dt)?;
        validate_time_span(t0, t_end)?;
        Ok(Self {
            t0,
            t_end,
            dt,
            index: 0,
            stagnated: false,
        })
    }

    /// The grid time at `index`, counted from the span's start.
    ///
    /// Fused, so `t0 + index * dt` is rounded once: rounding the product first
    /// moves the grid by an extra ulp, and a product that overflows would put
    /// the walk at infinity while the sum it belongs to is finite.
    fn at(&self, index: u64) -> f64 {
        if index == 0 {
            self.t0
        } else {
            (index as f64).mul_add(self.dt, self.t0)
        }
    }

    /// Where a step ending at `index` lands: the grid time, or the span's end
    /// once the grid would pass it. Assigning the end rather than landing near
    /// it is what lets a caller treat the last state as the state at `t_end`.
    fn landing(&self, index: u64) -> f64 {
        let grid = self.at(index);
        if grid >= self.t_end { self.t_end } else { grid }
    }
}

impl Iterator for FixedSteps {
    type Item = Result<Step, IntegrationError>;

    fn next(&mut self) -> Option<Result<Step, IntegrationError>> {
        if self.stagnated {
            return None;
        }
        let t = self.at(self.index);
        if t >= self.t_end {
            return None;
        }

        // Where the spacing of f64 at `t` is wider than `dt`, no grid can carry
        // the step that was asked for: the clock moves by the spacing instead,
        // and rounding each grid time would hand the solver a step wider than
        // `dt` — the contract says only the last step of a span is different,
        // and shorter. Reported here rather than in `new`, so a caller whose
        // event fires in the walkable part of the span still finishes: the grid
        // at `1e16` cannot carry a step of `1`, but a span from zero to there
        // can still end at `t = 1`.
        let spacing = t.next_up() - t;
        if self.dt < spacing {
            self.stagnated = true;
            // Below half the spacing the clock does not move at all, which is
            // the older and more specific failure.
            return Some(Err(if t + self.dt == t {
                IntegrationError::TimeStagnated { t, dt: self.dt }
            } else {
                IntegrationError::StepBelowSpacing {
                    t,
                    dt: self.dt,
                    spacing,
                }
            }));
        }

        let index = self.index + 1;
        let next_t = self.landing(index);
        // Above 2^53 the index itself stops being exact in f64, so the grid
        // time for `index + 1` can equal the one for `index`, and the spacing
        // check does not see it — the case is a walk from `-2^53` in steps of
        // `1`, where the collapse happens as it crosses zero and the spacing
        // there is a subnormal. No test drives it: reaching that index takes
        // 2^53 steps. The guard is here so that what a caller receives is
        // never a step of no width.
        if next_t <= t {
            self.stagnated = true;
            return Some(Err(IntegrationError::TimeStagnated { t, dt: self.dt }));
        }

        self.index = index;
        Some(Ok(Step {
            t,
            h: next_t - t,
            next_t,
        }))
    }
}

/// Advances a fixed-step solver towards successive target times.
///
/// Created via [`Integrator::stepper`]. Holds the state and the time it belongs
/// to, so a caller that splits its span — at output intervals, or at the
/// discontinuities a [`Segments`](crate::Segments) walk finds — asks for one
/// target after another and reads the state back between them. The adaptive
/// solvers offer the same three calls
/// ([`stepper`](crate::DormandPrince::stepper),
/// [`from_checked_state`](Self::from_checked_state),
/// [`advance_to`](Self::advance_to)), so a loop that chooses its solver at run
/// time writes one shape for all of them.
///
/// Each `advance_to` counts its steps from the time the stepper is at when it
/// is called, and lands its last step on the target. Two calls covering
/// `[0, 1]` and `[1, 2]` therefore step through different times than one call
/// covering `[0, 2]`: splitting a span is a choice about where the solver is
/// made to land, not a way to reproduce the same trajectory in pieces.
pub struct FixedStepper<'a, I: ?Sized, S: DynamicalSystem> {
    integrator: &'a I,
    system: &'a S,
    state: S::State,
    t: f64,
    dt: f64,
    /// Whether the state this stepper starts from has already been checked for
    /// events. Set through [`from_checked_state`](Self::from_checked_state).
    start_is_checked: bool,
}

impl<'a, I: Integrator + ?Sized, S: DynamicalSystem> FixedStepper<'a, I, S> {
    pub(crate) fn new(
        integrator: &'a I,
        system: &'a S,
        initial: S::State,
        t0: f64,
        dt: f64,
    ) -> Self {
        Self {
            integrator,
            system,
            state: initial,
            t: t0,
            dt,
            start_is_checked: false,
        }
    }

    /// Skip the event check on the state this stepper starts from.
    ///
    /// For a segment that continues where the previous one ended: the caller
    /// has asked its predicate about that state already. Checks after accepted
    /// steps are unaffected.
    pub fn from_checked_state(mut self) -> Self {
        self.start_is_checked = true;
        self
    }

    /// Advance to `t_target` in steps of the configured `dt`.
    ///
    /// - For a target that advances, `event_check` runs first on the state the
    ///   stepper currently holds, before any step. A level-triggered event can
    ///   already hold there, and reporting it after a step would name a state
    ///   the predicate itself rejects. Terminating there leaves the stepper
    ///   where it was and calls no callback, since the callback reports
    ///   accepted steps and none was taken. A target the stepper has already
    ///   reached takes no step, so it asks nothing and reports `Reached`.
    /// - Each step calls `callback(t, &state)`, then `event_check(t, &state)`;
    ///   `Break(reason)` reports `Event { reason }`.
    /// - A step whose result is not finite fails with
    ///   [`IntegrationError::NonFiniteState`] without storing that result: the
    ///   state after such a step is not a trajectory, and the stepper stays on
    ///   the last state it accepted.
    pub fn advance_to<F, E, B>(
        &mut self,
        t_target: f64,
        mut callback: F,
        event_check: E,
    ) -> Result<AdvanceOutcome<B>, IntegrationError>
    where
        F: FnMut(f64, &S::State),
        E: Fn(f64, &S::State) -> ControlFlow<B>,
    {
        // Before the walk, so an invalid `dt` or span is reported the same way
        // whether or not the walk would have reached the step that shows it.
        validate_step_size(self.dt)?;
        validate_time_span(self.t, t_target)?;

        // The predicate is asked about the state the stepper holds, before any
        // step. A level-triggered event — "below the surface", "past this
        // altitude" — can already hold here, and stepping first reports it one
        // step late with a state the caller's own predicate calls invalid.
        // Only for a target that advances: `advance_to(self.t, ..)` takes no
        // step, so it reports nothing and asks nothing.
        if !self.start_is_checked
            && self.t < t_target
            && let ControlFlow::Break(reason) = event_check(self.t, &self.state)
        {
            return Ok(AdvanceOutcome::Event { reason });
        }

        for step in FixedSteps::new(self.t, t_target, self.dt)? {
            let step = step?;
            let candidate = self
                .integrator
                .step(self.system, step.t, &self.state, step.h);

            if !candidate.is_finite() {
                return Err(IntegrationError::NonFiniteState { t: step.next_t });
            }

            self.state = candidate;
            self.t = step.next_t;

            callback(self.t, &self.state);

            if let ControlFlow::Break(reason) = event_check(self.t, &self.state) {
                return Ok(AdvanceOutcome::Event { reason });
            }
        }

        Ok(AdvanceOutcome::Reached)
    }

    /// The state of the last accepted step.
    pub fn state(&self) -> &S::State {
        &self.state
    }

    /// The time the stepper has reached.
    pub fn t(&self) -> f64 {
        self.t
    }

    /// Take the state of the last accepted step.
    pub fn into_state(self) -> S::State {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::FixedSteps;
    use crate::IntegrationError;

    fn walk(t0: f64, t_end: f64, dt: f64) -> Vec<(f64, f64)> {
        FixedSteps::new(t0, t_end, dt)
            .expect("the span and step are valid")
            .map(|step| {
                let step = step.expect("the walk advances");
                (step.h, step.next_t)
            })
            .collect()
    }

    #[test]
    fn a_span_a_step_divides_lands_on_its_end() {
        assert_eq!(walk(0.0, 0.5, 0.5), vec![(0.5, 0.5)]);
    }

    /// The case accumulation gets wrong: ten steps, not eleven, and the last
    /// one lands on the end.
    #[test]
    fn a_span_of_ten_tenths_takes_ten_steps_and_lands_on_its_end() {
        let steps = walk(0.0, 1.0, 0.1);
        assert_eq!(steps.len(), 10, "steps: {steps:?}");
        assert_eq!(steps.last().expect("ten steps").1, 1.0);

        let mut accumulated = 0.0;
        for _ in 0..10 {
            accumulated += 0.1;
        }
        assert_ne!(
            accumulated, 1.0,
            "precondition: accumulating ten tenths misses 1.0"
        );
    }

    /// A span the step does not divide: the last step carries the remainder.
    #[test]
    fn the_last_step_carries_what_is_left_of_the_span() {
        let steps = walk(0.0, 1.2, 0.5);
        assert_eq!(steps.len(), 3, "steps: {steps:?}");
        assert_eq!(steps[0], (0.5, 0.5));
        assert_eq!(steps[1], (0.5, 1.0));
        let (h, next) = steps[2];
        assert_eq!(next, 1.2);
        assert!((h - 0.2).abs() < 1e-15, "the remainder is {h}");
    }

    /// Every step time comes from the span's start, so the grid does not
    /// drift: the ninth step of a tenth starts within an ulp of 0.9, where an
    /// accumulated clock sits at 0.8999999999999999.
    #[test]
    fn the_grid_is_counted_from_the_span_start() {
        let ninth = FixedSteps::new(0.0, 2.0, 0.1)
            .expect("the span and step are valid")
            .nth(9)
            .expect("the span holds twenty steps")
            .expect("the walk advances");
        assert_eq!(ninth.t, 0.9);

        let mut accumulated = 0.0;
        for _ in 0..9 {
            accumulated += 0.1;
        }
        assert_ne!(accumulated, 0.9, "precondition: the sum drifts");
    }

    #[test]
    fn an_empty_span_takes_no_step() {
        assert!(walk(1.0, 1.0, 0.5).is_empty());
    }

    /// A span narrower than a step is one step wide.
    #[test]
    fn a_span_narrower_than_a_step_takes_one() {
        assert_eq!(walk(0.0, 0.1, 1.0), vec![(0.1, 0.1)]);
    }

    /// What would otherwise yield forever: a step that does not advance, one
    /// that walks backwards, and an end no comparison can order.
    #[test]
    fn a_walk_that_cannot_terminate_is_refused() {
        for (t0, t_end, dt) in [
            (0.0, 1.0, 0.0),
            (0.0, 1.0, -0.1),
            (0.0, 1.0, f64::NAN),
            (0.0, 1.0, f64::INFINITY),
            (0.0, f64::NAN, 0.1),
            (0.0, f64::INFINITY, 0.1),
            (f64::NAN, 1.0, 0.1),
            (1.0, 0.0, 0.1),
        ] {
            assert!(
                FixedSteps::new(t0, t_end, dt).is_err(),
                "[{t0}, {t_end}] in steps of {dt} was accepted"
            );
        }
    }

    /// A `dt` narrower than the spacing of f64 at a step's start is refused
    /// there, before a step wider than `dt` is handed out: at `1e15` the
    /// spacing is `0.125`, so the grid would move by that rather than by the
    /// `0.1` asked for, and the contract says only a span's last step differs
    /// — by being shorter.
    #[test]
    fn a_step_below_the_spacing_is_refused_where_it_starts() {
        assert_eq!(1e15f64.next_up() - 1e15, 0.125, "precondition: the spacing");
        assert!(
            1e15 + 0.1 > 1e15,
            "precondition: the clock does advance, it just advances too far"
        );

        let mut walk = FixedSteps::new(1e15, 1e15 + 1.0, 0.1).expect("the span and step are valid");
        assert!(
            matches!(
                walk.next(),
                Some(Err(IntegrationError::StepBelowSpacing { .. }))
            ),
            "a step of 0.1 cannot be carried at 1e15"
        );
        assert!(walk.next().is_none(), "the walk ends after saying so");

        // The spacing itself is a step the clock can take, and lands on the
        // span's end.
        let steps: Vec<_> = FixedSteps::new(1e15, 1e15 + 1.0, 0.125)
            .expect("the span and step are valid")
            .map(|step| step.expect("the walk advances"))
            .collect();
        assert_eq!(steps.len(), 8, "eight steps of the spacing cover 1.0");
        assert_eq!(steps.last().expect("eight steps").next_t, 1e15 + 1.0);
    }

    /// Below half the spacing the clock does not move at all, which is the
    /// older and more specific failure.
    #[test]
    fn a_step_the_clock_cannot_feel_stagnates() {
        let coarse = 9007199254740992.0_f64; // 2^53, spacing 2
        assert_eq!(
            coarse + 0.5,
            coarse,
            "precondition: the clock cannot feel it"
        );
        let mut walk =
            FixedSteps::new(coarse, coarse + 10.0, 0.5).expect("the span and step are valid");
        assert!(matches!(
            walk.next(),
            Some(Err(IntegrationError::TimeStagnated { .. }))
        ));
    }

    /// No step of a span is wider than the `dt` asked for, beyond the rounding
    /// of the two grid times it spans: `h` is their difference, and each is
    /// within half an ulp of the exact grid, so `h` can exceed `dt` by an ulp
    /// of its own magnitude and no more. Measured — `[0, 1]` in steps of `0.1`
    /// has a step of `0.10000000000000003` starting at `0.2`. What the spacing
    /// check rules out is `h` exceeding `dt` by the whole spacing, which is
    /// what rounding a grid at coarse times would do.
    #[test]
    fn no_step_is_wider_than_the_one_asked_for() {
        for (t0, t_end, dt) in [
            (0.0, 1.0, 0.1),
            (0.0, 1.2, 0.5),
            (5.0, 5.3, 0.1),
            (-1.0, 1.0, 0.3),
            (1e9, 1e9 + 1.0, 0.1),
        ] {
            for step in FixedSteps::new(t0, t_end, dt).expect("the span and step are valid") {
                let step = step.expect("the walk advances");
                let rounding = step.next_t.next_up() - step.next_t;
                assert!(
                    step.h <= dt + rounding,
                    "[{t0}, {t_end}] in steps of {dt}: a step of {} starting at {}, \
                     over the {rounding:e} an ulp there allows",
                    step.h,
                    step.t
                );
            }
        }
    }

    /// A span whose far end is too coarse for `dt` is still walked as far as
    /// the grid goes — a caller that ends before the coarse part never sees the
    /// stagnation. Judging the whole span up front would refuse this walk on
    /// its first step.
    #[test]
    fn a_span_reaching_into_coarse_times_walks_its_fine_part() {
        let coarse = 1e16_f64;
        assert_eq!(coarse.next_up() - coarse, 2.0, "precondition: the spacing");

        let mut walk = FixedSteps::new(0.0, coarse, 1.0).expect("the span and step are valid");
        let first = walk
            .next()
            .expect("the span holds a step")
            .expect("a step of 1 from zero is representable");
        assert_eq!((first.t, first.next_t), (0.0, 1.0));
    }

    /// A span that starts away from zero keeps its own anchor.
    #[test]
    fn a_span_away_from_zero_counts_from_its_own_start() {
        let steps = walk(5.0, 5.3, 0.1);
        assert_eq!(steps.len(), 3, "steps: {steps:?}");
        assert_eq!(steps.last().expect("three steps").1, 5.3);
    }
}
