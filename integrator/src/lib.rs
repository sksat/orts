mod error;
mod integrator;
mod solver;
mod state;

#[cfg(test)]
mod comparison;
#[cfg(test)]
pub(crate) mod test_systems;

pub use error::{IntegrationError, IntegrationOutcome, Tolerances};
pub use integrator::Integrator;
pub use solver::dop853::{AdaptiveStepper853, AdvanceOutcome853, Dop853};
pub use solver::dp45::{AdaptiveStepper, AdvanceOutcome, DormandPrince};
pub use solver::rk4::Rk4;
pub use solver::verlet::StormerVerlet;
pub use state::{DynamicalSystem, OdeState, State};

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    #[test]
    fn state_clone_and_debug() {
        let state = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
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
        let state = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![1.0, 0.0, 0.0]);
        let deriv = system.derivatives(0.0, &state);
        // deriv is a State used as derivative:
        // components[0] holds velocity, components[1] holds acceleration
        assert_eq!(*deriv.y(), vector![1.0, 0.0, 0.0]);
        assert_eq!(*deriv.dy(), vector![0.0, 0.0, 0.0]);
    }

    // --- OdeState trait tests ---

    #[test]
    fn ode_state_zero_like() {
        let state = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        let zero = state.zero_like();
        assert_eq!(*zero.y(), vector![0.0, 0.0, 0.0]);
        assert_eq!(*zero.dy(), vector![0.0, 0.0, 0.0]);
    }

    #[test]
    fn ode_state_axpy() {
        let a = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        let b = State::<3, 2>::new(vector![10.0, 20.0, 30.0], vector![40.0, 50.0, 60.0]);
        let result = a.axpy(0.5, &b);
        assert_eq!(*result.y(), vector![6.0, 12.0, 18.0]);
        assert_eq!(*result.dy(), vector![24.0, 30.0, 36.0]);
    }

    #[test]
    fn ode_state_scale() {
        let a = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        let result = a.scale(2.0);
        assert_eq!(*result.y(), vector![2.0, 4.0, 6.0]);
        assert_eq!(*result.dy(), vector![8.0, 10.0, 12.0]);
    }

    #[test]
    fn ode_state_is_finite() {
        let good = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        assert!(good.is_finite());

        let nan_pos = State::<3, 2>::new(vector![f64::NAN, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        assert!(!nan_pos.is_finite());

        let inf_vel = State::<3, 2>::new(vector![0.0, 0.0, 0.0], vector![0.0, f64::INFINITY, 0.0]);
        assert!(!inf_vel.is_finite());
    }

    #[test]
    fn ode_state_error_norm() {
        let y_n = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let y_next = State::<3, 2>::new(vector![1.0, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
        let error = State::<3, 2>::new(vector![1e-8, 0.0, 0.0], vector![0.0, 0.0, 0.0]);
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
        let deriv = State::<3, 2>::from_derivative(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        // components[0] holds dy, components[1] holds ddy
        assert_eq!(*deriv.y(), vector![1.0, 2.0, 3.0]);
        assert_eq!(*deriv.dy(), vector![4.0, 5.0, 6.0]);
    }

    // --- 1D State tests ---

    #[test]
    fn state_1d_order2_basics() {
        let state = State::<1, 2>::new(vector![3.0], vector![4.0]);
        assert_eq!(state.y()[0], 3.0);
        assert_eq!(state.dy()[0], 4.0);

        let zero = state.zero_like();
        assert_eq!(zero.y()[0], 0.0);
        assert_eq!(zero.dy()[0], 0.0);

        let scaled = state.scale(2.0);
        assert_eq!(scaled.y()[0], 6.0);
        assert_eq!(scaled.dy()[0], 8.0);
    }

    #[test]
    fn state_1d_order1_basics() {
        let a = State {
            components: [nalgebra::Vector1::new(5.0)],
        };
        let b = State {
            components: [nalgebra::Vector1::new(10.0)],
        };
        let result = a.axpy(0.5, &b);
        assert_eq!(result.components[0][0], 10.0); // 5 + 0.5 * 10
    }

    #[test]
    fn state_1d_error_norm() {
        let y_n = State::<1, 2>::new(vector![1.0], vector![0.0]);
        let y_next = State::<1, 2>::new(vector![1.0], vector![0.0]);
        let error = State::<1, 2>::new(vector![1e-8], vector![0.0]);
        let tol = Tolerances {
            atol: 1e-8,
            rtol: 1e-8,
        };
        let norm = y_n.error_norm(&y_next, &error, &tol);
        // sc = 1e-8 + 1e-8 * 1.0 = 2e-8
        // e = 1e-8 / 2e-8 = 0.5
        // sum_sq = 0.25, n = 2 (DIM=1, ORDER=2)
        // norm = sqrt(0.25/2) ≈ 0.3536
        assert!(
            (norm - (0.25_f64 / 2.0).sqrt()).abs() < 1e-12,
            "Expected ~0.3536, got {norm}"
        );
    }

    // --- 2D State tests ---

    #[test]
    fn state_2d_order2_basics() {
        let state = State::<2, 2>::new(vector![1.0, 2.0], vector![3.0, 4.0]);
        assert_eq!(state.y()[0], 1.0);
        assert_eq!(state.y()[1], 2.0);
        assert_eq!(state.dy()[0], 3.0);
        assert_eq!(state.dy()[1], 4.0);

        let deriv = State::<2, 2>::from_derivative(vector![5.0, 6.0], vector![7.0, 8.0]);
        assert_eq!(deriv.y()[0], 5.0);
        assert_eq!(deriv.dy()[1], 8.0);
    }

    #[test]
    fn state_2d_order1_axpy() {
        let a = State {
            components: [nalgebra::Vector2::new(1.0, 2.0)],
        };
        let b = State {
            components: [nalgebra::Vector2::new(10.0, 20.0)],
        };
        let result = a.axpy(0.5, &b);
        assert_eq!(result.components[0][0], 6.0); // 1 + 0.5 * 10
        assert_eq!(result.components[0][1], 12.0); // 2 + 0.5 * 20
    }

    #[test]
    fn ode_state_project_is_noop() {
        let mut state = State::<3, 2>::new(vector![1.0, 2.0, 3.0], vector![4.0, 5.0, 6.0]);
        let original = state.clone();
        state.project(0.0);
        assert_eq!(state, original);
    }
}
