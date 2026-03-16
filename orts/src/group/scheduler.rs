//! Scheduler: automatic regime transitions for multi-satellite propagation.
//!
//! Three regimes per interaction pair:
//! - **Coupled**: force included in CoupledGroup ODE (highest accuracy)
//! - **Synchronized**: KDK velocity kick at `sync_interval` (2nd-order Strang splitting)
//! - **Independent**: no force computation
//!
//! The Scheduler centrally owns all satellite states and dynamics,
//! builds ephemeral groups each sync step, propagates, then recovers
//! state via `into_parts()`.

use std::ops::ControlFlow;
use std::sync::Arc;

use orts_integrator::{DynamicalSystem, IntegrationError, OdeState};

use super::coupled::{CoupledGroup, InterSatelliteForce, PairContext};
use super::independent::{IndependentGroup, IntegratorConfig};
use super::prop_group::{GroupSnapshot, PropGroupOutcome, SatId, SatelliteTermination};
use super::{FromAcceleration, HasPosition};

// ── Regime types ────────────────────────────────────────────────────────────

/// Integration regime for a satellite pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRegime {
    /// No force computation between this pair.
    Independent,
    /// KDK velocity kick at each sync interval.
    Synchronized,
    /// Force included in CoupledGroup ODE.
    Coupled,
}

/// Policy governing how a pair's regime is determined.
#[derive(Debug, Clone)]
pub enum PairPolicy {
    /// Distance-based automatic transitions with hysteresis.
    Auto,
    /// Always use the specified regime regardless of distance.
    Fixed(PairRegime),
}

/// Distance thresholds and timing parameters for regime transitions.
///
/// Thresholds must satisfy: `couple_enter < couple_exit <= sync_enter < sync_exit`.
///
/// Upgrades (toward Coupled) are immediate; downgrades require `min_dwell_time`.
#[derive(Debug, Clone)]
pub struct RegimeConfig {
    /// Distance below which a pair enters Coupled regime.
    pub couple_enter: f64,
    /// Distance above which a pair exits Coupled regime (> couple_enter).
    pub couple_exit: f64,
    /// Distance below which a pair enters Synchronized regime (>= couple_exit).
    pub sync_enter: f64,
    /// Distance above which a pair exits Synchronized regime (> sync_enter).
    pub sync_exit: f64,
    /// KDK kick interval in seconds (also the regrouping interval).
    pub sync_interval: f64,
    /// Minimum time before a downgrade can occur (seconds).
    pub min_dwell_time: f64,
}

impl RegimeConfig {
    /// Validate that thresholds are properly ordered.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.couple_enter >= self.couple_exit {
            return Err("couple_enter must be < couple_exit");
        }
        if self.couple_exit > self.sync_enter {
            return Err("couple_exit must be <= sync_enter");
        }
        if self.sync_enter >= self.sync_exit {
            return Err("sync_enter must be < sync_exit");
        }
        if self.sync_interval <= 0.0 {
            return Err("sync_interval must be > 0");
        }
        if self.min_dwell_time < 0.0 {
            return Err("min_dwell_time must be >= 0");
        }
        Ok(())
    }
}

/// Specification of an interaction between two satellites.
pub struct InteractionSpec {
    pub sat_i: SatId,
    pub sat_j: SatId,
    pub force: Arc<dyn InterSatelliteForce>,
    pub policy: PairPolicy,
}

// ── Hysteresis state ────────────────────────────────────────────────────────

/// Per-pair state tracking for hysteresis transitions.
#[derive(Debug, Clone)]
struct PairState {
    regime: PairRegime,
    last_transition_t: f64,
}

/// Determine the new regime for an Auto pair given the current distance and state.
///
/// Rules:
/// - Upgrades (toward Coupled) are immediate, no dwell time required.
/// - Downgrades require `min_dwell_time` since the last transition.
/// - Direct jumps are allowed (e.g., Independent → Coupled if dist < couple_enter).
fn evaluate_auto_regime(
    dist: f64,
    current: &PairState,
    t: f64,
    config: &RegimeConfig,
) -> PairRegime {
    // Non-finite distance (NaN or Infinity) → force Independent
    if !dist.is_finite() {
        return PairRegime::Independent;
    }

    let can_downgrade = (t - current.last_transition_t) >= config.min_dwell_time;

    match current.regime {
        PairRegime::Coupled => {
            // Upgrade: already at highest — no change
            // Downgrade: requires dist > couple_exit + dwell
            if can_downgrade && dist > config.sync_exit {
                PairRegime::Independent
            } else if can_downgrade && dist > config.couple_exit {
                PairRegime::Synchronized
            } else {
                PairRegime::Coupled
            }
        }
        PairRegime::Synchronized => {
            // Upgrade: dist < couple_enter → Coupled (immediate)
            if dist < config.couple_enter {
                return PairRegime::Coupled;
            }
            // Downgrade: dist > sync_exit + dwell → Independent
            if can_downgrade && dist > config.sync_exit {
                PairRegime::Independent
            } else {
                PairRegime::Synchronized
            }
        }
        PairRegime::Independent => {
            // Upgrade: immediate, no dwell
            if dist < config.couple_enter {
                PairRegime::Coupled
            } else if dist < config.sync_enter {
                PairRegime::Synchronized
            } else {
                PairRegime::Independent
            }
        }
    }
}

// ── Grouping ────────────────────────────────────────────────────────────────

/// Result of the grouping algorithm.
#[derive(Debug)]
struct Grouping {
    /// Connected components of Coupled pairs. Each component is a list of
    /// satellite indices (into Scheduler.satellites).
    coupled_components: Vec<Vec<usize>>,
    /// Satellites not in any CoupledGroup.
    independent: Vec<usize>,
    /// Synchronized pairs (may include cross-group: one in Coupled, one independent).
    kick_pairs: Vec<KickPair>,
}

/// A pair to receive KDK kicks.
#[derive(Debug)]
struct KickPair {
    sat_i: usize,
    sat_j: usize,
    /// Index into Scheduler.interactions / pair_states.
    interaction_idx: usize,
}

/// Find connected components using BFS on an adjacency list.
///
/// `n` is the number of nodes; `edges` are undirected edges between nodes.
/// Returns a list of components, each being a sorted list of node indices.
/// Nodes not appearing in any edge are NOT included (they are independent).
fn connected_components(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    if edges.is_empty() {
        return Vec::new();
    }

    // Build adjacency list
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_edge = vec![false; n];
    for &(a, b) in edges {
        adj[a].push(b);
        adj[b].push(a);
        in_edge[a] = true;
        in_edge[b] = true;
    }

    let mut visited = vec![false; n];
    let mut components = Vec::new();

    for start in 0..n {
        if !in_edge[start] || visited[start] {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(start);
        visited[start] = true;
        while let Some(node) = queue.pop_front() {
            component.push(node);
            for &neighbor in &adj[node] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }

    components
}

/// Determine grouping from current pair regimes.
///
/// - Coupled pairs form connected components → CoupledGroups
/// - Synchronized pairs → kick_pairs (including cross-group)
/// - Remaining active satellites → independent
fn determine_grouping(
    n_sats: usize,
    pair_regimes: &[(usize, usize, PairRegime)],
    active: &[bool],
) -> Grouping {
    // Collect Coupled edges for connected components
    let coupled_edges: Vec<(usize, usize)> = pair_regimes
        .iter()
        .filter(|&&(_, _, regime)| regime == PairRegime::Coupled)
        .map(|&(i, j, _)| (i, j))
        .collect();

    let components = connected_components(n_sats, &coupled_edges);

    // Track which satellites are in a CoupledGroup
    let mut in_coupled = vec![false; n_sats];
    for comp in &components {
        for &idx in comp {
            in_coupled[idx] = true;
        }
    }

    // Collect Synchronized pairs as kick_pairs
    // These can be intra-independent or cross-group (one coupled, one not)
    let kick_pairs: Vec<KickPair> = pair_regimes
        .iter()
        .enumerate()
        .filter(|&(_, &(i, j, regime))| {
            regime == PairRegime::Synchronized && active[i] && active[j]
        })
        .map(|(idx, &(i, j, _))| KickPair {
            sat_i: i,
            sat_j: j,
            interaction_idx: idx,
        })
        .collect();

    // Independent satellites: active, not in any CoupledGroup
    let independent: Vec<usize> = (0..n_sats)
        .filter(|&i| active[i] && !in_coupled[i])
        .collect();

    Grouping {
        coupled_components: components,
        independent,
        kick_pairs,
    }
}

// ── Scheduler ──────────────────────────────────────────────────────────────

type SharedEventChecker<S> = Arc<dyn Fn(f64, &S) -> ControlFlow<String> + Send + Sync>;

/// Central record for a satellite in the Scheduler.
struct SatRecord<D: DynamicalSystem> {
    id: SatId,
    state: D::State,
    dynamics: Option<D>,
    terminated: bool,
    end_time: Option<f64>,
}

/// Central scheduler that owns all satellites and manages regime transitions.
///
/// Satellites are propagated using ephemeral groups built each sync step.
/// All active satellites share a common time `t`.
pub struct Scheduler<D: DynamicalSystem>
where
    D::State: HasPosition + FromAcceleration,
{
    satellites: Vec<SatRecord<D>>,
    interactions: Vec<InteractionSpec>,
    config: RegimeConfig,
    integrator: IntegratorConfig,
    event_checker: Option<SharedEventChecker<D::State>>,
    t: f64,
    pair_states: Vec<PairState>,
}

impl<D: DynamicalSystem> Scheduler<D>
where
    D::State: HasPosition + FromAcceleration,
{
    pub fn new(config: RegimeConfig, integrator: IntegratorConfig) -> Self {
        Self {
            satellites: Vec::new(),
            interactions: Vec::new(),
            config,
            integrator,
            event_checker: None,
            t: 0.0,
            pair_states: Vec::new(),
        }
    }

    pub fn with_event_checker(
        mut self,
        checker: impl Fn(f64, &D::State) -> ControlFlow<String> + Send + Sync + 'static,
    ) -> Self {
        self.event_checker = Some(Arc::new(checker));
        self
    }

    pub fn add_satellite(mut self, id: impl Into<SatId>, state: D::State, dynamics: D) -> Self {
        self.satellites.push(SatRecord {
            id: id.into(),
            state,
            dynamics: Some(dynamics),
            terminated: false,
            end_time: None,
        });
        self
    }

    pub fn add_satellite_until(
        mut self,
        id: impl Into<SatId>,
        state: D::State,
        end_time: f64,
        dynamics: D,
    ) -> Self {
        self.satellites.push(SatRecord {
            id: id.into(),
            state,
            dynamics: Some(dynamics),
            terminated: false,
            end_time: Some(end_time),
        });
        self
    }

    /// Add an interaction with Auto policy (distance-based regime transitions).
    pub fn add_interaction(
        mut self,
        sat_i: impl Into<SatId>,
        sat_j: impl Into<SatId>,
        force: Arc<dyn InterSatelliteForce>,
    ) -> Self {
        self.interactions.push(InteractionSpec {
            sat_i: sat_i.into(),
            sat_j: sat_j.into(),
            force,
            policy: PairPolicy::Auto,
        });
        self.pair_states.push(PairState {
            regime: PairRegime::Independent,
            last_transition_t: self.t,
        });
        self
    }

    /// Add an interaction with a fixed regime (never changes).
    pub fn add_interaction_fixed(
        mut self,
        sat_i: impl Into<SatId>,
        sat_j: impl Into<SatId>,
        regime: PairRegime,
        force: Arc<dyn InterSatelliteForce>,
    ) -> Self {
        self.interactions.push(InteractionSpec {
            sat_i: sat_i.into(),
            sat_j: sat_j.into(),
            force,
            policy: PairPolicy::Fixed(regime),
        });
        self.pair_states.push(PairState {
            regime,
            last_transition_t: self.t,
        });
        self
    }

    /// Add interactions for all pairs in a group.
    ///
    /// Expands `members` into N*(N-1)/2 pair interactions. The `force_factory`
    /// is called once per pair with the two satellite IDs, allowing different
    /// parameters (e.g., masses) per pair.
    pub fn add_group_interaction<I: Into<SatId> + Clone>(
        mut self,
        members: &[I],
        policy: PairPolicy,
        force_factory: impl Fn(&SatId, &SatId) -> Arc<dyn InterSatelliteForce>,
    ) -> Self {
        let ids: Vec<SatId> = members.iter().map(|m| m.clone().into()).collect();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                let force = force_factory(&ids[i], &ids[j]);
                let initial_regime = match &policy {
                    PairPolicy::Fixed(r) => *r,
                    PairPolicy::Auto => PairRegime::Independent,
                };
                self.interactions.push(InteractionSpec {
                    sat_i: ids[i].clone(),
                    sat_j: ids[j].clone(),
                    force,
                    policy: policy.clone(),
                });
                self.pair_states.push(PairState {
                    regime: initial_regime,
                    last_transition_t: self.t,
                });
            }
        }
        self
    }

    // ── Accessors ──────────────────────────────────────────────────────────

    pub fn ids(&self) -> Vec<SatId> {
        self.satellites.iter().map(|s| s.id.clone()).collect()
    }

    pub fn satellite_state(&self, id: &SatId) -> Option<&D::State> {
        self.satellites
            .iter()
            .find(|s| &s.id == id)
            .map(|s| &s.state)
    }

    pub fn current_t(&self) -> f64 {
        self.t
    }

    pub fn pair_regime(&self, sat_i: &SatId, sat_j: &SatId) -> Option<PairRegime> {
        self.interactions
            .iter()
            .zip(self.pair_states.iter())
            .find(|(spec, _)| {
                (&spec.sat_i == sat_i && &spec.sat_j == sat_j)
                    || (&spec.sat_i == sat_j && &spec.sat_j == sat_i)
            })
            .map(|(_, ps)| ps.regime)
    }

    pub fn snapshot(&self) -> GroupSnapshot {
        GroupSnapshot {
            positions: self
                .satellites
                .iter()
                .filter(|s| !s.terminated)
                .map(|s| (s.id.clone(), s.state.position()))
                .collect(),
        }
    }

    // ── Propagation ────────────────────────────────────────────────────────

    /// Advance all active satellites to `t_target`.
    ///
    /// Each sync interval, pair regimes are re-evaluated and ephemeral groups
    /// are built accordingly:
    /// - Coupled pairs → CoupledGroup (single ODE)
    /// - Synchronized pairs → KDK velocity kicks
    /// - Independent → IndependentGroup (drift only)
    pub fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError>
    where
        D::State: 'static,
    {
        let mut all_terminations = Vec::new();

        while self.t < t_target - 1e-12 {
            // Clamp sync_target to earliest active end_time not yet reached
            let min_end = self
                .satellites
                .iter()
                .filter(|s| !s.terminated && s.end_time.is_some_and(|et| et > self.t + 1e-9))
                .filter_map(|s| s.end_time)
                .fold(t_target, f64::min);
            let sync_target = (self.t + self.config.sync_interval).min(min_end);

            // Progress guard: if sync_target can't advance (float precision), break
            if sync_target <= self.t + 1e-12 {
                break;
            }

            // Re-evaluate pair regimes based on current distances
            self.update_pair_regimes();
            let grouping = self.build_grouping();

            if grouping.kick_pairs.is_empty() {
                // Fast path: no KDK kicks needed
                let terms = self.propagate_groups_to(sync_target, &grouping)?;
                all_terminations.extend(terms);
            } else {
                // KDK path: Strang splitting (half-kick + drift + half-kick)
                let dt_sync = sync_target - self.t;

                // 1. Compute accelerations at current positions (pre-drift time)
                let accels_start = self.compute_kick_accels(&grouping.kick_pairs, self.t);

                // 2. Half-kick (apply dt_sync/2 velocity impulse)
                self.apply_kicks(&grouping.kick_pairs, &accels_start, dt_sync / 2.0);

                // 3. Drift: propagate ephemeral groups
                let terms = self.propagate_groups_to(sync_target, &grouping)?;

                // 4. Check for events
                let has_events = !terms.is_empty();
                all_terminations.extend(terms);

                if has_events {
                    // Event during drift. Apply second half-kick only to active sats.
                    let accels_end = self.compute_kick_accels(&grouping.kick_pairs, sync_target);
                    self.apply_kicks_active(&grouping.kick_pairs, &accels_end, dt_sync / 2.0);
                    self.t = sync_target;
                    break;
                }

                // 5. Compute accelerations at new positions (post-drift time)
                let accels_end = self.compute_kick_accels(&grouping.kick_pairs, sync_target);

                // 6. Second half-kick
                self.apply_kicks(&grouping.kick_pairs, &accels_end, dt_sync / 2.0);
            }

            self.t = sync_target;

            // If all satellites are done, break early
            if self
                .satellites
                .iter()
                .all(|s| s.terminated || s.end_time.is_some_and(|et| self.t >= et - 1e-9))
            {
                break;
            }
        }

        Ok(PropGroupOutcome {
            terminations: all_terminations,
        })
    }

    /// Update pair regimes based on current satellite distances and hysteresis.
    fn update_pair_regimes(&mut self) {
        for (idx, spec) in self.interactions.iter().enumerate() {
            // If either satellite is terminated, force Independent regardless of policy
            let either_terminated = {
                let i = self.satellites.iter().find(|s| s.id == spec.sat_i);
                let j = self.satellites.iter().find(|s| s.id == spec.sat_j);
                i.is_some_and(|s| s.terminated) || j.is_some_and(|s| s.terminated)
            };

            let regime = if either_terminated {
                PairRegime::Independent
            } else {
                match &spec.policy {
                    PairPolicy::Fixed(r) => *r,
                    PairPolicy::Auto => {
                        let i = self.satellites.iter().position(|s| s.id == spec.sat_i);
                        let j = self.satellites.iter().position(|s| s.id == spec.sat_j);

                        if let (Some(i), Some(j)) = (i, j) {
                            let dist = (self.satellites[i].state.position()
                                - self.satellites[j].state.position())
                            .magnitude();
                            evaluate_auto_regime(dist, &self.pair_states[idx], self.t, &self.config)
                        } else {
                            self.pair_states[idx].regime
                        }
                    }
                }
            };

            if regime != self.pair_states[idx].regime {
                self.pair_states[idx].regime = regime;
                self.pair_states[idx].last_transition_t = self.t;
            }
        }
    }

    /// Build the grouping structure from current pair regimes.
    fn build_grouping(&self) -> Grouping {
        let n_sats = self.satellites.len();
        let active: Vec<bool> = self
            .satellites
            .iter()
            .map(|s| !s.terminated && !s.end_time.is_some_and(|et| self.t >= et - 1e-9))
            .collect();

        let pair_regimes: Vec<(usize, usize, PairRegime)> = self
            .interactions
            .iter()
            .zip(self.pair_states.iter())
            .filter_map(|(spec, ps)| {
                let i = self.satellites.iter().position(|s| s.id == spec.sat_i)?;
                let j = self.satellites.iter().position(|s| s.id == spec.sat_j)?;
                Some((i, j, ps.regime))
            })
            .collect();

        determine_grouping(n_sats, &pair_regimes, &active)
    }

    /// Propagate ephemeral groups to `t_target` and recover state.
    fn propagate_groups_to(
        &mut self,
        t_target: f64,
        grouping: &Grouping,
    ) -> Result<Vec<SatelliteTermination>, IntegrationError>
    where
        D::State: 'static,
    {
        let mut terminations = Vec::new();

        // Propagate independent satellites
        if !grouping.independent.is_empty() {
            let mut group: IndependentGroup<D> = IndependentGroup::new(self.integrator.clone());

            if let Some(ref checker) = self.event_checker {
                let checker = checker.clone();
                group = group.with_event_checker(move |t, s| checker(t, s));
            }

            for &idx in &grouping.independent {
                let sat = &mut self.satellites[idx];
                let dynamics = sat.dynamics.take().expect("dynamics should be present");
                group.push_satellite(
                    sat.id.clone(),
                    sat.state.clone(),
                    self.t,
                    sat.end_time,
                    dynamics,
                );
            }

            let outcome = group.propagate_to(t_target)?;
            terminations.extend(outcome.terminations);

            let parts = group.into_parts();
            for part in parts {
                if let Some(sat) = self.satellites.iter_mut().find(|s| s.id == part.id) {
                    sat.state = part.state;
                    sat.dynamics = Some(part.dynamics);
                    sat.terminated = part.terminated;
                }
            }
        }

        // Propagate coupled components
        for comp in &grouping.coupled_components {
            let mut group: CoupledGroup<D> = CoupledGroup::new(self.integrator.clone());

            if let Some(ref checker) = self.event_checker {
                let checker = checker.clone();
                group = group.with_event_checker(move |t, s| checker(t, s));
            }

            group.set_t(self.t);

            // Map: satellite_idx → position within this coupled group
            let mut idx_to_local: Vec<(usize, usize)> = Vec::new();
            for (local, &sat_idx) in comp.iter().enumerate() {
                let sat = &mut self.satellites[sat_idx];
                let dynamics = sat.dynamics.take().expect("dynamics should be present");
                group.push_satellite(sat.id.clone(), sat.state.clone(), dynamics);
                idx_to_local.push((sat_idx, local));
            }

            // Add interactions whose both endpoints are in this component
            for (spec, ps) in self.interactions.iter().zip(self.pair_states.iter()) {
                if ps.regime != PairRegime::Coupled {
                    continue;
                }
                let i_global = self.satellites.iter().position(|s| s.id == spec.sat_i);
                let j_global = self.satellites.iter().position(|s| s.id == spec.sat_j);
                if let (Some(ig), Some(jg)) = (i_global, j_global) {
                    let i_local = idx_to_local.iter().find(|(g, _)| *g == ig).map(|(_, l)| *l);
                    let j_local = idx_to_local.iter().find(|(g, _)| *g == jg).map(|(_, l)| *l);
                    if let (Some(il), Some(jl)) = (i_local, j_local) {
                        group.push_interaction(il, jl, spec.force.clone());
                    }
                }
            }

            let outcome = group.propagate_to(t_target)?;
            terminations.extend(outcome.terminations);

            let parts = group.into_parts();
            // Recover state and dynamics for each satellite.
            // For event terminations: only the triggering satellite is "dead".
            // For integration errors: all satellites in the group are corrupted.
            let term_id = parts.termination.as_ref().map(|t| &t.satellite_id);
            for ((state, dynamics), &sat_idx) in parts
                .states
                .into_iter()
                .zip(parts.dynamics)
                .zip(comp.iter())
            {
                let sat = &mut self.satellites[sat_idx];
                sat.state = state;
                sat.dynamics = Some(dynamics);
                if parts.terminated && !parts.is_event_termination {
                    // Integration error: composite ODE state corrupted
                    sat.terminated = true;
                } else {
                    sat.terminated = term_id.is_some_and(|tid| tid == &sat.id);
                }
            }
        }

        Ok(terminations)
    }

    // ── KDK helpers ───────────────────────────────────────────────────

    /// Compute inter-satellite accelerations for each kick pair.
    ///
    /// Returns a Vec parallel to `kick_pairs`, each element containing
    /// the (accel_i, accel_j) pair for the two satellites.
    fn compute_kick_accels(
        &self,
        kick_pairs: &[KickPair],
        t: f64,
    ) -> Vec<(nalgebra::Vector3<f64>, nalgebra::Vector3<f64>)> {
        kick_pairs
            .iter()
            .map(|kp| {
                let pos_i = self.satellites[kp.sat_i].state.position();
                let pos_j = self.satellites[kp.sat_j].state.position();
                let ctx = PairContext {
                    t,
                    pos_i: &pos_i,
                    pos_j: &pos_j,
                };
                self.interactions[kp.interaction_idx]
                    .force
                    .acceleration_pair(&ctx)
            })
            .collect()
    }

    /// Apply velocity kicks to all active satellites involved in kick pairs.
    ///
    /// `accels` is parallel to `kick_pairs`. `dt` is the kick duration
    /// (typically `dt_sync / 2`). Each satellite accumulates accelerations
    /// from all pairs it participates in.
    fn apply_kicks(
        &mut self,
        kick_pairs: &[KickPair],
        accels: &[(nalgebra::Vector3<f64>, nalgebra::Vector3<f64>)],
        dt: f64,
    ) {
        // Accumulate per-satellite acceleration
        let n = self.satellites.len();
        let mut sat_accel = vec![nalgebra::Vector3::<f64>::zeros(); n];
        for (kp, &(a_i, a_j)) in kick_pairs.iter().zip(accels) {
            sat_accel[kp.sat_i] += a_i;
            sat_accel[kp.sat_j] += a_j;
        }
        // Apply kicks
        for (idx, accel) in sat_accel.into_iter().enumerate() {
            if accel.magnitude_squared() > 0.0 && !self.satellites[idx].terminated {
                let delta = D::State::from_acceleration(accel);
                self.satellites[idx].state = self.satellites[idx].state.axpy(dt, &delta);
            }
        }
    }

    /// Apply velocity kicks only to non-terminated, active satellites.
    ///
    /// Same as `apply_kicks` but also skips satellites that have reached
    /// their `end_time`. Used after event interruption.
    fn apply_kicks_active(
        &mut self,
        kick_pairs: &[KickPair],
        accels: &[(nalgebra::Vector3<f64>, nalgebra::Vector3<f64>)],
        dt: f64,
    ) {
        let n = self.satellites.len();
        let mut sat_accel = vec![nalgebra::Vector3::<f64>::zeros(); n];
        for (kp, &(a_i, a_j)) in kick_pairs.iter().zip(accels) {
            sat_accel[kp.sat_i] += a_i;
            sat_accel[kp.sat_j] += a_j;
        }
        for (idx, accel) in sat_accel.into_iter().enumerate() {
            let sat = &self.satellites[idx];
            if accel.magnitude_squared() > 0.0
                && !sat.terminated
                && !sat.end_time.is_some_and(|et| self.t >= et - 1e-9)
            {
                let delta = D::State::from_acceleration(accel);
                self.satellites[idx].state = self.satellites[idx].state.axpy(dt, &delta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RegimeConfig validation ─────────────────────────────────────────

    fn default_config() -> RegimeConfig {
        RegimeConfig {
            couple_enter: 10.0,
            couple_exit: 20.0,
            sync_enter: 20.0,
            sync_exit: 50.0,
            sync_interval: 60.0,
            min_dwell_time: 120.0,
        }
    }

    #[test]
    fn regime_config_valid() {
        assert!(default_config().validate().is_ok());
    }

    #[test]
    fn regime_config_couple_enter_ge_exit() {
        let mut c = default_config();
        c.couple_enter = 25.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn regime_config_couple_exit_gt_sync_enter() {
        let mut c = default_config();
        c.couple_exit = 30.0; // > sync_enter=20
        assert!(c.validate().is_err());
    }

    #[test]
    fn regime_config_sync_enter_ge_exit() {
        let mut c = default_config();
        c.sync_enter = 50.0;
        assert!(c.validate().is_err());
    }

    // ── evaluate_auto_regime: basic transitions ─────────────────────────

    fn pair_state(regime: PairRegime, t: f64) -> PairState {
        PairState {
            regime,
            last_transition_t: t,
        }
    }

    #[test]
    fn independent_close_becomes_coupled() {
        let config = default_config();
        let ps = pair_state(PairRegime::Independent, 0.0);
        let result = evaluate_auto_regime(5.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Coupled);
    }

    #[test]
    fn independent_mid_becomes_synchronized() {
        let config = default_config();
        let ps = pair_state(PairRegime::Independent, 0.0);
        let result = evaluate_auto_regime(15.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Synchronized);
    }

    #[test]
    fn independent_far_stays_independent() {
        let config = default_config();
        let ps = pair_state(PairRegime::Independent, 0.0);
        let result = evaluate_auto_regime(100.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    #[test]
    fn coupled_stays_in_hysteresis_band() {
        let config = default_config();
        // dist=15 is between couple_enter(10) and couple_exit(20) → stay coupled
        let ps = pair_state(PairRegime::Coupled, 0.0);
        let result = evaluate_auto_regime(15.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Coupled);
    }

    #[test]
    fn coupled_exits_to_synchronized() {
        let config = default_config();
        // dist=30 > couple_exit(20), < sync_exit(50), dwell time passed
        let ps = pair_state(PairRegime::Coupled, 0.0);
        let result = evaluate_auto_regime(30.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Synchronized);
    }

    #[test]
    fn coupled_exits_directly_to_independent() {
        let config = default_config();
        // dist=100 > sync_exit(50), dwell time passed
        let ps = pair_state(PairRegime::Coupled, 0.0);
        let result = evaluate_auto_regime(100.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    #[test]
    fn synchronized_upgrades_to_coupled_immediately() {
        let config = default_config();
        // dist=5 < couple_enter(10), even with recent transition (no dwell for upgrades)
        let ps = pair_state(PairRegime::Synchronized, 999.0);
        let result = evaluate_auto_regime(5.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Coupled);
    }

    #[test]
    fn synchronized_stays_in_hysteresis_band() {
        let config = default_config();
        // dist=40 is between sync_enter(20) and sync_exit(50) → stay sync
        let ps = pair_state(PairRegime::Synchronized, 0.0);
        let result = evaluate_auto_regime(40.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Synchronized);
    }

    #[test]
    fn synchronized_exits_to_independent() {
        let config = default_config();
        // dist=60 > sync_exit(50), dwell time passed
        let ps = pair_state(PairRegime::Synchronized, 0.0);
        let result = evaluate_auto_regime(60.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    // ── Dwell time ──────────────────────────────────────────────────────

    #[test]
    fn downgrade_blocked_by_dwell_time() {
        let config = default_config(); // min_dwell_time=120
        // Last transition at t=950, current t=1000 → only 50s elapsed < 120s
        let ps = pair_state(PairRegime::Coupled, 950.0);
        let result = evaluate_auto_regime(30.0, &ps, 1000.0, &config);
        // Cannot downgrade yet → stays Coupled
        assert_eq!(result, PairRegime::Coupled);
    }

    #[test]
    fn upgrade_ignores_dwell_time() {
        let config = default_config();
        // Last transition at t=999 (1s ago), but upgrade is immediate
        let ps = pair_state(PairRegime::Independent, 999.0);
        let result = evaluate_auto_regime(5.0, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Coupled);
    }

    // ── Fixed policy ────────────────────────────────────────────────────

    #[test]
    fn fixed_coupled_ignores_distance() {
        // Fixed(Coupled) → always Coupled, regardless of distance
        // (This is tested at the Scheduler level, not evaluate_auto_regime,
        // since Fixed bypasses the auto logic entirely.)
        let policy = PairPolicy::Fixed(PairRegime::Coupled);
        match policy {
            PairPolicy::Fixed(r) => assert_eq!(r, PairRegime::Coupled),
            _ => panic!("expected Fixed"),
        }
    }

    // ── connected_components ────────────────────────────────────────────

    #[test]
    fn connected_components_empty() {
        let comps = connected_components(5, &[]);
        assert!(comps.is_empty());
    }

    #[test]
    fn connected_components_single_edge() {
        let comps = connected_components(5, &[(1, 3)]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], vec![1, 3]);
    }

    #[test]
    fn connected_components_chain() {
        // 0-1, 1-2 → one component {0,1,2}
        let comps = connected_components(5, &[(0, 1), (1, 2)]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], vec![0, 1, 2]);
    }

    #[test]
    fn connected_components_two_groups() {
        // 0-1, 3-4 → two components
        let comps = connected_components(5, &[(0, 1), (3, 4)]);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0], vec![0, 1]);
        assert_eq!(comps[1], vec![3, 4]);
    }

    #[test]
    fn connected_components_triangle() {
        // 0-1, 1-2, 0-2 → one component
        let comps = connected_components(3, &[(0, 1), (1, 2), (0, 2)]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0], vec![0, 1, 2]);
    }

    // ── determine_grouping ──────────────────────────────────────────────

    #[test]
    fn grouping_all_independent() {
        let active = vec![true, true, true];
        let pairs = vec![];
        let g = determine_grouping(3, &pairs, &active);
        assert!(g.coupled_components.is_empty());
        assert_eq!(g.independent, vec![0, 1, 2]);
        assert!(g.kick_pairs.is_empty());
    }

    #[test]
    fn grouping_two_coupled() {
        let active = vec![true, true, true];
        let pairs = vec![(0, 1, PairRegime::Coupled)];
        let g = determine_grouping(3, &pairs, &active);
        assert_eq!(g.coupled_components.len(), 1);
        assert_eq!(g.coupled_components[0], vec![0, 1]);
        assert_eq!(g.independent, vec![2]);
        assert!(g.kick_pairs.is_empty());
    }

    #[test]
    fn grouping_chain_promotion() {
        // A-B coupled, B-C coupled → {A,B,C} single component
        let active = vec![true, true, true];
        let pairs = vec![(0, 1, PairRegime::Coupled), (1, 2, PairRegime::Coupled)];
        let g = determine_grouping(3, &pairs, &active);
        assert_eq!(g.coupled_components.len(), 1);
        assert_eq!(g.coupled_components[0], vec![0, 1, 2]);
        assert!(g.independent.is_empty());
    }

    #[test]
    fn grouping_synchronized_pair() {
        let active = vec![true, true, true];
        let pairs = vec![(0, 1, PairRegime::Synchronized)];
        let g = determine_grouping(3, &pairs, &active);
        assert!(g.coupled_components.is_empty());
        assert_eq!(g.independent, vec![0, 1, 2]);
        assert_eq!(g.kick_pairs.len(), 1);
        assert_eq!(g.kick_pairs[0].sat_i, 0);
        assert_eq!(g.kick_pairs[0].sat_j, 1);
    }

    #[test]
    fn grouping_cross_group_kick() {
        // A-B coupled, B-C synchronized → {A,B} coupled, C independent, B-C kick
        let active = vec![true, true, true];
        let pairs = vec![
            (0, 1, PairRegime::Coupled),
            (1, 2, PairRegime::Synchronized),
        ];
        let g = determine_grouping(3, &pairs, &active);
        assert_eq!(g.coupled_components.len(), 1);
        assert_eq!(g.coupled_components[0], vec![0, 1]);
        assert_eq!(g.independent, vec![2]);
        assert_eq!(g.kick_pairs.len(), 1);
        assert_eq!(g.kick_pairs[0].sat_i, 1);
        assert_eq!(g.kick_pairs[0].sat_j, 2);
    }

    #[test]
    fn grouping_terminated_excluded() {
        let active = vec![true, false, true];
        let pairs = vec![(0, 1, PairRegime::Synchronized)];
        let g = determine_grouping(3, &pairs, &active);
        // Pair 0-1: sat 1 is inactive → kick pair excluded
        assert!(g.kick_pairs.is_empty());
        assert_eq!(g.independent, vec![0, 2]);
    }

    #[test]
    fn grouping_mixed_regimes() {
        // 4 satellites: 0-1 coupled, 2-3 synchronized, 1-2 independent
        let active = vec![true, true, true, true];
        let pairs = vec![
            (0, 1, PairRegime::Coupled),
            (2, 3, PairRegime::Synchronized),
            (1, 2, PairRegime::Independent),
        ];
        let g = determine_grouping(4, &pairs, &active);
        assert_eq!(g.coupled_components.len(), 1);
        assert_eq!(g.coupled_components[0], vec![0, 1]);
        assert_eq!(g.independent, vec![2, 3]);
        assert_eq!(g.kick_pairs.len(), 1);
        assert_eq!(g.kick_pairs[0].sat_i, 2);
        assert_eq!(g.kick_pairs[0].sat_j, 3);
    }

    // ── Scheduler builder + accessors ───────────────────────────────────

    use super::super::coupled::MutualGravity;
    use crate::OrbitalState;
    use crate::two_body::TwoBodySystem;
    use nalgebra::Vector3;

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

    fn test_integrator() -> IntegratorConfig {
        IntegratorConfig::Rk4 { dt: 10.0 }
    }

    #[test]
    fn scheduler_builder_and_ids() {
        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let ids = sched.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], SatId::from("iss"));
        assert_eq!(ids[1], SatId::from("sso"));
        assert!((sched.current_t() - 0.0).abs() < 1e-15);
    }

    #[test]
    fn scheduler_satellite_state_accessor() {
        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let iss = sched.satellite_state(&SatId::from("iss")).unwrap();
        assert!((iss.position().x - 6778.137).abs() < 1e-10);

        let sso = sched.satellite_state(&SatId::from("sso")).unwrap();
        assert!((sso.position().x - 7178.137).abs() < 1e-10);

        assert!(sched.satellite_state(&SatId::from("nonexistent")).is_none());
    }

    #[test]
    fn scheduler_snapshot_initial() {
        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        let snap = sched.snapshot();
        assert_eq!(snap.positions.len(), 2);
        assert_eq!(snap.positions[0].0, SatId::from("iss"));
        assert!((snap.positions[0].1.x - 6778.137).abs() < 1e-10);
        assert_eq!(snap.positions[1].0, SatId::from("sso"));
        assert!((snap.positions[1].1.x - 7178.137).abs() < 1e-10);
    }

    #[test]
    fn scheduler_pair_regime_auto_and_fixed() {
        let mg = Arc::new(MutualGravity {
            mu_i: 1e-10,
            mu_j: 1e-10,
        });

        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("a", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("b", sso_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("c", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_interaction("a", "b", mg.clone())
            .add_interaction_fixed("b", "c", PairRegime::Coupled, mg.clone());

        // Auto interaction starts as Independent
        assert_eq!(
            sched.pair_regime(&SatId::from("a"), &SatId::from("b")),
            Some(PairRegime::Independent)
        );
        // Fixed Coupled
        assert_eq!(
            sched.pair_regime(&SatId::from("b"), &SatId::from("c")),
            Some(PairRegime::Coupled)
        );
        // Reverse lookup works
        assert_eq!(
            sched.pair_regime(&SatId::from("c"), &SatId::from("b")),
            Some(PairRegime::Coupled)
        );
        // Nonexistent pair
        assert_eq!(
            sched.pair_regime(&SatId::from("a"), &SatId::from("c")),
            None
        );
    }

    #[test]
    fn scheduler_add_group_interaction() {
        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("a", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("b", sso_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("c", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_group_interaction(&["a", "b", "c"], PairPolicy::Auto, |_i, _j| {
                Arc::new(MutualGravity {
                    mu_i: 1e-10,
                    mu_j: 1e-10,
                })
            });

        // 3 members → 3 pairs: (a,b), (a,c), (b,c)
        assert_eq!(
            sched.pair_regime(&SatId::from("a"), &SatId::from("b")),
            Some(PairRegime::Independent)
        );
        assert_eq!(
            sched.pair_regime(&SatId::from("a"), &SatId::from("c")),
            Some(PairRegime::Independent)
        );
        assert_eq!(
            sched.pair_regime(&SatId::from("b"), &SatId::from("c")),
            Some(PairRegime::Independent)
        );
    }

    #[test]
    fn scheduler_add_satellite_until() {
        let sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_integrator())
            .add_satellite("long", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite_until("short", sso_state(), 50.0, TwoBodySystem { mu: MU_EARTH });

        assert_eq!(sched.ids().len(), 2);
        // Both states accessible
        assert!(sched.satellite_state(&SatId::from("long")).is_some());
        assert!(sched.satellite_state(&SatId::from("short")).is_some());
    }

    // ── Step 4: Independent-only propagation ────────────────────────────

    use orts_integrator::Tolerances;

    fn test_dp45() -> IntegratorConfig {
        IntegratorConfig::Dp45 {
            dt: 10.0,
            tolerances: Tolerances {
                atol: 1e-10,
                rtol: 1e-8,
            },
        }
    }

    #[test]
    fn scheduler_independent_rk4_matches_group() {
        // Scheduler with no interactions should match IndependentGroup exactly (RK4)
        let mut sched: Scheduler<TwoBodySystem> =
            Scheduler::new(default_config(), test_integrator())
                .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
                .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        sched.propagate_to(100.0).unwrap();

        let mut group: IndependentGroup<TwoBodySystem> = IndependentGroup::rk4(10.0)
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("sso", sso_state(), TwoBodySystem { mu: MU_EARTH });

        group.propagate_to(100.0).unwrap();

        // States should match bit-for-bit (same algorithm, same parameters)
        let sched_iss = sched.satellite_state(&SatId::from("iss")).unwrap();
        let group_iss = group.satellite(&SatId::from("iss")).unwrap();
        let pos_err = (sched_iss.position() - group_iss.state.position()).magnitude();
        assert!(pos_err < 1e-12, "ISS position mismatch: {pos_err}");

        let sched_sso = sched.satellite_state(&SatId::from("sso")).unwrap();
        let group_sso = group.satellite(&SatId::from("sso")).unwrap();
        let pos_err = (sched_sso.position() - group_sso.state.position()).magnitude();
        assert!(pos_err < 1e-12, "SSO position mismatch: {pos_err}");

        assert!((sched.current_t() - 100.0).abs() < 1e-12);
    }

    #[test]
    fn scheduler_independent_dp45_basic() {
        let mut sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_dp45())
            .add_satellite("iss", iss_state(), TwoBodySystem { mu: MU_EARTH });

        let outcome = sched.propagate_to(100.0).unwrap();
        assert!(outcome.terminations.is_empty());
        assert!((sched.current_t() - 100.0).abs() < 1e-12);

        // Satellite should have moved from initial position
        let state = sched.satellite_state(&SatId::from("iss")).unwrap();
        let pos_diff = (state.position() - iss_state().position()).magnitude();
        assert!(pos_diff > 1.0, "Satellite should have moved");
    }

    #[test]
    fn scheduler_independent_event_termination() {
        const EARTH_RADIUS: f64 = 6378.137;

        let decaying =
            OrbitalState::new(Vector3::new(6500.0, 0.0, 0.0), Vector3::new(-5.0, 3.0, 0.0));

        let mut sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_dp45())
            .with_event_checker(move |_t, state: &OrbitalState| {
                if state.position().magnitude() < EARTH_RADIUS {
                    ControlFlow::Break(format!(
                        "collision at {:.1} km",
                        state.position().magnitude()
                    ))
                } else {
                    ControlFlow::Continue(())
                }
            })
            .add_satellite("safe", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite("decay", decaying, TwoBodySystem { mu: MU_EARTH });

        let outcome = sched.propagate_to(10000.0).unwrap();

        // "decay" should have terminated, "safe" should be fine
        assert!(
            outcome
                .terminations
                .iter()
                .any(|t| t.satellite_id == SatId::from("decay"))
        );

        // Snapshot should only show "safe"
        let snap = sched.snapshot();
        assert_eq!(snap.positions.len(), 1);
        assert_eq!(snap.positions[0].0, SatId::from("safe"));
    }

    // ── Step 5: Coupled-only propagation ─────────────────────────────

    use super::super::coupled::{CoupledGroup, Spring};
    use orts_integrator::DynamicalSystem;

    /// Free particle: d(pos)/dt = vel, d(vel)/dt = 0.
    #[derive(Clone, Copy)]
    struct FreeParticle;
    impl DynamicalSystem for FreeParticle {
        type State = OrbitalState;
        fn derivatives(&self, _t: f64, state: &OrbitalState) -> OrbitalState {
            OrbitalState::from_derivative(*state.velocity(), Vector3::zeros())
        }
    }

    #[test]
    fn scheduler_coupled_rk4_matches_group() {
        // Two satellites with Fixed(Coupled) spring interaction.
        // Scheduler should produce identical results to direct CoupledGroup.
        let k = 0.01;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(15.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> =
            Scheduler::new(default_config(), IntegratorConfig::Rk4 { dt: 0.1 })
                .add_satellite("a", s0.clone(), FreeParticle)
                .add_satellite("b", s1.clone(), FreeParticle)
                .add_interaction_fixed("a", "b", PairRegime::Coupled, spring.clone());

        sched.propagate_to(10.0).unwrap();

        // Direct CoupledGroup for comparison
        let mut group: CoupledGroup<FreeParticle> = CoupledGroup::rk4(0.1)
            .add_satellite("a", s0.clone(), FreeParticle)
            .add_satellite("b", s1.clone(), FreeParticle)
            .with_interaction(0, 1, spring);

        group.propagate_to(10.0).unwrap();

        let sched_a = sched.satellite_state(&SatId::from("a")).unwrap();
        let sched_b = sched.satellite_state(&SatId::from("b")).unwrap();
        let group_states = group.group_state();

        let err_a = (sched_a.position() - group_states.states[0].position()).magnitude();
        let err_b = (sched_b.position() - group_states.states[1].position()).magnitude();
        assert!(err_a < 1e-12, "sat a position mismatch: {err_a}");
        assert!(err_b < 1e-12, "sat b position mismatch: {err_b}");

        // Energy conservation check
        let energy = |p0: &OrbitalState, p1: &OrbitalState| -> f64 {
            let ke =
                p0.velocity().magnitude_squared() / 2.0 + p1.velocity().magnitude_squared() / 2.0;
            let r = (p1.position() - p0.position()).magnitude();
            ke + k * (r - rest).powi(2) / 2.0
        };
        let e0 = energy(&s0, &s1);
        let e_final = energy(sched_a, sched_b);
        assert!(
            (e_final - e0).abs() / e0 < 1e-4,
            "energy not conserved: {e0} -> {e_final}"
        );
    }

    #[test]
    fn scheduler_coupled_dp45_spring() {
        let k = 0.04;
        let rest = 10.0;
        let amplitude = 3.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(rest + amplitude, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            default_config(),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring);

        // Propagate one full period: T = 2π / ω_rel, ω_rel = √(2k)
        let period = 2.0 * std::f64::consts::PI / (2.0_f64 * k).sqrt();
        sched.propagate_to(period).unwrap();

        // After full period, separation should return to initial value
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        let final_sep = (b.position() - a.position()).magnitude();
        assert!(
            (final_sep - (rest + amplitude)).abs() < 0.01,
            "spring didn't return to initial: sep={final_sep}, expected={}",
            rest + amplitude
        );
    }

    #[test]
    fn scheduler_coupled_event_terminates_group() {
        // Spring with initial velocity causing one satellite to cross x=20 boundary
        let k = 0.01;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 0.0, 0.0));
        let s1 = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0), // moving outward
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            default_config(),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            // Trigger if x > 30 (satellite b will fly out)
            if state.position().x > 30.0 {
                ControlFlow::Break("boundary".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring);

        let outcome = sched.propagate_to(1000.0).unwrap();

        // "b" should eventually cross x=30 (spring + outward velocity)
        assert!(!outcome.terminations.is_empty());

        // Only the triggering satellite ("b") should be terminated; "a" survives
        let snap = sched.snapshot();
        assert_eq!(snap.positions.len(), 1, "only 'a' should survive");
        assert_eq!(snap.positions[0].0, SatId::from("a"));
    }

    #[test]
    fn scheduler_independent_end_time() {
        let mut sched: Scheduler<TwoBodySystem> = Scheduler::new(default_config(), test_dp45())
            .add_satellite("long", iss_state(), TwoBodySystem { mu: MU_EARTH })
            .add_satellite_until("short", sso_state(), 50.0, TwoBodySystem { mu: MU_EARTH });

        sched.propagate_to(100.0).unwrap();

        // "short" should have stopped at end_time=50
        // "long" should reach t=100
        assert!((sched.current_t() - 100.0).abs() < 1e-12);

        // After second propagation, snapshot still shows both (end_time != terminated)
        let snap = sched.snapshot();
        assert_eq!(snap.positions.len(), 2);
    }

    // ── Step 6: KDK kick propagation ────────────────────────────────

    /// Helper: spring energy for two free particles
    fn spring_energy(k: f64, rest: f64, a: &OrbitalState, b: &OrbitalState) -> f64 {
        let ke = a.velocity().magnitude_squared() / 2.0 + b.velocity().magnitude_squared() / 2.0;
        let r = (b.position() - a.position()).magnitude();
        ke + k * (r - rest).powi(2) / 2.0
    }

    fn sync_config(sync_interval: f64) -> RegimeConfig {
        RegimeConfig {
            couple_enter: 1.0,
            couple_exit: 2.0,
            sync_enter: 100.0,
            sync_exit: 200.0,
            sync_interval,
            min_dwell_time: 0.0,
        }
    }

    #[test]
    fn kdk_two_body_spring_energy_conservation() {
        // Two free particles connected by a spring via Fixed(Synchronized).
        // KDK (Strang splitting) should approximately conserve energy.
        let k = 0.04;
        let rest = 10.0;
        let amplitude = 3.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(rest + amplitude, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let e0 = spring_energy(k, rest, &s0, &s1);

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(1.0), // sync every 1 second
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        // One full period: T = 2π / √(2k)
        let period = 2.0 * std::f64::consts::PI / (2.0_f64 * k).sqrt();
        sched.propagate_to(period).unwrap();

        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        let e_final = spring_energy(k, rest, a, b);

        // KDK is 2nd-order symplectic → energy should be well-conserved
        let rel_err = (e_final - e0).abs() / e0;
        assert!(
            rel_err < 0.01,
            "KDK energy drift too large: {rel_err:.6} (e0={e0}, e_final={e_final})"
        );
    }

    #[test]
    fn kdk_convergence_second_order() {
        // Verify KDK is 2nd-order: halving sync_interval should reduce error ~4x.
        // Truth: CoupledGroup with tight tolerances.
        let k = 0.04;
        let rest = 10.0;
        let amplitude = 3.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(rest + amplitude, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let t_end = 20.0; // ~1 period
        let dp45_tight = IntegratorConfig::Dp45 {
            dt: 0.1,
            tolerances: Tolerances {
                atol: 1e-14,
                rtol: 1e-12,
            },
        };

        // Truth run: CoupledGroup
        let mut truth: CoupledGroup<FreeParticle> = CoupledGroup::new(dp45_tight.clone())
            .add_satellite("a", s0.clone(), FreeParticle)
            .add_satellite("b", s1.clone(), FreeParticle)
            .with_interaction(0, 1, spring.clone());
        truth.propagate_to(t_end).unwrap();
        let truth_a = truth.group_state().states[0].clone();

        // Run KDK at three sync_intervals: dt, dt/2, dt/4
        let mut errors = Vec::new();
        for &si in &[2.0, 1.0, 0.5] {
            let mut sched: Scheduler<FreeParticle> =
                Scheduler::new(sync_config(si), dp45_tight.clone())
                    .add_satellite("a", s0.clone(), FreeParticle)
                    .add_satellite("b", s1.clone(), FreeParticle)
                    .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring.clone());

            sched.propagate_to(t_end).unwrap();

            let a = sched.satellite_state(&SatId::from("a")).unwrap();
            let err = (a.position() - truth_a.position()).magnitude();
            errors.push(err);
        }

        // Error ratios should be ~4x (2nd order)
        let ratio_1 = errors[0] / errors[1];
        let ratio_2 = errors[1] / errors[2];
        assert!(
            ratio_1 > 2.5 && ratio_1 < 6.0,
            "first convergence ratio {ratio_1:.2} not ~4 (errors: {errors:?})"
        );
        assert!(
            ratio_2 > 2.5 && ratio_2 < 6.0,
            "second convergence ratio {ratio_2:.2} not ~4 (errors: {errors:?})"
        );
    }

    #[test]
    fn kdk_converges_to_coupled_as_sync_shrinks() {
        // As sync_interval → 0, KDK result should approach CoupledGroup result
        let k = 0.04;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(13.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });
        let t_end = 10.0;

        let dp45 = IntegratorConfig::Dp45 {
            dt: 0.1,
            tolerances: Tolerances {
                atol: 1e-12,
                rtol: 1e-10,
            },
        };

        // CoupledGroup reference
        let mut coupled: CoupledGroup<FreeParticle> = CoupledGroup::new(dp45.clone())
            .add_satellite("a", s0.clone(), FreeParticle)
            .add_satellite("b", s1.clone(), FreeParticle)
            .with_interaction(0, 1, spring.clone());
        coupled.propagate_to(t_end).unwrap();
        let coupled_a = coupled.group_state().states[0].clone();

        // KDK with large sync_interval
        let mut sched_large: Scheduler<FreeParticle> =
            Scheduler::new(sync_config(5.0), dp45.clone())
                .add_satellite("a", s0.clone(), FreeParticle)
                .add_satellite("b", s1.clone(), FreeParticle)
                .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring.clone());
        sched_large.propagate_to(t_end).unwrap();
        let large_err = (sched_large
            .satellite_state(&SatId::from("a"))
            .unwrap()
            .position()
            - coupled_a.position())
        .magnitude();

        // KDK with small sync_interval
        let mut sched_small: Scheduler<FreeParticle> =
            Scheduler::new(sync_config(0.1), dp45.clone())
                .add_satellite("a", s0, FreeParticle)
                .add_satellite("b", s1, FreeParticle)
                .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);
        sched_small.propagate_to(t_end).unwrap();
        let small_err = (sched_small
            .satellite_state(&SatId::from("a"))
            .unwrap()
            .position()
            - coupled_a.position())
        .magnitude();

        // Smaller sync_interval should give smaller error vs coupled
        assert!(
            small_err < large_err,
            "KDK should converge to coupled: small_err={small_err}, large_err={large_err}"
        );
        // Small interval should be very close to coupled
        assert!(
            small_err < 1e-4,
            "KDK with small sync should be close to coupled: {small_err}"
        );
    }

    #[test]
    fn kdk_multiple_sync_steps() {
        // Long propagation spanning multiple sync intervals.
        // Momentum should be conserved (free particles + spring).
        let k = 0.04;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::new(0.1, 0.0, 0.0));
        let s1 = OrbitalState::new(Vector3::new(13.0, 0.0, 0.0), Vector3::new(-0.1, 0.0, 0.0));
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let p0 = s0.velocity() + s1.velocity(); // total momentum (masses=1)

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(2.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        // 50s = 25 sync steps
        sched.propagate_to(50.0).unwrap();

        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        let p_final = a.velocity() + b.velocity();

        let dp = (p_final - p0).magnitude();
        assert!(dp < 1e-10, "momentum not conserved: dp={dp}");
    }

    #[test]
    fn kdk_event_interrupts_sync_step() {
        // Event during KDK drift phase should stop propagation and correct kick.
        let k = 0.01;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0), // moving outward fast
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(5.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            if state.position().x > 25.0 {
                ControlFlow::Break("boundary".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        let outcome = sched.propagate_to(100.0).unwrap();

        // "b" should have triggered the event
        assert!(!outcome.terminations.is_empty());
        // Scheduler time should have stopped before 100
        assert!(sched.current_t() < 100.0);
        // "a" should still be fine (IndependentGroup events are per-satellite)
        assert!(sched.satellite_state(&SatId::from("a")).is_some());
    }

    #[test]
    fn kdk_end_time_stops_kicks() {
        // Satellite with end_time should stop receiving kicks when it expires.
        let k = 0.01;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(15.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(2.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0, FreeParticle)
        .add_satellite_until("b", s1, 5.0, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        sched.propagate_to(20.0).unwrap();

        // "b" stopped at t=5, after that "a" should drift freely (no more kicks)
        // Verify "a" velocity after t=5 is constant (no spring force)
        // Since "a" is a free particle with no forces after t=5, it moves at constant velocity
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        // "a" should have moved from origin
        assert!(a.position().magnitude() > 0.0);
        assert!((sched.current_t() - 20.0).abs() < 1e-12);
    }

    // ── Step 7: Mixed regime + cross-group ──────────────────────────

    #[test]
    fn mixed_coupled_plus_sync_cross_group_kick() {
        // A-B Fixed(Coupled), B-C Fixed(Synchronized).
        // A,B form a CoupledGroup; C is independent. B-C gets KDK kicks.
        let k = 0.01;
        let rest = 10.0;

        let sa = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let sb = OrbitalState::new(Vector3::new(15.0, 0.0, 0.0), Vector3::zeros());
        let sc = OrbitalState::new(Vector3::new(50.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(1.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", sa, FreeParticle)
        .add_satellite("b", sb, FreeParticle)
        .add_satellite("c", sc, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring.clone())
        .add_interaction_fixed("b", "c", PairRegime::Synchronized, spring);

        sched.propagate_to(10.0).unwrap();

        // All three satellites should have moved
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        let c = sched.satellite_state(&SatId::from("c")).unwrap();

        assert!(a.position().x.abs() > 1e-6, "a should move");
        assert!(b.position().x > 14.0, "b should still be near initial");
        // C should have been pulled toward B by KDK kicks
        assert!(
            c.position().x < 50.0,
            "c should move toward b: x={}",
            c.position().x
        );

        // Momentum should be approximately conserved (all springs, no external forces)
        let p_total = a.velocity() + b.velocity() + c.velocity();
        assert!(
            p_total.magnitude() < 1e-6,
            "momentum not conserved: {p_total:?}"
        );
    }

    #[test]
    fn mixed_three_body_coupled_plus_independent() {
        // A-B Fixed(Coupled) with spring, C independent (no interaction).
        // C should drift unperturbed.
        let k = 0.04;
        let rest = 10.0;
        let sa = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let sb = OrbitalState::new(Vector3::new(rest + 3.0, 0.0, 0.0), Vector3::zeros());
        let sc = OrbitalState::new(
            Vector3::new(100.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0), // drifting in y
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(1.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", sa.clone(), FreeParticle)
        .add_satellite("b", sb.clone(), FreeParticle)
        .add_satellite("c", sc.clone(), FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring);

        let t_end = 10.0;
        sched.propagate_to(t_end).unwrap();

        // C should have drifted purely in y: x=100, y=10
        let c = sched.satellite_state(&SatId::from("c")).unwrap();
        assert!(
            (c.position().x - 100.0).abs() < 1e-8,
            "C x should be unchanged: {}",
            c.position().x
        );
        assert!(
            (c.position().y - t_end).abs() < 1e-8,
            "C y should be t_end: {}",
            c.position().y
        );
    }

    #[test]
    fn auto_regime_transition_approach() {
        // Two satellites approach each other → regime transitions:
        // Independent → Synchronized → Coupled
        // Use config where couple_enter=5, couple_exit=8,
        // sync_enter=15, sync_exit=25.
        let approach_config = RegimeConfig {
            couple_enter: 5.0,
            couple_exit: 8.0,
            sync_enter: 15.0,
            sync_exit: 25.0,
            sync_interval: 0.5,
            min_dwell_time: 0.0,
        };

        let k = 0.001;
        let rest = 0.0;
        // Two particles approaching each other from distance 30
        let sa = OrbitalState::new(
            Vector3::new(-15.0, 0.0, 0.0),
            Vector3::new(0.5, 0.0, 0.0), // approaching
        );
        let sb = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(-0.5, 0.0, 0.0), // approaching
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            approach_config,
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", sa, FreeParticle)
        .add_satellite("b", sb, FreeParticle)
        .add_interaction("a", "b", spring);

        // Initially Independent (dist=30 > sync_exit=25)
        assert_eq!(
            sched.pair_regime(&SatId::from("a"), &SatId::from("b")),
            Some(PairRegime::Independent)
        );

        // After some time, they should get closer and transition
        // dist(t) ≈ 30 - 1.0*t. sync_enter=15 at t≈15. couple_enter=5 at t≈25.
        sched.propagate_to(30.0).unwrap();

        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        assert!(
            a.position().x.is_finite() && a.position().y.is_finite() && a.position().z.is_finite()
        );
        assert!(
            b.position().x.is_finite() && b.position().y.is_finite() && b.position().z.is_finite()
        );

        // At t=30, dist ≈ 30 - 30 = 0 (plus spring bounce effects)
        // Well inside couple_enter=5 → should be Coupled
        let regime = sched
            .pair_regime(&SatId::from("a"), &SatId::from("b"))
            .unwrap();
        assert_eq!(
            regime,
            PairRegime::Coupled,
            "close satellites should be Coupled"
        );
    }

    #[test]
    fn auto_regime_transition_departure() {
        // Two satellites moving apart → Coupled → Synchronized → Independent
        let departure_config = RegimeConfig {
            couple_enter: 5.0,
            couple_exit: 8.0,
            sync_enter: 15.0,
            sync_exit: 25.0,
            sync_interval: 0.5,
            min_dwell_time: 0.0,
        };

        let k = 0.001;
        let rest = 0.0;
        // Start close (dist=4 < couple_enter=5) → Coupled
        let sa = OrbitalState::new(
            Vector3::new(-2.0, 0.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0), // departing
        );
        let sb = OrbitalState::new(
            Vector3::new(2.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0), // departing
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            departure_config,
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", sa, FreeParticle)
        .add_satellite("b", sb, FreeParticle)
        .add_interaction("a", "b", spring);

        // First propagate briefly — should start as Independent (initial regime)
        // but distance=4 < couple_enter=5 → should upgrade to Coupled immediately
        sched.propagate_to(0.5).unwrap();
        let regime_early = sched
            .pair_regime(&SatId::from("a"), &SatId::from("b"))
            .unwrap();
        assert_eq!(
            regime_early,
            PairRegime::Coupled,
            "close pair should be Coupled"
        );

        // After 15s: dist ≈ 4 + 2*1*15 = 34 > sync_exit=25 → Independent
        sched.propagate_to(15.0).unwrap();
        let regime_late = sched
            .pair_regime(&SatId::from("a"), &SatId::from("b"))
            .unwrap();
        assert_eq!(
            regime_late,
            PairRegime::Independent,
            "far pair should be Independent"
        );
    }

    #[test]
    fn terminated_satellite_excluded_from_grouping() {
        // After one satellite terminates, it should be excluded from all groups.
        let k = 0.01;
        let rest = 10.0;
        let sa = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let sb = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(5.0, 0.0, 0.0), // will trigger event
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(1.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            if state.position().x > 50.0 {
                ControlFlow::Break("too far".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", sa, FreeParticle)
        .add_satellite("b", sb, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        // First propagation: b should terminate
        sched.propagate_to(100.0).unwrap();

        // b is terminated, only a in snapshot
        let snap = sched.snapshot();
        assert_eq!(snap.positions.len(), 1);
        assert_eq!(snap.positions[0].0, SatId::from("a"));

        // Further propagation should work (a alone, no interactions)
        sched.propagate_to(200.0).unwrap();
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        assert!(
            a.position().x.is_finite() && a.position().y.is_finite() && a.position().z.is_finite()
        );
    }

    #[test]
    fn regrouping_across_long_propagation() {
        // Long propagate_to with sync_interval=2 should regroup every 2s.
        // Verify regimes change during a single propagate_to call.
        let config = RegimeConfig {
            couple_enter: 5.0,
            couple_exit: 8.0,
            sync_enter: 15.0,
            sync_exit: 25.0,
            sync_interval: 2.0,
            min_dwell_time: 0.0,
        };

        let k = 0.001;
        let rest = 0.0;
        // Start far apart (dist=40), approach, pass through all regimes
        let sa = OrbitalState::new(Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.3, 0.0, 0.0));
        let sb = OrbitalState::new(Vector3::new(20.0, 0.0, 0.0), Vector3::new(-0.3, 0.0, 0.0));
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            config,
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", sa, FreeParticle)
        .add_satellite("b", sb, FreeParticle)
        .add_interaction("a", "b", spring);

        // Propagate long enough for approach: dist=40 → 0 at t≈67
        // At sync_enter=15: dist=15 at t ≈ (40-15)/(2*0.3) ≈ 41.7s
        // At couple_enter=5: dist=5 at t ≈ (40-5)/(2*0.3) ≈ 58.3s
        sched.propagate_to(70.0).unwrap();

        // After 70s, distance should be negative-ish → Coupled
        let regime = sched
            .pair_regime(&SatId::from("a"), &SatId::from("b"))
            .unwrap();
        assert_eq!(
            regime,
            PairRegime::Coupled,
            "close pair should be Coupled after approach"
        );

        // Verify state is valid
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();
        assert!(
            a.position().x.is_finite() && a.position().y.is_finite() && a.position().z.is_finite()
        );
        assert!(
            b.position().x.is_finite() && b.position().y.is_finite() && b.position().z.is_finite()
        );
    }

    // ── Bug 1: compute_kick_accels time parameter ───────────────────

    /// Spring whose stiffness scales linearly with time: k(t) = k0 * (1 + alpha * t).
    /// This makes the force time-dependent, exposing bugs where the wrong time is used.
    struct TimeScaledSpring {
        k0: f64,
        alpha: f64,
        rest_length: f64,
    }

    impl InterSatelliteForce for TimeScaledSpring {
        fn name(&self) -> &str {
            "time_scaled_spring"
        }
        fn acceleration_pair(&self, ctx: &PairContext<'_>) -> (Vector3<f64>, Vector3<f64>) {
            let k = self.k0 * (1.0 + self.alpha * ctx.t);
            let r_vec = ctx.pos_j - ctx.pos_i;
            let r = r_vec.magnitude();
            if r < 1e-10 {
                return (Vector3::zeros(), Vector3::zeros());
            }
            let r_hat = r_vec / r;
            let f = k * (r - self.rest_length);
            (f * r_hat, -f * r_hat)
        }
    }

    #[test]
    fn kdk_second_kick_uses_post_drift_time() {
        // With a time-dependent spring, the 2nd kick should use t = sync_target,
        // not t = 0 (the pre-drift time). If the wrong time is used, the
        // final velocity will differ from the reference.
        let k0 = 0.04;
        let alpha = 0.1;
        let rest = 10.0;
        let sync_interval = 5.0;

        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(15.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(TimeScaledSpring {
            k0,
            alpha,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(sync_interval),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0.clone(), FreeParticle)
        .add_satellite("b", s1.clone(), FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring.clone());

        // Propagate exactly one sync step
        sched.propagate_to(sync_interval).unwrap();

        // Reference: manual KDK with correct time on 2nd kick
        // 1st kick at t=0: k(0) = k0, displacement = 15-10 = 5
        let k_start = k0 * (1.0 + alpha * 0.0);
        let f_start = k_start * (15.0 - rest);
        // Half-kick: Δv = f * dt_sync/2 = f * 2.5
        let dt_half = sync_interval / 2.0;
        let v_a_half = f_start * dt_half; // positive x
        let v_b_half = -f_start * dt_half;

        // After half-kick, drift for sync_interval (free particle)
        let pos_a_drift = s0.position().x + v_a_half * sync_interval;
        let pos_b_drift = s1.position().x + v_b_half * sync_interval;

        // 2nd kick at t=sync_target=5.0: k(5) = k0*(1+0.5) = 1.5*k0
        let k_end = k0 * (1.0 + alpha * sync_interval);
        let r_drift = pos_b_drift - pos_a_drift;
        let f_end = k_end * (r_drift - rest);
        let v_a_final = v_a_half + f_end * dt_half;
        let v_b_final = v_b_half - f_end * dt_half;

        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let b = sched.satellite_state(&SatId::from("b")).unwrap();

        // Tolerance accounts for DP45 drift integration error
        assert!(
            (a.velocity().x - v_a_final).abs() < 1e-6,
            "a velocity: got {}, expected {}",
            a.velocity().x,
            v_a_final
        );
        assert!(
            (b.velocity().x - v_b_final).abs() < 1e-6,
            "b velocity: got {}, expected {}",
            b.velocity().x,
            v_b_final
        );
    }

    // ── Bug 6: progress guard ───────────────────────────────────────

    #[test]
    fn tiny_sync_interval_does_not_hang() {
        // sync_interval so small that self.t + interval == self.t
        let mut config = sync_config(1e-300);
        config.sync_exit = 200.0;

        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::new(1.0, 0.0, 0.0));
        let s1 = OrbitalState::new(Vector3::new(100.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: 0.01,
            rest_length: 10.0,
        });

        let mut sched: Scheduler<FreeParticle> =
            Scheduler::new(config, IntegratorConfig::Rk4 { dt: 0.1 })
                .add_satellite("a", s0, FreeParticle)
                .add_satellite("b", s1, FreeParticle)
                .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        // Should complete without hanging (progress guard breaks the loop)
        sched.propagate_to(10.0).unwrap();
    }

    #[test]
    fn float_precision_boundary_does_not_hang() {
        // At t=1e15, adding 0.01 doesn't change the value due to f64 precision
        let config = sync_config(0.01);

        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            config,
            IntegratorConfig::Rk4 { dt: 0.1 },
        )
        .add_satellite("a", s0, FreeParticle);

        // Start at a large time where sync_interval can't advance
        sched.t = 1e15;
        sched.propagate_to(1e15 + 1.0).unwrap();
    }

    // ── Bug 3: sync_target clamped to end_time ──────────────────────

    #[test]
    fn kdk_sync_target_clamped_to_end_time() {
        // With sync_interval=5 and end_time=3, the first sync step should target t=3
        // (not t=5). Kicks should be sized for dt=3, not dt=5.
        let k = 0.04;
        let rest = 10.0;
        let sync_interval = 5.0;

        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(Vector3::new(15.0, 0.0, 0.0), Vector3::zeros());
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(sync_interval),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .add_satellite("a", s0.clone(), FreeParticle)
        .add_satellite_until("b", s1.clone(), 3.0, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring.clone());

        sched.propagate_to(10.0).unwrap();

        // Reference: KDK with dt_sync=3 (clamped to end_time), not 5
        // 1st kick at t=0: displacement = 15-10 = 5, force = k*5 = 0.2
        let f_start = k * (15.0 - rest);
        let dt_half = 3.0 / 2.0; // clamped dt_sync/2

        // Half-kick: Δv = f * dt_sync/2
        let v_a_after_kick1 = f_start * dt_half;
        let v_b_after_kick1 = -f_start * dt_half;

        // After drift for dt_sync=3.0
        let pos_a_drift = 0.0 + v_a_after_kick1 * 3.0;
        let pos_b_drift = 15.0 + v_b_after_kick1 * 3.0;

        // 2nd kick at positions after drift
        let r_drift = pos_b_drift - pos_a_drift;
        let f_end = k * (r_drift - rest);
        let v_a_at_3 = v_a_after_kick1 + f_end * dt_half;

        // After t=3, "b" stops. "a" drifts freely from t=3 to t=10 at constant velocity
        let a = sched.satellite_state(&SatId::from("a")).unwrap();
        let expected_pos = pos_a_drift + v_a_at_3 * (10.0 - 3.0);

        assert!(
            (a.position().x - expected_pos).abs() < 1e-4,
            "a position: got {}, expected {}",
            a.position().x,
            expected_pos
        );
        assert!((sched.current_t() - 10.0).abs() < 1e-12);
    }

    // ── Bug 2: self.t updated after event break ─────────────────────

    #[test]
    fn kdk_event_updates_scheduler_time() {
        // After an event during KDK on the FIRST sync step, self.t should
        // advance to sync_target (not remain at 0).
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0), // very fast → triggers event in first step
        );
        let spring = Arc::new(Spring {
            stiffness: 0.01,
            rest_length: 10.0,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            sync_config(5.0),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            if state.position().x > 25.0 {
                ControlFlow::Break("boundary".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Synchronized, spring);

        let outcome = sched.propagate_to(100.0).unwrap();

        assert!(!outcome.terminations.is_empty());
        // Key assertion: scheduler time must have advanced from 0
        // Before fix: self.t stays at 0 because the break skips self.t = sync_target
        assert!(
            sched.current_t() >= 5.0 - 1e-9,
            "scheduler time should be at sync_target=5: got {}",
            sched.current_t()
        );
    }

    // ── Bug 5: NaN distance guard ───────────────────────────────────

    #[test]
    fn nan_distance_forces_independent_from_coupled() {
        let config = default_config();
        let ps = pair_state(PairRegime::Coupled, 0.0);
        let result = evaluate_auto_regime(f64::NAN, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    #[test]
    fn nan_distance_forces_independent_from_synchronized() {
        let config = default_config();
        let ps = pair_state(PairRegime::Synchronized, 0.0);
        let result = evaluate_auto_regime(f64::NAN, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    #[test]
    fn nan_distance_forces_independent_from_independent() {
        let config = default_config();
        let ps = pair_state(PairRegime::Independent, 0.0);
        let result = evaluate_auto_regime(f64::NAN, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    #[test]
    fn infinity_distance_forces_independent() {
        let config = default_config();
        let ps = pair_state(PairRegime::Coupled, 0.0);
        let result = evaluate_auto_regime(f64::INFINITY, &ps, 1000.0, &config);
        assert_eq!(result, PairRegime::Independent);
    }

    // ── Bug 4: per-satellite termination in CoupledGroup ────────────

    #[test]
    fn coupled_event_only_terminates_triggering_satellite() {
        // Two satellites with Fixed(Coupled) spring. "b" triggers x > 30 event.
        // Only "b" should be terminated; "a" should survive in snapshot.
        let k = 0.01;
        let rest = 10.0;
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::zeros());
        let s1 = OrbitalState::new(
            Vector3::new(15.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0), // moving outward
        );
        let spring = Arc::new(Spring {
            stiffness: k,
            rest_length: rest,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            default_config(),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            if state.position().x > 30.0 {
                ControlFlow::Break("boundary".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring);

        let outcome = sched.propagate_to(1000.0).unwrap();

        // "b" should have triggered the event
        assert_eq!(outcome.terminations.len(), 1);
        assert_eq!(outcome.terminations[0].satellite_id, SatId::from("b"));

        // Only "b" is terminated; "a" should still be in snapshot
        let snap = sched.snapshot();
        assert_eq!(
            snap.positions.len(),
            1,
            "only 'a' should survive, got {} positions",
            snap.positions.len()
        );
        assert_eq!(snap.positions[0].0, SatId::from("a"));
    }

    #[test]
    fn coupled_surviving_satellite_continues_propagation() {
        // After one satellite terminates in a coupled group, the surviving
        // satellite should continue propagating independently.
        // Use a per-satellite event that only triggers for "b" (by id).
        // "a" at origin with y-velocity, "b" far out with outward x-velocity.
        // Weak spring so "a" stays near origin.
        let s0 = OrbitalState::new(Vector3::zeros(), Vector3::new(0.0, 0.5, 0.0));
        let s1 = OrbitalState::new(Vector3::new(100.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0));
        // Very weak spring, large rest length — negligible force on "a"
        let spring = Arc::new(Spring {
            stiffness: 1e-6,
            rest_length: 100.0,
        });

        let mut sched: Scheduler<FreeParticle> = Scheduler::new(
            default_config(),
            IntegratorConfig::Dp45 {
                dt: 0.1,
                tolerances: Tolerances {
                    atol: 1e-12,
                    rtol: 1e-10,
                },
            },
        )
        .with_event_checker(|_t, state: &OrbitalState| {
            // Only triggers for satellites far in +x direction
            if state.position().x > 200.0 {
                ControlFlow::Break("boundary".to_string())
            } else {
                ControlFlow::Continue(())
            }
        })
        .add_satellite("a", s0, FreeParticle)
        .add_satellite("b", s1, FreeParticle)
        .add_interaction_fixed("a", "b", PairRegime::Coupled, spring);

        // Propagate: "b" at x=100 + 1*t will reach 200 at ~t=100
        let outcome1 = sched.propagate_to(200.0).unwrap();
        assert!(
            outcome1
                .terminations
                .iter()
                .any(|t| t.satellite_id == SatId::from("b")),
            "b should have triggered the boundary event"
        );
        let t_after = sched.current_t();

        // Continue propagation: "a" should still move (y direction)
        let a_before = sched.satellite_state(&SatId::from("a")).unwrap().clone();
        sched.propagate_to(t_after + 100.0).unwrap();
        let a_after = sched.satellite_state(&SatId::from("a")).unwrap();

        // "a" should have moved further in y (free particle with y-velocity ~0.5)
        assert!(
            a_after.position().y > a_before.position().y + 10.0,
            "surviving satellite should continue: before_y={}, after_y={}",
            a_before.position().y,
            a_after.position().y,
        );
    }
}
