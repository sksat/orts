//! Simulation event detection for orbital mechanics.
//!
//! Provides event checkers compatible with `Rk4::integrate_with_events`.

use std::ops::ControlFlow;

use orts_integrator::State;

/// Events that can terminate a simulation early.
#[derive(Debug, Clone, PartialEq)]
pub enum SimulationEvent {
    /// Satellite impacted the central body surface.
    Collision {
        /// Altitude at impact [km] (negative = below surface).
        altitude_km: f64,
    },
}

/// Create an event checker for surface collision detection.
///
/// Returns `ControlFlow::Break(SimulationEvent::Collision)` when `|r| < body_radius`.
pub fn collision_check(
    body_radius: f64,
) -> impl Fn(f64, &State) -> ControlFlow<SimulationEvent> {
    move |_t: f64, state: &State| {
        let r = state.position.magnitude();
        if r < body_radius {
            ControlFlow::Break(SimulationEvent::Collision {
                altitude_km: r - body_radius,
            })
        } else {
            ControlFlow::Continue(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MU_EARTH, R_EARTH};
    use nalgebra::vector;

    #[test]
    fn collision_check_above_surface() {
        let check = collision_check(R_EARTH);
        let state = State {
            position: vector![R_EARTH + 400.0, 0.0, 0.0],
            velocity: vector![0.0, 7.66, 0.0],
        };
        assert!(matches!(check(0.0, &state), ControlFlow::Continue(())));
    }

    #[test]
    fn collision_check_below_surface() {
        let check = collision_check(R_EARTH);
        let state = State {
            position: vector![R_EARTH - 10.0, 0.0, 0.0],
            velocity: vector![0.0, 7.0, 0.0],
        };
        match check(0.0, &state) {
            ControlFlow::Break(SimulationEvent::Collision { altitude_km }) => {
                assert!(altitude_km < 0.0);
                assert!((altitude_km - (-10.0)).abs() < 1e-10);
            }
            other => panic!("Expected collision, got {other:?}"),
        }
    }

    #[test]
    fn collision_check_at_surface() {
        let check = collision_check(R_EARTH);
        // Exactly at surface: r == body_radius, should NOT trigger (not < body_radius)
        let state = State {
            position: vector![R_EARTH, 0.0, 0.0],
            velocity: vector![0.0, 7.0, 0.0],
        };
        assert!(matches!(check(0.0, &state), ControlFlow::Continue(())));
    }

    #[test]
    fn collision_check_3d_position() {
        let check = collision_check(R_EARTH);
        // Position magnitude = sqrt(3000^2 * 3) ≈ 5196 km < R_EARTH (6378 km)
        let state = State {
            position: vector![3000.0, 3000.0, 3000.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        assert!(matches!(
            check(0.0, &state),
            ControlFlow::Break(SimulationEvent::Collision { .. })
        ));
    }

    #[test]
    fn suborbital_trajectory_terminates_on_collision() {
        use crate::gravity::PointMass;
        use crate::orbital_system::OrbitalSystem;
        use orts_integrator::{IntegrationOutcome, Rk4};

        let system = OrbitalSystem::new(MU_EARTH, Box::new(PointMass))
            .with_body_radius(R_EARTH);

        // Start at 100 km altitude with only 80% of circular velocity → will fall back
        let r = R_EARTH + 100.0;
        let v_circular = (MU_EARTH / r).sqrt();
        let initial = State {
            position: vector![r, 0.0, 0.0],
            velocity: vector![0.0, v_circular * 0.8, 0.0],
        };

        let event_checker = collision_check(R_EARTH);
        let outcome = Rk4::integrate_with_events(
            &system,
            initial,
            0.0,
            100_000.0,
            1.0,
            |_t, _state| {},
            event_checker,
        );

        match outcome {
            IntegrationOutcome::Terminated {
                reason: SimulationEvent::Collision { altitude_km },
                t,
                ..
            } => {
                assert!(altitude_km < 0.0, "Altitude should be negative at collision");
                assert!(t < 100_000.0, "Should terminate before t_end");
                assert!(t > 0.0, "Should not terminate immediately");
            }
            other => panic!("Expected collision termination, got {other:?}"),
        }
    }
}
