use utsuroi::{OdeState, Tolerances};

/// Composite ODE state for N satellites.
///
/// Wraps `Vec<S>` and implements `OdeState` via element-wise delegation.
/// Used by coupled-regime integrators (C3) where all satellites share
/// a single adaptive stepper. The error norm takes the max across
/// per-satellite norms so no individual satellite's error is ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupState<S: OdeState> {
    pub states: Vec<S>,
}

impl<S: OdeState> GroupState<S> {
    pub fn new(states: Vec<S>) -> Self {
        Self { states }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

impl<S: OdeState> OdeState for GroupState<S> {
    fn zero_like(&self) -> Self {
        GroupState {
            states: self.states.iter().map(|s| s.zero_like()).collect(),
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        assert_eq!(
            self.states.len(),
            other.states.len(),
            "GroupState::axpy: length mismatch ({} vs {})",
            self.states.len(),
            other.states.len()
        );
        GroupState {
            states: self
                .states
                .iter()
                .zip(&other.states)
                .map(|(a, b)| a.axpy(scale, b))
                .collect(),
        }
    }

    fn scale(&self, factor: f64) -> Self {
        GroupState {
            states: self.states.iter().map(|s| s.scale(factor)).collect(),
        }
    }

    fn is_finite(&self) -> bool {
        self.states.iter().all(|s| s.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        if self.states.is_empty() {
            return 0.0;
        }
        self.states
            .iter()
            .zip(&y_next.states)
            .zip(&error.states)
            .map(|((yn, ynext), err)| yn.error_norm(ynext, err, tol))
            .fold(0.0_f64, f64::max)
    }

    fn project(&mut self, t: f64) {
        for s in &mut self.states {
            s.project(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use nalgebra::{Vector3, Vector4};

    use crate::SpacecraftState;

    fn orbit_state(x: f64, vx: f64) -> OrbitalState {
        OrbitalState::new(Vector3::new(x, 0.0, 0.0), Vector3::new(vx, 0.0, 0.0))
    }

    fn two_orbit_group() -> GroupState<OrbitalState> {
        GroupState::new(vec![orbit_state(7000.0, 0.0), orbit_state(7200.0, 1.0)])
    }

    #[test]
    fn zero_like_produces_zeros() {
        let group = two_orbit_group();
        let zero = group.zero_like();
        assert_eq!(zero.len(), 2);
        for s in &zero.states {
            assert_eq!(*s.position(), Vector3::zeros());
            assert_eq!(*s.velocity(), Vector3::zeros());
        }
    }

    #[test]
    fn axpy_element_wise() {
        let a = GroupState::new(vec![orbit_state(1.0, 2.0), orbit_state(3.0, 4.0)]);
        let b = GroupState::new(vec![orbit_state(10.0, 20.0), orbit_state(30.0, 40.0)]);
        let result = a.axpy(0.5, &b);
        assert!((result.states[0].position()[0] - 6.0).abs() < 1e-15);
        assert!((result.states[0].velocity()[0] - 12.0).abs() < 1e-15);
        assert!((result.states[1].position()[0] - 18.0).abs() < 1e-15);
        assert!((result.states[1].velocity()[0] - 24.0).abs() < 1e-15);
    }

    #[test]
    #[should_panic(expected = "length mismatch")]
    fn axpy_length_mismatch_panics() {
        let a = GroupState::new(vec![orbit_state(1.0, 2.0)]);
        let b = GroupState::new(vec![orbit_state(1.0, 2.0), orbit_state(3.0, 4.0)]);
        let _ = a.axpy(1.0, &b);
    }

    #[test]
    fn scale_element_wise() {
        let group = GroupState::new(vec![orbit_state(10.0, 20.0), orbit_state(30.0, 40.0)]);
        let scaled = group.scale(0.5);
        assert!((scaled.states[0].position()[0] - 5.0).abs() < 1e-15);
        assert!((scaled.states[1].position()[0] - 15.0).abs() < 1e-15);
    }

    #[test]
    fn scale_zero_gives_zeros() {
        let group = two_orbit_group();
        let scaled = group.scale(0.0);
        for s in &scaled.states {
            assert_eq!(*s.position(), Vector3::zeros());
            assert_eq!(*s.velocity(), Vector3::zeros());
        }
    }

    #[test]
    fn is_finite_all_normal() {
        assert!(two_orbit_group().is_finite());
    }

    #[test]
    fn is_finite_nan_in_one_element() {
        let mut group = two_orbit_group();
        group.states[1].position_mut()[0] = f64::NAN;
        assert!(!group.is_finite());
    }

    #[test]
    fn error_norm_max_of_per_satellite() {
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        // Satellite 0: small error, Satellite 1: large error
        let y_n = GroupState::new(vec![orbit_state(7000.0, 7.5), orbit_state(7200.0, 7.3)]);
        let y_next = y_n.clone();
        let error = GroupState::new(vec![
            orbit_state(1e-12, 1e-12), // tiny error
            orbit_state(1.0, 0.01),    // large error
        ]);

        let group_norm = y_n.error_norm(&y_next, &error, &tol);
        let sat1_norm = y_n.states[1].error_norm(&y_next.states[1], &error.states[1], &tol);

        // Group norm should equal the worst (satellite 1)
        assert!((group_norm - sat1_norm).abs() < 1e-10);
        assert!(group_norm > 0.0);
    }

    #[test]
    fn error_norm_single_element_matches_raw() {
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let state = orbit_state(7000.0, 7.5);
        let error_state = orbit_state(0.001, 0.0001);

        let raw_norm = state.error_norm(&state, &error_state, &tol);
        let group = GroupState::new(vec![state.clone()]);
        let group_error = GroupState::new(vec![error_state]);
        let group_norm = group.error_norm(&group, &group_error, &tol);

        assert!((raw_norm - group_norm).abs() < 1e-15);
    }

    #[test]
    fn error_norm_empty_returns_zero() {
        let tol = Tolerances::default();
        let empty: GroupState<OrbitalState> = GroupState::new(vec![]);
        assert_eq!(empty.error_norm(&empty, &empty, &tol), 0.0);
    }

    #[test]
    fn is_finite_empty_returns_true() {
        let empty: GroupState<OrbitalState> = GroupState::new(vec![]);
        assert!(empty.is_finite());
    }

    #[test]
    fn project_delegates_to_elements() {
        // SpacecraftState.project() normalizes the quaternion
        let sc = SpacecraftState {
            orbit: orbit_state(7000.0, 7.5),
            attitude: AttitudeState {
                quaternion: Vector4::new(2.0, 0.0, 0.0, 0.0), // unnormalized
                angular_velocity: Vector3::new(0.1, 0.0, 0.0),
            },
            mass: 500.0,
        };
        let mut group = GroupState::new(vec![sc]);
        group.project(0.0);
        let q_norm = group.states[0].attitude.quaternion.magnitude();
        assert!((q_norm - 1.0).abs() < 1e-15);
    }

    #[test]
    fn len_and_is_empty() {
        let group = two_orbit_group();
        assert_eq!(group.len(), 2);
        assert!(!group.is_empty());

        let empty: GroupState<OrbitalState> = GroupState::new(vec![]);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }
}
