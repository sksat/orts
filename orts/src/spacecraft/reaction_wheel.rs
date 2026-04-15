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
    /// Maximum spin speed [rad/s]. Independent of max_momentum (bearing/motor limit).
    pub max_speed: f64,
}

impl Rw {
    /// Create a reaction wheel with the given spin axis (will be normalized).
    ///
    /// `max_speed` is derived as `max_momentum / inertia`. For a custom
    /// speed limit (e.g., bearing-limited), use [`Rw::with_max_speed`].
    ///
    /// # Panics
    /// Panics if `axis` is zero-length, `inertia` is not positive/finite,
    /// or `max_momentum`/`max_torque` are negative.
    pub fn new(axis: Vector3<f64>, inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        let norm = axis.magnitude();
        assert!(norm > 1e-15, "Wheel axis must be non-zero");
        assert!(
            inertia.is_finite() && inertia > 0.0,
            "inertia must be positive and finite, got {inertia}"
        );
        assert!(
            max_momentum >= 0.0 && max_momentum.is_finite(),
            "max_momentum must be non-negative and finite, got {max_momentum}"
        );
        assert!(
            max_torque >= 0.0 && max_torque.is_finite(),
            "max_torque must be non-negative and finite, got {max_torque}"
        );
        let max_speed = max_momentum / inertia;
        Self {
            axis: axis / norm,
            inertia,
            max_momentum,
            max_torque,
            max_speed,
        }
    }

    /// Create a reaction wheel with an explicit max speed limit.
    ///
    /// The effective max speed is `min(max_speed, max_momentum / inertia)`.
    /// The effective max momentum is also tightened to
    /// `min(max_momentum, inertia * max_speed)` so that ODE auxiliary
    /// bounds are consistent with the speed limit.
    pub fn with_max_speed(
        axis: Vector3<f64>,
        inertia: f64,
        max_momentum: f64,
        max_torque: f64,
        max_speed: f64,
    ) -> Self {
        assert!(
            max_speed >= 0.0 && max_speed.is_finite(),
            "max_speed must be non-negative and finite, got {max_speed}"
        );
        let mut rw = Self::new(axis, inertia, max_momentum, max_torque);
        let derived_max_speed = max_momentum / inertia;
        rw.max_speed = max_speed.min(derived_max_speed);
        // Tighten momentum bound to match speed limit
        rw.max_momentum = rw.max_momentum.min(inertia * rw.max_speed);
        rw
    }

    /// Get the spin axis unit vector.
    pub fn axis(&self) -> &Vector3<f64> {
        &self.axis
    }

    /// Current spin speed from angular momentum [rad/s].
    pub fn speed_from_momentum(&self, h: f64) -> f64 {
        h / self.inertia
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

    /// Apply rate limiting, momentum saturation, and speed saturation
    /// to per-wheel commanded torques. Returns clamped per-wheel torques.
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
                // Speed saturation: prevent exceeding max_speed
                let speed = wheel.speed_from_momentum(momentum[i]);
                if (speed >= wheel.max_speed && tau > 0.0)
                    || (speed <= -wheel.max_speed && tau < 0.0)
                {
                    tau = 0.0;
                }
                tau
            })
            .collect()
    }

    /// Convert per-wheel target speeds to per-wheel torques via
    /// proportional control. Does NOT apply rate limiting or saturation
    /// — pass the result to [`clamp_torques`] for that.
    ///
    /// `tau_i = gain * (target_speed_i - current_speed_i)`
    ///
    /// Target speeds are clamped to `[-max_speed, max_speed]`.
    ///
    /// # Panics
    /// Panics if `target_speeds.len()` or `momentum.len()` != `self.num_wheels()`.
    pub fn speed_to_torque(&self, target_speeds: &[f64], momentum: &[f64], gain: f64) -> Vec<f64> {
        assert_eq!(target_speeds.len(), self.wheels.len());
        assert_eq!(momentum.len(), self.wheels.len());
        self.wheels
            .iter()
            .enumerate()
            .map(|(i, wheel)| {
                let target = target_speeds[i].clamp(-wheel.max_speed, wheel.max_speed);
                let current = wheel.speed_from_momentum(momentum[i]);
                gain * (target - current)
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

/// Per-wheel RW command used by [`RwAssembly`].
///
/// Re-exported from `crate::plugin::command::RwCommand` for convenience.
pub use crate::plugin::command::RwCommand;

/// Default speed control bandwidth [rad/s] for deriving the gain.
///
/// The proportional gain is `I_wheel * bandwidth`. A bandwidth of
/// 10 rad/s is reasonable for a typical small satellite RW.
const DEFAULT_SPEED_CONTROL_BANDWIDTH: f64 = 10.0;

/// RW assembly as a [`StateEffector`].
///
/// Wraps [`RwAssemblyCore`] with ODE auxiliary state (per-wheel angular
/// momentum) and a per-wheel command (speed or torque, zero-order hold).
///
/// Aux state: angular momentum `h_i` [N·m·s] for each wheel.
/// Reaction torque on spacecraft: `τ_body = -Σ (dh_i/dt · axis_i) − ω × H_rw`.
#[derive(Clone)]
pub struct RwAssembly {
    core: RwAssemblyCore,
    /// Per-wheel RW command (set externally by controller).
    pub command: RwCommand,
    /// Proportional gain for speed → torque conversion [N·m / (rad/s)].
    pub speed_control_gain: f64,
}

impl RwAssembly {
    /// Create an assembly from a list of reaction wheels.
    ///
    /// Default speed control gain is derived from the first wheel's
    /// inertia × bandwidth (10 rad/s).
    pub fn new(wheels: Vec<Rw>) -> Self {
        let n = wheels.len();
        let gain = wheels
            .first()
            .map(|w| w.inertia * DEFAULT_SPEED_CONTROL_BANDWIDTH)
            .unwrap_or(0.1);
        Self {
            core: RwAssemblyCore::new(wheels),
            command: RwCommand::Torques(vec![0.0; n]),
            speed_control_gain: gain,
        }
    }

    /// Standard 3-axis orthogonal arrangement with identical wheels.
    pub fn three_axis(inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        let core = RwAssemblyCore::three_axis(inertia, max_momentum, max_torque);
        let n = core.num_wheels();
        Self {
            core,
            command: RwCommand::Torques(vec![0.0; n]),
            speed_control_gain: inertia * DEFAULT_SPEED_CONTROL_BANDWIDTH,
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

        // Resolve the command variant into effective torques.
        let effective_torques = match &self.command {
            RwCommand::Torques(t) => t.clone(),
            RwCommand::Speeds(s) => self.core.speed_to_torque(s, aux, self.speed_control_gain),
        };

        // Clamp per-wheel commanded torques (rate limiting + saturation)
        let clamped = self.core.clamp_torques(&effective_torques, aux);

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

    // ── Rw validation tests ──

    #[test]
    #[should_panic(expected = "inertia must be positive")]
    fn rw_zero_inertia_panics() {
        Rw::new(Vector3::x(), 0.0, 1.0, 0.1);
    }

    #[test]
    #[should_panic(expected = "inertia must be positive")]
    fn rw_negative_inertia_panics() {
        Rw::new(Vector3::x(), -1.0, 1.0, 0.1);
    }

    #[test]
    #[should_panic(expected = "inertia must be positive")]
    fn rw_nan_inertia_panics() {
        Rw::new(Vector3::x(), f64::NAN, 1.0, 0.1);
    }

    #[test]
    fn rw_max_speed_derived() {
        let rw = Rw::new(Vector3::x(), 0.01, 1.0, 0.1);
        assert!((rw.max_speed - 100.0).abs() < 1e-10); // 1.0 / 0.01
    }

    #[test]
    fn rw_max_speed_custom_lower() {
        // Bearing limit lower than momentum-derived speed
        let rw = Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 50.0);
        assert!((rw.max_speed - 50.0).abs() < 1e-10);
    }

    #[test]
    fn rw_max_speed_custom_higher_clamped() {
        // Custom limit higher than momentum-derived → clamped to derived
        let rw = Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 200.0);
        assert!((rw.max_speed - 100.0).abs() < 1e-10); // min(200, 1.0/0.01)
    }

    #[test]
    fn rw_max_speed_tightens_max_momentum() {
        // max_speed = 50 → effective max_momentum = 0.01 * 50 = 0.5
        let rw = Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 50.0);
        assert!((rw.max_momentum - 0.5).abs() < 1e-10);
    }

    // ── Core tests ──

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

    #[test]
    fn clamp_torques_speed_saturation() {
        // max_speed = 50 rad/s (bearing-limited), inertia = 0.01
        // At 50 rad/s, h = 0.5 N·m·s (well below max_momentum = 1.0)
        let core =
            RwAssemblyCore::new(vec![Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 50.0)]);
        // h = 0.5 → speed = 50 rad/s = max_speed
        // Positive torque (would increase speed) → clamped to 0
        let clamped = core.clamp_torques(&[0.05], &[0.5]);
        assert!(clamped[0].abs() < 1e-15);
    }

    #[test]
    fn clamp_torques_speed_saturation_allows_despin() {
        let core =
            RwAssemblyCore::new(vec![Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 50.0)]);
        // At max speed, negative torque (despin) is allowed
        let clamped = core.clamp_torques(&[-0.05], &[0.5]);
        assert!((clamped[0] - (-0.05)).abs() < 1e-15);
    }

    #[test]
    fn speed_to_torque_basic() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let gain = 0.5; // [N·m / (rad/s)]
        // Target: 10 rad/s, current momentum = 0 → current speed = 0
        // tau = 0.5 * (10 - 0) = 5.0
        let torques = core.speed_to_torque(&[10.0, 0.0, 0.0], &[0.0, 0.0, 0.0], gain);
        assert!((torques[0] - 5.0).abs() < 1e-14);
        assert!(torques[1].abs() < 1e-14);
        assert!(torques[2].abs() < 1e-14);
    }

    #[test]
    fn speed_to_torque_at_target() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let gain = 0.5;
        // Target: 10 rad/s, current momentum = 0.1 → speed = 10 rad/s → error = 0
        let torques = core.speed_to_torque(&[10.0, 0.0, 0.0], &[0.1, 0.0, 0.0], gain);
        assert!(torques[0].abs() < 1e-14);
    }

    #[test]
    fn speed_to_torque_clamps_target_to_max_speed() {
        // max_speed = 50 rad/s
        let core =
            RwAssemblyCore::new(vec![Rw::with_max_speed(Vector3::x(), 0.01, 1.0, 0.1, 50.0)]);
        let gain = 1.0;
        // Target: 100 rad/s → clamped to 50, current = 0 → tau = 50
        let torques = core.speed_to_torque(&[100.0], &[0.0], gain);
        assert!((torques[0] - 50.0).abs() < 1e-14);
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
        rw.command = RwCommand::Torques(vec![0.0, 0.0, -0.05]);

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
        rw.command = RwCommand::Torques(vec![10.0, 0.0, 0.0]);

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
        rw.command = RwCommand::Torques(vec![0.05, 0.0, 0.0]);

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
        rw.command = RwCommand::Torques(vec![-0.05, 0.0, 0.0]);

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
        rw.command = RwCommand::Torques(vec![-0.05, 0.0, 0.0]);

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
        rw.command = RwCommand::Torques(rw.core().allocate(&desired));

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
