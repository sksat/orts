//! The times a fixed-step integration steps through.
//!
//! Walking a span by adding `dt` to a running clock drifts: nine steps of
//! `0.1` from zero reach `0.8999999999999999`, so the remainder is
//! `0.10000000000000009` — a hair over `dt`. The walk then takes one more full
//! step and leaves an ulp-wide tail to cover in an eleventh, and the clock the
//! callback sees never equals the span's end.
//!
//! Counting from the span's start instead — `t0 + n * dt`, with the last step
//! assigned `t_end` — keeps every step time within an ulp of the exact grid,
//! makes the number of steps the one the caller asked for, and lands the last
//! callback on `t_end` itself. A caller that ends a step at a switch of the
//! right-hand side needs that last part: the state there is the state at the
//! switch, and the clock has to say so.

use crate::error::{IntegrationError, validate_step_size, validate_time_span};

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
    taken: u64,
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
    /// positive and finite, a non-finite end of the span, or an end before the
    /// start. Each of those would otherwise yield forever — a `dt` of zero
    /// never leaves `t0`, a negative one walks backwards, and every comparison
    /// against a NaN end is false.
    ///
    /// An empty span (`t0 == t_end`) is valid and yields nothing.
    pub fn new(t0: f64, t_end: f64, dt: f64) -> Result<Self, IntegrationError> {
        validate_step_size(dt)?;
        validate_time_span(t0, t_end)?;
        Ok(Self {
            t0,
            t_end,
            dt,
            taken: 0,
        })
    }
}

impl Iterator for FixedSteps {
    type Item = Step;

    fn next(&mut self) -> Option<Step> {
        let t = if self.taken == 0 {
            self.t0
        } else {
            self.t0 + self.taken as f64 * self.dt
        };
        if t >= self.t_end {
            return None;
        }
        // The grid's next time, or the span's end when the grid would pass it.
        // Assigning the end rather than landing near it is what lets a caller
        // treat the last state as the state at `t_end`.
        let grid_next = self.t0 + (self.taken + 1) as f64 * self.dt;
        let next_t = if grid_next >= self.t_end {
            self.t_end
        } else {
            grid_next
        };
        self.taken += 1;
        Some(Step {
            t,
            h: next_t - t,
            next_t,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::FixedSteps;

    fn walk(t0: f64, t_end: f64, dt: f64) -> Vec<(f64, f64)> {
        FixedSteps::new(t0, t_end, dt)
            .expect("the span and step are valid")
            .map(|step| (step.h, step.next_t))
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
            .expect("the span holds twenty steps");
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

    /// A span that starts away from zero keeps its own anchor.
    #[test]
    fn a_span_away_from_zero_counts_from_its_own_start() {
        let steps = walk(5.0, 5.3, 0.1);
        assert_eq!(steps.len(), 3, "steps: {steps:?}");
        assert_eq!(steps.last().expect("three steps").1, 5.3);
    }
}
