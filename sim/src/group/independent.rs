use std::ops::ControlFlow;

use orts_integrator::{
    AdvanceOutcome, DynamicalSystem, DormandPrince, IntegrationError, OdeState, Tolerances,
};

use super::HasPosition;
use super::prop_group::{GroupSnapshot, PropGroupOutcome, SatId, SatelliteTermination};

/// Entry tracking an individual satellite's state and status.
pub struct SatelliteEntry<S: OdeState> {
    pub id: SatId,
    pub state: S,
    pub t: f64,
    pub terminated: bool,
}

/// Group of independently propagated satellites, each with its own stepper.
///
/// Each satellite has its own `DynamicalSystem` instance and adaptive step size.
/// This matches the current CLI serve-mode behavior where satellites are
/// propagated independently in chunks.
pub struct IndependentGroup<D: DynamicalSystem>
where
    D::State: HasPosition,
{
    satellites: Vec<(SatelliteEntry<D::State>, D)>,
    tolerances: Tolerances,
    dt: f64,
}

impl<D: DynamicalSystem> IndependentGroup<D>
where
    D::State: HasPosition,
{
    pub fn new(dt: f64, tolerances: Tolerances) -> Self {
        Self {
            satellites: Vec::new(),
            tolerances,
            dt,
        }
    }

    pub fn add_satellite(mut self, id: impl Into<SatId>, state: D::State, dynamics: D) -> Self {
        let entry = SatelliteEntry {
            id: id.into(),
            state,
            t: 0.0,
            terminated: false,
        };
        self.satellites.push((entry, dynamics));
        self
    }

    pub fn add_satellite_at(
        mut self,
        id: impl Into<SatId>,
        state: D::State,
        t0: f64,
        dynamics: D,
    ) -> Self {
        let entry = SatelliteEntry {
            id: id.into(),
            state,
            t: t0,
            terminated: false,
        };
        self.satellites.push((entry, dynamics));
        self
    }

    /// Access the satellite entries (read-only).
    pub fn satellites(&self) -> impl Iterator<Item = &SatelliteEntry<D::State>> {
        self.satellites.iter().map(|(entry, _)| entry)
    }
}

impl<D: DynamicalSystem + Send> super::prop_group::PropGroup for IndependentGroup<D>
where
    D::State: HasPosition + Send,
{
    fn ids(&self) -> Vec<SatId> {
        self.satellites.iter().map(|(e, _)| e.id.clone()).collect()
    }

    fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError> {
        let mut terminations = Vec::new();

        for (entry, dynamics) in &mut self.satellites {
            if entry.terminated || entry.t >= t_target {
                continue;
            }

            let dp = DormandPrince;
            let mut stepper = dp.stepper(
                dynamics,
                entry.state.clone(),
                entry.t,
                self.dt,
                self.tolerances.clone(),
            );

            match stepper.advance_to(
                t_target,
                |_, _| {},
                |_, _| ControlFlow::<&str>::Continue(()),
            ) {
                Ok(AdvanceOutcome::Reached) => {
                    entry.state = stepper.into_state();
                    entry.t = t_target;
                }
                Ok(AdvanceOutcome::Event { reason }) => {
                    // Event callback never breaks in this impl, so unreachable
                    let t = stepper.t();
                    entry.state = stepper.into_state();
                    entry.t = t;
                    entry.terminated = true;
                    terminations.push(SatelliteTermination {
                        satellite_id: entry.id.clone(),
                        t,
                        reason: reason.to_string(),
                    });
                }
                Err(e) => {
                    // NonFiniteState or StepSizeTooSmall
                    entry.terminated = true;
                    let t = match &e {
                        IntegrationError::NonFiniteState { t } => *t,
                        IntegrationError::StepSizeTooSmall { t, .. } => *t,
                    };
                    terminations.push(SatelliteTermination {
                        satellite_id: entry.id.clone(),
                        t,
                        reason: format!("{e:?}"),
                    });
                }
            }
        }

        Ok(PropGroupOutcome { terminations })
    }

    fn snapshot(&self) -> GroupSnapshot {
        GroupSnapshot {
            positions: self
                .satellites
                .iter()
                .filter(|(e, _)| !e.terminated)
                .map(|(e, _)| (e.id.clone(), e.state.position()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;
    use orts_integrator::{IntegrationOutcome, State};
    use orts_orbits::two_body::TwoBodySystem;

    use super::super::prop_group::PropGroup;

    const MU_EARTH: f64 = 398600.4418;

    fn iss_state() -> State {
        let r: f64 = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        State {
            position: Vector3::new(r, 0.0, 0.0),
            velocity: Vector3::new(0.0, v, 0.0),
        }
    }

    fn sso_state() -> State {
        let r: f64 = 7178.137;
        let v = (MU_EARTH / r).sqrt();
        State {
            position: Vector3::new(r, 0.0, 0.0),
            velocity: Vector3::new(0.0, v, 0.0),
        }
    }

    fn default_tol() -> Tolerances {
        Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        }
    }

    #[test]
    fn single_satellite_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());

        // Should have advanced to t=100
        let entry = group.satellites().next().unwrap();
        assert!((entry.t - 100.0).abs() < 1e-9);
        assert!(!entry.terminated);
    }

    #[test]
    fn single_satellite_matches_direct_integration() {
        // Propagate with IndependentGroup
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });
        group.propagate_to(100.0).unwrap();
        let group_state = group.satellites().next().unwrap().state.clone();

        // Propagate directly with DP45
        let outcome = DormandPrince.integrate_adaptive_with_events(
            &TwoBodySystem { mu: MU_EARTH },
            iss_state(),
            0.0,
            100.0,
            10.0,
            &default_tol(),
            |_, _| {},
            |_: f64, _: &State| ControlFlow::<()>::Continue(()),
        );
        let direct_state = match outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("Direct integration failed"),
        };

        // Should match exactly (same algorithm, same parameters)
        let pos_err = (group_state.position - direct_state.position).magnitude();
        assert!(
            pos_err < 1e-12,
            "Position difference: {pos_err} km"
        );
    }

    #[test]
    fn two_satellites_independent_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());

        let entries: Vec<_> = group.satellites().collect();
        assert_eq!(entries.len(), 2);
        assert!((entries[0].t - 100.0).abs() < 1e-9);
        assert!((entries[1].t - 100.0).abs() < 1e-9);

        // Positions should differ (different orbits)
        let pos_diff = (entries[0].state.position - entries[1].state.position).magnitude();
        assert!(pos_diff > 1.0, "Different orbits should have different positions");
    }

    #[test]
    fn snapshot_returns_current_positions() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let snap = group.snapshot();
        assert_eq!(snap.positions.len(), 2);
        assert_eq!(snap.positions[0].0, SatId::from("iss"));
        assert_eq!(snap.positions[1].0, SatId::from("sso"));

        // Initial positions
        assert!((snap.positions[0].1[0] - 6778.137).abs() < 1e-10);
        assert!((snap.positions[1].1[0] - 7178.137).abs() < 1e-10);

        // After propagation
        group.propagate_to(100.0).unwrap();
        let snap2 = group.snapshot();
        assert_eq!(snap2.positions.len(), 2);
        // Positions should have changed
        assert!((snap2.positions[0].1 - snap.positions[0].1).magnitude() > 1.0);
    }

    #[test]
    fn ids_returns_correct_list() {
        let group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("alpha", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("beta", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let ids = group.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], SatId::from("alpha"));
        assert_eq!(ids[1], SatId::from("beta"));
    }

    #[test]
    fn propagate_to_already_at_target() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();
        let state_after_first = group.satellites().next().unwrap().state.clone();

        // Second call to same time should be a no-op
        group.propagate_to(100.0).unwrap();
        let state_after_second = group.satellites().next().unwrap().state.clone();
        assert_eq!(state_after_first.position, state_after_second.position);
    }

    #[test]
    fn multiple_propagate_steps() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        // Propagate in two steps: 0→50, 50→100
        group.propagate_to(50.0).unwrap();
        group.propagate_to(100.0).unwrap();
        let two_step = group.satellites().next().unwrap().state.clone();

        // Propagate in one step: 0→100
        let mut group2: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });
        group2.propagate_to(100.0).unwrap();
        let one_step = group2.satellites().next().unwrap().state.clone();

        // Stepper is recreated each propagate_to call (no FSAL persistence),
        // so step sizes may differ slightly. Results should still match
        // within adaptive tolerance (~1e-6 km).
        let pos_err = (two_step.position - one_step.position).magnitude();
        assert!(
            pos_err < 1e-6,
            "Two-step vs one-step position difference: {pos_err} km"
        );
    }

    #[test]
    fn nan_terminates_satellite_others_continue() {
        // Create a state at origin (will cause division by zero in TwoBody)
        let degenerate = State {
            position: Vector3::new(1e-15, 0.0, 0.0),
            velocity: Vector3::new(0.0, 1e10, 0.0),
        };
        let mut group2: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        let outcome = group2.propagate_to(100.0).unwrap();

        // The "bad" satellite should have terminated
        let entries: Vec<_> = group2.satellites().collect();
        let good = entries.iter().find(|e| e.id == SatId::from("good")).unwrap();
        let bad = entries.iter().find(|e| e.id == SatId::from("bad")).unwrap();

        assert!(!good.terminated, "Good satellite should not be terminated");
        assert!(bad.terminated, "Bad satellite should be terminated");
        assert!((good.t - 100.0).abs() < 1e-9, "Good satellite should reach t=100");

        // Outcome should report termination of bad satellite
        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("bad"));
    }

    #[test]
    fn terminated_satellite_skipped_on_next_propagate() {
        let degenerate = State {
            position: Vector3::new(1e-15, 0.0, 0.0),
            velocity: Vector3::new(0.0, 1e10, 0.0),
        };
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        // First propagation: bad terminates
        group.propagate_to(50.0).unwrap();

        // Second propagation: bad should be skipped
        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty(), "No new terminations expected");

        let entries: Vec<_> = group.satellites().collect();
        let good = entries.iter().find(|e| e.id == SatId::from("good")).unwrap();
        assert!((good.t - 100.0).abs() < 1e-9);
    }

    #[test]
    fn snapshot_excludes_terminated() {
        let degenerate = State {
            position: Vector3::new(1e-15, 0.0, 0.0),
            velocity: Vector3::new(0.0, 1e10, 0.0),
        };
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        let snap = group.snapshot();
        assert_eq!(snap.positions.len(), 1, "Only non-terminated sats in snapshot");
        assert_eq!(snap.positions[0].0, SatId::from("good"));
    }

    #[test]
    fn builder_api() {
        let group: IndependentGroup<TwoBodySystem> = IndependentGroup::new(10.0, default_tol())
            .add_satellite("a", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite_at("b", sso_state(), 5.0, TwoBodySystem { mu: MU_EARTH });

        let entries: Vec<_> = group.satellites().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, SatId::from("a"));
        assert!((entries[0].t - 0.0).abs() < 1e-15);
        assert_eq!(entries[1].id, SatId::from("b"));
        assert!((entries[1].t - 5.0).abs() < 1e-15);
    }

    #[test]
    fn two_body_equivalence_iss_sso() {
        // Compare group propagation vs individual for ISS + SSO
        let duration: f64 = 1000.0;

        // Individual propagation
        let iss_outcome = DormandPrince.integrate_adaptive_with_events(
            &TwoBodySystem { mu: MU_EARTH },
            iss_state(),
            0.0,
            duration,
            10.0,
            &default_tol(),
            |_, _| {},
            |_: f64, _: &State| ControlFlow::<()>::Continue(()),
        );
        let iss_direct = match iss_outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("ISS direct failed"),
        };

        let sso_outcome = DormandPrince.integrate_adaptive_with_events(
            &TwoBodySystem { mu: MU_EARTH },
            sso_state(),
            0.0,
            duration,
            10.0,
            &default_tol(),
            |_, _| {},
            |_: f64, _: &State| ControlFlow::<()>::Continue(()),
        );
        let sso_direct = match sso_outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("SSO direct failed"),
        };

        // Group propagation
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::new(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });
        group.propagate_to(duration).unwrap();

        let entries: Vec<_> = group.satellites().collect();
        let iss_group = &entries[0].state;
        let sso_group = &entries[1].state;

        let iss_pos_err = (iss_group.position - iss_direct.position).magnitude();
        let sso_pos_err = (sso_group.position - sso_direct.position).magnitude();

        // Should match (same adaptive algorithm, no inter-satellite coupling)
        assert!(
            iss_pos_err < 1e-12,
            "ISS position error: {iss_pos_err} km"
        );
        assert!(
            sso_pos_err < 1e-12,
            "SSO position error: {sso_pos_err} km"
        );
    }
}
