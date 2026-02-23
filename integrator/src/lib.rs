mod state;
mod error;
mod integrator;
mod rk4;
mod dp45;

#[cfg(test)]
pub(crate) mod test_systems;

pub use state::{OdeState, State, DynamicalSystem};
pub use error::{IntegrationError, IntegrationOutcome, Tolerances};
pub use integrator::Integrator;
pub use rk4::Rk4;
pub use dp45::{AdaptiveStepper, AdvanceOutcome, DormandPrince, error_norm};

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    #[test]
    fn state_clone_and_debug() {
        let state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let cloned = state.clone();
        assert_eq!(state, cloned);
        // Debug should not panic
        let _debug = format!("{:?}", state);
    }

    #[test]
    fn dynamical_system_trait_usage() {
        use test_systems::UniformMotion;

        let system = UniformMotion {
            constant_velocity: vector![1.0, 0.0, 0.0],
        };
        let state = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![1.0, 0.0, 0.0],
        };
        let deriv = system.derivatives(0.0, &state);
        // deriv is a State used as derivative:
        // .position holds velocity, .velocity holds acceleration
        assert_eq!(deriv.position, vector![1.0, 0.0, 0.0]);
        assert_eq!(deriv.velocity, vector![0.0, 0.0, 0.0]);
    }

    // --- OdeState trait tests ---

    #[test]
    fn ode_state_zero_like() {
        let state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let zero = state.zero_like();
        assert_eq!(zero.position, vector![0.0, 0.0, 0.0]);
        assert_eq!(zero.velocity, vector![0.0, 0.0, 0.0]);
    }

    #[test]
    fn ode_state_axpy() {
        let a = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let b = State {
            position: vector![10.0, 20.0, 30.0],
            velocity: vector![40.0, 50.0, 60.0],
        };
        let result = a.axpy(0.5, &b);
        assert_eq!(result.position, vector![6.0, 12.0, 18.0]);
        assert_eq!(result.velocity, vector![24.0, 30.0, 36.0]);
    }

    #[test]
    fn ode_state_scale() {
        let a = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let result = a.scale(2.0);
        assert_eq!(result.position, vector![2.0, 4.0, 6.0]);
        assert_eq!(result.velocity, vector![8.0, 10.0, 12.0]);
    }

    #[test]
    fn ode_state_is_finite() {
        let good = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        assert!(good.is_finite());

        let nan_pos = State {
            position: vector![f64::NAN, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        assert!(!nan_pos.is_finite());

        let inf_vel = State {
            position: vector![0.0, 0.0, 0.0],
            velocity: vector![0.0, f64::INFINITY, 0.0],
        };
        assert!(!inf_vel.is_finite());
    }

    #[test]
    fn ode_state_error_norm() {
        let y_n = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let y_next = State {
            position: vector![1.0, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let error = State {
            position: vector![1e-8, 0.0, 0.0],
            velocity: vector![0.0, 0.0, 0.0],
        };
        let tol = Tolerances {
            atol: 1e-8,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);
        // sc = 1e-8 + 1e-8 * 1.0 = 2e-8
        // e = 1e-8 / 2e-8 = 0.5
        // sum_sq = 0.25, n = 6
        // norm = sqrt(0.25/6) ≈ 0.2041
        assert!(
            (norm - (0.25_f64 / 6.0).sqrt()).abs() < 1e-12,
            "Expected ~0.2041, got {norm}"
        );
    }

    #[test]
    fn ode_state_from_derivative() {
        let deriv = State::from_derivative(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        // position holds velocity, velocity holds acceleration
        assert_eq!(deriv.position, vector![1.0, 2.0, 3.0]);
        assert_eq!(deriv.velocity, vector![4.0, 5.0, 6.0]);
    }

    #[test]
    fn ode_state_project_is_noop() {
        let mut state = State {
            position: vector![1.0, 2.0, 3.0],
            velocity: vector![4.0, 5.0, 6.0],
        };
        let original = state.clone();
        state.project(0.0);
        assert_eq!(state, original);
    }
}
