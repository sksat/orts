//! The propellant a spacecraft carries, as one pool with one floor.
//!
//! A spacecraft has a single tank in this model: every thruster draws from it,
//! and the mass the vehicle can never burn below is its dry mass. Both are
//! properties of the vehicle rather than of any one thruster, which is why they
//! live here and not in [`ThrusterSpec`](super::ThrusterSpec) — two thrusters
//! carrying their own floor could disagree about when the spacecraft is empty.
//!
//! Independent tanks and bipropellant systems are outside this model.

/// The floor under a spacecraft's mass, and what is left above it.
///
/// The mass itself is the integrated state
/// ([`SpacecraftState::mass`](crate::SpacecraftState::mass)); this is the
/// constant it is measured against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropellantPool {
    dry_mass: f64,
}

impl PropellantPool {
    /// A pool whose spacecraft masses `dry_mass` [kg] with nothing left to
    /// burn.
    ///
    /// # Panics
    ///
    /// Panics unless the floor is positive and finite. Zero would put the
    /// floor on the singularity of `F/m`: a trial step of a boundary search
    /// steps past the floor by design — it re-steps the interval under the
    /// mode that held before the crossing — and a floor above zero is what
    /// keeps the right-hand side finite when it does.
    pub fn new(dry_mass: f64) -> Self {
        assert!(
            dry_mass.is_finite() && dry_mass > 0.0,
            "dry mass must be positive and finite, got {dry_mass}"
        );
        Self { dry_mass }
    }

    /// The mass with no propellant left [kg].
    pub fn dry_mass(&self) -> f64 {
        self.dry_mass
    }

    /// Propellant left at `mass` [kg], and never negative.
    ///
    /// A boundary search steps past the floor on purpose, so a state below it
    /// reaches this; what is left there is nothing.
    pub fn remaining(&self, mass: f64) -> f64 {
        (mass - self.dry_mass).max(0.0)
    }

    /// Whether a spacecraft of this mass has anything left to burn.
    ///
    /// Exactly on the floor counts as empty: the propellant is what is *above*
    /// it.
    pub fn is_empty(&self, mass: f64) -> bool {
        // `partial_cmp` rather than a negated `>`: a mass that is no number is
        // comparable to nothing, and what it means here is that there is no
        // propellant to burn.
        matches!(
            mass.partial_cmp(&self.dry_mass),
            Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal) | None
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The floor is the mass with nothing left, so the propellant is what the
    /// state carries above it — and a state below the floor, which a search
    /// reaches on purpose, has none rather than a negative amount.
    #[test]
    fn what_is_left_is_what_the_state_carries_above_the_floor() {
        let pool = PropellantPool::new(100.0);
        assert_eq!(pool.dry_mass(), 100.0);
        assert_eq!(pool.remaining(140.0), 40.0);
        assert_eq!(pool.remaining(100.0), 0.0);
        assert_eq!(pool.remaining(99.0), 0.0);
    }

    /// Exactly on the floor is empty: the propellant is what is above it, and
    /// a spacecraft on the floor has nothing above it.
    #[test]
    fn a_spacecraft_on_the_floor_is_empty() {
        let pool = PropellantPool::new(100.0);
        assert!(!pool.is_empty(100.001));
        assert!(pool.is_empty(100.0));
        assert!(pool.is_empty(99.0));
        assert!(
            pool.is_empty(f64::NAN),
            "a mass that is no number is not fuel"
        );
    }

    /// Zero would put the floor on the singularity of `F/m`, which a trial
    /// step of a search steps past by design.
    #[test]
    #[should_panic(expected = "dry mass must be positive")]
    fn a_floor_of_zero_is_refused() {
        PropellantPool::new(0.0);
    }
}
