//! Integrating between two known discontinuities.
//!
//! [`DynamicalSystem::next_discontinuity_after`] lets a system say when its
//! right-hand side switches, so a propagation loop can end a step there. That
//! alone leaves the switch time itself ambiguous: a solver's last stage lands
//! on the step's end, and a term that is on over `[a, b)` reads off there. RK4
//! weights that stage `1/6`, so a burn covering a whole step integrates to
//! `5/6` of its value — measured on `[0, 1)` with `dt = 1`.
//!
//! A segment fixes which side of its ends the right-hand side is read on:
//! every stage of a segment `[a, b]`, the one at `b` included, reads the mode
//! that holds over `[a, b)`, and the next segment reads the mode that starts
//! at `b`. [`SegmentSystem`] carries that choice by wrapping the system a
//! solver steps, so no solver needs to know about segments at all.

use crate::state::DynamicalSystem;

/// The interval a solver is stepping through, between two switches of the
/// right-hand side.
///
/// `start` and `end` are integration times, `start < end`. A system that reads
/// a schedule answers for the whole segment as it stood at `start`, whatever
/// stage time it is handed.
///
/// The state at `start` is deliberately absent. Freezing a state-dependent
/// term over a segment would turn a continuous-time feedback law into a
/// sampled-data one, and make the result depend on how the span happens to be
/// split. A schedule in time is what a segment fixes; a term that reads the
/// state keeps reading the stage state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentContext {
    /// Integration time the segment starts at.
    pub start: f64,
    /// Integration time the segment ends at.
    pub end: f64,
}

impl SegmentContext {
    /// A segment spanning `[start, end]`.
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }
}

/// A system bound to one segment, for handing to a solver.
///
/// [`derivatives`](DynamicalSystem::derivatives) forwards to
/// [`derivatives_in_segment`](DynamicalSystem::derivatives_in_segment) with the
/// bound segment, so every stage the solver evaluates — including the one at
/// the segment's end — reads the segment's mode. Build one per segment and drop
/// it at the end: an adaptive stepper built on it holds its FSAL derivative for
/// the segment's lifetime only, which is what keeps a derivative from one side
/// of a switch out of the step on the other side.
pub struct SegmentSystem<'a, S> {
    system: &'a S,
    segment: SegmentContext,
}

impl<'a, S: DynamicalSystem> SegmentSystem<'a, S> {
    /// Bind `system` to `segment`.
    pub fn new(system: &'a S, segment: SegmentContext) -> Self {
        Self { system, segment }
    }

    /// The segment this system is bound to.
    pub fn segment(&self) -> &SegmentContext {
        &self.segment
    }
}

impl<S: DynamicalSystem> DynamicalSystem for SegmentSystem<'_, S> {
    type State = S::State;

    fn derivatives(&self, t: f64, state: &Self::State) -> Self::State {
        self.system.derivatives_in_segment(&self.segment, t, state)
    }

    fn next_discontinuity_after(&self, t: f64) -> Option<f64> {
        self.system.next_discontinuity_after(t)
    }
}

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;

    use nalgebra::Vector1;

    use super::*;
    use crate::{Dop853, DormandPrince, Integrator, Rk4, State, Tolerances};

    fn rate(value: f64) -> State<1, 1> {
        State {
            components: [Vector1::new(value)],
        }
    }

    /// `y' = 1` while `t` is in `[0.1, 0.2)`, `y' = 0` elsewhere.
    ///
    /// The analytic answer over `[0, 1]` is `0.1`, and nothing else in the
    /// system moves, so what a solver returns is the length of the gate it
    /// actually sampled. A stage on the gate's end reading off costs that
    /// stage's weight; missing the gate entirely returns `0`.
    struct Gate {
        start: f64,
        end: f64,
    }

    impl Gate {
        fn open_at(&self, t: f64) -> f64 {
            if t >= self.start && t < self.end {
                1.0
            } else {
                0.0
            }
        }
    }

    impl DynamicalSystem for Gate {
        type State = State<1, 1>;

        fn derivatives(&self, t: f64, _state: &Self::State) -> Self::State {
            rate(self.open_at(t))
        }

        fn derivatives_in_segment(
            &self,
            segment: &SegmentContext,
            _t: f64,
            _state: &Self::State,
        ) -> Self::State {
            rate(self.open_at(segment.start))
        }

        fn next_discontinuity_after(&self, t: f64) -> Option<f64> {
            [self.start, self.end]
                .into_iter()
                .filter(|edge| *edge > t)
                .min_by(f64::total_cmp)
        }
    }

    const GATE: Gate = Gate {
        start: 0.1,
        end: 0.2,
    };
    const T_END: f64 = 1.0;

    /// Walk the segments the system declares, integrating each with its own
    /// stepper, the way a propagation loop is meant to.
    fn integrate_by_segment(
        step: impl Fn(&SegmentSystem<'_, Gate>, f64, f64, State<1, 1>) -> State<1, 1>,
    ) -> f64 {
        let mut t = 0.0;
        let mut y = rate(0.0);
        while t < T_END {
            let end = GATE
                .next_discontinuity_after(t)
                .map_or(T_END, |next| next.min(T_END));
            let bound = SegmentSystem::new(&GATE, SegmentContext::new(t, end));
            y = step(&bound, t, end, y);
            t = end;
        }
        y.components[0][0]
    }

    #[test]
    fn a_gate_shorter_than_a_step_is_integrated_once_the_span_is_split_at_its_edges() {
        // One RK4 step over the whole span samples t = 0, 0.5, 0.5, 1 and
        // never opens the gate.
        let missed = Rk4.integrate(&GATE, rate(0.0), 0.0, T_END, T_END, |_, _| {});
        assert_eq!(missed.components[0][0], 0.0);

        let integrated = integrate_by_segment(|bound, t, end, y| {
            Rk4.integrate(bound, y, t, end, end - t, |_, _| {})
        });
        assert!(
            (integrated - 0.1).abs() < 1e-15,
            "expected the gate's length 0.1, got {integrated}"
        );
    }

    #[test]
    fn the_stage_on_a_segment_end_reads_the_mode_that_held_inside_it() {
        // The gate covers this segment exactly. Read at the stage times, the
        // stage at t = 0.2 is outside `[0.1, 0.2)` and RK4 would lose its
        // weight 1/6, leaving 5/6 of the interval.
        let by_stage_time = Rk4.integrate(&GATE, rate(0.0), 0.1, 0.2, 0.1, |_, _| {});
        assert!(
            (by_stage_time.components[0][0] - 0.1 * 5.0 / 6.0).abs() < 1e-15,
            "expected 5/6 of the interval, got {}",
            by_stage_time.components[0][0]
        );

        let bound = SegmentSystem::new(&GATE, SegmentContext::new(0.1, 0.2));
        let in_segment = Rk4.integrate(&bound, rate(0.0), 0.1, 0.2, 0.1, |_, _| {});
        assert!(
            (in_segment.components[0][0] - 0.1).abs() < 1e-15,
            "expected the whole interval 0.1, got {}",
            in_segment.components[0][0]
        );
    }

    #[test]
    fn every_solver_integrates_the_gate_to_its_length() {
        let rk4 = integrate_by_segment(|bound, t, end, y| {
            Rk4.integrate(bound, y, t, end, end - t, |_, _| {})
        });
        let dp45 = integrate_by_segment(|bound, t, end, y| {
            let mut stepper = DormandPrince.stepper(bound, y, t, end - t, Tolerances::default());
            stepper
                .advance_to(end, |_, _| {}, |_, _| ControlFlow::<()>::Continue(()))
                .expect("the gate is finite everywhere");
            stepper.into_state()
        });
        let dop853 = integrate_by_segment(|bound, t, end, y| {
            let mut stepper = Dop853.stepper(bound, y, t, end - t, Tolerances::default());
            stepper
                .advance_to(end, |_, _| {}, |_, _| ControlFlow::<()>::Continue(()))
                .expect("the gate is finite everywhere");
            stepper.into_state()
        });

        for (name, got) in [("RK4", rk4), ("DP45", dp45), ("DOP853", dop853)] {
            assert!(
                (got - 0.1).abs() < 1e-12,
                "{name} integrated the gate to {got}, expected 0.1"
            );
        }
    }
}
