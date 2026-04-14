//! Reaction wheel (RW) assembly as a [`StateEffector`].
//!
//! Models a set of reaction wheels, each with its own spin axis, inertia,
//! maximum angular momentum, and maximum torque. The assembly's auxiliary
//! state is the angular momentum of each wheel about its spin axis.
//!
//! The reaction torque on the spacecraft follows Newton's third law:
//! `τ_body = -Σ (dh_i/dt · axis_i)`.
//!
//! ## Architecture: Core + Wrapper
//!
//! [`RwAssemblyCore`] encapsulates geometry, constraint logic (rate limiting,
//! momentum saturation), and torque allocation — all testable without ODE
//! integration. [`RwAssembly`] wraps the core as a [`StateEffector`] that
//! integrates wheel angular momentum.

use arika::epoch::Epoch;
use nalgebra::Vector3;

use super::ExternalLoads;
use crate::effector::StateEffector;
use crate::model::HasAttitude;

/// A single reaction wheel with physical limits.
#[derive(Debug, Clone)]
pub struct Rw {
    /// Spin axis in body frame (unit vector, normalized on construction).
    axis: Vector3<f64>,
    /// Moment of inertia about spin axis [kg·m²].
    pub inertia: f64,
    /// Maximum angular momentum [N·m·s].
    pub max_momentum: f64,
    /// Maximum torque (acceleration rate limit) [N·m].
    pub max_torque: f64,
}

impl Rw {
    /// Create a reaction wheel with the given spin axis (will be normalized).
    ///
    /// # Panics
    /// Panics if `axis` is zero-length, or `max_momentum`/`max_torque` are negative.
    pub fn new(axis: Vector3<f64>, inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        let norm = axis.magnitude();
        assert!(norm > 1e-15, "Wheel axis must be non-zero");
        assert!(
            max_momentum >= 0.0,
            "max_momentum must be non-negative, got {max_momentum}"
        );
        assert!(
            max_torque >= 0.0,
            "max_torque must be non-negative, got {max_torque}"
        );
        Self {
            axis: axis / norm,
            inertia,
            max_momentum,
            max_torque,
        }
    }

    /// Get the spin axis unit vector.
    pub fn axis(&self) -> &Vector3<f64> {
        &self.axis
    }
}

/// RW assembly geometry and constraint logic (no ODE state integration).
///
/// This core struct handles per-wheel torque clamping, momentum saturation,
/// reaction torque computation, and torque allocation without depending on
/// the ODE system. It is designed to be unit-tested independently.
#[derive(Debug, Clone)]
pub struct RwAssemblyCore {
    wheels: Vec<Rw>,
}

impl RwAssemblyCore {
    /// Create an assembly core from a list of reaction wheels.
    pub fn new(wheels: Vec<Rw>) -> Self {
        Self { wheels }
    }

    /// Standard 3-axis orthogonal arrangement with identical wheels.
    pub fn three_axis(inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        Self::new(vec![
            Rw::new(Vector3::x(), inertia, max_momentum, max_torque),
            Rw::new(Vector3::y(), inertia, max_momentum, max_torque),
            Rw::new(Vector3::z(), inertia, max_momentum, max_torque),
        ])
    }

    /// Access the wheels.
    pub fn wheels(&self) -> &[Rw] {
        &self.wheels
    }

    /// Number of wheels in the assembly.
    pub fn num_wheels(&self) -> usize {
        self.wheels.len()
    }

    /// Apply rate limiting and momentum saturation to per-wheel commanded
    /// torques. Returns clamped per-wheel torques.
    ///
    /// # Panics
    /// Panics if `commanded.len()` or `momentum.len()` != `self.num_wheels()`.
    pub fn clamp_torques(&self, commanded: &[f64], momentum: &[f64]) -> Vec<f64> {
        assert_eq!(commanded.len(), self.wheels.len());
        assert_eq!(momentum.len(), self.wheels.len());
        self.wheels
            .iter()
            .enumerate()
            .map(|(i, wheel)| {
                let mut tau = commanded[i].clamp(-wheel.max_torque, wheel.max_torque);
                // Momentum saturation: prevent exceeding limits
                if (momentum[i] >= wheel.max_momentum && tau > 0.0)
                    || (momentum[i] <= -wheel.max_momentum && tau < 0.0)
                {
                    tau = 0.0;
                }
                tau
            })
            .collect()
    }

    /// Compute the reaction torque on the spacecraft body from clamped
    /// per-wheel motor torques (Newton's third law).
    ///
    /// `τ_body = -Σ (tau_i · axis_i)`
    pub fn reaction_torque(&self, clamped: &[f64]) -> Vector3<f64> {
        let mut total = Vector3::zeros();
        for (wheel, &tau) in self.wheels.iter().zip(clamped.iter()) {
            total -= tau * wheel.axis;
        }
        total
    }

    /// Compute the gyroscopic coupling torque: `-ω × H_rw`.
    ///
    /// `H_rw = Σ (h_i · axis_i)` is the total wheel angular momentum
    /// in the body frame.
    pub fn gyroscopic_torque(&self, omega: &Vector3<f64>, momentum: &[f64]) -> Vector3<f64> {
        let mut h_rw = Vector3::zeros();
        for (wheel, &h) in self.wheels.iter().zip(momentum.iter()) {
            h_rw += h * wheel.axis;
        }
        -omega.cross(&h_rw)
    }

    /// Allocate a desired body-frame torque to per-wheel torques via
    /// axis projection.
    ///
    /// For orthogonal wheel arrangements this is exact; for non-orthogonal
    /// layouts this is an approximation.
    pub fn allocate(&self, desired: &Vector3<f64>) -> Vec<f64> {
        self.wheels
            .iter()
            .map(|wheel| {
                // Wheel motor torque = negative of desired body torque projected onto axis
                // (Newton's third law: to get +torque on body, wheels must absorb -torque)
                -desired.dot(&wheel.axis)
            })
            .collect()
    }
}

/// RW assembly as a [`StateEffector`].
///
/// Wraps [`RwAssemblyCore`] with ODE auxiliary state (per-wheel angular
/// momentum) and per-wheel commanded torques (zero-order hold).
///
/// Aux state: angular momentum `h_i` [N·m·s] for each wheel.
/// Reaction torque on spacecraft: `τ_body = -Σ (dh_i/dt · axis_i) − ω × H_rw`.
#[derive(Clone)]
pub struct RwAssembly {
    core: RwAssemblyCore,
    /// Per-wheel commanded torque [N·m] (set externally by controller).
    /// Sign convention: positive value → wheel absorbs positive angular momentum.
    pub commanded_torques: Vec<f64>,
}

impl RwAssembly {
    /// Create an assembly from a list of reaction wheels.
    pub fn new(wheels: Vec<Rw>) -> Self {
        let n = wheels.len();
        Self {
            core: RwAssemblyCore::new(wheels),
            commanded_torques: vec![0.0; n],
        }
    }

    /// Standard 3-axis orthogonal arrangement with identical wheels.
    pub fn three_axis(inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        let core = RwAssemblyCore::three_axis(inertia, max_momentum, max_torque);
        let n = core.num_wheels();
        Self {
            core,
            commanded_torques: vec![0.0; n],
        }
    }

    /// Access the core (geometry + constraint logic).
    pub fn core(&self) -> &RwAssemblyCore {
        &self.core
    }

    /// Access the wheels.
    pub fn wheels(&self) -> &[Rw] {
        self.core.wheels()
    }
}

impl<S: HasAttitude + Send + Sync> StateEffector<S> for RwAssembly {
    fn name(&self) -> &str {
        "reaction_wheels"
    }

    fn state_dim(&self) -> usize {
        self.core.num_wheels()
    }

    fn aux_bounds(&self) -> Vec<(f64, f64)> {
        self.core
            .wheels
            .iter()
            .map(|w| (-w.max_momentum, w.max_momentum))
            .collect()
    }

    fn derivatives(
        &self,
        _t: f64,
        state: &S,
        aux: &[f64],
        aux_rates: &mut [f64],
        _epoch: Option<&Epoch>,
    ) -> ExternalLoads {
        let omega = &state.attitude().angular_velocity;

        // Clamp per-wheel commanded torques (rate limiting + saturation)
        let clamped = self.core.clamp_torques(&self.commanded_torques, aux);

        // Set aux rates (wheel momentum derivatives)
        for (i, &tau) in clamped.iter().enumerate() {
            aux_rates[i] = tau;
        }

        // Reaction torque from wheel acceleration (Newton's third law)
        let reaction = self.core.reaction_torque(&clamped);

        // Gyroscopic coupling: −ω × H_rw
        let gyro = self.core.gyroscopic_torque(omega, aux);

        ExternalLoads::torque(reaction + gyro)
    }
}

// Backward-compat aliases for gradual migration of external references.
/// Alias for [`Rw`].
pub type ReactionWheel = Rw;
/// Alias for [`RwAssembly`].
pub type ReactionWheelAssembly = RwAssembly;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;

    fn test_state_at_rest() -> AttitudeState {
        AttitudeState::identity()
    }

    // ── Core tests ──

    #[test]
    fn three_axis_has_three_wheels() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        assert_eq!(core.num_wheels(), 3);
    }

    #[test]
    fn clamp_torques_rate_limiting() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let momentum = [0.0, 0.0, 0.0];
        // Command 10 N·m but max is 0.1
        let clamped = core.clamp_torques(&[10.0, -10.0, 0.05], &momentum);
        assert!((clamped[0] - 0.1).abs() < 1e-15);
        assert!((clamped[1] - (-0.1)).abs() < 1e-15);
        assert!((clamped[2] - 0.05).abs() < 1e-15);
    }

    #[test]
    fn clamp_torques_saturation_positive() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // X-wheel at positive max momentum, positive command → clamped to 0
        let clamped = core.clamp_torques(&[0.05, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!(clamped[0].abs() < 1e-15);
    }

    #[test]
    fn clamp_torques_saturation_allows_desaturation() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // X-wheel at positive max, negative command → allowed (desaturation)
        let clamped = core.clamp_torques(&[-0.05, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!((clamped[0] - (-0.05)).abs() < 1e-15);
    }

    #[test]
    fn reaction_torque_opposes_wheel_acceleration() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let clamped = [0.05, 0.03, -0.02];
        let tau = core.reaction_torque(&clamped);
        // τ_body = -Σ(tau_i * axis_i)
        assert!((tau.x - (-0.05)).abs() < 1e-15);
        assert!((tau.y - (-0.03)).abs() < 1e-15);
        assert!((tau.z - 0.02).abs() < 1e-15);
    }

    #[test]
    fn gyroscopic_torque() {
        let core = RwAssemblyCore::three_axis(0.01, 10.0, 0.5);
        let omega = Vector3::new(0.0, 0.0, 1.0);
        let momentum = [5.0, 0.0, 0.0];
        let tau = core.gyroscopic_torque(&omega, &momentum);
        // H_rw = [5, 0, 0], omega = [0, 0, 1]
        // -omega × H_rw = -[0,0,1] × [5,0,0] = -[0,5,0] = [0,-5,0]
        assert!(tau.x.abs() < 1e-15);
        assert!((tau.y - (-5.0)).abs() < 1e-15);
        assert!(tau.z.abs() < 1e-15);
    }

    #[test]
    fn allocate_orthogonal() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // Desired body torque [0.05, 0.03, 0.02]
        // Allocation: per-wheel = -desired.dot(axis)
        let allocated = core.allocate(&Vector3::new(0.05, 0.03, 0.02));
        assert!((allocated[0] - (-0.05)).abs() < 1e-15);
        assert!((allocated[1] - (-0.03)).abs() < 1e-15);
        assert!((allocated[2] - (-0.02)).abs() < 1e-15);
    }

    // ── Assembly (StateEffector) tests ──

    #[test]
    fn assembly_three_axis_has_three_wheels() {
        let rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        assert_eq!(rw.wheels().len(), 3);
        assert_eq!(StateEffector::<AttitudeState>::state_dim(&rw), 3);
    }

    #[test]
    fn name_is_reaction_wheels() {
        let rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        assert_eq!(StateEffector::<AttitudeState>::name(&rw), "reaction_wheels");
    }

    #[test]
    fn zero_command_zero_output() {
        let rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);
        assert_eq!(rates, vec![0.0, 0.0, 0.0]);
        assert!(loads.torque_body.magnitude() < 1e-15);
    }

    #[test]
    fn commanded_torque_z_produces_rates() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Per-wheel: command Z-wheel with -0.05 (wheel absorbs → body gets +Z)
        rw.commanded_torques = vec![0.0, 0.0, -0.05];

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Z-wheel rate = -0.05
        assert!(rates[0].abs() < 1e-15);
        assert!(rates[1].abs() < 1e-15);
        assert!((rates[2] - (-0.05)).abs() < 1e-15);

        // Body torque should be +Z (reaction to wheel absorbing -Z)
        assert!((loads.torque_body.z() - 0.05).abs() < 1e-15);
    }

    #[test]
    fn torque_rate_limiting() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Command 10 N·m on X-wheel, but wheel max is 0.1
        rw.commanded_torques = vec![10.0, 0.0, 0.0];

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Clamped to 0.1
        assert!((rates[0] - 0.1).abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_positive() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Positive command on X-wheel at positive max momentum → clamped to 0
        rw.commanded_torques = vec![0.05, 0.0, 0.0];

        let state = test_state_at_rest();
        let aux = vec![1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        assert!(rates[0].abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_negative() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Negative command on X-wheel at negative max → clamped to 0
        rw.commanded_torques = vec![-0.05, 0.0, 0.0];

        let state = test_state_at_rest();
        let aux = vec![-1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        assert!(rates[0].abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_allows_opposite_direction() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Negative command on X-wheel at positive max → desaturation allowed
        rw.commanded_torques = vec![-0.05, 0.0, 0.0];

        let state = test_state_at_rest();
        let aux = vec![1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        assert!((rates[0] - (-0.05)).abs() < 1e-15);
    }

    #[test]
    fn reaction_torque_opposes_wheel_acceleration_at_rest() {
        let mut rw = RwAssembly::three_axis(0.01, 1.0, 0.1);
        // Per-wheel commanded: allocated from desired body torque [0.05, 0.03, 0.02]
        let desired = Vector3::new(0.05, 0.03, 0.02);
        rw.commanded_torques = rw.core().allocate(&desired);

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // At rest (no gyroscopic coupling), body torque should match desired
        let tb = loads.torque_body.into_inner();
        assert!((tb[0] - desired[0]).abs() < 1e-15);
        assert!((tb[1] - desired[1]).abs() < 1e-15);
        assert!((tb[2] - desired[2]).abs() < 1e-15);
    }

    #[test]
    fn gyroscopic_coupling_with_spinning_body() {
        let rw = RwAssembly::three_axis(0.01, 10.0, 0.5);

        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.0, 0.0, 1.0),
        };
        let aux = vec![5.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // H_rw = [5, 0, 0], omega = [0, 0, 1]
        // gyro = -omega × H_rw = -[0,5,0] = [0,-5,0]
        assert!(loads.torque_body.x().abs() < 1e-15);
        assert!((loads.torque_body.y() - (-5.0)).abs() < 1e-15);
        assert!(loads.torque_body.z().abs() < 1e-15);
    }
}
