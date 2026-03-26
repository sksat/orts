use std::ops::ControlFlow;

use utsuroi::{
    AdvanceOutcome, AdvanceOutcome853, Dop853, DormandPrince, DynamicalSystem, IntegrationError,
    Integrator, OdeState, Rk4, Tolerances,
};

use super::HasPosition;
use super::prop_group::{GroupSnapshot, PropGroupOutcome, SatId, SatelliteTermination};

/// Integrator selection for `IndependentGroup`.
#[derive(Debug, Clone)]
pub enum IntegratorConfig {
    Rk4 { dt: f64 },
    Dp45 { dt: f64, tolerances: Tolerances },
    Dop853 { dt: f64, tolerances: Tolerances },
}

/// Entry tracking an individual satellite's state and status.
pub struct SatelliteEntry<S: OdeState> {
    pub id: SatId,
    pub state: S,
    pub t: f64,
    pub terminated: bool,
    pub end_time: Option<f64>,
}

/// Extracted satellite data returned by [`IndependentGroup::into_parts`].
///
/// Contains everything needed to reconstruct an `IndependentGroup` or
/// transfer satellites to a different group type (e.g., `CoupledGroup`).
pub struct SatelliteParts<S, D> {
    pub id: SatId,
    pub state: S,
    pub t: f64,
    pub terminated: bool,
    pub end_time: Option<f64>,
    pub dynamics: D,
}

/// Group of independently propagated satellites, each with its own stepper.
///
/// Each satellite has its own `DynamicalSystem` instance and adaptive step size.
/// This matches the current CLI serve-mode behavior where satellites are
/// propagated independently in chunks.
/// Event checker callback type for satellite termination events.
type EventChecker<S> = Box<dyn Fn(f64, &S) -> ControlFlow<String> + Send>;

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
    integrator: IntegratorConfig,
    event_checker: Option<EventChecker<D::State>>,
}

impl<D: DynamicalSystem> IndependentGroup<D>
where
    D::State: HasPosition,
{
    pub fn new(integrator: IntegratorConfig) -> Self {
        Self {
            satellites: Vec::new(),
            integrator,
            event_checker: None,
        }
    }

    /// Create with Dormand-Prince 4/5 adaptive integrator.
    pub fn dp45(dt: f64, tolerances: Tolerances) -> Self {
        Self::new(IntegratorConfig::Dp45 { dt, tolerances })
    }

    /// Create with DOP853 (8th-order Dormand-Prince) adaptive integrator.
    pub fn dop853(dt: f64, tolerances: Tolerances) -> Self {
        Self::new(IntegratorConfig::Dop853 { dt, tolerances })
    }

    /// Create with fixed-step RK4 integrator.
    pub fn rk4(dt: f64) -> Self {
        Self::new(IntegratorConfig::Rk4 { dt })
    }

    /// Set an event checker that is called after each integration step.
    ///
    /// If the checker returns `ControlFlow::Break(reason)`, the satellite is
    /// terminated with that reason string.
    pub fn with_event_checker(
        mut self,
        checker: impl Fn(f64, &D::State) -> ControlFlow<String> + Send + 'static,
    ) -> Self {
        self.event_checker = Some(Box::new(checker));
        self
    }

    pub fn add_satellite(mut self, id: impl Into<SatId>, state: D::State, dynamics: D) -> Self {
        let entry = SatelliteEntry {
            id: id.into(),
            state,
            t: 0.0,
            terminated: false,
            end_time: None,
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
            end_time: None,
        };
        self.satellites.push((entry, dynamics));
        self
    }

    /// Add a satellite with a finite end time.
    ///
    /// Propagation will stop at `end_time` even if the group target is later.
    pub fn add_satellite_until(
        mut self,
        id: impl Into<SatId>,
        state: D::State,
        end_time: f64,
        dynamics: D,
    ) -> Self {
        let entry = SatelliteEntry {
            id: id.into(),
            state,
            t: 0.0,
            terminated: false,
            end_time: Some(end_time),
        };
        self.satellites.push((entry, dynamics));
        self
    }

    /// Access the satellite entries (read-only).
    pub fn satellites(&self) -> impl Iterator<Item = &SatelliteEntry<D::State>> {
        self.satellites.iter().map(|(entry, _)| entry)
    }

    /// Access satellite entries together with their dynamics (read-only).
    pub fn satellites_with_dynamics(
        &self,
    ) -> impl Iterator<Item = (&SatelliteEntry<D::State>, &D)> {
        self.satellites
            .iter()
            .map(|(entry, dyn_sys)| (entry, dyn_sys))
    }

    /// Look up a single satellite by id.
    pub fn satellite(&self, id: &SatId) -> Option<&SatelliteEntry<D::State>> {
        self.satellites
            .iter()
            .find(|(e, _)| &e.id == id)
            .map(|(e, _)| e)
    }

    /// Replace a satellite's state (keeping its current `t` and `terminated` flag).
    pub fn reset_state(&mut self, id: &SatId, new_state: D::State) {
        if let Some((entry, _)) = self.satellites.iter_mut().find(|(e, _)| &e.id == id) {
            entry.state = new_state;
        }
    }

    /// Consume the group, returning all satellite data and dynamics.
    ///
    /// Used by the Scheduler to recover state and dynamics after ephemeral
    /// group propagation.
    pub fn into_parts(self) -> Vec<SatelliteParts<D::State, D>> {
        self.satellites
            .into_iter()
            .map(|(entry, dynamics)| SatelliteParts {
                id: entry.id,
                state: entry.state,
                t: entry.t,
                terminated: entry.terminated,
                end_time: entry.end_time,
                dynamics,
            })
            .collect()
    }

    /// Returns `true` if every satellite is either terminated or has reached its `end_time`.
    pub fn all_finished(&self) -> bool {
        self.satellites.iter().all(|(entry, _)| {
            entry.terminated || entry.end_time.is_some_and(|et| entry.t >= et - 1e-9)
        })
    }

    /// Add a satellite to an already-constructed group (mutable reference).
    ///
    /// Unlike the builder methods, this allows specifying both start time and end time.
    /// Used by the Scheduler when building ephemeral groups.
    pub fn push_satellite(
        &mut self,
        id: impl Into<SatId>,
        state: D::State,
        t: f64,
        end_time: Option<f64>,
        dynamics: D,
    ) {
        let entry = SatelliteEntry {
            id: id.into(),
            state,
            t,
            terminated: false,
            end_time,
        };
        self.satellites.push((entry, dynamics));
    }

    /// Add a satellite to an already-constructed group at a specified time (mutable reference).
    ///
    /// Convenience wrapper around [`push_satellite`] with no end time.
    /// Used for dynamic satellite addition in serve mode.
    pub fn push_satellite_at(
        &mut self,
        id: impl Into<SatId>,
        state: D::State,
        t0: f64,
        dynamics: D,
    ) {
        self.push_satellite(id, state, t0, None, dynamics);
    }

    pub fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError> {
        let mut terminations = Vec::new();
        let integrator = self.integrator.clone();
        let event_checker = &self.event_checker;

        for (entry, dynamics) in &mut self.satellites {
            if entry.terminated || entry.t >= t_target {
                continue;
            }

            // Clamp to per-satellite end_time if set
            let effective_target = match entry.end_time {
                Some(et) => t_target.min(et),
                None => t_target,
            };

            if entry.t >= effective_target {
                continue;
            }

            match &integrator {
                IntegratorConfig::Dp45 { dt, tolerances } => {
                    let mut stepper = DormandPrince.stepper(
                        dynamics,
                        entry.state.clone(),
                        entry.t,
                        *dt,
                        tolerances.clone(),
                    );

                    let result = if let Some(checker) = event_checker {
                        stepper.advance_to(effective_target, |_, _| {}, |t, s| checker(t, s))
                    } else {
                        stepper.advance_to(
                            effective_target,
                            |_, _| {},
                            |_, _| ControlFlow::<String>::Continue(()),
                        )
                    };

                    match result {
                        Ok(AdvanceOutcome::Reached) => {
                            entry.state = stepper.into_state();
                            entry.t = effective_target;
                        }
                        Ok(AdvanceOutcome::Event { reason }) => {
                            let t = stepper.t();
                            entry.state = stepper.into_state();
                            entry.t = t;
                            entry.terminated = true;
                            terminations.push(SatelliteTermination {
                                satellite_id: entry.id.clone(),
                                t,
                                reason,
                            });
                        }
                        Err(e) => {
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
                IntegratorConfig::Dop853 { dt, tolerances } => {
                    let mut stepper = Dop853.stepper(
                        dynamics,
                        entry.state.clone(),
                        entry.t,
                        *dt,
                        tolerances.clone(),
                    );

                    let result = if let Some(checker) = event_checker {
                        stepper.advance_to(effective_target, |_, _| {}, |t, s| checker(t, s))
                    } else {
                        stepper.advance_to(
                            effective_target,
                            |_, _| {},
                            |_, _| ControlFlow::<String>::Continue(()),
                        )
                    };

                    match result {
                        Ok(AdvanceOutcome853::Reached) => {
                            entry.state = stepper.into_state();
                            entry.t = effective_target;
                        }
                        Ok(AdvanceOutcome853::Event { reason }) => {
                            let t = stepper.t();
                            entry.state = stepper.into_state();
                            entry.t = t;
                            entry.terminated = true;
                            terminations.push(SatelliteTermination {
                                satellite_id: entry.id.clone(),
                                t,
                                reason,
                            });
                        }
                        Err(e) => {
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
                IntegratorConfig::Rk4 { dt } => {
                    let dt = *dt;
                    let mut current_t = entry.t;
                    let mut current_state = entry.state.clone();

                    let mut terminated = false;
                    while current_t < effective_target - 1e-12 {
                        let h = dt.min(effective_target - current_t);
                        current_state = Rk4.step(dynamics, current_t, &current_state, h);
                        current_t += h;

                        if !current_state.is_finite() {
                            entry.t = current_t;
                            entry.terminated = true;
                            terminated = true;
                            terminations.push(SatelliteTermination {
                                satellite_id: entry.id.clone(),
                                t: current_t,
                                reason: "NonFiniteState".to_string(),
                            });
                            break;
                        }

                        if let Some(checker) = event_checker
                            && let ControlFlow::Break(reason) = checker(current_t, &current_state)
                        {
                            entry.t = current_t;
                            entry.terminated = true;
                            terminated = true;
                            terminations.push(SatelliteTermination {
                                satellite_id: entry.id.clone(),
                                t: current_t,
                                reason,
                            });
                            break;
                        }
                    }

                    entry.state = current_state;
                    if !terminated {
                        entry.t = current_t;
                    }
                }
            }
        }

        Ok(PropGroupOutcome { terminations })
    }

    pub fn snapshot(&self) -> GroupSnapshot {
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

impl<D: DynamicalSystem + Send> super::prop_group::PropGroup for IndependentGroup<D>
where
    D::State: HasPosition + Send,
{
    fn ids(&self) -> Vec<SatId> {
        self.satellites.iter().map(|(e, _)| e.id.clone()).collect()
    }

    fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError> {
        self.propagate_to(t_target)
    }

    fn snapshot(&self) -> GroupSnapshot {
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::orbital::two_body::TwoBodySystem;
    use nalgebra::Vector3;
    use utsuroi::IntegrationOutcome;

    use super::super::prop_group::PropGroup;

    const MU_EARTH: f64 = 398600.4418;

    fn iss_state() -> OrbitalState {
        let r: f64 = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0))
    }

    fn sso_state() -> OrbitalState {
        let r: f64 = 7178.137;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0))
    }

    fn default_tol() -> Tolerances {
        Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        }
    }

    #[test]
    fn single_satellite_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
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
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
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
            |_: f64, _: &OrbitalState| ControlFlow::<()>::Continue(()),
        );
        let direct_state = match outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("Direct integration failed"),
        };

        // Should match exactly (same algorithm, same parameters)
        let pos_err = (*group_state.position() - *direct_state.position()).magnitude();
        assert!(pos_err < 1e-12, "Position difference: {pos_err} km");
    }

    #[test]
    fn two_satellites_independent_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());

        let entries: Vec<_> = group.satellites().collect();
        assert_eq!(entries.len(), 2);
        assert!((entries[0].t - 100.0).abs() < 1e-9);
        assert!((entries[1].t - 100.0).abs() < 1e-9);

        // Positions should differ (different orbits)
        let pos_diff = (*entries[0].state.position() - *entries[1].state.position()).magnitude();
        assert!(
            pos_diff > 1.0,
            "Different orbits should have different positions"
        );
    }

    #[test]
    fn snapshot_returns_current_positions() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
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
        let group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(10.0, default_tol())
            .add_satellite("alpha", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("beta", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let ids = group.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], SatId::from("alpha"));
        assert_eq!(ids[1], SatId::from("beta"));
    }

    #[test]
    fn propagate_to_already_at_target() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();
        let state_after_first = group.satellites().next().unwrap().state.clone();

        // Second call to same time should be a no-op
        group.propagate_to(100.0).unwrap();
        let state_after_second = group.satellites().next().unwrap().state.clone();
        assert_eq!(state_after_first.position(), state_after_second.position());
    }

    #[test]
    fn multiple_propagate_steps() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        // Propagate in two steps: 0→50, 50→100
        group.propagate_to(50.0).unwrap();
        group.propagate_to(100.0).unwrap();
        let two_step = group.satellites().next().unwrap().state.clone();

        // Propagate in one step: 0→100
        let mut group2: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });
        group2.propagate_to(100.0).unwrap();
        let one_step = group2.satellites().next().unwrap().state.clone();

        // Stepper is recreated each propagate_to call (no FSAL persistence),
        // so step sizes may differ slightly. Results should still match
        // within adaptive tolerance (~1e-6 km).
        let pos_err = (*two_step.position() - *one_step.position()).magnitude();
        assert!(
            pos_err < 1e-6,
            "Two-step vs one-step position difference: {pos_err} km"
        );
    }

    #[test]
    fn nan_terminates_satellite_others_continue() {
        // Create a state at origin (will cause division by zero in TwoBody)
        let degenerate =
            OrbitalState::new(Vector3::new(1e-15, 0.0, 0.0), Vector3::new(0.0, 1e10, 0.0));
        let mut group2: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        let outcome = group2.propagate_to(100.0).unwrap();

        // The "bad" satellite should have terminated
        let entries: Vec<_> = group2.satellites().collect();
        let good = entries
            .iter()
            .find(|e| e.id == SatId::from("good"))
            .unwrap();
        let bad = entries.iter().find(|e| e.id == SatId::from("bad")).unwrap();

        assert!(!good.terminated, "Good satellite should not be terminated");
        assert!(bad.terminated, "Bad satellite should be terminated");
        assert!(
            (good.t - 100.0).abs() < 1e-9,
            "Good satellite should reach t=100"
        );

        // Outcome should report termination of bad satellite
        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("bad"));
    }

    #[test]
    fn terminated_satellite_skipped_on_next_propagate() {
        let degenerate =
            OrbitalState::new(Vector3::new(1e-15, 0.0, 0.0), Vector3::new(0.0, 1e10, 0.0));
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        // First propagation: bad terminates
        group.propagate_to(50.0).unwrap();

        // Second propagation: bad should be skipped
        let outcome = group.propagate_to(100.0).unwrap();
        assert!(
            outcome.terminations.is_empty(),
            "No new terminations expected"
        );

        let entries: Vec<_> = group.satellites().collect();
        let good = entries
            .iter()
            .find(|e| e.id == SatId::from("good"))
            .unwrap();
        assert!((good.t - 100.0).abs() < 1e-9);
    }

    #[test]
    fn snapshot_excludes_terminated() {
        let degenerate =
            OrbitalState::new(Vector3::new(1e-15, 0.0, 0.0), Vector3::new(0.0, 1e10, 0.0));
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        let snap = group.snapshot();
        assert_eq!(
            snap.positions.len(),
            1,
            "Only non-terminated sats in snapshot"
        );
        assert_eq!(snap.positions[0].0, SatId::from("good"));
    }

    #[test]
    fn builder_api() {
        let group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(10.0, default_tol())
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
            |_: f64, _: &OrbitalState| ControlFlow::<()>::Continue(()),
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
            |_: f64, _: &OrbitalState| ControlFlow::<()>::Continue(()),
        );
        let sso_direct = match sso_outcome {
            IntegrationOutcome::Completed(s) => s,
            _ => panic!("SSO direct failed"),
        };

        // Group propagation
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });
        group.propagate_to(duration).unwrap();

        let entries: Vec<_> = group.satellites().collect();
        let iss_group = &entries[0].state;
        let sso_group = &entries[1].state;

        let iss_pos_err = (*iss_group.position() - *iss_direct.position()).magnitude();
        let sso_pos_err = (*sso_group.position() - *sso_direct.position()).magnitude();

        // Should match (same adaptive algorithm, no inter-satellite coupling)
        assert!(iss_pos_err < 1e-12, "ISS position error: {iss_pos_err} km");
        assert!(sso_pos_err < 1e-12, "SSO position error: {sso_pos_err} km");
    }

    // --- RK4 tests ---

    #[test]
    fn rk4_single_satellite_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0).add_satellite(
            "iss",
            iss_state(),
            TwoBodySystem { mu: MU_EARTH },
        );

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());

        let entry = group.satellites().next().unwrap();
        assert!((entry.t - 100.0).abs() < 1e-9);
        assert!(!entry.terminated);
    }

    #[test]
    fn rk4_matches_direct_step() {
        // Propagate with IndependentGroup RK4
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0).add_satellite(
            "iss",
            iss_state(),
            TwoBodySystem { mu: MU_EARTH },
        );
        group.propagate_to(100.0).unwrap();
        let group_state = group.satellites().next().unwrap().state.clone();

        // Propagate directly with Rk4.step
        let sys = TwoBodySystem { mu: MU_EARTH };
        let mut state = iss_state();
        let mut t: f64 = 0.0;
        let dt: f64 = 10.0;
        while t < 100.0 - 1e-12 {
            let h = dt.min(100.0 - t);
            state = Rk4.step(&sys, t, &state, h);
            t += h;
        }

        let pos_err = (*group_state.position() - *state.position()).magnitude();
        assert!(pos_err < 1e-12, "RK4 position difference: {pos_err} km");
    }

    #[test]
    fn rk4_two_satellites() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0)
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());

        let entries: Vec<_> = group.satellites().collect();
        assert_eq!(entries.len(), 2);
        assert!((entries[0].t - 100.0).abs() < 1e-9);
        assert!((entries[1].t - 100.0).abs() < 1e-9);

        let pos_diff = (*entries[0].state.position() - *entries[1].state.position()).magnitude();
        assert!(pos_diff > 1.0, "Different orbits should diverge");
    }

    #[test]
    fn rk4_nan_terminates_satellite() {
        let degenerate = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 1.0, 0.0));
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0)
            .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();

        let entries: Vec<_> = group.satellites().collect();
        let good = entries
            .iter()
            .find(|e| e.id == SatId::from("good"))
            .unwrap();
        let bad = entries.iter().find(|e| e.id == SatId::from("bad")).unwrap();

        assert!(!good.terminated);
        assert!(bad.terminated);
        assert!((good.t - 100.0).abs() < 1e-9);
        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("bad"));
    }

    #[test]
    fn rk4_energy_conservation() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0).add_satellite(
            "iss",
            iss_state(),
            TwoBodySystem { mu: MU_EARTH },
        );

        let r0 = iss_state().position().magnitude();
        let v0 = iss_state().velocity().magnitude();
        let initial_energy = v0 * v0 / 2.0 - MU_EARTH / r0;

        // Propagate for 500s
        group.propagate_to(500.0).unwrap();

        let entry = group.satellites().next().unwrap();
        let r = entry.state.position().magnitude();
        let v = entry.state.velocity().magnitude();
        let final_energy = v * v / 2.0 - MU_EARTH / r;

        // RK4 with dt=10 on circular orbit: energy drift < 1e-6
        assert!(
            (final_energy - initial_energy).abs() < 1e-6,
            "Energy drift: {:.2e}",
            (final_energy - initial_energy).abs()
        );
    }

    // --- Event checker tests ---

    const EARTH_RADIUS: f64 = 6378.137;

    fn collision_checker() -> impl Fn(f64, &OrbitalState) -> ControlFlow<String> + Send + 'static {
        move |_t: f64, state: &OrbitalState| {
            let r = state.position().magnitude();
            if r < EARTH_RADIUS {
                ControlFlow::Break(format!("collision at {:.1} km", r - EARTH_RADIUS))
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    #[test]
    fn dp45_event_terminates_satellite() {
        // Give satellite a decaying trajectory (towards Earth center)
        let decaying =
            OrbitalState::new(Vector3::new(6500.0, 0.0, 0.0), Vector3::new(-5.0, 3.0, 0.0));

        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .with_event_checker(collision_checker())
                .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(10000.0).unwrap();

        assert_eq!(outcome.terminations.len(), 1);
        assert!(outcome.terminations[0].reason.contains("collision"));

        let entry = group.satellites().next().unwrap();
        assert!(entry.terminated);
        assert!(entry.t < 10000.0, "Should terminate before target time");
    }

    #[test]
    fn rk4_event_terminates_satellite() {
        let decaying =
            OrbitalState::new(Vector3::new(6500.0, 0.0, 0.0), Vector3::new(-5.0, 3.0, 0.0));

        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(1.0)
            .with_event_checker(collision_checker())
            .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(10000.0).unwrap();

        assert_eq!(outcome.terminations.len(), 1);
        assert!(outcome.terminations[0].reason.contains("collision"));

        let entry = group.satellites().next().unwrap();
        assert!(entry.terminated);
        assert!(entry.t < 10000.0);
    }

    #[test]
    fn event_one_terminated_other_continues() {
        let decaying =
            OrbitalState::new(Vector3::new(6500.0, 0.0, 0.0), Vector3::new(-5.0, 3.0, 0.0));

        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .with_event_checker(collision_checker())
                .add_satellite("safe", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(1000.0).unwrap();

        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("decay"));

        let entries: Vec<_> = group.satellites().collect();
        let safe = entries
            .iter()
            .find(|e| e.id == SatId::from("safe"))
            .unwrap();
        let decay = entries
            .iter()
            .find(|e| e.id == SatId::from("decay"))
            .unwrap();

        assert!(!safe.terminated);
        assert!((safe.t - 1000.0).abs() < 1e-9);
        assert!(decay.terminated);
    }

    #[test]
    fn no_event_checker_default() {
        // Without event checker, satellite at low altitude just passes through
        let low = OrbitalState::new(
            Vector3::new(6500.0, 0.0, 0.0),
            Vector3::new(0.0, (MU_EARTH / 6500.0_f64).sqrt(), 0.0),
        );

        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("low", low, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());
        assert!(!group.satellites().next().unwrap().terminated);
    }

    // --- end_time + accessor tests ---

    #[test]
    fn end_time_clamps_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite_until("short", iss_state(), 50.0, TwoBodySystem { mu: MU_EARTH })
                .add_satellite("long", sso_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        let entries: Vec<_> = group.satellites().collect();
        let short = entries
            .iter()
            .find(|e| e.id == SatId::from("short"))
            .unwrap();
        let long = entries
            .iter()
            .find(|e| e.id == SatId::from("long"))
            .unwrap();

        assert!(
            (short.t - 50.0).abs() < 1e-9,
            "short should stop at end_time=50"
        );
        assert!((long.t - 100.0).abs() < 1e-9, "long should reach t=100");
    }

    #[test]
    fn different_end_times() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0)
            .add_satellite_until("a", iss_state(), 30.0, TwoBodySystem { mu: MU_EARTH })
            .add_satellite_until("b", sso_state(), 70.0, TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        let a = group.satellite(&SatId::from("a")).unwrap();
        let b = group.satellite(&SatId::from("b")).unwrap();

        assert!((a.t - 30.0).abs() < 1e-9);
        assert!((b.t - 70.0).abs() < 1e-9);
    }

    #[test]
    fn all_finished_with_end_times() {
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite_until("a", iss_state(), 50.0, TwoBodySystem { mu: MU_EARTH })
                .add_satellite_until("b", sso_state(), 100.0, TwoBodySystem { mu: MU_EARTH });

        assert!(!group.all_finished());
        group.propagate_to(50.0).unwrap();
        assert!(!group.all_finished()); // "b" still hasn't reached end_time
        group.propagate_to(100.0).unwrap();
        assert!(group.all_finished());
    }

    #[test]
    fn all_finished_with_termination() {
        let degenerate = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 1.0, 0.0));
        let mut group: IndependentGroup<TwoBodySystem> =
            IndependentGroup::dp45(10.0, default_tol())
                .add_satellite_until("good", iss_state(), 100.0, TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        // "bad" terminated, "good" reached end_time → all finished
        assert!(group.all_finished());
    }

    #[test]
    fn satellites_with_dynamics_accessor() {
        let group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(10.0, default_tol())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        let items: Vec<_> = group.satellites_with_dynamics().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.id, SatId::from("iss"));
        assert!((items[0].1.mu - MU_EARTH).abs() < 1e-6);
    }

    #[test]
    fn reset_state_changes_position() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();
        let t_before = group.satellite(&SatId::from("iss")).unwrap().t;

        // Reset state to initial
        group.reset_state(&SatId::from("iss"), iss_state());

        let entry = group.satellite(&SatId::from("iss")).unwrap();
        // t should be preserved
        assert!((entry.t - t_before).abs() < 1e-12);
        // Position should be back to initial
        assert!((entry.state.position().x - 6778.137).abs() < 1e-10);
    }

    // --- into_parts tests ---

    #[test]
    fn into_parts_preserves_state_and_dynamics() {
        let group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(10.0, default_tol())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite_until("sso", sso_state(), 50.0, TwoBodySystem { mu: MU_EARTH });

        let parts = group.into_parts();
        assert_eq!(parts.len(), 2);

        assert_eq!(parts[0].id, SatId::from("iss"));
        assert!((parts[0].state.position().x - 6778.137).abs() < 1e-10);
        assert!((parts[0].t - 0.0).abs() < 1e-15);
        assert!(!parts[0].terminated);
        assert!(parts[0].end_time.is_none());
        assert!((parts[0].dynamics.mu - MU_EARTH).abs() < 1e-6);

        assert_eq!(parts[1].id, SatId::from("sso"));
        assert!((parts[1].end_time.unwrap() - 50.0).abs() < 1e-15);
    }

    #[test]
    fn into_parts_after_propagation() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        // Capture state before consuming
        let expected_t = group.satellites().next().unwrap().t;
        let expected_pos = *group.satellites().next().unwrap().state.position();

        let parts = group.into_parts();
        assert_eq!(parts.len(), 1);
        assert!((parts[0].t - expected_t).abs() < 1e-15);
        assert_eq!(*parts[0].state.position(), expected_pos);
        assert!(!parts[0].terminated);
    }

    #[test]
    fn push_satellite_at_adds_to_running_group() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        // Propagate to t=50
        group.propagate_to(50.0).unwrap();
        assert_eq!(group.satellites().count(), 1);

        // Dynamically add a satellite at t=50
        group.push_satellite_at("sso", sso_state(), 50.0, TwoBodySystem { mu: MU_EARTH });
        assert_eq!(group.satellites().count(), 2);

        // Both satellites should propagate to t=100
        group.propagate_to(100.0).unwrap();

        let entries: Vec<_> = group.satellites().collect();
        assert_eq!(entries.len(), 2);
        assert!(
            (entries[0].t - 100.0).abs() < 1e-9,
            "ISS should reach t=100"
        );
        assert!(
            (entries[1].t - 100.0).abs() < 1e-9,
            "SSO should reach t=100"
        );
        assert_eq!(entries[1].id, SatId::from("sso"));
    }

    #[test]
    fn all_finished_no_end_time_never_finished() {
        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::dp45(
            10.0,
            default_tol(),
        )
        .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(1000.0).unwrap();
        // Without end_time and not terminated, never "finished"
        assert!(!group.all_finished());
    }
}
