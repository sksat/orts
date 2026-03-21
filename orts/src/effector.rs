//! StateEffector trait, AugmentedState, and AuxRegistry for components
//! with internal state that couples to spacecraft dynamics.
//!
//! Unlike [`Model<S>`](crate::model::Model) which is a pure function,
//! a [`StateEffector`] has auxiliary state variables (e.g., reaction wheel
//! angular momentum) that are integrated by the ODE solver alongside the
//! plant state.

use kaname::epoch::Epoch;
use utsuroi::{OdeState, Tolerances};

use crate::spacecraft::ExternalLoads;

// ---------------------------------------------------------------------------
// StateEffector trait
// ---------------------------------------------------------------------------

/// A physical component with internal state that couples to spacecraft dynamics.
///
/// Unlike `Model<S>` (pure function), StateEffector has auxiliary state
/// that is integrated by the ODE solver alongside the plant state.
/// Examples: reaction wheels (angular momentum), gimbals, fuel slosh.
///
/// The `derivatives` method writes aux_rates into a caller-owned buffer
/// to avoid allocation in the ODE hot path.
pub trait StateEffector<S>: Send + Sync + std::any::Any {
    /// Human-readable name for this effector (e.g., "reaction_wheels").
    fn name(&self) -> &str;

    /// Number of scalar state variables this effector contributes.
    fn state_dim(&self) -> usize;

    /// Compute loads on spacecraft and derivatives of auxiliary state.
    ///
    /// `aux` is the current auxiliary state slice (length = `state_dim()`).
    /// `aux_rates` is the output buffer for derivatives (length = `state_dim()`).
    /// Returns `ExternalLoads` contribution to the plant dynamics.
    fn derivatives(
        &self,
        t: f64,
        state: &S,
        aux: &[f64],
        aux_rates: &mut [f64],
        epoch: Option<&Epoch>,
    ) -> ExternalLoads;
}

// ---------------------------------------------------------------------------
// AugmentedState
// ---------------------------------------------------------------------------

/// Plant state augmented with auxiliary effector state.
///
/// The ODE solver integrates this composite state, where `plant` is the
/// primary dynamics state (e.g., `AttitudeState`) and `aux` holds the
/// concatenated auxiliary variables from all registered [`StateEffector`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct AugmentedState<S: OdeState> {
    /// Primary dynamics state (e.g., attitude quaternion + angular velocity).
    pub plant: S,
    /// Concatenated auxiliary state from all registered effectors.
    pub aux: Vec<f64>,
}

impl<S: OdeState> OdeState for AugmentedState<S> {
    fn zero_like(&self) -> Self {
        Self {
            plant: self.plant.zero_like(),
            aux: vec![0.0; self.aux.len()],
        }
    }

    fn axpy(&self, scale: f64, other: &Self) -> Self {
        let mut aux = self.aux.clone();
        for (a, o) in aux.iter_mut().zip(other.aux.iter()) {
            *a += scale * o;
        }
        Self {
            plant: self.plant.axpy(scale, &other.plant),
            aux,
        }
    }

    fn scale(&self, factor: f64) -> Self {
        Self {
            plant: self.plant.scale(factor),
            aux: self.aux.iter().map(|v| v * factor).collect(),
        }
    }

    fn is_finite(&self) -> bool {
        self.plant.is_finite() && self.aux.iter().all(|v| v.is_finite())
    }

    fn error_norm(&self, y_next: &Self, error: &Self, tol: &Tolerances) -> f64 {
        let plant_norm = self.plant.error_norm(&y_next.plant, &error.plant, tol);

        // Aux error norm: RMS of scaled errors
        if self.aux.is_empty() {
            return plant_norm;
        }

        let mut sum_sq = 0.0;
        for i in 0..self.aux.len() {
            let sc = tol.atol + tol.rtol * self.aux[i].abs().max(y_next.aux[i].abs());
            let e = error.aux[i] / sc;
            sum_sq += e * e;
        }
        let aux_norm = (sum_sq / self.aux.len() as f64).sqrt();
        plant_norm.max(aux_norm)
    }

    fn project(&mut self, t: f64) {
        self.plant.project(t);
        // No projection needed for aux state (no constraints like quaternion normalization)
    }
}

// ---------------------------------------------------------------------------
// AuxRegistry
// ---------------------------------------------------------------------------

/// Metadata for a registered auxiliary state block.
#[derive(Debug, Clone)]
pub struct AuxEntry {
    /// Human-readable name of the effector owning this block.
    pub name: String,
    /// Starting index within the concatenated aux vector.
    pub offset: usize,
    /// Number of scalar variables in this block.
    pub dim: usize,
}

/// Registry mapping [`StateEffector`]s to their auxiliary state slices.
///
/// Each effector is assigned a contiguous block of the aux vector.
/// The registry tracks the offset and dimension of each block.
#[derive(Debug, Default)]
pub struct AuxRegistry {
    entries: Vec<AuxEntry>,
    total_dim: usize,
}

impl AuxRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Default::default()
    }

    /// Register a new effector and return its offset into the aux vector.
    pub fn register(&mut self, name: &str, dim: usize) -> usize {
        let offset = self.total_dim;
        self.entries.push(AuxEntry {
            name: name.to_string(),
            offset,
            dim,
        });
        self.total_dim += dim;
        offset
    }

    /// Total number of auxiliary state variables across all effectors.
    pub fn total_dim(&self) -> usize {
        self.total_dim
    }

    /// All registered entries.
    pub fn entries(&self) -> &[AuxEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use nalgebra::{Vector3, Vector4};

    // ------- AuxRegistry tests -------

    #[test]
    fn registry_empty() {
        let reg = AuxRegistry::new();
        assert_eq!(reg.total_dim(), 0);
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn registry_single_effector() {
        let mut reg = AuxRegistry::new();
        let offset = reg.register("rw", 3);
        assert_eq!(offset, 0);
        assert_eq!(reg.total_dim(), 3);
        assert_eq!(reg.entries().len(), 1);
        assert_eq!(reg.entries()[0].name, "rw");
        assert_eq!(reg.entries()[0].offset, 0);
        assert_eq!(reg.entries()[0].dim, 3);
    }

    #[test]
    fn registry_multiple_effectors() {
        let mut reg = AuxRegistry::new();
        let o1 = reg.register("rw", 3);
        let o2 = reg.register("gimbal", 2);
        assert_eq!(o1, 0);
        assert_eq!(o2, 3);
        assert_eq!(reg.total_dim(), 5);
        assert_eq!(reg.entries().len(), 2);
    }

    // ------- AugmentedState OdeState tests -------

    fn sample_augmented() -> AugmentedState<AttitudeState> {
        AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            aux: vec![1.0, 2.0, 3.0],
        }
    }

    #[test]
    fn zero_like() {
        let s = sample_augmented();
        let z = s.zero_like();
        assert_eq!(z.plant.quaternion, Vector4::zeros());
        assert_eq!(z.plant.angular_velocity, Vector3::zeros());
        assert_eq!(z.aux, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn axpy_identity() {
        let s = sample_augmented();
        let other = s.zero_like();
        let result = s.axpy(0.0, &other);
        assert_eq!(result.aux, s.aux);
    }

    #[test]
    fn axpy_adds() {
        let s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![1.0, 2.0],
        };
        let other = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![10.0, 20.0],
        };
        let result = s.axpy(0.5, &other);
        assert!((result.aux[0] - 6.0).abs() < 1e-15);
        assert!((result.aux[1] - 12.0).abs() < 1e-15);
    }

    #[test]
    fn scale_multiplies() {
        let s = sample_augmented();
        let scaled = s.scale(2.0);
        assert!((scaled.aux[0] - 2.0).abs() < 1e-15);
        assert!((scaled.aux[1] - 4.0).abs() < 1e-15);
        assert!((scaled.aux[2] - 6.0).abs() < 1e-15);
    }

    #[test]
    fn is_finite_true() {
        let s = sample_augmented();
        assert!(s.is_finite());
    }

    #[test]
    fn is_finite_false_nan_aux() {
        let mut s = sample_augmented();
        s.aux[1] = f64::NAN;
        assert!(!s.is_finite());
    }

    #[test]
    fn is_finite_false_inf_aux() {
        let mut s = sample_augmented();
        s.aux[0] = f64::INFINITY;
        assert!(!s.is_finite());
    }

    #[test]
    fn project_normalizes_quaternion() {
        let mut s = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(2.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            aux: vec![5.0, 10.0],
        };
        s.project(0.0);
        let norm = s.plant.quaternion.magnitude();
        assert!((norm - 1.0).abs() < 1e-15);
        // Aux should be unchanged
        assert_eq!(s.aux, vec![5.0, 10.0]);
    }

    #[test]
    fn error_norm_empty_aux() {
        let s = AugmentedState {
            plant: AttitudeState::identity(),
            aux: vec![],
        };
        let y_next = s.clone();
        let error = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1e-8, 1e-8, 1e-8, 1e-8),
                angular_velocity: Vector3::new(1e-8, 1e-8, 1e-8),
            },
            aux: vec![],
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = s.error_norm(&y_next, &error, &tol);
        assert!(norm > 0.0);
        assert!(norm.is_finite());
    }

    #[test]
    fn error_norm_with_aux() {
        let s = sample_augmented();
        let y_next = s.clone();
        let error = AugmentedState {
            plant: AttitudeState {
                quaternion: Vector4::new(1e-8, 1e-8, 1e-8, 1e-8),
                angular_velocity: Vector3::new(1e-8, 1e-8, 1e-8),
            },
            aux: vec![1e-8, 1e-8, 1e-8],
        };
        let tol = Tolerances {
            atol: 1e-10,
            rtol: 1e-8,
        };
        let norm = s.error_norm(&y_next, &error, &tol);
        assert!(norm > 0.0);
        assert!(norm.is_finite());
    }
}
