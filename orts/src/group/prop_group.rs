use std::fmt;

use nalgebra::Vector3;
use utsuroi::IntegrationError;

/// Unique identifier for a satellite within a group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SatId(String);

impl SatId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl From<&str> for SatId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl fmt::Display for SatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SatId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Snapshot of satellite positions at a point in time.
#[derive(Debug, Clone)]
pub struct GroupSnapshot {
    pub positions: Vec<(SatId, Vector3<f64>)>,
}

/// Record of a satellite termination event during propagation.
#[derive(Debug, Clone)]
pub struct SatelliteTermination {
    pub satellite_id: SatId,
    pub t: f64,
    pub reason: String,
}

/// Result of a group propagation step.
#[derive(Debug)]
pub struct PropGroupOutcome {
    /// Satellites that terminated during this propagation.
    /// Empty means all satellites reached t_target successfully.
    pub terminations: Vec<SatelliteTermination>,
}

/// Type-erased interface for propagating a group of satellites.
///
/// Provides the scheduler layer with a uniform API regardless of
/// the internal state type or integration strategy.
pub trait PropGroup: Send {
    /// Return the IDs of all satellites in this group.
    fn ids(&self) -> Vec<SatId>;

    /// Advance all non-terminated satellites to `t_target`.
    ///
    /// Continues propagating remaining satellites even if some terminate.
    /// Returns all terminations collected during this call.
    fn propagate_to(&mut self, t_target: f64) -> Result<PropGroupOutcome, IntegrationError>;

    /// Snapshot current positions of all non-terminated satellites.
    fn snapshot(&self) -> GroupSnapshot;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sat_id_from_str() {
        let id = SatId::from("iss");
        assert_eq!(id.as_ref(), "iss");
        assert_eq!(id.to_string(), "iss");
    }

    #[test]
    fn sat_id_equality_and_hash() {
        let a = SatId::new("sat-1");
        let b = SatId::from("sat-1");
        let c = SatId::new("sat-2");
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b);
        assert_eq!(set.len(), 1);
        set.insert(c);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn group_snapshot_construction() {
        let snap = GroupSnapshot {
            positions: vec![
                (SatId::from("a"), Vector3::new(1.0, 2.0, 3.0)),
                (SatId::from("b"), Vector3::new(4.0, 5.0, 6.0)),
            ],
        };
        assert_eq!(snap.positions.len(), 2);
        assert_eq!(snap.positions[0].0, SatId::from("a"));
        assert_eq!(snap.positions[1].1, Vector3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn prop_group_outcome_empty_means_all_reached() {
        let outcome = PropGroupOutcome {
            terminations: vec![],
        };
        assert!(outcome.terminations.is_empty());
    }
}
