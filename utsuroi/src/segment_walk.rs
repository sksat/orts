//! Walking a span's segments, one per switch of the right-hand side.
//!
//! A propagation loop that honours [`next_discontinuity_after`] does the same
//! four things at every switch: ask for the next one, keep it only if it lies
//! ahead and is finite, clamp it to the span's end, and bind the system to the
//! interval so the stage landing on it reads the mode that held inside. Three
//! loops in this workspace did that by hand, and they had already diverged.
//!
//! [`Segments`] does it once. What a loop keeps is the part that differs: which
//! solver to run, what to observe, which predicate to ask, and how to record a
//! termination.
//!
//! [`next_discontinuity_after`]: DynamicalSystem::next_discontinuity_after

use crate::error::{IntegrationError, validate_time_span};
use crate::segment::{SegmentContext, SegmentSystem};
use crate::state::DynamicalSystem;

/// The segments of `[t0, t_end]`, split at the switches the system reports.
///
/// Each item is the system bound to one segment, in order, covering the span
/// end to end. A system that reports no switch yields the whole span as one
/// segment.
///
/// A loop that stops partway — on an event, or on an error — stops taking
/// items. The walk has no way to know that the state it would hand the next
/// segment was never reached, so resuming after a stop means building a new
/// `Segments` from where the loop actually is.
pub struct Segments<'a, S: DynamicalSystem> {
    system: &'a S,
    t: f64,
    t_end: f64,
    taken: usize,
}

/// One segment of a [`Segments`] walk.
pub struct Segment<'a, S: DynamicalSystem> {
    system: SegmentSystem<'a, S>,
    position: SegmentPosition,
}

/// Where a segment sits in its walk.
///
/// A solver asks its event predicate about the state it starts from, since a
/// level-triggered event can already hold there. For a [`Continuation`] that
/// state is the one the previous segment ended on, which the loop has already
/// asked about — the position says which case a segment is, and the loop
/// decides what that means for its own predicate.
///
/// [`Continuation`]: SegmentPosition::Continuation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentPosition {
    /// The first segment of the walk: its start is the span's start.
    Initial,
    /// A later segment: its start is where the previous segment ended.
    Continuation,
}

impl<'a, S: DynamicalSystem> Segments<'a, S> {
    /// Walk the segments of `[t0, t_end]`.
    ///
    /// Rejects a span no loop can walk: a non-finite start, a non-finite end,
    /// or an end before the start. A loop that tests `t < t_end` before
    /// stepping cannot leave this to the solver — every comparison is false
    /// for a NaN, so it takes no step and the solver never sees the span to
    /// reject it.
    ///
    /// An empty span (`t0 == t_end`) yields nothing.
    pub fn new(system: &'a S, t0: f64, t_end: f64) -> Result<Self, IntegrationError> {
        validate_time_span(t0, t_end)?;
        Ok(Self {
            system,
            t: t0,
            t_end,
            taken: 0,
        })
    }
}

impl<'a, S: DynamicalSystem> Iterator for Segments<'a, S> {
    type Item = Segment<'a, S>;

    fn next(&mut self) -> Option<Segment<'a, S>> {
        if self.t >= self.t_end {
            return None;
        }
        let end = self
            .system
            .next_discontinuity_after(self.t)
            // The contract asks for a finite time strictly after `t`; a system
            // that breaks it would otherwise stall the walk or send a solver
            // backwards.
            .filter(|next| *next > self.t && next.is_finite())
            .map_or(self.t_end, |next| next.min(self.t_end));
        let position = if self.taken == 0 {
            SegmentPosition::Initial
        } else {
            SegmentPosition::Continuation
        };
        let segment = Segment {
            system: SegmentSystem::new(self.system, SegmentContext::new(self.t, end)),
            position,
        };
        self.t = end;
        self.taken += 1;
        Some(segment)
    }
}

impl<'a, S: DynamicalSystem> Segment<'a, S> {
    /// The system to hand a solver, bound to this segment.
    pub fn system(&self) -> &SegmentSystem<'a, S> {
        &self.system
    }

    /// The interval this segment covers.
    pub fn context(&self) -> &SegmentContext {
        self.system.segment()
    }

    /// Time the segment starts at.
    pub fn start(&self) -> f64 {
        self.context().start
    }

    /// Time the segment ends at.
    pub fn end(&self) -> f64 {
        self.context().end
    }

    /// Where this segment sits in its walk.
    pub fn position(&self) -> SegmentPosition {
        self.position
    }

    /// Whether an earlier segment of this walk came before it.
    pub fn is_continuation(&self) -> bool {
        self.position == SegmentPosition::Continuation
    }
}

#[cfg(test)]
mod tests {
    use nalgebra::Vector1;

    use super::{SegmentPosition, Segments};
    use crate::{DynamicalSystem, IntegrationError, State};

    /// Reports the switches it was given, in the order it was given them.
    struct Switches {
        at: Vec<f64>,
    }

    impl DynamicalSystem for Switches {
        type State = State<1, 1>;

        fn derivatives(&self, _t: f64, _state: &Self::State) -> Self::State {
            State {
                components: [Vector1::new(0.0)],
            }
        }

        fn next_discontinuity_after(&self, t: f64) -> Option<f64> {
            self.at
                .iter()
                .copied()
                .filter(|switch| *switch > t)
                .min_by(f64::total_cmp)
        }
    }

    fn walk(at: Vec<f64>, t0: f64, t_end: f64) -> Vec<(f64, f64, SegmentPosition)> {
        let system = Switches { at };
        Segments::new(&system, t0, t_end)
            .expect("the span is finite and forward")
            .map(|segment| (segment.start(), segment.end(), segment.position()))
            .collect()
    }

    #[test]
    fn a_system_with_no_switch_yields_the_whole_span() {
        assert_eq!(
            walk(vec![], 0.0, 10.0),
            vec![(0.0, 10.0, SegmentPosition::Initial)]
        );
    }

    #[test]
    fn switches_inside_the_span_split_it_end_to_end() {
        assert_eq!(
            walk(vec![0.1, 0.2], 0.0, 1.0),
            vec![
                (0.0, 0.1, SegmentPosition::Initial),
                (0.1, 0.2, SegmentPosition::Continuation),
                (0.2, 1.0, SegmentPosition::Continuation),
            ]
        );
    }

    #[test]
    fn a_switch_past_the_span_is_clamped_to_its_end() {
        assert_eq!(
            walk(vec![5.0], 0.0, 1.0),
            vec![(0.0, 1.0, SegmentPosition::Initial)]
        );
    }

    /// A switch at the span's end is the end: it closes the last segment
    /// rather than opening an empty one.
    #[test]
    fn a_switch_on_the_span_end_opens_no_further_segment() {
        assert_eq!(
            walk(vec![1.0], 0.0, 1.0),
            vec![(0.0, 1.0, SegmentPosition::Initial)]
        );
    }

    /// A system that breaks the contract — a switch at or before `t`, or a
    /// non-finite one — is ignored rather than allowed to stall the walk.
    #[test]
    fn a_switch_that_does_not_advance_is_ignored() {
        for at in [vec![0.0], vec![-1.0], vec![f64::NAN], vec![f64::INFINITY]] {
            assert_eq!(
                walk(at.clone(), 0.0, 1.0),
                vec![(0.0, 1.0, SegmentPosition::Initial)],
                "switches {at:?}"
            );
        }
    }

    #[test]
    fn an_empty_span_yields_nothing() {
        assert!(walk(vec![0.5], 1.0, 1.0).is_empty());
    }

    #[test]
    fn a_span_no_loop_can_walk_is_rejected() {
        let system = Switches { at: vec![] };
        for (t0, t_end) in [
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
            (0.0, f64::NEG_INFINITY),
            (f64::NAN, 1.0),
            (1.0, 0.0),
        ] {
            assert!(
                matches!(
                    Segments::new(&system, t0, t_end),
                    Err(IntegrationError::InvalidTimeSpan { .. })
                ),
                "the span [{t0}, {t_end}] was accepted"
            );
        }
    }

    /// A loop that stops partway leaves the rest of the walk untaken, and a
    /// walk built from where it actually stopped covers what is left. This is
    /// the shape every propagation loop in the workspace uses on an event or
    /// an error.
    #[test]
    fn a_walk_resumed_from_a_stop_covers_the_rest_of_the_span() {
        let system = Switches { at: vec![0.1, 0.2] };
        let mut walk = Segments::new(&system, 0.0, 1.0).expect("the span is finite and forward");
        let first = walk.next().expect("the span holds segments");
        assert_eq!((first.start(), first.end()), (0.0, 0.1));
        drop(walk);

        let rest: Vec<_> = Segments::new(&system, first.end(), 1.0)
            .expect("the span is finite and forward")
            .map(|segment| (segment.start(), segment.end(), segment.position()))
            .collect();
        assert_eq!(
            rest,
            vec![
                (0.1, 0.2, SegmentPosition::Initial),
                (0.2, 1.0, SegmentPosition::Continuation),
            ],
            "a resumed walk starts over: its first segment is Initial"
        );
    }

    /// The bound system is what a solver steps: at the end of a segment it
    /// answers for the segment, not for the stage time.
    #[test]
    fn the_bound_system_answers_for_its_own_segment() {
        let system = Switches { at: vec![0.5] };
        let first = Segments::new(&system, 0.0, 1.0)
            .expect("the span is finite and forward")
            .next()
            .expect("the span holds a segment");
        assert_eq!(first.context().start, 0.0);
        assert_eq!(first.context().end, 0.5);
        assert_eq!(first.system().segment().end, 0.5);
    }
}
