use utsuroi::DynamicalSystem;

use super::state::GroupState;

/// Independent group dynamics: each satellite's derivatives are computed
/// independently with no inter-satellite coupling.
///
/// This is the simplest GroupDynamics implementation, serving as:
/// (a) test infrastructure for validating GroupState + integrator plumbing
/// (b) baseline for the future CoupledGroupDynamics (C3)
pub struct IndependentGroupDynamics<D: DynamicalSystem> {
    pub dynamics: Vec<D>,
}

impl<D: DynamicalSystem> IndependentGroupDynamics<D> {
    pub fn new(dynamics: Vec<D>) -> Self {
        Self { dynamics }
    }
}

impl<D: DynamicalSystem> DynamicalSystem for IndependentGroupDynamics<D> {
    type State = GroupState<D::State>;

    fn derivatives(&self, t: f64, state: &GroupState<D::State>) -> GroupState<D::State> {
        assert_eq!(
            self.dynamics.len(),
            state.states.len(),
            "IndependentGroupDynamics: dynamics count ({}) != state count ({})",
            self.dynamics.len(),
            state.states.len()
        );
        GroupState {
            states: self
                .dynamics
                .iter()
                .zip(&state.states)
                .map(|(dyn_sys, s)| dyn_sys.derivatives(t, s))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::orbital::two_body::TwoBodySystem;
    use nalgebra::Vector3;
    use std::ops::ControlFlow;
    use utsuroi::{DormandPrince, IntegrationOutcome, Integrator, Rk4, Tolerances};

    /// Simple harmonic oscillator: dv/dt = -x (ω = 1).
    struct HarmonicOscillator;

    impl DynamicalSystem for HarmonicOscillator {
        type State = OrbitalState;
        fn derivatives(&self, _t: f64, state: &OrbitalState) -> OrbitalState {
            OrbitalState::from_derivative(*state.velocity(), -*state.position())
        }
    }

    /// Uniform motion: dx/dt = v, dv/dt = 0.
    struct UniformMotion {
        velocity: Vector3<f64>,
    }

    impl DynamicalSystem for UniformMotion {
        type State = OrbitalState;
        fn derivatives(&self, _t: f64, _state: &OrbitalState) -> OrbitalState {
            OrbitalState::from_derivative(self.velocity, Vector3::zeros())
        }
    }

    fn iss_state() -> OrbitalState {
        let r: f64 = 6778.137;
        let v = (398600.4418_f64 / r).sqrt();
        OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0))
    }

    fn sso_state() -> OrbitalState {
        let r: f64 = 6378.137 + 800.0;
        let v = (398600.4418_f64 / r).sqrt();
        OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0))
    }

    #[test]
    fn derivatives_match_individual() {
        let mu = 398600.4418;
        let sys1 = TwoBodySystem { mu };
        let sys2 = TwoBodySystem { mu };
        let group_dyn = IndependentGroupDynamics::new(vec![sys1, sys2]);

        let s1 = iss_state();
        let s2 = sso_state();
        let group_state = GroupState::new(vec![s1.clone(), s2.clone()]);

        let group_deriv = group_dyn.derivatives(0.0, &group_state);

        // Compare against individual derivatives
        let d1 = TwoBodySystem { mu }.derivatives(0.0, &s1);
        let d2 = TwoBodySystem { mu }.derivatives(0.0, &s2);

        // Bit-identical
        assert_eq!(*group_deriv.states[0].position(), *d1.position());
        assert_eq!(*group_deriv.states[0].velocity(), *d1.velocity());
        assert_eq!(*group_deriv.states[1].position(), *d2.position());
        assert_eq!(*group_deriv.states[1].velocity(), *d2.velocity());
    }

    #[test]
    #[should_panic(expected = "dynamics count")]
    fn length_mismatch_panics() {
        let group_dyn = IndependentGroupDynamics::new(vec![TwoBodySystem { mu: 1.0 }]);
        let group_state = GroupState::new(vec![iss_state(), sso_state()]);
        let _ = group_dyn.derivatives(0.0, &group_state);
    }

    #[test]
    fn rk4_group_matches_individual() {
        let mu: f64 = 398600.4418;
        let dt: f64 = 10.0;
        let duration: f64 = 100.0;
        let n_steps = (duration / dt).round() as usize;

        // Individual propagation
        let sys1 = TwoBodySystem { mu };
        let sys2 = TwoBodySystem { mu };
        let mut s1 = iss_state();
        let mut s2 = sso_state();
        let mut t = 0.0;
        for _ in 0..n_steps {
            s1 = Rk4.step(&sys1, t, &s1, dt);
            s2 = Rk4.step(&sys2, t, &s2, dt);
            t += dt;
        }

        // Group propagation
        let group_dyn =
            IndependentGroupDynamics::new(vec![TwoBodySystem { mu }, TwoBodySystem { mu }]);
        let mut group_state = GroupState::new(vec![iss_state(), sso_state()]);
        t = 0.0;
        for _ in 0..n_steps {
            group_state = Rk4.step(&group_dyn, t, &group_state, dt);
            t += dt;
        }

        // Should be bit-identical (same floating-point operations in same order)
        assert_eq!(*group_state.states[0].position(), *s1.position());
        assert_eq!(*group_state.states[0].velocity(), *s1.velocity());
        assert_eq!(*group_state.states[1].position(), *s2.position());
        assert_eq!(*group_state.states[1].velocity(), *s2.velocity());
    }

    #[test]
    fn dp45_group_matches_individual() {
        let mu = 398600.4418;
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let duration = 100.0;
        let no_event = |_t: f64, _s: &OrbitalState| ControlFlow::<()>::Continue(());
        let no_event_group =
            |_t: f64, _s: &GroupState<OrbitalState>| ControlFlow::<()>::Continue(());

        // Individual DP45
        let outcome1 = DormandPrince.integrate_adaptive_with_events(
            &TwoBodySystem { mu },
            iss_state(),
            0.0,
            duration,
            10.0,
            &tol,
            |_, _| {},
            no_event,
        );
        let s1 = match outcome1 {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("ISS integration failed"),
        };

        let outcome2 = DormandPrince.integrate_adaptive_with_events(
            &TwoBodySystem { mu },
            sso_state(),
            0.0,
            duration,
            10.0,
            &tol,
            |_, _| {},
            |_t: f64, _s: &OrbitalState| ControlFlow::<()>::Continue(()),
        );
        let s2 = match outcome2 {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("SSO integration failed"),
        };

        // Group DP45
        let group_dyn =
            IndependentGroupDynamics::new(vec![TwoBodySystem { mu }, TwoBodySystem { mu }]);
        let group_outcome = DormandPrince.integrate_adaptive_with_events(
            &group_dyn,
            GroupState::new(vec![iss_state(), sso_state()]),
            0.0,
            duration,
            10.0,
            &tol,
            |_, _| {},
            no_event_group,
        );
        let group_state = match group_outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("Group integration failed"),
        };

        // Group uses max error_norm across satellites, so step sizes may differ
        // from individual propagation. Check results match within tolerance.
        let pos_err_1 = (group_state.states[0].position() - s1.position()).magnitude();
        let pos_err_2 = (group_state.states[1].position() - s2.position()).magnitude();

        // Within tolerance (both should be ~1e-6 km or better)
        assert!(pos_err_1 < 1e-6, "ISS position difference: {pos_err_1} km");
        assert!(pos_err_2 < 1e-6, "SSO position difference: {pos_err_2} km");
    }

    #[test]
    fn harmonic_oscillator_energy_conservation() {
        // E = (v² + x²) / 2 should be conserved for each satellite
        let group_dyn = IndependentGroupDynamics::new(vec![HarmonicOscillator, HarmonicOscillator]);

        let s1 = OrbitalState::new(Vector3::new(1.0, 0.0, 0.0), Vector3::zeros());
        let s2 = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 2.0, 0.0));
        let initial = GroupState::new(vec![s1.clone(), s2.clone()]);

        let energy = |s: &OrbitalState| -> f64 {
            (s.velocity().magnitude_squared() + s.position().magnitude_squared()) / 2.0
        };
        let e0_1 = energy(&s1);
        let e0_2 = energy(&s2);

        let dt = 0.01;
        let mut state = initial;
        let mut t = 0.0;
        for _ in 0..1000 {
            state = Rk4.step(&group_dyn, t, &state, dt);
            t += dt;

            let e1 = energy(&state.states[0]);
            let e2 = energy(&state.states[1]);
            assert!(
                (e1 - e0_1).abs() / e0_1 < 1e-9,
                "satellite 0 energy drift at t={t}: {e1} vs {e0_1}"
            );
            assert!(
                (e2 - e0_2).abs() / e0_2 < 1e-9,
                "satellite 1 energy drift at t={t}: {e2} vs {e0_2}"
            );
        }
    }

    #[test]
    fn two_uniform_motions_with_different_velocities() {
        let sys1 = UniformMotion {
            velocity: Vector3::new(1.0, 0.0, 0.0),
        };
        let sys2 = UniformMotion {
            velocity: Vector3::new(0.0, 2.0, 0.0),
        };
        let group_dyn = IndependentGroupDynamics::new(vec![sys1, sys2]);

        let s1 = OrbitalState::new(Vector3::zeros(), Vector3::new(1.0, 0.0, 0.0));
        let s2 = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 2.0, 0.0));
        let initial = GroupState::new(vec![s1, s2]);

        let dt = 1.0;
        let result = Rk4.step(&group_dyn, 0.0, &initial, dt);

        // Uniform motion: x(t) = x0 + v*t (exact for any RK method)
        assert!((result.states[0].position()[0] - 1.0).abs() < 1e-15);
        assert!((result.states[1].position()[1] - 2.0).abs() < 1e-15);
    }
}
