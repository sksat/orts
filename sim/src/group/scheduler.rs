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

use std::sync::Arc;

use super::coupled::InterSatelliteForce;
use super::prop_group::SatId;

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
        let pairs = vec![
            (0, 1, PairRegime::Coupled),
            (1, 2, PairRegime::Coupled),
        ];
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
}
