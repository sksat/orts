use std::collections::HashMap;
use std::ops::ControlFlow;

use orts_integrator::{AdvanceOutcome, DormandPrince, Integrator, Rk4, State, Tolerances};
use orts_orbits::{events::SimulationEvent, kepler::KeplerianElements, orbital_system::OrbitalSystem};

use crate::sim::core::{HistoryState, make_history_state, accel_breakdown};
use crate::commands::serve::protocol::WsMessage;

/// Reason a satellite simulation was terminated in serve mode.
#[derive(Debug)]
pub enum TerminationReason {
    Collision { altitude_km: f64 },
    AtmosphericEntry { altitude_km: f64 },
    NonFiniteState,
}

impl std::fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Collision { altitude_km } => {
                write!(f, "collision at altitude {altitude_km:.1} km")
            }
            Self::AtmosphericEntry { altitude_km } => {
                write!(f, "atmospheric entry at altitude {altitude_km:.1} km")
            }
            Self::NonFiniteState => write!(f, "numerical divergence (NaN/Inf)"),
        }
    }
}

/// Compute RK4 integration from t_start to chunk_end, collecting output states
/// at output_interval boundaries. Pure computation with no IO.
///
/// Returns (output_states, final_state, final_t, optional termination reason).
#[allow(clippy::too_many_arguments)]
pub fn compute_output_chunk(
    satellite_id: &str,
    system: &OrbitalSystem,
    mut state: State,
    t_start: f64,
    chunk_end: f64,
    dt: f64,
    output_interval: f64,
    next_output_t: &mut f64,
    atmosphere_altitude: Option<f64>,
) -> (Vec<HistoryState>, State, f64, Option<TerminationReason>) {
    let mu = system.mu;
    let body_radius = system.body_radius;
    let mut outputs = Vec::new();
    let mut t = t_start;

    while t < chunk_end {
        let h = dt.min(chunk_end - t);
        state = Rk4.step(system, t, &state, h);
        t += h;

        // Check for NaN/Inf
        if !state
            .position
            .iter()
            .chain(state.velocity.iter())
            .all(|v| v.is_finite())
        {
            return (outputs, state, t, Some(TerminationReason::NonFiniteState));
        }

        // Check for collision and atmospheric entry
        if let Some(r_body) = body_radius {
            let r = state.position.magnitude();
            if r < r_body {
                return (
                    outputs,
                    state,
                    t,
                    Some(TerminationReason::Collision {
                        altitude_km: r - r_body,
                    }),
                );
            }
            if let Some(atm_alt) = atmosphere_altitude
                && r < r_body + atm_alt
            {
                return (
                    outputs,
                    state,
                    t,
                    Some(TerminationReason::AtmosphericEntry {
                        altitude_km: r - r_body,
                    }),
                );
            }
        }

        if t >= *next_output_t - 1e-9 {
            let accels = accel_breakdown(system, t, &state);
            outputs.push(make_history_state(satellite_id, t, &state.position, &state.velocity, mu, accels));
            *next_output_t += output_interval;
        }
    }

    (outputs, state, t, None)
}

/// Create an event checker for the serve mode adaptive loop.
///
/// Like `events::collision_check` but handles `body_radius: Option<f64>`.
pub fn make_serve_event_checker(
    body_radius: Option<f64>,
    atmosphere_altitude: Option<f64>,
) -> impl Fn(f64, &State) -> ControlFlow<SimulationEvent> {
    move |_t: f64, state: &State| {
        if let Some(r_body) = body_radius {
            let r = state.position.magnitude();
            if r < r_body {
                return ControlFlow::Break(SimulationEvent::Collision {
                    altitude_km: r - r_body,
                });
            }
            if let Some(atm_alt) = atmosphere_altitude
                && r < r_body + atm_alt
            {
                return ControlFlow::Break(SimulationEvent::AtmosphericEntry {
                    altitude_km: r - r_body,
                });
            }
        }
        ControlFlow::Continue(())
    }
}

/// Adaptive Dormand-Prince version of compute_output_chunk.
/// Step size adapts automatically; outputs are produced at output_interval boundaries
/// by clamping the step to not overshoot the next output time.
#[allow(clippy::too_many_arguments)]
pub fn compute_output_chunk_adaptive(
    satellite_id: &str,
    system: &OrbitalSystem,
    state: State,
    t_start: f64,
    chunk_end: f64,
    dt_hint: f64,
    tol: &Tolerances,
    output_interval: f64,
    next_output_t: &mut f64,
    atmosphere_altitude: Option<f64>,
) -> (Vec<HistoryState>, State, f64, Option<TerminationReason>) {
    let mu = system.mu;
    let body_radius = system.body_radius;
    let mut outputs = Vec::new();

    let event_checker = make_serve_event_checker(body_radius, atmosphere_altitude);

    let mut stepper = DormandPrince.stepper(system, state, t_start, dt_hint, tol.clone());
    stepper.dt_min = 1e-12 * (chunk_end - t_start).abs().max(1.0);

    while stepper.t() < chunk_end - 1e-12 {
        let t_target = (*next_output_t).min(chunk_end);
        if t_target - stepper.t() < 1e-14 {
            break;
        }

        match stepper.advance_to(t_target, |_, _| {}, &event_checker) {
            Ok(AdvanceOutcome::Reached) => {
                if stepper.t() >= *next_output_t - 1e-9 {
                    let accels = accel_breakdown(system, stepper.t(), stepper.state());
                    outputs.push(make_history_state(
                        satellite_id,
                        stepper.t(),
                        &stepper.state().position,
                        &stepper.state().velocity,
                        mu,
                        accels,
                    ));
                    *next_output_t += output_interval;
                }
            }
            Ok(AdvanceOutcome::Event { reason }) => {
                let t = stepper.t();
                let termination = match reason {
                    SimulationEvent::Collision { altitude_km } => {
                        TerminationReason::Collision { altitude_km }
                    }
                    SimulationEvent::AtmosphericEntry { altitude_km } => {
                        TerminationReason::AtmosphericEntry { altitude_km }
                    }
                };
                return (outputs, stepper.into_state(), t, Some(termination));
            }
            Err(_) => {
                let t = stepper.t();
                return (
                    outputs,
                    stepper.into_state(),
                    t,
                    Some(TerminationReason::NonFiniteState),
                );
            }
        }
    }

    let t = stepper.t();
    (outputs, stepper.into_state(), t, None)
}

pub fn state_message(
    satellite_id: &str,
    t: f64,
    state: &State,
    mu: f64,
    accelerations: HashMap<String, f64>,
) -> String {
    let elements = KeplerianElements::from_state_vector(&state.position, &state.velocity, mu);
    let msg = WsMessage::State {
        satellite_id: satellite_id.to_string(),
        t,
        position: [state.position.x, state.position.y, state.position.z],
        velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        accelerations,
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;
    use orts_orbits::gravity;

    #[test]
    fn chunk_output_count_matches_interval() {
        // dt=10, output_interval=10, chunk=100s → expect 10 outputs
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _final_state, final_t, _term) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 10);
        assert!((final_t - 100.0).abs() < 1e-9);
    }

    #[test]
    fn chunk_fine_dt_batches_steps() {
        // dt=1, output_interval=10, chunk=100s → still 10 outputs but 100 RK4 steps
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 1.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 10);
        // Verify output times are at 10s intervals
        for (i, out) in outputs.iter().enumerate() {
            let expected_t = (i + 1) as f64 * 10.0;
            assert!(
                (out.t - expected_t).abs() < 0.1,
                "output[{i}].t = {}, expected {expected_t}",
                out.t
            );
        }
    }

    #[test]
    fn chunk_energy_conservation() {
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };
        let initial_energy = v0 * v0 / 2.0 - mu / r0;

        let mut next_output = 10.0;
        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 500.0, 10.0, 10.0, &mut next_output, None);

        for out in &outputs {
            let r = (out.position[0].powi(2) + out.position[1].powi(2) + out.position[2].powi(2))
                .sqrt();
            let v = (out.velocity[0].powi(2) + out.velocity[1].powi(2) + out.velocity[2].powi(2))
                .sqrt();
            let energy = v * v / 2.0 - mu / r;
            assert!(
                (energy - initial_energy).abs() < 1e-6,
                "energy drift at t={}: {:.2e}",
                out.t,
                (energy - initial_energy).abs()
            );
        }
    }

    #[test]
    fn chunk_partial_end() {
        // chunk_end doesn't align perfectly with output_interval
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let mut next_output = 10.0;
        // chunk_end=55 with output_interval=10 → outputs at 10,20,30,40,50 (5 outputs)
        let (outputs, _, final_t, _) =
            compute_output_chunk("test", &system, initial, 0.0, 55.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(outputs.len(), 5);
        assert!((final_t - 55.0).abs() < 1e-9);
        // next_output should be 60.0 now
        assert!((next_output - 60.0).abs() < 1e-9);
    }

    #[test]
    fn chunk_dual_intervals() {
        // stream_interval=2, output_interval=10, dt=1, chunk=20s
        // → 10 stream outputs, of which 2 are at save boundaries (t=10, t=20)
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        let stream_interval = 2.0;
        let output_interval = 10.0;
        let mut next_stream = stream_interval;

        let (outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 20.0, 1.0, stream_interval, &mut next_stream, None);

        assert_eq!(outputs.len(), 10); // 20s / 2s = 10 stream outputs

        // Filter for save boundaries (same logic as simulation_loop will use)
        let mut next_save = output_interval;
        let mut save_count = 0;
        for out in &outputs {
            if out.t >= next_save - 1e-9 {
                save_count += 1;
                next_save += output_interval;
            }
        }
        assert_eq!(save_count, 2); // t=10 and t=20
    }

    #[test]
    fn chunk_matches_step_by_step() {
        // Verify that chunked computation gives identical results to step-by-step
        let mu: f64 = 398600.4418;
        let r0: f64 = 6778.137;
        let v0 = (mu / r0).sqrt();
        let system = OrbitalSystem::new(mu, Box::new(gravity::PointMass));
        let initial = State {
            position: vector![r0, 0.0, 0.0],
            velocity: vector![0.0, v0, 0.0],
        };

        // Step-by-step (original approach)
        let mut state_ss = initial.clone();
        let mut t = 0.0;
        let dt = 10.0;
        let mut step_outputs = Vec::new();
        for _ in 0..10 {
            state_ss = Rk4.step(&system, t, &state_ss, dt);
            t += dt;
            step_outputs.push(make_history_state("test", t, &state_ss.position, &state_ss.velocity, mu, HashMap::new()));
        }

        // Chunked
        let mut next_output = 10.0;
        let (chunk_outputs, _, _, _) =
            compute_output_chunk("test", &system, initial, 0.0, 100.0, 10.0, 10.0, &mut next_output, None);

        assert_eq!(chunk_outputs.len(), step_outputs.len());
        for (c, s) in chunk_outputs.iter().zip(step_outputs.iter()) {
            assert!((c.t - s.t).abs() < 1e-12, "t mismatch: {} vs {}", c.t, s.t);
            for i in 0..3 {
                assert!(
                    (c.position[i] - s.position[i]).abs() < 1e-12,
                    "position[{i}] mismatch at t={}: {} vs {}",
                    c.t,
                    c.position[i],
                    s.position[i]
                );
            }
        }
    }
}
