use std::ops::ControlFlow;

use nalgebra::Vector3;
use orts_integrator::{
    AdvanceOutcome, DynamicalSystem, DormandPrince, IntegrationError, Integrator, OdeState, Rk4,
    Tolerances,
};

use super::prop_group::{GroupSnapshot, PropGroupOutcome, SatId, SatelliteTermination};
use super::state::GroupState;
use super::{FromAcceleration, HasPosition, IntegratorConfig};

/// Context passed to [`InterSatelliteForce::acceleration_pair`].
///
/// Marked `#[non_exhaustive]` so that future fields (e.g., `vel_i`, `vel_j`
/// for velocity-dependent forces) can be added without breaking existing
/// implementations.
#[non_exhaustive]
pub struct PairContext<'a> {
    pub t: f64,
    pub pos_i: &'a Vector3<f64>,
    pub pos_j: &'a Vector3<f64>,
}

/// A pairwise force between two satellites.
///
/// Returns accelerations (not forces) on each satellite of the pair.
/// Newton's third law compliance is the implementor's responsibility.
pub trait InterSatelliteForce: Send + Sync {
    fn name(&self) -> &str;
    fn acceleration_pair(&self, ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>);
}

/// A specific satellite-pair interaction: indices into the group + force model.
pub struct InteractionPair {
    pub i: usize,
    pub j: usize,
    pub force: Box<dyn InterSatelliteForce>,
}

/// Mutual gravitational attraction between two bodies.
///
/// `mu_i = G * m_i` and `mu_j = G * m_j` in km³/s².
/// Newton's third law: `m_i * a_i + m_j * a_j = 0`.
///
/// For `r < 1e-10 km`, returns zero accelerations (singularity guard).
pub struct MutualGravity {
    pub mu_i: f64,
    pub mu_j: f64,
}

impl InterSatelliteForce for MutualGravity {
    fn name(&self) -> &str {
        "mutual_gravity"
    }

    fn acceleration_pair(&self, ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>) {
        let r_vec = ctx.pos_j - ctx.pos_i; // from i toward j
        let r_sq = r_vec.magnitude_squared();
        if r_sq < 1e-20 {
            return (Vector3::zeros(), Vector3::zeros());
        }
        let r = r_sq.sqrt();
        let r_cubed = r * r_sq;
        let a_i = (self.mu_j / r_cubed) * r_vec;
        let a_j = -(self.mu_i / r_cubed) * r_vec;
        (a_i, a_j)
    }
}

/// Equal-mass spring force for testing coupled dynamics.
///
/// **Assumes equal mass**: `a_j = -a_i` holds only when `m_i = m_j`.
/// For unequal masses, use a separate force/mass formulation.
/// This is intentionally simplified for analytical solution testing:
/// relative motion is simple harmonic with period `2π/√(2k)`.
///
/// For `|r| < 1e-10 km`, returns zero accelerations (singularity guard).
pub struct Spring {
    pub stiffness: f64,
    pub rest_length: f64,
}

impl InterSatelliteForce for Spring {
    fn name(&self) -> &str {
        "spring"
    }

    fn acceleration_pair(&self, ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>) {
        let r_vec = ctx.pos_j - ctx.pos_i;
        let r = r_vec.magnitude();
        if r < 1e-10 {
            return (Vector3::zeros(), Vector3::zeros());
        }
        let r_hat = r_vec / r;
        let a_i = self.stiffness * (r - self.rest_length) * r_hat;
        let a_j = -a_i;
        (a_i, a_j)
    }
}

/// Extracted data returned by [`CoupledGroup::into_parts`].
///
/// Contains everything needed to transfer satellites back to the Scheduler.
/// Interactions are NOT returned (forces are stateless; the Scheduler
/// recreates them from `InteractionSpec` factories).
pub struct CoupledGroupParts<S, D> {
    pub ids: Vec<SatId>,
    pub states: Vec<S>,
    pub dynamics: Vec<D>,
    pub t: f64,
    pub terminated: bool,
    pub termination: Option<SatelliteTermination>,
}

/// Coupled group dynamics: each satellite's derivatives plus inter-satellite
/// force contributions, integrated as a single ODE.
pub struct CoupledGroupDynamics<D: DynamicalSystem>
where
    D::State: HasPosition + FromAcceleration,
{
    pub(crate) dynamics: Vec<D>,
    pub(crate) interactions: Vec<InteractionPair>,
}

impl<D: DynamicalSystem> CoupledGroupDynamics<D>
where
    D::State: HasPosition + FromAcceleration,
{
    pub fn new(dynamics: Vec<D>, interactions: Vec<InteractionPair>) -> Self {
        Self {
            dynamics,
            interactions,
        }
    }
}

impl<D: DynamicalSystem> DynamicalSystem for CoupledGroupDynamics<D>
where
    D::State: HasPosition + FromAcceleration,
{
    type State = GroupState<D::State>;

    fn derivatives(&self, t: f64, state: &GroupState<D::State>) -> GroupState<D::State> {
        assert_eq!(
            self.dynamics.len(),
            state.states.len(),
            "CoupledGroupDynamics: dynamics count ({}) != state count ({})",
            self.dynamics.len(),
            state.states.len()
        );

        // 1. Per-satellite derivatives (gravity, drag, etc.)
        let mut derivs: Vec<D::State> = self
            .dynamics
            .iter()
            .zip(&state.states)
            .map(|(d, s)| d.derivatives(t, s))
            .collect();

        // 2. Add inter-satellite force accelerations
        for pair in &self.interactions {
            assert!(
                pair.i < state.states.len() && pair.j < state.states.len(),
                "InteractionPair indices ({}, {}) out of range for {} satellites",
                pair.i,
                pair.j,
                state.states.len()
            );

            let pos_i = state.states[pair.i].position();
            let pos_j = state.states[pair.j].position();
            let ctx = PairContext {
                t,
                pos_i: &pos_i,
                pos_j: &pos_j,
            };
            let (a_i, a_j) = pair.force.acceleration_pair(&ctx);
            derivs[pair.i] = derivs[pair.i].axpy(1.0, &D::State::from_acceleration(a_i));
            derivs[pair.j] = derivs[pair.j].axpy(1.0, &D::State::from_acceleration(a_j));
        }

        GroupState { states: derivs }
    }
}

// ── CoupledGroup (PropGroup impl) ──────────────────────────────────────────

type EventChecker<S> = Box<dyn Fn(f64, &S) -> ControlFlow<String> + Send>;

/// Group of coupled satellites integrated as a single ODE.
///
/// All satellites share a single adaptive/fixed stepper and advance with
/// a common time step. If any satellite triggers an event, the entire
/// group terminates (the stepper cannot continue without that satellite).
pub struct CoupledGroup<D: DynamicalSystem>
where
    D::State: HasPosition + FromAcceleration,
{
    ids: Vec<SatId>,
    dynamics: CoupledGroupDynamics<D>,
    state: GroupState<D::State>,
    t: f64,
    terminated: bool,
    termination: Option<SatelliteTermination>,
    integrator: IntegratorConfig,
    event_checker: Option<EventChecker<D::State>>,
}

impl<D: DynamicalSystem> CoupledGroup<D>
where
    D::State: HasPosition + FromAcceleration,
{
    pub fn new(integrator: IntegratorConfig) -> Self {
        Self {
            ids: Vec::new(),
            dynamics: CoupledGroupDynamics::new(Vec::new(), Vec::new()),
            state: GroupState::new(Vec::new()),
            t: 0.0,
            terminated: false,
            termination: None,
            integrator,
            event_checker: None,
        }
    }

    pub fn dp45(dt: f64, tolerances: Tolerances) -> Self {
        Self::new(IntegratorConfig::Dp45 { dt, tolerances })
    }

    pub fn rk4(dt: f64) -> Self {
        Self::new(IntegratorConfig::Rk4 { dt })
    }

    pub fn with_event_checker(
        mut self,
        checker: impl Fn(f64, &D::State) -> ControlFlow<String> + Send + 'static,
    ) -> Self {
        self.event_checker = Some(Box::new(checker));
        self
    }

    pub fn add_satellite(
        mut self,
        id: impl Into<SatId>,
        state: D::State,
        dynamics: D,
    ) -> Self {
        self.ids.push(id.into());
        self.dynamics.dynamics.push(dynamics);
        self.state.states.push(state);
        self
    }

    pub fn with_interaction(
        mut self,
        i: usize,
        j: usize,
        force: Box<dyn InterSatelliteForce>,
    ) -> Self {
        self.dynamics
            .interactions
            .push(InteractionPair { i, j, force });
        self
    }

    /// Add a satellite to an already-constructed group (mutable reference).
    ///
    /// Unlike [`add_satellite`](Self::add_satellite) (builder pattern, consumes self),
    /// this method borrows `&mut self` for use by the Scheduler when building
    /// ephemeral groups.
    pub fn push_satellite(&mut self, id: impl Into<SatId>, state: D::State, dynamics: D) {
        self.ids.push(id.into());
        self.dynamics.dynamics.push(dynamics);
        self.state.states.push(state);
    }

    /// Add an interaction to an already-constructed group (mutable reference).
    pub fn push_interaction(&mut self, i: usize, j: usize, force: Box<dyn InterSatelliteForce>) {
        self.dynamics
            .interactions
            .push(InteractionPair { i, j, force });
    }

    /// Set the current time for this group.
    ///
    /// Used by the Scheduler to set the start time of an ephemeral group.
    pub fn set_t(&mut self, t: f64) {
        self.t = t;
    }

    /// Consume the group, returning all satellite data and dynamics.
    ///
    /// Used by the Scheduler to recover state and dynamics after ephemeral
    /// group propagation. Interactions are NOT returned (forces are stateless;
    /// the Scheduler recreates them from `InteractionSpec` factories).
    pub fn into_parts(self) -> CoupledGroupParts<D::State, D> {
        CoupledGroupParts {
            ids: self.ids,
            states: self.state.states,
            dynamics: self.dynamics.dynamics,
            t: self.t,
            terminated: self.terminated,
            termination: self.termination,
        }
    }

    pub fn group_state(&self) -> &GroupState<D::State> {
        &self.state
    }

    pub fn current_t(&self) -> f64 {
        self.t
    }

    pub fn is_terminated(&self) -> bool {
        self.terminated
    }
}

impl<D: DynamicalSystem + Send> super::prop_group::PropGroup for CoupledGroup<D>
where
    D::State: HasPosition + FromAcceleration + Send,
{
    fn ids(&self) -> Vec<SatId> {
        self.ids.clone()
    }

    fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError> {
        if self.terminated || self.t >= t_target {
            return Ok(PropGroupOutcome {
                terminations: Vec::new(),
            });
        }

        match &self.integrator {
            IntegratorConfig::Dp45 { dt, tolerances } => {
                let mut stepper = DormandPrince.stepper(
                    &self.dynamics,
                    self.state.clone(),
                    self.t,
                    *dt,
                    tolerances.clone(),
                );

                // Build group-level event checker that iterates over all satellites
                let ids = self.ids.clone();
                let event_checker = &self.event_checker;

                let result = if let Some(checker) = event_checker {
                    stepper.advance_to(
                        t_target,
                        |_, _| {},
                        |t: f64, gs: &GroupState<D::State>| {
                            for (id, sat_state) in ids.iter().zip(&gs.states) {
                                if let ControlFlow::Break(reason) = checker(t, sat_state) {
                                    return ControlFlow::Break((id.clone(), reason));
                                }
                            }
                            ControlFlow::Continue(())
                        },
                    )
                } else {
                    stepper.advance_to(
                        t_target,
                        |_, _| {},
                        |_: f64, _: &GroupState<D::State>| {
                            ControlFlow::<(SatId, String)>::Continue(())
                        },
                    )
                };

                match result {
                    Ok(AdvanceOutcome::Reached) => {
                        self.state = stepper.into_state();
                        self.t = t_target;
                        Ok(PropGroupOutcome {
                            terminations: Vec::new(),
                        })
                    }
                    Ok(AdvanceOutcome::Event {
                        reason: (sat_id, reason),
                    }) => {
                        let t = stepper.t();
                        self.state = stepper.into_state();
                        self.t = t;
                        self.terminated = true;
                        let term = SatelliteTermination {
                            satellite_id: sat_id,
                            t,
                            reason,
                        };
                        self.termination = Some(term.clone());
                        Ok(PropGroupOutcome {
                            terminations: vec![term],
                        })
                    }
                    Err(e) => {
                        self.terminated = true;
                        let t = match &e {
                            IntegrationError::NonFiniteState { t } => *t,
                            IntegrationError::StepSizeTooSmall { t, .. } => *t,
                        };
                        let term = SatelliteTermination {
                            satellite_id: self.ids.first().cloned().unwrap_or_else(|| SatId::from("unknown")),
                            t,
                            reason: format!("{e:?}"),
                        };
                        self.termination = Some(term.clone());
                        Ok(PropGroupOutcome {
                            terminations: vec![term],
                        })
                    }
                }
            }
            IntegratorConfig::Rk4 { dt } => {
                let dt = *dt;
                let mut current_t = self.t;
                let mut current_state = self.state.clone();

                while current_t < t_target - 1e-12 {
                    let h = dt.min(t_target - current_t);
                    current_state = Rk4.step(&self.dynamics, current_t, &current_state, h);
                    current_t += h;

                    if !current_state.is_finite() {
                        self.state = current_state;
                        self.t = current_t;
                        self.terminated = true;
                        let term = SatelliteTermination {
                            satellite_id: self.ids.first().cloned().unwrap_or_else(|| SatId::from("unknown")),
                            t: current_t,
                            reason: "NonFiniteState".to_string(),
                        };
                        self.termination = Some(term.clone());
                        return Ok(PropGroupOutcome {
                            terminations: vec![term],
                        });
                    }

                    if let Some(ref checker) = self.event_checker {
                        for (id, sat_state) in self.ids.iter().zip(&current_state.states) {
                            if let ControlFlow::Break(reason) = checker(current_t, sat_state) {
                                self.state = current_state;
                                self.t = current_t;
                                self.terminated = true;
                                let term = SatelliteTermination {
                                    satellite_id: id.clone(),
                                    t: current_t,
                                    reason,
                                };
                                self.termination = Some(term.clone());
                                return Ok(PropGroupOutcome {
                                    terminations: vec![term],
                                });
                            }
                        }
                    }
                }

                self.state = current_state;
                self.t = current_t;
                Ok(PropGroupOutcome {
                    terminations: Vec::new(),
                })
            }
        }
    }

    fn snapshot(&self) -> GroupSnapshot {
        if self.terminated {
            return GroupSnapshot {
                positions: Vec::new(),
            };
        }
        GroupSnapshot {
            positions: self
                .ids
                .iter()
                .zip(&self.state.states)
                .map(|(id, s)| (id.clone(), s.position()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;
    use orts_integrator::{Integrator, Rk4, State, Tolerances};
    use super::super::prop_group::PropGroup;

    // ── InterSatelliteForce tests ──────────────────────────────────────────

    fn pair_ctx<'a>(
        t: f64,
        pos_i: &'a Vector3<f64>,
        pos_j: &'a Vector3<f64>,
    ) -> PairContext<'a> {
        PairContext { t, pos_i, pos_j }
    }

    #[test]
    fn mutual_gravity_newton_third_law() {
        // Newton's 3rd: F_i + F_j = 0, i.e., m_i * a_i + m_j * a_j = 0
        // Since mu = G*m, this is: mu_i * a_i + mu_j * a_j = 0
        let mg = MutualGravity {
            mu_i: 1.0,
            mu_j: 2.0,
        };
        let pi = Vector3::new(0.0, 0.0, 0.0);
        let pj = Vector3::new(10.0, 0.0, 0.0);
        let (a_i, a_j) = mg.acceleration_pair(&pair_ctx(0.0, &pi, &pj));

        // a_i = mu_j/r³ * r_vec, a_j = -mu_i/r³ * r_vec
        // mu_i * a_i = mu_i * mu_j / r³ * r_vec
        // mu_j * a_j = -mu_j * mu_i / r³ * r_vec
        // Sum = 0 ✓
        let momentum_check = mg.mu_i * a_i + mg.mu_j * a_j;
        assert!(momentum_check.magnitude() < 1e-15);
    }

    #[test]
    fn mutual_gravity_inverse_square() {
        let mg = MutualGravity {
            mu_i: 1.0,
            mu_j: 1.0,
        };
        let pi = Vector3::zeros();
        let pj1 = Vector3::new(1.0, 0.0, 0.0);
        let pj2 = Vector3::new(2.0, 0.0, 0.0);

        let (a1, _) = mg.acceleration_pair(&pair_ctx(0.0, &pi, &pj1));
        let (a2, _) = mg.acceleration_pair(&pair_ctx(0.0, &pi, &pj2));

        // At double distance, acceleration should be 1/4
        let ratio = a1.magnitude() / a2.magnitude();
        assert!((ratio - 4.0).abs() < 1e-12);
    }

    #[test]
    fn mutual_gravity_symmetric() {
        let mg = MutualGravity {
            mu_i: 3.0,
            mu_j: 3.0,
        };
        let pi = Vector3::new(-5.0, 0.0, 0.0);
        let pj = Vector3::new(5.0, 0.0, 0.0);
        let (a_i, a_j) = mg.acceleration_pair(&pair_ctx(0.0, &pi, &pj));

        assert!((a_i.magnitude() - a_j.magnitude()).abs() < 1e-15);
        // Directions should be opposite
        assert!((a_i + a_j).magnitude() < 1e-15);
    }

    #[test]
    fn mutual_gravity_singularity_guard() {
        let mg = MutualGravity {
            mu_i: 1.0,
            mu_j: 1.0,
        };
        let p = Vector3::new(1.0, 2.0, 3.0);
        let (a_i, a_j) = mg.acceleration_pair(&pair_ctx(0.0, &p, &p));

        assert_eq!(a_i, Vector3::zeros());
        assert_eq!(a_j, Vector3::zeros());
    }

    #[test]
    fn spring_equilibrium_zero_force() {
        let spring = Spring {
            stiffness: 1.0,
            rest_length: 10.0,
        };
        let pi = Vector3::new(0.0, 0.0, 0.0);
        let pj = Vector3::new(10.0, 0.0, 0.0); // exactly rest_length apart
        let (a_i, a_j) = spring.acceleration_pair(&pair_ctx(0.0, &pi, &pj));

        assert!(a_i.magnitude() < 1e-15);
        assert!(a_j.magnitude() < 1e-15);
    }

    #[test]
    fn spring_newton_third_law() {
        let spring = Spring {
            stiffness: 2.0,
            rest_length: 5.0,
        };
        let pi = Vector3::new(1.0, 2.0, 3.0);
        let pj = Vector3::new(4.0, 6.0, 8.0);
        let (a_i, a_j) = spring.acceleration_pair(&pair_ctx(0.0, &pi, &pj));

        // Equal-mass: a_j = -a_i
        assert!((a_i + a_j).magnitude() < 1e-15);
    }

    #[test]
    fn spring_singularity_guard() {
        let spring = Spring {
            stiffness: 1.0,
            rest_length: 5.0,
        };
        let p = Vector3::new(1.0, 2.0, 3.0);
        let (a_i, a_j) = spring.acceleration_pair(&pair_ctx(0.0, &p, &p));

        assert_eq!(a_i, Vector3::zeros());
        assert_eq!(a_j, Vector3::zeros());
    }

    // ── CoupledGroupDynamics tests ─────────────────────────────────────────

    use orts_orbits::two_body::TwoBodySystem;

    fn iss_state() -> State {
        let r: f64 = 6778.137;
        let v = (398600.4418_f64 / r).sqrt();
        State {
            position: Vector3::new(r, 0.0, 0.0),
            velocity: Vector3::new(0.0, v, 0.0),
        }
    }

    fn sso_state() -> State {
        let r: f64 = 6378.137 + 800.0;
        let v = (398600.4418_f64 / r).sqrt();
        State {
            position: Vector3::new(r, 0.0, 0.0),
            velocity: Vector3::new(0.0, v, 0.0),
        }
    }

    #[test]
    fn no_interactions_matches_independent() {
        use super::super::IndependentGroupDynamics;

        let mu = 398600.4418;
        let coupled = CoupledGroupDynamics::new(
            vec![TwoBodySystem { mu }, TwoBodySystem { mu }],
            vec![], // no interactions
        );
        let independent = IndependentGroupDynamics::new(vec![
            TwoBodySystem { mu },
            TwoBodySystem { mu },
        ]);

        let state = GroupState::new(vec![iss_state(), sso_state()]);
        let d_coupled = coupled.derivatives(0.0, &state);
        let d_independent = independent.derivatives(0.0, &state);

        // Should be bit-identical
        assert_eq!(d_coupled.states[0].position, d_independent.states[0].position);
        assert_eq!(d_coupled.states[0].velocity, d_independent.states[0].velocity);
        assert_eq!(d_coupled.states[1].position, d_independent.states[1].position);
        assert_eq!(d_coupled.states[1].velocity, d_independent.states[1].velocity);
    }

    #[test]
    fn mutual_gravity_adds_acceleration() {
        let mu = 398600.4418;
        let coupled = CoupledGroupDynamics::new(
            vec![TwoBodySystem { mu }, TwoBodySystem { mu }],
            vec![InteractionPair {
                i: 0,
                j: 1,
                force: Box::new(MutualGravity {
                    mu_i: 1e-10,
                    mu_j: 1e-10,
                }),
            }],
        );
        let independent = super::super::IndependentGroupDynamics::new(vec![
            TwoBodySystem { mu },
            TwoBodySystem { mu },
        ]);

        let state = GroupState::new(vec![iss_state(), sso_state()]);
        let d_coupled = coupled.derivatives(0.0, &state);
        let d_independent = independent.derivatives(0.0, &state);

        // Coupled should differ from independent (mutual gravity adds acceleration)
        let diff0 = (d_coupled.states[0].velocity - d_independent.states[0].velocity).magnitude();
        let diff1 = (d_coupled.states[1].velocity - d_independent.states[1].velocity).magnitude();
        assert!(diff0 > 0.0);
        assert!(diff1 > 0.0);
    }

    #[test]
    fn three_satellites_pair_accounting() {
        // 3 satellites with 3 pairs: (0,1), (0,2), (1,2)
        // Verify all pairs contribute correctly
        let mu = 398600.4418;
        let s0 = State {
            position: Vector3::new(7000.0, 0.0, 0.0),
            velocity: Vector3::new(0.0, 7.5, 0.0),
        };
        let s1 = State {
            position: Vector3::new(0.0, 7200.0, 0.0),
            velocity: Vector3::new(-7.3, 0.0, 0.0),
        };
        let s2 = State {
            position: Vector3::new(0.0, 0.0, 7400.0),
            velocity: Vector3::new(0.0, 0.0, 7.1),
        };

        let mg = |mu_i, mu_j| -> Box<dyn InterSatelliteForce> {
            Box::new(MutualGravity { mu_i, mu_j })
        };

        let coupled = CoupledGroupDynamics::new(
            vec![
                TwoBodySystem { mu },
                TwoBodySystem { mu },
                TwoBodySystem { mu },
            ],
            vec![
                InteractionPair { i: 0, j: 1, force: mg(1.0, 2.0) },
                InteractionPair { i: 0, j: 2, force: mg(1.0, 3.0) },
                InteractionPair { i: 1, j: 2, force: mg(2.0, 3.0) },
            ],
        );

        let state = GroupState::new(vec![s0.clone(), s1.clone(), s2.clone()]);
        let derivs = coupled.derivatives(0.0, &state);

        // Compare with independent (no interactions)
        let independent = super::super::IndependentGroupDynamics::new(vec![
            TwoBodySystem { mu },
            TwoBodySystem { mu },
            TwoBodySystem { mu },
        ]);
        let d_indep = independent.derivatives(0.0, &state);

        // All 3 satellites should have different accelerations from independent
        for k in 0..3 {
            let diff = (derivs.states[k].velocity - d_indep.states[k].velocity).magnitude();
            assert!(diff > 0.0, "satellite {k} should have inter-satellite acceleration");
        }
    }

    #[test]
    fn three_satellites_momentum_conservation() {
        // For MutualGravity: Σ m_i * a_i = 0 (total momentum conserved)
        // mu_i = G * m_i, so check Σ mu_i * a_extra_i = 0
        let mu_0 = 1.0;
        let mu_1 = 2.0;
        let mu_2 = 3.0;

        let s0 = State {
            position: Vector3::new(10.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(0.0, 10.0, 0.0),
            velocity: Vector3::zeros(),
        };
        let s2 = State {
            position: Vector3::new(0.0, 0.0, 10.0),
            velocity: Vector3::zeros(),
        };

        // Use a dummy DynamicalSystem that returns zero derivatives
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let coupled = CoupledGroupDynamics::new(
            vec![FreeParticle, FreeParticle, FreeParticle],
            vec![
                InteractionPair {
                    i: 0,
                    j: 1,
                    force: Box::new(MutualGravity { mu_i: mu_0, mu_j: mu_1 }),
                },
                InteractionPair {
                    i: 0,
                    j: 2,
                    force: Box::new(MutualGravity { mu_i: mu_0, mu_j: mu_2 }),
                },
                InteractionPair {
                    i: 1,
                    j: 2,
                    force: Box::new(MutualGravity { mu_i: mu_1, mu_j: mu_2 }),
                },
            ],
        );

        let state = GroupState::new(vec![s0, s1, s2]);
        let derivs = coupled.derivatives(0.0, &state);

        // Σ mu_i * a_i = 0 (since FreeParticle → only inter-satellite forces)
        let momentum = mu_0 * derivs.states[0].velocity
            + mu_1 * derivs.states[1].velocity
            + mu_2 * derivs.states[2].velocity;
        assert!(
            momentum.magnitude() < 1e-15,
            "total momentum should be conserved, got {momentum:?}"
        );
    }

    #[test]
    fn spring_energy_conservation_rk4() {
        // Two equal-mass bodies connected by spring, no other forces
        // Total energy = KE + PE = (v1² + v2²)/2 + k*(|r12| - L)²/2
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let k = 0.01; // stiffness
        let rest = 10.0;
        let coupled = CoupledGroupDynamics::new(
            vec![FreeParticle, FreeParticle],
            vec![InteractionPair {
                i: 0,
                j: 1,
                force: Box::new(Spring {
                    stiffness: k,
                    rest_length: rest,
                }),
            }],
        );

        // Initial: stretched spring (15 km apart, rest = 10 km), both at rest
        let s0 = State {
            position: Vector3::new(0.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(15.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };

        let energy = |gs: &GroupState<State>| -> f64 {
            let ke = gs.states[0].velocity.magnitude_squared() / 2.0
                + gs.states[1].velocity.magnitude_squared() / 2.0;
            let r = (gs.states[1].position - gs.states[0].position).magnitude();
            let pe = k * (r - rest).powi(2) / 2.0;
            ke + pe
        };

        let mut state = GroupState::new(vec![s0, s1]);
        let e0 = energy(&state);

        let dt = 0.01;
        let mut t = 0.0;
        for _ in 0..10000 {
            state = Rk4.step(&coupled, t, &state, dt);
            t += dt;
            let e = energy(&state);
            assert!(
                (e - e0).abs() / e0 < 1e-7,
                "energy drift at t={t}: {e} vs {e0}, relative = {}",
                (e - e0).abs() / e0
            );
        }
    }

    #[test]
    fn spring_relative_oscillation_period() {
        // Two equal-mass bodies on a spring: relative motion is SHM
        // ω_rel = √(2k) (each mass feels k*Δx, relative accel = 2k*Δx)
        // Period T = 2π/√(2k)
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let k = 0.04; // ω_rel = √0.08 ≈ 0.2828, T ≈ 22.21 s
        let rest = 10.0;
        let amplitude = 3.0; // initial stretch
        let coupled = CoupledGroupDynamics::new(
            vec![FreeParticle, FreeParticle],
            vec![InteractionPair {
                i: 0,
                j: 1,
                force: Box::new(Spring {
                    stiffness: k,
                    rest_length: rest,
                }),
            }],
        );

        let s0 = State {
            position: Vector3::new(0.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(rest + amplitude, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };

        let expected_period = 2.0 * std::f64::consts::PI / (2.0 * k).sqrt();

        // Propagate for one full period and check that relative displacement returns
        let mut state = GroupState::new(vec![s0, s1]);
        let dt = 0.01;
        let n_steps = (expected_period / dt).round() as usize;
        let mut t = 0.0;
        for _ in 0..n_steps {
            state = Rk4.step(&coupled, t, &state, dt);
            t += dt;
        }

        let final_sep = (state.states[1].position - state.states[0].position).magnitude();
        // After one full period, separation should return to rest + amplitude
        assert!(
            (final_sep - (rest + amplitude)).abs() < 0.01,
            "after period T={expected_period:.2}, separation = {final_sep:.4}, expected {:.4}",
            rest + amplitude
        );
    }

    #[test]
    fn dt_convergence_rk4_fourth_order() {
        // RK4 error should decrease by factor ~16 when dt is halved
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let k = 1.0; // higher stiffness for faster dynamics and larger errors
        let rest = 10.0;
        let coupled = CoupledGroupDynamics::new(
            vec![FreeParticle, FreeParticle],
            vec![InteractionPair {
                i: 0,
                j: 1,
                force: Box::new(Spring {
                    stiffness: k,
                    rest_length: rest,
                }),
            }],
        );

        let s0 = State {
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(15.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };

        let t_end = 10.0; // ~2.25 relative oscillation periods

        let propagate = |dt: f64| -> GroupState<State> {
            let mut state = GroupState::new(vec![s0.clone(), s1.clone()]);
            let n_steps = (t_end / dt).round() as usize;
            let mut t = 0.0;
            for _ in 0..n_steps {
                state = Rk4.step(&coupled, t, &state, dt);
                t += dt;
            }
            state
        };

        // Reference solution with very small dt
        let ref_state = propagate(0.0001);

        let state_coarse = propagate(0.02);
        let state_fine = propagate(0.01);

        let err_coarse =
            (state_coarse.states[0].position - ref_state.states[0].position).magnitude();
        let err_fine =
            (state_fine.states[0].position - ref_state.states[0].position).magnitude();

        assert!(err_coarse > 0.0, "coarse error should be nonzero");
        assert!(err_fine > 0.0, "fine error should be nonzero");

        // RK4: error ~ O(dt⁴), so err_coarse/err_fine ≈ (0.02/0.01)⁴ = 16
        let ratio = err_coarse / err_fine;
        assert!(
            ratio > 12.0 && ratio < 20.0,
            "expected ~16x convergence, got {ratio:.2} (coarse={err_coarse:.2e}, fine={err_fine:.2e})"
        );
    }

    // ── CoupledGroup (PropGroup) tests ─────────────────────────────────────

    const MU_EARTH: f64 = 398600.4418;
    const EARTH_RADIUS: f64 = 6378.137;

    fn default_tol() -> Tolerances {
        Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        }
    }

    #[test]
    fn coupled_group_dp45_basic_propagation() {
        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());
        assert!((group.current_t() - 100.0).abs() < 1e-9);
        assert!(!group.is_terminated());
    }

    #[test]
    fn coupled_group_rk4_basic_propagation() {
        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::rk4(10.0)
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());
        assert!((group.current_t() - 100.0).abs() < 1e-9);
        assert!(!group.is_terminated());
    }

    #[test]
    fn coupled_group_rk4_matches_independent() {
        // CoupledGroup with no interactions should match IndependentGroup (RK4)
        use super::super::IndependentGroup;

        let dt = 10.0;
        let mut coupled: CoupledGroup<TwoBodySystem> =
            CoupledGroup::rk4(dt)
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });
        coupled.propagate_to(100.0).unwrap();

        let mut independent: IndependentGroup<TwoBodySystem> =
            IndependentGroup::rk4(dt)
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });
        independent.propagate_to(100.0).unwrap();

        let coupled_states = &coupled.group_state().states;
        let indep_entries: Vec<_> = independent.satellites().collect();

        // RK4 with same dt: should be bit-identical
        let iss_pos_err = (coupled_states[0].position - indep_entries[0].state.position).magnitude();
        let sso_pos_err = (coupled_states[1].position - indep_entries[1].state.position).magnitude();
        assert!(iss_pos_err < 1e-12, "ISS position error: {iss_pos_err}");
        assert!(sso_pos_err < 1e-12, "SSO position error: {sso_pos_err}");
    }

    #[test]
    fn coupled_group_spring_dp45_energy() {
        // Spring-coupled two-body with DP45: energy should be well conserved
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let k = 0.01;
        let rest = 10.0;
        let s0 = State {
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(15.0, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };

        let mut group: CoupledGroup<FreeParticle> =
            CoupledGroup::dp45(1.0, Tolerances { atol: 1e-12, rtol: 1e-10 })
                .add_satellite("a", s0, FreeParticle)
                .add_satellite("b", s1, FreeParticle)
                .with_interaction(0, 1, Box::new(Spring { stiffness: k, rest_length: rest }));

        let energy = |gs: &GroupState<State>| -> f64 {
            let ke = gs.states[0].velocity.magnitude_squared() / 2.0
                + gs.states[1].velocity.magnitude_squared() / 2.0;
            let r = (gs.states[1].position - gs.states[0].position).magnitude();
            ke + k * (r - rest).powi(2) / 2.0
        };

        let e0 = energy(group.group_state());

        // Propagate for ~2 oscillation periods
        group.propagate_to(200.0).unwrap();

        let e_final = energy(group.group_state());
        let rel_err = (e_final - e0).abs() / e0;
        assert!(
            rel_err < 1e-8,
            "DP45 energy relative error = {rel_err:.2e}"
        );
    }

    #[test]
    fn coupled_group_spring_rk4_oscillation() {
        /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
        struct FreeParticle;
        impl DynamicalSystem for FreeParticle {
            type State = State;
            fn derivatives(&self, _t: f64, state: &State) -> State {
                State::from_derivative(state.velocity, Vector3::zeros())
            }
        }

        let k = 0.04;
        let rest = 10.0;
        let amplitude = 3.0;
        let s0 = State {
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
        };
        let s1 = State {
            position: Vector3::new(rest + amplitude, 0.0, 0.0),
            velocity: Vector3::zeros(),
        };

        let expected_period = 2.0 * std::f64::consts::PI / (2.0_f64 * k).sqrt();

        let mut group: CoupledGroup<FreeParticle> =
            CoupledGroup::rk4(0.01)
                .add_satellite("a", s0, FreeParticle)
                .add_satellite("b", s1, FreeParticle)
                .with_interaction(0, 1, Box::new(Spring { stiffness: k, rest_length: rest }));

        group.propagate_to(expected_period).unwrap();

        let final_sep = (group.group_state().states[1].position
            - group.group_state().states[0].position)
            .magnitude();
        assert!(
            (final_sep - (rest + amplitude)).abs() < 0.01,
            "after period T={expected_period:.2}, separation = {final_sep:.4}"
        );
    }

    #[test]
    fn coupled_group_event_terminates_whole_group() {
        // One satellite hits Earth → entire coupled group terminates
        let decaying = State {
            position: Vector3::new(6500.0, 0.0, 0.0),
            velocity: Vector3::new(-5.0, 3.0, 0.0),
        };

        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .with_event_checker(move |_t: f64, state: &State| {
                    if state.position.magnitude() < EARTH_RADIUS {
                        ControlFlow::Break("collision".to_string())
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .add_satellite("safe", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(10000.0).unwrap();

        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("decay"));
        assert!(outcome.terminations[0].reason.contains("collision"));
        assert!(group.is_terminated());
        assert!(group.current_t() < 10000.0);
    }

    #[test]
    fn coupled_group_multiple_events_first_wins() {
        // Two decaying satellites: first one detected wins
        let decay1 = State {
            position: Vector3::new(6500.0, 0.0, 0.0),
            velocity: Vector3::new(-5.0, 3.0, 0.0),
        };
        let decay2 = State {
            position: Vector3::new(6500.0, 0.0, 0.0),
            velocity: Vector3::new(-5.0, 3.0, 0.0),
        };

        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .with_event_checker(move |_t: f64, state: &State| {
                    if state.position.magnitude() < EARTH_RADIUS {
                        ControlFlow::Break("collision".to_string())
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .add_satellite("first", decay1, TwoBodySystem { mu: MU_EARTH })
                .add_satellite("second", decay2, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(10000.0).unwrap();

        // Exactly one termination (first detected, in iteration order)
        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("first"));
        assert!(group.is_terminated());
    }

    #[test]
    fn coupled_group_nan_terminates() {
        let degenerate = State {
            position: Vector3::zeros(),
            velocity: Vector3::new(0.0, 1.0, 0.0),
        };

        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::rk4(10.0)
                .add_satellite("good", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("bad", degenerate, TwoBodySystem { mu: MU_EARTH });

        let outcome = group.propagate_to(100.0).unwrap();

        assert_eq!(outcome.terminations.len(), 1);
        assert!(outcome.terminations[0].reason.contains("NonFinite"));
        assert!(group.is_terminated());
    }

    #[test]
    fn coupled_group_snapshot() {
        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let snap = group.snapshot();
        assert_eq!(snap.positions.len(), 2);
        assert_eq!(snap.positions[0].0, SatId::from("iss"));
        assert!((snap.positions[0].1[0] - 6778.137).abs() < 1e-10);

        group.propagate_to(100.0).unwrap();
        let snap2 = group.snapshot();
        assert_eq!(snap2.positions.len(), 2);
        assert!((snap2.positions[0].1 - snap.positions[0].1).magnitude() > 1.0);
    }

    // ── into_parts + push API tests ──────────────────────────────────────

    #[test]
    fn into_parts_preserves_state_and_dynamics() {
        let group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::rk4(10.0)
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH })
                .with_interaction(
                    0,
                    1,
                    Box::new(MutualGravity {
                        mu_i: 1e-10,
                        mu_j: 1e-10,
                    }),
                );

        let parts = group.into_parts();
        assert_eq!(parts.ids.len(), 2);
        assert_eq!(parts.states.len(), 2);
        assert_eq!(parts.dynamics.len(), 2);
        assert_eq!(parts.ids[0], SatId::from("iss"));
        assert_eq!(parts.ids[1], SatId::from("sso"));
        assert!((parts.states[0].position.x - 6778.137).abs() < 1e-10);
        assert!((parts.dynamics[0].mu - MU_EARTH).abs() < 1e-6);
        assert!((parts.t - 0.0).abs() < 1e-15);
        assert!(!parts.terminated);
        assert!(parts.termination.is_none());
    }

    #[test]
    fn into_parts_after_termination() {
        let decaying = State {
            position: Vector3::new(6500.0, 0.0, 0.0),
            velocity: Vector3::new(-5.0, 3.0, 0.0),
        };

        let mut group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .with_event_checker(move |_t: f64, state: &State| {
                    if state.position.magnitude() < EARTH_RADIUS {
                        ControlFlow::Break("collision".to_string())
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .add_satellite("safe", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(10000.0).unwrap();
        assert!(group.is_terminated());

        let parts = group.into_parts();
        assert!(parts.terminated);
        assert!(parts.termination.is_some());
        assert!(parts.termination.unwrap().reason.contains("collision"));
        // Dynamics are still returned even after termination
        assert_eq!(parts.dynamics.len(), 2);
    }

    #[test]
    fn push_satellite_and_interaction() {
        let mut group: CoupledGroup<TwoBodySystem> = CoupledGroup::rk4(10.0);
        group.push_satellite("a", iss_state(), TwoBodySystem { mu: MU_EARTH });
        group.push_satellite("b", sso_state(), TwoBodySystem { mu: MU_EARTH });
        group.push_interaction(
            0,
            1,
            Box::new(Spring {
                stiffness: 0.01,
                rest_length: 10.0,
            }),
        );
        group.set_t(42.0);

        assert_eq!(group.ids(), vec![SatId::from("a"), SatId::from("b")]);
        assert!((group.current_t() - 42.0).abs() < 1e-15);

        // Should be able to propagate after push construction
        group.propagate_to(52.0).unwrap();
        assert!((group.current_t() - 52.0).abs() < 1e-9);
    }

    #[test]
    fn coupled_group_ids() {
        let group: CoupledGroup<TwoBodySystem> =
            CoupledGroup::dp45(10.0, default_tol())
                .add_satellite("alpha", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("beta", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let ids = group.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], SatId::from("alpha"));
        assert_eq!(ids[1], SatId::from("beta"));
    }
}
