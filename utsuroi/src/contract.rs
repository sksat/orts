//! What every integrate loop promises, checked once per loop.
//!
//! Eight loops carry a caller from `t0` to `t_end`. Three solvers each write
//! two — a plain one and an event-checking one — for the [`Integrator`]
//! default, Verlet and Yoshida, and the adaptive steppers behind DP5(4) and
//! DOP853 add one apiece. They were written separately, so they can break
//! separately.
//!
//! Measured before this module existed: replacing five of them with "report
//! nothing and return the initial state unchanged" left 147 of the crate's
//! 167 tests passing. A one-period harmonic-oscillator test cannot see it,
//! because after one period the analytic state *is* the initial state. The
//! tests here integrate a span whose analytic answer differs from the input,
//! and check the states the loop reported along the way, not only the one it
//! returned.
//!
//! Both systems below have a solution that depends on `t`. The shared
//! systems in [`crate::test_systems`] all ignore it (`derivatives(&self,
//! _t, ...)`), and the ones that do read it are the per-test `Exploding`
//! systems, which return `INFINITY` past a threshold to reach the
//! non-finite-state error path. None of them has a finite solution to
//! compare against, so nothing else in the suite can see a loop that
//! evaluates a stage at the wrong time, or that starts from the wrong `t0`.

use core::ops::ControlFlow;

use nalgebra::{Vector1, vector};

use crate::test_systems::HarmonicOscillator1D;
use crate::{
    Dop853, DormandPrince, DynamicalSystem, IntegrationOutcome, Integrator, Rk4, State,
    StormerVerlet, Tolerances, Yoshida4,
};

/// A span that starts away from the origin and ends off the step grid.
///
/// `t0 != 0` catches a loop that starts its stages at zero instead of at the
/// caller's `t0`. `(T_END - T0) / DT = 10.67`, so a loop that walks the span
/// in steps of `DT` has to shorten the last one to land on `T_END`.
const T0: f64 = 0.7;
const T_END: f64 = 2.3;
const DT: f64 = 0.15;

/// The whole steps of `DT` that fit in the span, before the short one.
const WHOLE_STEPS: usize = 10;

/// First-order and non-autonomous: `dy/dt = t`.
///
/// `f` is a linear function of `t` alone, so RK4 and both adaptive main
/// formulas integrate it exactly up to rounding. That makes the oracle tight
/// enough to catch a single stage evaluated at the wrong time.
struct RampDerivative;

impl DynamicalSystem for RampDerivative {
    type State = State<1, 1>;
    fn derivatives(&self, t: f64, _state: &Self::State) -> Self::State {
        State {
            components: [Vector1::new(t)],
        }
    }
}

/// `y(t) = y0 + (t² - t0²) / 2`.
fn ramp_solution(y0: f64, t0: f64, t: f64) -> f64 {
    y0 + (t * t - t0 * t0) / 2.0
}

/// Second-order and non-autonomous: `dv/dt = t`, for the Verlet-family loops.
struct RampAcceleration;

impl DynamicalSystem for RampAcceleration {
    type State = State<1, 2>;
    fn derivatives(&self, t: f64, state: &Self::State) -> Self::State {
        State::from_derivative(*state.dy(), Vector1::new(t))
    }
}

/// `v(t) = v0 + (t² - t0²) / 2`, and integrating that once more,
/// `x(t) = x0 + v0 (t - t0) + (t³ - t0³) / 6 - t0² (t - t0) / 2`.
fn ramp_acceleration_solution(x0: f64, v0: f64, t0: f64, t: f64) -> (f64, f64) {
    let x = x0 + v0 * (t - t0) + (t * t * t - t0 * t0 * t0) / 6.0 - t0 * t0 * (t - t0) / 2.0;
    let v = v0 + (t * t - t0 * t0) / 2.0;
    (x, v)
}

/// What a loop told its caller, in the order it told it.
struct Reported<St> {
    samples: Vec<(f64, St)>,
}

impl<St> Default for Reported<St> {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
        }
    }
}

impl<St> Reported<St> {
    fn times(&self) -> Vec<f64> {
        self.samples.iter().map(|(t, _)| *t).collect()
    }

    /// The promises that hold for every loop over a non-empty span.
    fn assert_walked_the_span(&self, label: &str) {
        let times = self.times();
        assert!(
            !times.is_empty(),
            "{label}: a non-empty span must report at least one state"
        );
        for pair in times.windows(2) {
            assert!(
                pair[1] > pair[0],
                "{label}: reported times must increase: {times:?}"
            );
        }
        assert!(
            times[0] > T0,
            "{label}: the first reported state is after t0, not at it: {}",
            times[0]
        );
        let last = *times.last().expect("checked non-empty above");
        // Not an exact comparison: a fixed-step loop reaches `T_END` by
        // accumulating `t += h`, so the landing carries the rounding of the
        // subtraction that sized the last step and the addition that took it.
        let slack = 8.0 * f64::EPSILON * T_END.abs().max(1.0);
        assert!(
            (last - T_END).abs() <= slack,
            "{label}: the last reported state must land on t_end: {last} vs {T_END}"
        );
    }

    /// A fixed-step loop walks in steps of `dt` and shortens only the last.
    ///
    /// Without this, a loop that crossed the whole span in one step of
    /// `T_END - T0` would satisfy every assertion above, and on the two
    /// systems here RK4 and Yoshida4 would still return the analytic answer.
    fn assert_stepped_by(&self, label: &str, dt: f64) {
        let times = self.times();
        assert_eq!(
            times.len(),
            WHOLE_STEPS + 1,
            "{label}: {WHOLE_STEPS} whole steps of {dt} fit in the span, then a \
             short one: {times:?}"
        );
        let mut previous = T0;
        for (i, t) in times.iter().enumerate().take(WHOLE_STEPS) {
            let step = t - previous;
            assert!(
                (step - dt).abs() < 1e-9,
                "{label}: step {i} is {step}, expected {dt}: {times:?}"
            );
            previous = *t;
        }
        let last_step = times[WHOLE_STEPS] - previous;
        assert!(
            last_step > 0.0 && last_step < dt,
            "{label}: the last step is shortened to land on t_end, so it is in \
             (0, {dt}): {last_step}"
        );
    }
}

/// Tight enough that only an exact integration passes, loose enough to
/// survive a different order of rounding.
const EXACT_ORACLE_TOL: f64 = 1e-12;

/// Ten times the error measured at `DT` (5.8e-3). Verlet's position update
/// drops each step's `a'(t) h³/6`, and that constant belongs to the problem;
/// the halving check is what pins the error to truncation.
const VERLET_POSITION_CAP: f64 = 6e-2;

fn assert_ramp_samples(reported: &Reported<State<1, 1>>, y0: f64, label: &str) {
    for (t, state) in &reported.samples {
        let expected = ramp_solution(y0, T0, *t);
        assert!(
            (state.components[0][0] - expected).abs() < EXACT_ORACLE_TOL,
            "{label}: reported y({t}) = {}, expected {expected}",
            state.components[0][0]
        );
    }
}

fn assert_ramp_acceleration_samples(
    reported: &Reported<State<1, 2>>,
    x0: f64,
    v0: f64,
    position_tol: f64,
    label: &str,
) {
    for (t, state) in &reported.samples {
        let (expected_x, expected_v) = ramp_acceleration_solution(x0, v0, T0, *t);
        assert!(
            (state.y()[0] - expected_x).abs() < position_tol,
            "{label}: reported x({t}) = {}, expected {expected_x}",
            state.y()[0]
        );
        assert!(
            (state.dy()[0] - expected_v).abs() < EXACT_ORACLE_TOL,
            "{label}: reported v({t}) = {}, expected {expected_v}",
            state.dy()[0]
        );
    }
}

#[test]
fn the_integrator_default_loops_honor_the_contract() {
    let y0 = 1.0;
    let initial = State::<1, 1> {
        components: [Vector1::new(y0)],
    };

    let mut plain = Reported::default();
    let final_state = Rk4
        .try_integrate(&RampDerivative, initial.clone(), T0, T_END, DT, |t, s| {
            plain.samples.push((t, s.clone()))
        })
        .expect("a valid span integrates");

    plain.assert_walked_the_span("Integrator try_integrate");
    plain.assert_stepped_by("Integrator try_integrate", DT);
    assert_ramp_samples(&plain, y0, "Integrator try_integrate");
    assert_eq!(
        final_state, plain.samples[WHOLE_STEPS].1,
        "try_integrate returns the last state it reported"
    );

    // `integrate_with_events` is a second loop, not a wrapper around the
    // first, so it can drift from the contract on its own.
    let mut with_events = Reported::default();
    let outcome = Rk4.integrate_with_events(
        &RampDerivative,
        initial.clone(),
        T0,
        T_END,
        DT,
        |t, s| with_events.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(events_state) = outcome else {
        panic!("Integrator integrate_with_events: no event fires here, got {outcome:?}");
    };
    with_events.assert_walked_the_span("Integrator integrate_with_events");
    with_events.assert_stepped_by("Integrator integrate_with_events", DT);
    assert_ramp_samples(&with_events, y0, "Integrator integrate_with_events");
    assert_eq!(
        with_events.times(),
        plain.times(),
        "both loops walk the same span"
    );
    assert_eq!(
        events_state, with_events.samples[WHOLE_STEPS].1,
        "integrate_with_events returns the last state it reported"
    );
    assert_eq!(
        events_state, final_state,
        "both loops end at the same state"
    );

    // The wrapper that panics instead of returning `Err` must walk the same
    // span. `integrate` delegates to `try_integrate`, so this pins the
    // delegation, not the arithmetic.
    let mut wrapper = Reported::default();
    let wrapper_state = Rk4.integrate(&RampDerivative, initial, T0, T_END, DT, |t, s| {
        wrapper.samples.push((t, s.clone()))
    });
    assert_eq!(
        wrapper.times(),
        plain.times(),
        "integrate must report what try_integrate reports"
    );
    assert_eq!(
        wrapper_state, final_state,
        "integrate must return what try_integrate returns"
    );
}

#[test]
fn the_verlet_loops_honor_the_contract() {
    let (x0, v0) = (1.0, 0.5);
    let initial = State::<1, 2>::new(vector![x0], vector![v0]);

    let mut plain = Reported::default();
    let final_state = StormerVerlet
        .try_integrate(&RampAcceleration, initial.clone(), T0, T_END, DT, |t, s| {
            plain.samples.push((t, s.clone()))
        })
        .expect("a valid span integrates");

    plain.assert_walked_the_span("Verlet try_integrate");
    plain.assert_stepped_by("Verlet try_integrate", DT);
    // Verlet's velocity update averages the acceleration at both ends of the
    // step, which is exact for an acceleration linear in `t`: measured error
    // 8.9e-16. Shift either end of a step and the average stops matching the
    // integral, so the velocity is a tight oracle on the stage times.
    assert_ramp_acceleration_samples(&plain, x0, v0, VERLET_POSITION_CAP, "Verlet");

    assert_eq!(
        final_state, plain.samples[WHOLE_STEPS].1,
        "try_integrate returns the last state it reported"
    );

    let (expected_x, _) = ramp_acceleration_solution(x0, v0, T0, T_END);
    let coarse_error = (final_state.y()[0] - expected_x).abs();
    let halved = StormerVerlet
        .try_integrate(
            &RampAcceleration,
            initial.clone(),
            T0,
            T_END,
            DT / 2.0,
            |_, _| {},
        )
        .expect("a valid span integrates");
    let fine_error = (halved.y()[0] - expected_x).abs();
    let ratio = coarse_error / fine_error;
    // Each step drops `a'(t) h³/6`, so the total scales as `h²` — about 3.92
    // here rather than exactly 4, because the short final step changes with
    // `dt` too. The window covers that and stays far from rounding.
    assert!(
        (3.0..5.0).contains(&ratio),
        "Verlet position error is second order, so halving dt divides it by \
         about 3.92: {coarse_error:e} / {fine_error:e} = {ratio}"
    );

    let mut with_events = Reported::default();
    let outcome = StormerVerlet.integrate_with_events(
        &RampAcceleration,
        initial,
        T0,
        T_END,
        DT,
        |t, s| with_events.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(events_state) = outcome else {
        panic!("Verlet integrate_with_events: no event fires here, got {outcome:?}");
    };
    with_events.assert_walked_the_span("Verlet integrate_with_events");
    with_events.assert_stepped_by("Verlet integrate_with_events", DT);
    assert_ramp_acceleration_samples(
        &with_events,
        x0,
        v0,
        VERLET_POSITION_CAP,
        "Verlet integrate_with_events",
    );
    assert_eq!(
        events_state, with_events.samples[WHOLE_STEPS].1,
        "integrate_with_events returns the last state it reported"
    );
    assert_eq!(
        events_state, final_state,
        "both loops end at the same state"
    );
}

#[test]
fn the_yoshida_loops_honor_the_contract() {
    let (x0, v0) = (1.0, 0.5);
    let initial = State::<1, 2>::new(vector![x0], vector![v0]);

    let mut plain = Reported::default();
    let final_state = Yoshida4
        .try_integrate(&RampAcceleration, initial.clone(), T0, T_END, DT, |t, s| {
            plain.samples.push((t, s.clone()))
        })
        .expect("a valid span integrates");

    plain.assert_walked_the_span("Yoshida4 try_integrate");
    plain.assert_stepped_by("Yoshida4 try_integrate", DT);
    // Exact here, measured 8.9e-16 in position and 4.4e-16 in velocity: each
    // Verlet substep's `h³` position error scales with that substep's weight
    // cubed, and the triple-jump weights cube to zero, so they cancel.
    assert_ramp_acceleration_samples(&plain, x0, v0, EXACT_ORACLE_TOL, "Yoshida4");
    assert_eq!(
        final_state, plain.samples[WHOLE_STEPS].1,
        "try_integrate returns the last state it reported"
    );

    let mut with_events = Reported::default();
    let outcome = Yoshida4.integrate_with_events(
        &RampAcceleration,
        initial,
        T0,
        T_END,
        DT,
        |t, s| with_events.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(events_state) = outcome else {
        panic!("Yoshida4 integrate_with_events: no event fires here, got {outcome:?}");
    };
    with_events.assert_walked_the_span("Yoshida4 integrate_with_events");
    with_events.assert_stepped_by("Yoshida4 integrate_with_events", DT);
    assert_ramp_acceleration_samples(
        &with_events,
        x0,
        v0,
        EXACT_ORACLE_TOL,
        "Yoshida4 integrate_with_events",
    );
    assert_eq!(
        events_state, with_events.samples[WHOLE_STEPS].1,
        "integrate_with_events returns the last state it reported"
    );
    assert_eq!(
        events_state, final_state,
        "both loops end at the same state"
    );
}

#[test]
fn the_dp45_adaptive_loop_honors_the_contract() {
    let y0 = 1.0;
    let initial = State::<1, 1> {
        components: [Vector1::new(y0)],
    };
    let tol = Tolerances {
        atol: 1e-10,
        rtol: 1e-10,
    };

    let mut reported = Reported::default();
    let outcome = DormandPrince.integrate_adaptive_with_events(
        &RampDerivative,
        initial,
        T0,
        T_END,
        DT,
        &tol,
        |t, s| reported.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );

    let IntegrationOutcome::Completed(final_state) = outcome else {
        panic!("DP45: a valid span completes, got {outcome:?}");
    };
    // No step-spacing assertion: `DT` is only the first step's estimate, and
    // the controller is free to grow it from there.
    reported.assert_walked_the_span("DP45");
    assert_ramp_samples(&reported, y0, "DP45");
    let last = &reported.samples.last().expect("reported states").1;
    assert_eq!(
        &final_state, last,
        "the returned state is the last one reported"
    );
}

#[test]
fn the_dop853_adaptive_loop_honors_the_contract() {
    let y0 = 1.0;
    let initial = State::<1, 1> {
        components: [Vector1::new(y0)],
    };
    let tol = Tolerances {
        atol: 1e-10,
        rtol: 1e-10,
    };

    let mut reported = Reported::default();
    let outcome = Dop853.integrate_adaptive_with_events(
        &RampDerivative,
        initial,
        T0,
        T_END,
        DT,
        &tol,
        |t, s| reported.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );

    let IntegrationOutcome::Completed(final_state) = outcome else {
        panic!("DOP853: a valid span completes, got {outcome:?}");
    };
    reported.assert_walked_the_span("DOP853");
    assert_ramp_samples(&reported, y0, "DOP853");
    let last = &reported.samples.last().expect("reported states").1;
    assert_eq!(
        &final_state, last,
        "the returned state is the last one reported"
    );
}

/// An empty span returns the state it was given, whole, and reports nothing.
///
/// The three fixed-step solvers only. The adaptive wrappers size their first
/// step as `dt_initial.min(t_end - t0)`, which is `0` for an empty span, and
/// answer `Error(InvalidStepSize { dt: 0.0 })` — measured, and blaming a `dt`
/// the caller never passed. `orts` never asks: `propagate_to` returns early
/// when `t >= t_target`.
///
/// Checked as an equality on the whole state: the existing empty-span test
/// looks at `y()[0]`, which is `0.0` before the call as well as after, so a
/// loop that returned a zeroed state of the right shape passes it.
#[test]
fn an_empty_span_returns_the_initial_state_untouched() {
    let initial = State::<1, 2>::new(vector![3.0], vector![-4.0]);

    let mut reported = Reported::default();
    let fixed = Rk4
        .try_integrate(
            &HarmonicOscillator1D,
            initial.clone(),
            T0,
            T0,
            DT,
            |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        )
        .expect("an empty span is valid");
    assert_eq!(*fixed.y(), *initial.y(), "Integrator default: position");
    assert_eq!(*fixed.dy(), *initial.dy(), "Integrator default: velocity");
    assert!(
        reported.samples.is_empty(),
        "Integrator default: an empty span steps nowhere, so it reports nothing"
    );

    let mut reported = Reported::default();
    let verlet = StormerVerlet
        .try_integrate(
            &HarmonicOscillator1D,
            initial.clone(),
            T0,
            T0,
            DT,
            |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        )
        .expect("an empty span is valid");
    assert_eq!(*verlet.y(), *initial.y(), "Verlet: position");
    assert_eq!(*verlet.dy(), *initial.dy(), "Verlet: velocity");
    assert!(reported.samples.is_empty(), "Verlet: reports nothing");

    let mut reported = Reported::default();
    let yoshida = Yoshida4
        .try_integrate(
            &HarmonicOscillator1D,
            initial.clone(),
            T0,
            T0,
            DT,
            |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        )
        .expect("an empty span is valid");
    assert_eq!(*yoshida.y(), *initial.y(), "Yoshida4: position");
    assert_eq!(*yoshida.dy(), *initial.dy(), "Yoshida4: velocity");
    assert!(reported.samples.is_empty(), "Yoshida4: reports nothing");

    // The event-checking loops answer with an outcome rather than a `Result`,
    // and each is a separate loop, so each promises this separately.
    let mut reported = Reported::default();
    let outcome = Rk4.integrate_with_events(
        &HarmonicOscillator1D,
        initial.clone(),
        T0,
        T0,
        DT,
        |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(state) = outcome else {
        panic!("Integrator integrate_with_events: an empty span completes, got {outcome:?}");
    };
    assert_eq!(
        state,
        initial.clone(),
        "Integrator integrate_with_events: an empty span returns the state it was given"
    );
    assert!(
        reported.samples.is_empty(),
        "Integrator integrate_with_events: reports nothing"
    );

    let mut reported = Reported::default();
    let outcome = StormerVerlet.integrate_with_events(
        &HarmonicOscillator1D,
        initial.clone(),
        T0,
        T0,
        DT,
        |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(state) = outcome else {
        panic!("Verlet integrate_with_events: an empty span completes, got {outcome:?}");
    };
    assert_eq!(
        state,
        initial.clone(),
        "Verlet integrate_with_events: an empty span returns the state it was given"
    );
    assert!(
        reported.samples.is_empty(),
        "Verlet integrate_with_events: reports nothing"
    );

    let mut reported = Reported::default();
    let outcome = Yoshida4.integrate_with_events(
        &HarmonicOscillator1D,
        initial.clone(),
        T0,
        T0,
        DT,
        |t, s: &State<1, 2>| reported.samples.push((t, s.clone())),
        |_, _| ControlFlow::<()>::Continue(()),
    );
    let IntegrationOutcome::Completed(state) = outcome else {
        panic!("Yoshida4 integrate_with_events: an empty span completes, got {outcome:?}");
    };
    assert_eq!(
        state, initial,
        "Yoshida4 integrate_with_events: an empty span returns the state it was given"
    );
    assert!(
        reported.samples.is_empty(),
        "Yoshida4 integrate_with_events: reports nothing"
    );
}
