//! The propellant a spacecraft carries, as one pool with one floor.
//!
//! A spacecraft has a single tank in this model: every thruster draws from it,
//! and the mass the vehicle can never burn below is its dry mass. Both are
//! properties of the vehicle rather than of any one thruster, which is why they
//! live here and not in [`ThrusterSpec`](super::ThrusterSpec) — two thrusters
//! carrying their own floor could disagree about when the spacecraft is empty.
//!
//! Independent tanks and bipropellant systems are outside this model.

use crate::effector::{
    BoundaryExchange, BoundaryKind, ConstraintMode, EffectorBoundary, EffectorInput, StateEffector,
};
use crate::model::{ExternalLoads, HasFrame, HasMass};

/// Width of the margin within which a spacecraft still counts as standing on
/// its floor [kg].
///
/// The mass is put exactly on the floor when the boundary is settled, so only
/// rounding moves it after that; the width is what keeps that rounding from
/// reading as a fresh crossing. It is far below any propellant a spacecraft
/// carries.
const MASS_TOLERANCE: f64 = 1e-12;

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

/// The pool as an effector: it carries no continuous state of its own — the
/// mass is the plant's — but it carries the one thing a search needs, which is
/// the mode saying whether there is anything left to burn.
///
/// Running dry is a boundary the propagation locates rather than a comparison
/// the right-hand side makes: a comparison inside the right-hand side flips
/// between the stages of a re-stepped interval, and the search that re-steps it
/// then reports the crossing at the wrong time. There is no release: a tank
/// does not refill, so the depleted mode absorbs.
impl<S: HasFrame + HasMass + Send + Sync> StateEffector<S> for PropellantPool {
    fn name(&self) -> &str {
        "propellant_pool"
    }

    fn state_dim(&self) -> usize {
        0
    }

    fn mode_dim(&self) -> usize {
        1
    }

    fn derivatives(
        &self,
        _input: EffectorInput<'_, S>,
        _aux_rates: &mut [f64],
    ) -> ExternalLoads<S::Frame> {
        // The pool pushes nothing: its consumers do, and the system stops
        // asking them once this says the tank is empty.
        ExternalLoads::zeros()
    }

    fn boundaries(&self) -> Vec<EffectorBoundary> {
        vec![EffectorBoundary {
            kind: BoundaryKind::ReachedLower { index: 0 },
            boundary_tolerance: MASS_TOLERANCE,
        }]
    }

    fn boundary_value(&self, _kind: BoundaryKind, input: EffectorInput<'_, S>) -> f64 {
        // The propellant left, which is the margin that runs out.
        input.state.mass() - self.dry_mass
    }

    fn settle_boundary(&self, _kind: BoundaryKind, _aux: &mut [f64]) -> Option<BoundaryExchange> {
        // The mass belongs on the floor. What it burned below the floor is
        // gone with the exhaust, so the impulse for it stays in the velocity —
        // the error the localization left, bounded by the time tolerance.
        Some(BoundaryExchange {
            mass: Some(self.dry_mass),
            ..Default::default()
        })
    }
}

impl PropellantPool {
    /// The mode a spacecraft of this mass starts in.
    ///
    /// Three cases, and the middle one is why this is not a comparison the
    /// propagation repeats: below the floor is an input error, *on* the floor
    /// is a vehicle that starts empty — which no search can find, since a
    /// margin of exactly zero has not been crossed — and above it is a vehicle
    /// with propellant.
    ///
    /// # Panics
    ///
    /// Panics below the floor: a spacecraft cannot have less than no
    /// propellant, and settling it up to the floor would hide the input error
    /// by adding mass.
    pub fn initial_mode(&self, mass: f64) -> ConstraintMode {
        assert!(
            mass.is_finite() && mass >= self.dry_mass,
            "a spacecraft cannot start below its own dry mass: mass {mass}, floor {}",
            self.dry_mass
        );
        if mass > self.dry_mass {
            ConstraintMode::Free
        } else {
            ConstraintMode::Lower
        }
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

    /// Three cases at the start, and the middle one is what a search cannot
    /// find: a margin of exactly zero has not been crossed.
    #[test]
    fn the_mode_a_spacecraft_starts_in() {
        let pool = PropellantPool::new(100.0);
        assert_eq!(pool.initial_mode(140.0), ConstraintMode::Free);
        assert_eq!(pool.initial_mode(100.0), ConstraintMode::Lower);
    }

    /// Settling it up to the floor would hide an input error by adding mass.
    #[test]
    #[should_panic(expected = "cannot start below its own dry mass")]
    fn a_spacecraft_starting_below_the_floor_is_refused() {
        PropellantPool::new(100.0).initial_mode(99.0);
    }

    /// Zero would put the floor on the singularity of `F/m`, which a trial
    /// step of a search steps past by design.
    #[test]
    #[should_panic(expected = "dry mass must be positive")]
    fn a_floor_of_zero_is_refused() {
        PropellantPool::new(0.0);
    }
}
