use kaname::epoch::Epoch;
use nalgebra::Matrix3;
use orts_integrator::DynamicalSystem;
use crate::gravity::GravityField;

use super::{ExternalLoads, LoadModel, SpacecraftState};

/// Coupled orbit-attitude dynamics for a rigid spacecraft.
///
/// Composes a gravitational field, inertia tensor, and external load models
/// into a [`DynamicalSystem`] for the 14-dimensional [`SpacecraftState`].
///
/// The gravity model `G` is resolved statically; load models remain trait objects
/// since they form a heterogeneous collection.
///
/// Equations of motion:
/// - Translation: dr/dt = v, dv/dt = a_gravity + Σ a_loads
/// - Rotation: dq/dt = ½ q ⊗ (0,ω), dω/dt = I⁻¹(τ − ω × Iω)
pub struct SpacecraftDynamics<G: GravityField> {
    mu: f64,
    gravity: G,
    inertia: Matrix3<f64>,
    inertia_inv: Matrix3<f64>,
    loads: Vec<Box<dyn LoadModel>>,
    epoch_0: Option<Epoch>,
    body_radius: Option<f64>,
}

impl<G: GravityField> SpacecraftDynamics<G> {
    /// Create with gravitational parameter, gravity model, and inertia tensor.
    ///
    /// # Panics
    /// Panics if `inertia` is singular (not invertible).
    pub fn new(mu: f64, gravity: G, inertia: Matrix3<f64>) -> Self {
        let inertia_inv = inertia
            .try_inverse()
            .expect("Inertia tensor must be invertible");
        Self {
            mu,
            gravity,
            inertia,
            inertia_inv,
            loads: Vec::new(),
            epoch_0: None,
            body_radius: None,
        }
    }

    /// Add an external load model (builder pattern).
    pub fn with_load(mut self, load: Box<dyn LoadModel>) -> Self {
        self.loads.push(load);
        self
    }

    /// Set the initial epoch corresponding to integration time t = 0.
    pub fn with_epoch(mut self, epoch: Epoch) -> Self {
        self.epoch_0 = Some(epoch);
        self
    }

    /// Set the central body radius for event detection.
    pub fn with_body_radius(mut self, radius: f64) -> Self {
        self.body_radius = Some(radius);
        self
    }

    /// Get the inertia tensor.
    pub fn inertia(&self) -> &Matrix3<f64> {
        &self.inertia
    }

    /// Get the central body radius (if set).
    pub fn body_radius(&self) -> Option<f64> {
        self.body_radius
    }

    /// Names of active load models.
    pub fn load_names(&self) -> Vec<&str> {
        self.loads.iter().map(|l| l.name()).collect()
    }

    /// Per-model load breakdown at the given state.
    pub fn load_breakdown(&self, t: f64, state: &SpacecraftState) -> Vec<(&str, ExternalLoads)> {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));
        self.loads
            .iter()
            .map(|l| (l.name(), l.loads(t, state, epoch.as_ref())))
            .collect()
    }
}

impl<G: GravityField> DynamicalSystem for SpacecraftDynamics<G> {
    type State = SpacecraftState;

    fn derivatives(&self, t: f64, state: &SpacecraftState) -> SpacecraftState {
        let epoch = self.epoch_0.map(|e| e.add_seconds(t));

        // Gravitational acceleration
        let grav_accel = self.gravity.acceleration(self.mu, state.orbit.position());

        // Accumulate external loads
        let mut total = ExternalLoads::zeros();
        for load in &self.loads {
            total += load.loads(t, state, epoch.as_ref());
        }

        // Total translational acceleration
        let total_accel = grav_accel + total.acceleration_inertial;

        // Quaternion kinematics: dq/dt = ½ q ⊗ (0, ω)
        let q_dot = state.attitude.q_dot();

        // Euler's rotation equation: dω/dt = I⁻¹(τ − ω × (I·ω))
        let iw = self.inertia * state.attitude.angular_velocity;
        let alpha = self.inertia_inv
            * (total.torque_body - state.attitude.angular_velocity.cross(&iw));

        SpacecraftState::from_derivative(*state.orbit.velocity(), total_accel, q_dot, alpha, total.mass_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{Vector3, Vector4};
    use crate::attitude::{AttitudeState, TorqueModel};
    use orts_integrator::{Integrator, OdeState, Rk4};
    use crate::OrbitalState;
    use kaname::constants::MU_EARTH;
    use crate::gravity::PointMass;
    use crate::orbital_system::OrbitalSystem;
    use crate::perturbations::ForceModel;

    use super::super::{ForceModelAtCoM, TorqueModelOnly};

    // --- Helpers ---

    fn symmetric_inertia(i: f64) -> Matrix3<f64> {
        Matrix3::from_diagonal(&Vector3::new(i, i, i))
    }

    fn sample_orbit() -> OrbitalState {
        OrbitalState::new(
            Vector3::new(7000.0, 0.0, 0.0),
            Vector3::new(0.0, 7.5, 0.0),
        )
    }

    fn sample_spacecraft() -> SpacecraftState {
        SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    // --- Mock models ---

    struct ConstantForce(Vector3<f64>);

    impl ForceModel for ConstantForce {
        fn name(&self) -> &str {
            "const_force"
        }
        fn acceleration(
            &self,
            _t: f64,
            _state: &OrbitalState,
            _epoch: Option<&Epoch>,
        ) -> Vector3<f64> {
            self.0
        }
    }

    struct ConstantTorque(Vector3<f64>);

    impl TorqueModel for ConstantTorque {
        fn name(&self) -> &str {
            "const_torque"
        }
        fn torque(
            &self,
            _t: f64,
            _state: &AttitudeState,
            _epoch: Option<&Epoch>,
        ) -> Vector3<f64> {
            self.0
        }
    }

    /// Returns different loads depending on whether epoch is Some.
    struct EpochSensitiveLoad;

    impl LoadModel for EpochSensitiveLoad {
        fn name(&self) -> &str {
            "epoch_sensitive"
        }
        fn loads(
            &self,
            _t: f64,
            _state: &SpacecraftState,
            epoch: Option<&Epoch>,
        ) -> ExternalLoads {
            match epoch {
                Some(e) => ExternalLoads {
                    acceleration_inertial: Vector3::new(e.jd() * 1e-10, 0.0, 0.0),
                    torque_body: Vector3::zeros(),
                    mass_rate: 0.0,
                },
                None => ExternalLoads::zeros(),
            }
        }
    }

    // ======== Step 1: Gravity only (no LoadModel) ========

    #[test]
    fn gravity_only_matches_orbital_system() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let dyn_orb = OrbitalSystem::new(MU_EARTH, Box::new(PointMass));

        let d_sc = dyn_sc.derivatives(0.0, &sc);
        let d_orb = dyn_orb.derivatives(0.0, &sc.orbit);

        // Translational acceleration should match
        assert!((d_sc.orbit.velocity() - d_orb.velocity()).magnitude() < 1e-15);
    }

    #[test]
    fn gravity_only_velocity_derivative() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);

        // Position derivative = input velocity
        assert_eq!(*d.orbit.position(), *sc.orbit.velocity());
    }

    #[test]
    fn torque_free_symmetric_inertia() {
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            mass: 500.0,
        };
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);

        // Symmetric inertia + no torque → gyroscopic term vanishes → α = 0
        assert!(d.attitude.angular_velocity.magnitude() < 1e-15);
    }

    #[test]
    fn mass_rate_always_zero() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);
        assert_eq!(d.mass, 0.0);
    }

    // ======== Step 2: Euler equation ========

    #[test]
    fn euler_diagonal_inertia_known_torque() {
        // I = diag(10, 20, 30), ω = 0, τ = (1, 2, 3)
        // α = I⁻¹ τ = (0.1, 0.1, 0.1)
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let torque = Vector3::new(1.0, 2.0, 3.0);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_load(Box::new(TorqueModelOnly(Box::new(ConstantTorque(torque)))));

        let d = dyn_sc.derivatives(0.0, &sc);

        let expected_alpha = Vector3::new(0.1, 0.1, 0.1);
        assert!((d.attitude.angular_velocity - expected_alpha).magnitude() < 1e-14);
    }

    #[test]
    fn euler_gyroscopic_term() {
        // I = diag(10, 20, 30), ω = (1, 1, 0), no torque
        // Iω = (10, 20, 0)
        // ω × Iω = (1,1,0) × (10,20,0) = (0, 0, 1·20-1·10) = (0, 0, 10)
        // α = I⁻¹ (0 - (0,0,10)) = (0, 0, -10/30) = (0, 0, -1/3)
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 1.0, 0.0),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &sc);

        let expected_alpha = Vector3::new(0.0, 0.0, -1.0 / 3.0);
        assert!(
            (d.attitude.angular_velocity - expected_alpha).magnitude() < 1e-14,
            "Expected α = {expected_alpha:?}, got {:?}",
            d.attitude.angular_velocity
        );
    }

    #[test]
    fn euler_non_diagonal_inertia() {
        // I = [[4,1,0],[1,4,0],[0,0,6]], ω = (1, 0, 1), τ = 0
        // Iω = (4, 1, 6)
        // ω × Iω = (1,0,1) × (4,1,6) = (0·6-1·1, 1·4-1·6, 1·1-0·4) = (-1, -2, 1)
        // α = I⁻¹(0 - (-1,-2,1)) = I⁻¹(1, 2, -1)
        // I⁻¹ = (1/90)[[24,-6,0],[-6,24,0],[0,0,15]]
        // I⁻¹(1,2,-1) = (1/90)(24-12, -6+48, -15) = (12/90, 42/90, -15/90)
        //             = (2/15, 7/15, -1/6)
        let inertia = Matrix3::new(4.0, 1.0, 0.0, 1.0, 4.0, 0.0, 0.0, 0.0, 6.0);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 0.0, 1.0),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &sc);

        let expected_alpha = Vector3::new(2.0 / 15.0, 7.0 / 15.0, -1.0 / 6.0);
        assert!(
            (d.attitude.angular_velocity - expected_alpha).magnitude() < 1e-13,
            "Expected α = {expected_alpha:?}, got {:?}",
            d.attitude.angular_velocity
        );
    }

    // ======== Step 3: LoadModel integration ========

    #[test]
    fn force_adapter_adds_acceleration() {
        let accel = Vector3::new(1e-6, 2e-6, 3e-6);
        let sc = sample_spacecraft();

        let dyn_with = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(ForceModelAtCoM(Box::new(ConstantForce(accel)))));
        let d_with = dyn_with.derivatives(0.0, &sc);

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &sc);

        let diff = d_with.orbit.velocity() - d_grav.orbit.velocity();
        assert!((diff - accel).magnitude() < 1e-15);
    }

    #[test]
    fn torque_adapter_adds_torque() {
        let torque = Vector3::new(0.01, 0.02, 0.03);
        let sc = sample_spacecraft(); // ω = 0 → no gyroscopic term

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(TorqueModelOnly(Box::new(ConstantTorque(torque)))));

        let d = dyn_sc.derivatives(0.0, &sc);

        // α = I⁻¹ τ = τ / 10
        let expected_alpha = torque / 10.0;
        assert!((d.attitude.angular_velocity - expected_alpha).magnitude() < 1e-15);
    }

    #[test]
    fn multiple_loads_accumulate() {
        let accel = Vector3::new(1e-6, 0.0, 0.0);
        let torque = Vector3::new(0.0, 0.01, 0.0);
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(ForceModelAtCoM(Box::new(ConstantForce(accel)))))
            .with_load(Box::new(TorqueModelOnly(Box::new(ConstantTorque(torque)))));
        let d = dyn_sc.derivatives(0.0, &sc);

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &sc);

        // Force contribution
        let accel_diff = d.orbit.velocity() - d_grav.orbit.velocity();
        assert!((accel_diff - accel).magnitude() < 1e-15);

        // Torque contribution (ω = 0 → α = I⁻¹ τ = τ / 10)
        assert!((d.attitude.angular_velocity - torque / 10.0).magnitude() < 1e-15);
    }

    // ======== Step 4: Builder + telemetry ========

    #[test]
    fn builder_with_load_epoch_body_radius() {
        let epoch = Epoch::from_jd(2460000.5);
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(ForceModelAtCoM(Box::new(ConstantForce(
                Vector3::zeros(),
            )))))
            .with_epoch(epoch)
            .with_body_radius(6378.137);

        assert_eq!(dyn_sc.loads.len(), 1);
        assert_eq!(dyn_sc.epoch_0, Some(epoch));
        assert_eq!(dyn_sc.body_radius, Some(6378.137));
    }

    #[test]
    fn load_names_returns_all() {
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(ForceModelAtCoM(Box::new(ConstantForce(
                Vector3::zeros(),
            )))))
            .with_load(Box::new(TorqueModelOnly(Box::new(ConstantTorque(
                Vector3::zeros(),
            )))));

        let names = dyn_sc.load_names();
        assert_eq!(names, vec!["const_force", "const_torque"]);
    }

    #[test]
    fn load_breakdown_per_model() {
        let accel = Vector3::new(1e-6, 0.0, 0.0);
        let torque = Vector3::new(0.0, 0.01, 0.0);
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(ForceModelAtCoM(Box::new(ConstantForce(accel)))))
            .with_load(Box::new(TorqueModelOnly(Box::new(ConstantTorque(torque)))));

        let breakdown = dyn_sc.load_breakdown(0.0, &sc);
        assert_eq!(breakdown.len(), 2);
        assert_eq!(breakdown[0].0, "const_force");
        assert_eq!(breakdown[0].1.acceleration_inertial, accel);
        assert_eq!(breakdown[0].1.torque_body, Vector3::zeros());
        assert_eq!(breakdown[1].0, "const_torque");
        assert_eq!(breakdown[1].1.acceleration_inertial, Vector3::zeros());
        assert_eq!(breakdown[1].1.torque_body, torque);
    }

    // ======== Step 5: Epoch + integration + edge cases ========

    #[test]
    fn epoch_forwarded_to_loads() {
        let epoch = Epoch::from_jd(2460000.5);
        let t = 100.0;
        let sc = sample_spacecraft();

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(EpochSensitiveLoad))
            .with_epoch(epoch);

        let d = dyn_sc.derivatives(t, &sc);

        // EpochSensitiveLoad returns accel.x = epoch.jd() * 1e-10 when epoch is Some
        let expected_epoch = epoch.add_seconds(t);
        let expected_accel_x = expected_epoch.jd() * 1e-10;

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(t, &sc);
        let diff_x = d.orbit.velocity()[0] - d_grav.orbit.velocity()[0];

        let rel_err = (diff_x - expected_accel_x).abs() / expected_accel_x.abs();
        assert!(
            rel_err < 1e-14,
            "Epoch not forwarded correctly: diff_x={diff_x}, expected={expected_accel_x}, rel_err={rel_err:.3e}"
        );
    }

    #[test]
    fn epoch_none_no_panic() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0))
            .with_load(Box::new(EpochSensitiveLoad));

        // epoch_0 = None → loads get None → EpochSensitiveLoad returns zeros
        let d = dyn_sc.derivatives(0.0, &sc);

        let dyn_grav = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d_grav = dyn_grav.derivatives(0.0, &sc);

        assert!((d.orbit.velocity() - d_grav.orbit.velocity()).magnitude() < 1e-15);
    }

    #[test]
    fn integrable_with_rk4() {
        let sc = sample_spacecraft();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let result = Rk4.integrate(&dyn_sc, sc, 0.0, 60.0, 10.0, |_, _| {});

        assert!(result.orbit.position().magnitude() > 0.0);
        assert!(result.is_finite());
    }

    #[test]
    #[should_panic(expected = "Inertia tensor must be invertible")]
    fn singular_inertia_panics() {
        let _dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, Matrix3::zeros());
    }

    // ======== Step 6: Derivative-level conservation laws ========

    #[test]
    fn derivative_preserves_two_body_energy() {
        // dE/dt = v · a + (μ/r³)(r · v) = 0 for point-mass gravity
        let sc = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(7000.0, 1000.0, 500.0),
                Vector3::new(-1.0, 7.0, 0.5),
            ),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);

        let r = sc.orbit.position();
        let v = sc.orbit.velocity();
        let a = d.orbit.velocity(); // acceleration
        let r_mag = r.magnitude();

        let de_dt = v.dot(a) + MU_EARTH / (r_mag.powi(3)) * r.dot(v);
        assert!(
            de_dt.abs() < 1e-12,
            "dE/dt should be ≈ 0, got {de_dt:.3e}"
        );
    }

    #[test]
    fn derivative_preserves_angular_momentum() {
        // dL/dt = r × a = 0 for central gravity
        let sc = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(7000.0, 1000.0, 500.0),
                Vector3::new(-1.0, 7.0, 0.5),
            ),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);

        let r = sc.orbit.position();
        let a = d.orbit.velocity();
        let dl_dt = r.cross(a);

        assert!(
            dl_dt.magnitude() < 1e-12,
            "dL/dt should be ≈ 0, got magnitude {:.3e}",
            dl_dt.magnitude()
        );
    }

    #[test]
    fn derivative_preserves_rotational_energy() {
        // dT/dt = ω · I·α = 0 for torque-free system
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 20.0, 30.0));
        let omega = Vector3::new(0.1, 0.2, 0.3);
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: omega,
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia);
        let d = dyn_sc.derivatives(0.0, &sc);

        let alpha = &d.attitude.angular_velocity;
        let dt_rot = omega.dot(&(inertia * alpha));

        assert!(
            dt_rot.abs() < 1e-14,
            "dT_rot/dt should be ≈ 0, got {dt_rot:.3e}"
        );
    }

    #[test]
    fn derivative_preserves_quaternion_norm() {
        // d/dt(|q|²) = 2 q · q̇ = 0 (skew-symmetric kinematic matrix)
        let sc = SpacecraftState {
            orbit: sample_orbit(),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.3),
            },
            mass: 500.0,
        };

        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, symmetric_inertia(10.0));
        let d = dyn_sc.derivatives(0.0, &sc);

        let q = &sc.attitude.quaternion;
        let q_dot = &d.attitude.quaternion;
        let d_norm_sq = 2.0 * q.dot(q_dot);

        assert!(
            d_norm_sq.abs() < 1e-15,
            "d/dt(|q|²) should be ≈ 0, got {d_norm_sq:.3e}"
        );
    }
}
