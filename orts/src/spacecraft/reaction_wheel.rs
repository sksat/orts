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
    /// Motor time constant [s]. `None` = instantaneous (legacy behavior).
    ///
    /// When set, the realized torque follows the commanded torque via
    /// a first-order lag: `dτ_realized/dt = (τ_target - τ_realized) / T_m`.
    pub motor_time_constant: Option<f64>,
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
            motor_time_constant: None,
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

    /// Set the motor time constant [s] (builder pattern).
    ///
    /// # Panics
    /// Panics if `t_m` is not positive and finite.
    pub fn with_motor_lag(mut self, t_m: f64) -> Self {
        assert!(
            t_m > 0.0 && t_m.is_finite(),
            "motor_time_constant must be positive and finite, got {t_m}"
        );
        self.motor_time_constant = Some(t_m);
        self
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

/// Build a pseudo-inverse allocation matrix from device axes.
///
/// Given axes `a_0, a_1, ..., a_{n-1}` (each a 3D unit vector),
/// construct `B: 3×n` where column `i` is `a_i`, then return
/// `B^+: n×3` (the Moore-Penrose pseudo-inverse).
///
/// - `n >= 3` and rank 3 → minimum-norm solution (overactuated)
/// - `n < 3` or rank < 3 → least-squares approximation (underactuated)
///
/// Used by both RW and MTQ allocation.
pub(super) fn build_allocation_pinv(axes: &[Vector3<f64>]) -> nalgebra::DMatrix<f64> {
    use nalgebra::DMatrix;
    let n = axes.len();
    if n == 0 {
        return DMatrix::zeros(0, 3);
    }
    // B: 3×n, columns are unit axes
    let b = DMatrix::from_fn(3, n, |row, col| axes[col][row]);
    let eps = 1e-12;
    b.pseudo_inverse(eps)
        .expect("pseudo_inverse with eps=1e-12 should not fail for unit axes")
}

/// RW assembly geometry and constraint logic (no ODE state integration).
///
/// This core struct handles per-wheel torque clamping, momentum saturation,
/// reaction torque computation, and torque allocation without depending on
/// the ODE system. It is designed to be unit-tested independently.
///
/// Allocation uses a precomputed pseudo-inverse matrix, supporting
/// non-orthogonal wheel arrangements (e.g., pyramid 4-wheel).
#[derive(Debug, Clone)]
pub struct RwAssemblyCore {
    wheels: Vec<Rw>,
    /// Allocation matrix (pseudo-inverse of axis matrix), `n×3`.
    alloc_pinv: nalgebra::DMatrix<f64>,
    /// True if any wheel has a motor time constant.
    has_motor_lag: bool,
}

impl RwAssemblyCore {
    /// Create an assembly core from a list of reaction wheels.
    pub fn new(wheels: Vec<Rw>) -> Self {
        let axes: Vec<_> = wheels.iter().map(|w| *w.axis()).collect();
        let alloc_pinv = build_allocation_pinv(&axes);
        let has_motor_lag = wheels.iter().any(|w| w.motor_time_constant.is_some());
        Self {
            wheels,
            alloc_pinv,
            has_motor_lag,
        }
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

    /// Whether any wheel has a motor time constant (first-order lag).
    pub fn has_motor_lag(&self) -> bool {
        self.has_motor_lag
    }

    /// Auxiliary state dimension: `n` (instantaneous) or `2n` (motor lag).
    ///
    /// Layout when motor lag is active:
    /// `[h_0, ..., h_{n-1}, τ_realized_0, ..., τ_realized_{n-1}]`
    pub fn state_dim(&self) -> usize {
        let n = self.wheels.len();
        if self.has_motor_lag { 2 * n } else { n }
    }

    /// Extract the momentum slice from the aux state vector.
    pub fn momentum_slice<'a>(&self, aux: &'a [f64]) -> &'a [f64] {
        &aux[..self.wheels.len()]
    }

    /// Extract the realized torque slice from the aux state vector.
    ///
    /// Returns `None` if there is no motor lag (instantaneous mode).
    pub fn realized_torque_slice<'a>(&self, aux: &'a [f64]) -> Option<&'a [f64]> {
        if self.has_motor_lag {
            let n = self.wheels.len();
            Some(&aux[n..2 * n])
        } else {
            None
        }
    }

    /// Apply rate limiting, momentum saturation, and speed saturation
    /// to per-wheel commanded torques. Returns clamped per-wheel torques.
    ///
    /// This is a convenience method that chains [`clamp_command`] and
    /// [`clamp_physical`].
    ///
    /// # Panics
    /// Panics if `commanded.len()` or `momentum.len()` != `self.num_wheels()`.
    pub fn clamp_torques(&self, commanded: &[f64], momentum: &[f64]) -> Vec<f64> {
        let accepted = self.clamp_command(commanded);
        self.clamp_physical(&accepted, momentum)
    }

    /// Command acceptance clamp: limit torques to motor driver's range
    /// `[-max_torque, max_torque]` per wheel.
    ///
    /// This simulates what the RW driver accepts as a valid command.
    /// No state-dependent constraints (momentum/speed saturation) are
    /// applied here — use [`clamp_physical`] for that.
    ///
    /// # Panics
    /// Panics if `commanded.len()` != `self.num_wheels()`.
    pub fn clamp_command(&self, commanded: &[f64]) -> Vec<f64> {
        assert_eq!(commanded.len(), self.wheels.len());
        self.wheels
            .iter()
            .enumerate()
            .map(|(i, wheel)| commanded[i].clamp(-wheel.max_torque, wheel.max_torque))
            .collect()
    }

    /// Physical constraint clamp: zero out torques that would push a
    /// wheel past its momentum or speed limits.
    ///
    /// This enforces the physics — a wheel at max momentum cannot
    /// accelerate further, regardless of what the motor is trying to do.
    /// Deceleration (desaturation) is always allowed.
    ///
    /// # Panics
    /// Panics if `torques.len()` or `momentum.len()` != `self.num_wheels()`.
    pub fn clamp_physical(&self, torques: &[f64], momentum: &[f64]) -> Vec<f64> {
        assert_eq!(torques.len(), self.wheels.len());
        assert_eq!(momentum.len(), self.wheels.len());
        self.wheels
            .iter()
            .enumerate()
            .map(|(i, wheel)| {
                let mut tau = torques[i];
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

    /// Allocate a desired body-frame torque to per-wheel motor torques.
    ///
    /// Uses the precomputed pseudo-inverse of the axis matrix for
    /// correct allocation in non-orthogonal layouts (pyramid, skewed).
    /// For orthogonal 3-axis this produces the same result as simple
    /// axis projection.
    ///
    /// The result is **unclamped** — pass to [`clamp_torques`] for
    /// rate limiting and saturation.
    ///
    /// When underactuated (fewer axes than 3), the least-squares
    /// approximation is returned (unrealizable components are dropped).
    pub fn allocate(&self, desired: &Vector3<f64>) -> Vec<f64> {
        // RW sign convention: u = pinv * (-desired)
        // (Newton's 3rd law: body torque = -Σ u_i axis_i)
        let neg_desired =
            nalgebra::DVector::from_column_slice(&[-desired.x, -desired.y, -desired.z]);
        let result = &self.alloc_pinv * neg_desired;
        result.as_slice().to_vec()
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
        self.core.state_dim()
    }

    fn aux_bounds(&self) -> Vec<(f64, f64)> {
        let mut bounds: Vec<_> = self
            .core
            .wheels
            .iter()
            .map(|w| (-w.max_momentum, w.max_momentum))
            .collect();
        if self.core.has_motor_lag() {
            // τ_realized bounds: [-max_torque, max_torque] per wheel
            for w in &self.core.wheels {
                bounds.push((-w.max_torque, w.max_torque));
            }
        }
        bounds
    }

    fn derivatives(
        &self,
        _t: f64,
        state: &S,
        aux: &[f64],
        aux_rates: &mut [f64],
        _epoch: Option<&Epoch>,
    ) -> ExternalLoads {
        let n = self.core.num_wheels();
        let omega = &state.attitude().angular_velocity;
        let momentum = self.core.momentum_slice(aux);

        // Resolve the command variant into effective torques.
        let effective_torques = match &self.command {
            RwCommand::Torques(t) => t.clone(),
            RwCommand::Speeds(s) => self
                .core
                .speed_to_torque(s, momentum, self.speed_control_gain),
        };

        // Command acceptance clamp (motor driver range)
        let accepted = self.core.clamp_command(&effective_torques);

        let applied = if self.core.has_motor_lag() {
            let tau_realized = self.core.realized_torque_slice(aux).unwrap();

            // Motor lag ODE: dτ_realized/dt = (τ_target - τ_realized) / T_m
            let tau_target = self.core.clamp_physical(&accepted, momentum);
            for (i, wheel) in self.core.wheels.iter().enumerate() {
                if let Some(t_m) = wheel.motor_time_constant {
                    aux_rates[n + i] = (tau_target[i] - tau_realized[i]) / t_m;
                } else {
                    // Instantaneous wheel in 2n layout: snap realized to
                    // target via a fast tracking rate. Physics uses
                    // tau_target directly (see effective_realized below),
                    // so this only affects telemetry convergence.
                    // Effective T_m ≈ 0.01s; stable with dt ≤ 0.028s (RK4).
                    // For larger dt the aux_bounds projection keeps it bounded.
                    aux_rates[n + i] = (tau_target[i] - tau_realized[i]) * 100.0;
                }
            }

            // Physical constraint on realized torque for dh/dt
            let effective_realized: Vec<f64> = self
                .core
                .wheels
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    if w.motor_time_constant.is_some() {
                        tau_realized[i]
                    } else {
                        tau_target[i]
                    }
                })
                .collect();
            self.core.clamp_physical(&effective_realized, momentum)
        } else {
            // No motor lag: combined clamp (legacy path)
            self.core.clamp_physical(&accepted, momentum)
        };

        // Set aux rates: dh_i/dt = τ_applied_i
        for (i, &tau) in applied.iter().enumerate() {
            aux_rates[i] = tau;
        }

        // Reaction torque from wheel acceleration (Newton's third law)
        let reaction = self.core.reaction_torque(&applied);

        // Gyroscopic coupling: −ω × H_rw
        let gyro = self.core.gyroscopic_torque(omega, momentum);

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

    // ── clamp_command tests ──

    #[test]
    fn clamp_command_rate_limiting() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let accepted = core.clamp_command(&[10.0, -10.0, 0.05]);
        assert!((accepted[0] - 0.1).abs() < 1e-15);
        assert!((accepted[1] - (-0.1)).abs() < 1e-15);
        assert!((accepted[2] - 0.05).abs() < 1e-15);
    }

    #[test]
    fn clamp_command_ignores_momentum() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // clamp_command doesn't know about momentum — only rate limits
        let accepted = core.clamp_command(&[0.05, 0.0, 0.0]);
        assert!((accepted[0] - 0.05).abs() < 1e-15);
    }

    // ── clamp_physical tests ──

    #[test]
    fn clamp_physical_saturation_positive() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // X-wheel at positive max momentum → positive torque zeroed
        let applied = core.clamp_physical(&[0.05, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!(applied[0].abs() < 1e-15);
    }

    #[test]
    fn clamp_physical_allows_desaturation() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        // X-wheel at positive max, negative torque → allowed (desaturation)
        let applied = core.clamp_physical(&[-0.05, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!((applied[0] - (-0.05)).abs() < 1e-15);
    }

    #[test]
    fn clamp_physical_passes_through_below_limits() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let applied = core.clamp_physical(&[0.05, -0.03, 0.02], &[0.0, 0.0, 0.0]);
        assert!((applied[0] - 0.05).abs() < 1e-15);
        assert!((applied[1] - (-0.03)).abs() < 1e-15);
        assert!((applied[2] - 0.02).abs() < 1e-15);
    }

    // ── clamp_torques (combined) backward compat ──

    #[test]
    fn clamp_torques_rate_limiting() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let momentum = [0.0, 0.0, 0.0];
        let clamped = core.clamp_torques(&[10.0, -10.0, 0.05], &momentum);
        assert!((clamped[0] - 0.1).abs() < 1e-15);
        assert!((clamped[1] - (-0.1)).abs() < 1e-15);
        assert!((clamped[2] - 0.05).abs() < 1e-15);
    }

    #[test]
    fn clamp_torques_saturation_positive() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let clamped = core.clamp_torques(&[0.05, 0.0, 0.0], &[1.0, 0.0, 0.0]);
        assert!(clamped[0].abs() < 1e-15);
    }

    #[test]
    fn clamp_torques_saturation_allows_desaturation() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
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
        let desired = Vector3::new(0.05, 0.03, 0.02);
        let allocated = core.allocate(&desired);
        // Allocation should produce correct per-wheel values
        assert!((allocated[0] - (-0.05)).abs() < 1e-12);
        assert!((allocated[1] - (-0.03)).abs() < 1e-12);
        assert!((allocated[2] - (-0.02)).abs() < 1e-12);
    }

    #[test]
    fn allocate_orthogonal_roundtrip() {
        // allocate → reaction_torque should recover the desired body torque
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let desired = Vector3::new(0.05, 0.03, 0.02);
        let allocated = core.allocate(&desired);
        let realized = core.reaction_torque(&allocated);
        assert!(
            (realized - desired).magnitude() < 1e-12,
            "roundtrip error: {:.3e}",
            (realized - desired).magnitude()
        );
    }

    #[test]
    fn allocate_pyramid_4wheel_roundtrip() {
        // 4-wheel pyramid: overactuated (n=4, rank=3)
        let angle = std::f64::consts::FRAC_PI_4; // 45°
        let sin = angle.sin();
        let cos = angle.cos();
        let core = RwAssemblyCore::new(vec![
            Rw::new(Vector3::new(sin, 0.0, cos), 0.01, 1.0, 0.1),
            Rw::new(Vector3::new(0.0, sin, cos), 0.01, 1.0, 0.1),
            Rw::new(Vector3::new(-sin, 0.0, cos), 0.01, 1.0, 0.1),
            Rw::new(Vector3::new(0.0, -sin, cos), 0.01, 1.0, 0.1),
        ]);
        let desired = Vector3::new(0.05, 0.03, 0.02);
        let allocated = core.allocate(&desired);
        assert_eq!(allocated.len(), 4);
        let realized = core.reaction_torque(&allocated);
        assert!(
            (realized - desired).magnitude() < 1e-12,
            "pyramid roundtrip error: {:.3e}",
            (realized - desired).magnitude()
        );
    }

    #[test]
    fn allocate_2axis_drops_unrealizable() {
        // 2-wheel X/Y only (underactuated, rank=2)
        let core = RwAssemblyCore::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1),
            Rw::new(Vector3::y(), 0.01, 1.0, 0.1),
        ]);
        // Desired includes Z component which can't be realized
        let desired = Vector3::new(0.05, 0.03, 0.02);
        let allocated = core.allocate(&desired);
        assert_eq!(allocated.len(), 2);
        let realized = core.reaction_torque(&allocated);
        // X and Y should be realized, Z dropped
        assert!((realized.x - 0.05).abs() < 1e-12);
        assert!((realized.y - 0.03).abs() < 1e-12);
        assert!(realized.z.abs() < 1e-12);
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

    // ── Motor lag tests ──

    #[test]
    fn rw_with_motor_lag() {
        let rw = Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05);
        assert_eq!(rw.motor_time_constant, Some(0.05));
    }

    #[test]
    #[should_panic(expected = "motor_time_constant must be positive")]
    fn rw_motor_lag_zero_panics() {
        Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.0);
    }

    #[test]
    #[should_panic(expected = "motor_time_constant must be positive")]
    fn rw_motor_lag_negative_panics() {
        Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(-1.0);
    }

    #[test]
    fn core_has_motor_lag_false() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        assert!(!core.has_motor_lag());
        assert_eq!(core.state_dim(), 3);
    }

    #[test]
    fn core_has_motor_lag_true() {
        let core = RwAssemblyCore::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
            Rw::new(Vector3::y(), 0.01, 1.0, 0.1),
            Rw::new(Vector3::z(), 0.01, 1.0, 0.1).with_motor_lag(0.1),
        ]);
        assert!(core.has_motor_lag());
        assert_eq!(core.state_dim(), 6); // 2n
    }

    #[test]
    fn momentum_slice_no_lag() {
        let core = RwAssemblyCore::three_axis(0.01, 1.0, 0.1);
        let aux = [1.0, 2.0, 3.0];
        assert_eq!(core.momentum_slice(&aux), &[1.0, 2.0, 3.0]);
        assert!(core.realized_torque_slice(&aux).is_none());
    }

    #[test]
    fn momentum_and_torque_slices_with_lag() {
        let core = RwAssemblyCore::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
            Rw::new(Vector3::y(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
        ]);
        let aux = [0.5, -0.3, 0.01, -0.02];
        assert_eq!(core.momentum_slice(&aux), &[0.5, -0.3]);
        assert_eq!(core.realized_torque_slice(&aux), Some(&[0.01, -0.02][..]));
    }

    #[test]
    fn assembly_motor_lag_state_dim() {
        let rw = RwAssembly::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
            Rw::new(Vector3::y(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
            Rw::new(Vector3::z(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
        ]);
        assert_eq!(StateEffector::<AttitudeState>::state_dim(&rw), 6);
    }

    #[test]
    fn assembly_motor_lag_aux_bounds() {
        let rw = RwAssembly::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
            Rw::new(Vector3::y(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
        ]);
        let bounds = StateEffector::<AttitudeState>::aux_bounds(&rw);
        assert_eq!(bounds.len(), 4); // 2n = 4
        // First 2: momentum bounds
        assert_eq!(bounds[0], (-1.0, 1.0));
        assert_eq!(bounds[1], (-1.0, 1.0));
        // Next 2: torque bounds
        assert_eq!(bounds[2], (-0.1, 0.1));
        assert_eq!(bounds[3], (-0.1, 0.1));
    }

    #[test]
    fn assembly_motor_lag_derivatives_has_torque_lag() {
        let mut rw = RwAssembly::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
        ]);
        rw.command = RwCommand::Torques(vec![0.1]); // full torque

        let state = test_state_at_rest();
        // aux = [h, τ_realized], both start at 0
        let aux = vec![0.0, 0.0];
        let mut rates = vec![0.0, 0.0];
        let _loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // dτ_realized/dt = (τ_target - τ_realized) / T_m = (0.1 - 0.0) / 0.05 = 2.0
        assert!((rates[1] - 2.0).abs() < 1e-12);
        // dh/dt = τ_applied = clamp_physical(τ_realized=0, h=0) = 0
        // (realized is still 0, so no torque applied yet)
        assert!(rates[0].abs() < 1e-15);
    }

    #[test]
    fn assembly_motor_lag_partially_realized() {
        let mut rw = RwAssembly::new(vec![
            Rw::new(Vector3::x(), 0.01, 1.0, 0.1).with_motor_lag(0.05),
        ]);
        rw.command = RwCommand::Torques(vec![0.1]);

        let state = test_state_at_rest();
        // τ_realized has caught up to 0.06
        let aux = vec![0.0, 0.06];
        let mut rates = vec![0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // dτ_realized/dt = (0.1 - 0.06) / 0.05 = 0.8
        assert!((rates[1] - 0.8).abs() < 1e-12);
        // dh/dt = τ_applied = clamp_physical(0.06, 0) = 0.06
        assert!((rates[0] - 0.06).abs() < 1e-12);
        // Body torque = -0.06 * x_axis
        assert!((loads.torque_body.x() - (-0.06)).abs() < 1e-12);
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
