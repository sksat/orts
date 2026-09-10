//! The times a fixed-step integration steps through.
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

use crate::error::{IntegrationError, validate_step_size, validate_time_span};
#[cfg(not(feature = "std"))]
use crate::math::F64Ext;

/// The step times of a fixed-step walk from `t0` to `t_end`.
///
/// For a loop that steps a system itself rather than through
/// [`Integrator::integrate`](crate::Integrator::integrate) — a propagation
/// loop that checks events per step, or one that splits its span at known
/// discontinuities. Yields each step's start, width and end, counting from
/// `t0` so the grid does not drift, and lands the last step on `t_end`.
pub struct FixedSteps {
    t0: f64,
    t_end: f64,
    dt: f64,
    /// Grid index of the time the next step starts from: the walk is at
    /// `t0 + index * dt`.
    index: u64,
}

/// One step of a [`FixedSteps`] walk: where it starts, how wide it is, and
/// where it lands.
pub struct Step {
    /// Time the step starts at.
    pub t: f64,
    /// Width of the step.
    pub h: f64,
    /// Time the step lands on. The last step of a walk lands on the span's
    /// end exactly rather than on `t + h`.
    pub next_t: f64,
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
    /// Also rejects a `dt` narrower than the spacing of f64 at the times a
    /// step could start from, as [`IntegrationError::TimeStagnated`]. Such a
    /// step cannot be walked on any grid: consecutive grid times round onto the
    /// same double, and a solver handed the difference would step nowhere.
    /// Accumulating a clock hid this rather than solving it — adding `0.1` at
    /// `1e15` moves the clock by the spacing, `0.125`, while the solver is told
    /// `0.1`, so the state it returns belongs to a time the clock has already
    /// passed.
    ///
    /// The span's end is only a landing, never the start of a step, so the
    /// spacing there does not decide: `[2^53 - 1, 2^53]` is one step of `1`
    /// even though the spacing at `2^53` is `2`. An empty span starts no step
    /// at all and is accepted whatever `dt` is.
    pub fn new(t0: f64, t_end: f64, dt: f64) -> Result<Self, IntegrationError> {
        validate_step_size(dt)?;
        validate_time_span(t0, t_end)?;
        let walk = Self {
            t0,
            t_end,
            dt,
            index: 0,
        };
        if t0 == t_end {
            return Ok(walk);
        }
        // The coarsest time a step can start from: the span's own start, or the
        // last time before its end.
        let last_start = t_end.next_down();
        let coarsest = if t0.abs() >= last_start.abs() {
            t0
        } else {
            last_start
        };
        let spacing = coarsest.next_up() - coarsest;
        if dt < spacing {
            return Err(IntegrationError::TimeStagnated { t: coarsest, dt });
        }
        Ok(walk)
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
        let t = self.at(self.index);
        if t >= self.t_end {
            return None;
        }

        let index = self.index + 1;
        let next_t = self.landing(index);
        // `new` refused a `dt` below the spacing at this span's times, which is
        // what would collapse two grid indices onto the same double.
        debug_assert!(next_t > t, "the grid did not advance from {t}");

        self.index = index;
        Some(Ok(Step {
            t,
            h: next_t - t,
            next_t,
        }))
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

    /// A `dt` narrower than the spacing of f64 at the span's times is refused
    /// before the walk starts, rather than partway through it: at `1e15` the
    /// spacing is `0.125`, so a `dt` of `0.1` would collapse every fourth grid
    /// index — but only from the third step on, and a walk that fails there has
    /// already handed out steps.
    #[test]
    fn a_step_below_the_spacing_is_refused_up_front() {
        assert_eq!(1e15f64.next_up() - 1e15, 0.125, "precondition: the spacing");
        assert!(
            matches!(
                FixedSteps::new(1e15, 1e15 + 1.0, 0.1),
                Err(IntegrationError::TimeStagnated { .. })
            ),
            "a step of 0.1 cannot be walked at 1e15"
        );
        // The spacing itself is walkable, and lands on the span's end.
        let steps: Vec<_> = FixedSteps::new(1e15, 1e15 + 1.0, 0.125)
            .expect("the spacing is a walkable step")
            .map(|step| step.expect("the walk advances"))
            .collect();
        assert_eq!(steps.len(), 8, "eight steps of the spacing cover 1.0");
        assert_eq!(steps.last().expect("eight steps").next_t, 1e15 + 1.0);
    }

    /// The span's coarsest end decides: a walk that starts where the spacing is
    /// fine but ends where it is not is refused too.
    #[test]
    fn the_coarsest_end_of_the_span_decides() {
        let coarse = 1e16_f64;
        assert_eq!(coarse.next_up() - coarse, 2.0, "precondition: the spacing");
        assert!(
            matches!(
                FixedSteps::new(coarse - 10.0, coarse + 10.0, 0.5),
                Err(IntegrationError::TimeStagnated { .. })
            ),
            "a step of 0.5 cannot be walked through 1e16"
        );
    }

    /// The span's end is a landing, not the start of a step, so the spacing
    /// there does not decide.
    #[test]
    fn a_span_ending_on_a_binade_boundary_is_walkable() {
        let boundary = 9007199254740992.0_f64; // 2^53
        assert_eq!(
            boundary.next_up() - boundary,
            2.0,
            "precondition: the spacing above 2^53"
        );
        assert_eq!(
            walk(boundary - 1.0, boundary, 1.0),
            vec![(1.0, boundary)],
            "one step of 1 covers it"
        );
    }

    /// An empty span starts no step, so no step size can stagnate on it.
    #[test]
    fn an_empty_span_accepts_any_step() {
        assert!(FixedSteps::new(1.0, 1.0, 1e-20).is_ok());
        assert!(walk(1.0, 1.0, 1e-20).is_empty());
    }

    /// A span that starts away from zero keeps its own anchor.
    #[test]
    fn a_span_away_from_zero_counts_from_its_own_start() {
        let steps = walk(5.0, 5.3, 0.1);
        assert_eq!(steps.len(), 3, "steps: {steps:?}");
        assert_eq!(steps.last().expect("three steps").1, 5.3);
    }
}
