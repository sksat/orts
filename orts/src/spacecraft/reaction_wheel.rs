//! Reaction wheel assembly as a [`StateEffector`].
//!
//! Models a set of reaction wheels, each with its own spin axis, inertia,
//! maximum angular momentum, and maximum torque. The assembly's auxiliary
//! state is the angular momentum of each wheel about its spin axis.
//!
//! The reaction torque on the spacecraft follows Newton's third law:
//! `τ_body = -Σ (dh_i/dt · axis_i)`.

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use super::ExternalLoads;
use crate::effector::StateEffector;
use crate::model::HasAttitude;

/// A single reaction wheel with physical limits.
#[derive(Debug, Clone)]
pub struct ReactionWheel {
    /// Spin axis in body frame (unit vector, normalized on construction).
    axis: Vector3<f64>,
    /// Moment of inertia about spin axis [kg·m²].
    pub inertia: f64,
    /// Maximum angular momentum [N·m·s].
    pub max_momentum: f64,
    /// Maximum torque (acceleration rate limit) [N·m].
    pub max_torque: f64,
}

impl ReactionWheel {
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

/// Assembly of reaction wheels as a [`StateEffector`].
///
/// Aux state: angular momentum `h_i` [N·m·s] for each wheel.
/// Reaction torque on spacecraft: `τ_body = -Σ (dh_i/dt · axis_i)`.
///
/// The `commanded_torque` field is `pub` so it can be updated between
/// integration segments (similar to `CommandedMagnetorquer`).
#[derive(Clone)]
pub struct ReactionWheelAssembly {
    wheels: Vec<ReactionWheel>,
    /// Desired torque on spacecraft body [N·m] (set externally by controller).
    /// Wheels produce the reaction: wheel torque = -commanded_torque projected onto axes.
    pub commanded_torque: Vector3<f64>,
}

impl ReactionWheelAssembly {
    /// Create an assembly from a list of reaction wheels.
    ///
    /// **Note**: Torque allocation uses simple axis projection, which is only
    /// exact for orthogonal wheel arrangements. For non-orthogonal layouts
    /// (e.g., pyramid), use [`three_axis`] or implement pseudo-inverse allocation.
    pub fn new(wheels: Vec<ReactionWheel>) -> Self {
        Self {
            wheels,
            commanded_torque: Vector3::zeros(),
        }
    }

    /// Standard 3-axis orthogonal arrangement with identical wheels.
    ///
    /// Torque allocation is exact for this configuration (each wheel axis
    /// is an orthonormal basis vector).
    pub fn three_axis(inertia: f64, max_momentum: f64, max_torque: f64) -> Self {
        Self::new(vec![
            ReactionWheel::new(Vector3::x(), inertia, max_momentum, max_torque),
            ReactionWheel::new(Vector3::y(), inertia, max_momentum, max_torque),
            ReactionWheel::new(Vector3::z(), inertia, max_momentum, max_torque),
        ])
    }

    /// Access the wheels.
    pub fn wheels(&self) -> &[ReactionWheel] {
        &self.wheels
    }
}

impl<S: HasAttitude + Send + Sync> StateEffector<S> for ReactionWheelAssembly {
    fn name(&self) -> &str {
        "reaction_wheels"
    }

    fn state_dim(&self) -> usize {
        self.wheels.len()
    }

    fn aux_bounds(&self) -> Vec<(f64, f64)> {
        self.wheels
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
        let mut total_reaction_torque = Vector3::zeros();

        // Compute total wheel angular momentum in body frame: H_rw = Σ h_i · axis_i
        let mut h_rw = Vector3::zeros();
        for (i, wheel) in self.wheels.iter().enumerate() {
            h_rw += aux[i] * wheel.axis;
        }

        for (i, wheel) in self.wheels.iter().enumerate() {
            let h_i = aux[i]; // current wheel momentum

            // Wheel motor torque = negative of desired body torque projected onto axis
            // (Newton's third law: to get +torque on body, wheels must absorb -torque)
            let mut tau_cmd = -self.commanded_torque.dot(&wheel.axis);

            // Torque rate limiting
            tau_cmd = tau_cmd.clamp(-wheel.max_torque, wheel.max_torque);

            // Momentum saturation: prevent exceeding limits
            if (h_i >= wheel.max_momentum && tau_cmd > 0.0)
                || (h_i <= -wheel.max_momentum && tau_cmd < 0.0)
            {
                tau_cmd = 0.0;
            }

            // Wheel momentum derivative: dh_i/dt = tau_cmd
            aux_rates[i] = tau_cmd;

            // Reaction torque from wheel acceleration (Newton's third law)
            total_reaction_torque -= tau_cmd * wheel.axis;
        }

        // Gyroscopic coupling: −ω × H_rw
        // The full reaction torque on the spacecraft body is:
        //   τ_rw = −dH_rw/dt − ω × H_rw
        // The first term (−dH_rw/dt) is computed per-wheel above.
        // The second term (−ω × H_rw) is the gyroscopic coupling.
        total_reaction_torque -= omega.cross(&h_rw);

        ExternalLoads::torque(total_reaction_torque)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;

    /// Test state with zero angular velocity (no gyroscopic coupling).
    fn test_state_at_rest() -> AttitudeState {
        AttitudeState::identity()
    }

    #[test]
    fn three_axis_has_three_wheels() {
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        assert_eq!(rw.wheels().len(), 3);
        assert_eq!(StateEffector::<AttitudeState>::state_dim(&rw), 3);
    }

    #[test]
    fn name_is_reaction_wheels() {
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        assert_eq!(StateEffector::<AttitudeState>::name(&rw), "reaction_wheels");
    }

    #[test]
    fn zero_command_zero_output() {
        let rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);
        assert_eq!(rates, vec![0.0, 0.0, 0.0]);
        assert!(loads.torque_body.magnitude() < 1e-15);
    }

    #[test]
    fn commanded_torque_z_produces_rates() {
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        // Command +Z body torque → wheel absorbs -Z momentum
        rw.commanded_torque = Vector3::new(0.0, 0.0, 0.05);

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Z-wheel absorbs reaction: dh/dt = -0.05
        assert!((rates[0]).abs() < 1e-15);
        assert!((rates[1]).abs() < 1e-15);
        assert!((rates[2] - (-0.05)).abs() < 1e-15);

        // Body torque should be +Z as commanded
        assert!((loads.torque_body.z() - 0.05).abs() < 1e-15);
    }

    #[test]
    fn torque_rate_limiting() {
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        // Command 10 N·m body torque, but wheel max is 0.1 N·m
        rw.commanded_torque = Vector3::new(10.0, 0.0, 0.0);

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Wheel torque clamped to 0.1, so wheel rate = -0.1
        assert!((rates[0] - (-0.1)).abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_positive() {
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        // Command -X body torque → wheel gets +X torque, but already at +max
        rw.commanded_torque = Vector3::new(-0.05, 0.0, 0.0);

        let state = test_state_at_rest();
        // X-wheel is already at max momentum
        let aux = vec![1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Wheel at +max and command would push further positive → clamped to 0
        assert!((rates[0]).abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_negative() {
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        // Command +X body torque → wheel gets -X torque, but already at -max
        rw.commanded_torque = Vector3::new(0.05, 0.0, 0.0);

        let state = test_state_at_rest();
        // X-wheel is at negative max momentum
        let aux = vec![-1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Wheel at -max and command would push further negative → clamped to 0
        assert!((rates[0]).abs() < 1e-15);
    }

    #[test]
    fn momentum_saturation_allows_opposite_direction() {
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        // Command +X body torque → wheel gets -X torque, and wheel is at +max → allowed (desaturation)
        rw.commanded_torque = Vector3::new(0.05, 0.0, 0.0);

        let state = test_state_at_rest();
        // X-wheel is at positive max momentum
        let aux = vec![1.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Negative wheel torque when at positive limit → desaturation allowed
        assert!((rates[0] - (-0.05)).abs() < 1e-15);
    }

    #[test]
    fn reaction_torque_opposes_wheel_acceleration_at_rest() {
        // At rest (omega=0), the reaction torque is purely -dH_rw/dt (no gyroscopic term)
        let mut rw = ReactionWheelAssembly::three_axis(0.01, 1.0, 0.1);
        rw.commanded_torque = Vector3::new(0.05, 0.03, 0.02);

        let state = test_state_at_rest();
        let aux = vec![0.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // Reaction torque should be opposite to wheel acceleration
        let tb = loads.torque_body.into_inner();
        for i in 0..3 {
            assert!(
                (tb[i] + rates[i]).abs() < 1e-15,
                "Reaction torque[{i}] should equal -rates[{i}]"
            );
        }
    }

    #[test]
    fn gyroscopic_coupling_with_spinning_body() {
        // When body rotates about Z and X-wheel has momentum,
        // the gyroscopic term -ω × H_rw creates a torque
        let rw = ReactionWheelAssembly::three_axis(0.01, 10.0, 0.5);

        let state = AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::new(0.0, 0.0, 1.0), // spinning about Z
        };
        // X-wheel has momentum h_x = 5.0
        let aux = vec![5.0, 0.0, 0.0];
        let mut rates = vec![0.0, 0.0, 0.0];
        let loads = rw.derivatives(0.0, &state, &aux, &mut rates, None);

        // H_rw = [5, 0, 0], omega = [0, 0, 1]
        // omega x H_rw = [0,0,1] x [5,0,0] = [0*0-1*0, 1*5-0*0, 0*0-0*5] = [0, 5, 0]
        // gyroscopic torque = -omega x H_rw = [0, -5, 0]
        // No commanded torque, so dH/dt = 0, total torque should be [0, -5, 0]
        assert!(
            (loads.torque_body.x()).abs() < 1e-15,
            "tau_x should be 0, got {}",
            loads.torque_body.x()
        );
        assert!(
            (loads.torque_body.y() - (-5.0)).abs() < 1e-15,
            "tau_y should be -5, got {}",
            loads.torque_body.y()
        );
        assert!(
            (loads.torque_body.z()).abs() < 1e-15,
            "tau_z should be 0, got {}",
            loads.torque_body.z()
        );
    }
}
