use arika::epoch::Epoch;
use nalgebra::Vector3;

use crate::model::{HasAttitude, HasMass, HasOrbit, Model};

use super::{ExternalLoads, SpacecraftState};

/// Standard gravitational acceleration [m/s²].
pub const G0: f64 = 9.80665;

// ---------------------------------------------------------------------------
// ThrustProfile trait — control/scheduling layer
// ---------------------------------------------------------------------------

/// Determines the throttle level based on simulation state.
///
/// Separates firing logic (when/how much) from the thruster actuator
/// (force production, mass flow, torque).  Implement this trait to define
/// custom firing schedules, event-triggered burns, or feedback controllers.
pub trait ThrustProfile: Send + Sync {
    /// Returns throttle level in \[0.0, 1.0\].
    ///
    /// 0.0 = off, 1.0 = full thrust.  Values outside this range are clamped
    /// by the [`Thruster`] actuator.
    fn throttle(&self, t: f64, state: &SpacecraftState, epoch: Option<&Epoch>) -> f64;
}

// ---------------------------------------------------------------------------
// Profile implementations
// ---------------------------------------------------------------------------

/// Always returns a fixed throttle level.
pub struct ConstantThrottle(pub f64);

impl ThrustProfile for ConstantThrottle {
    fn throttle(&self, _t: f64, _state: &SpacecraftState, _epoch: Option<&Epoch>) -> f64 {
        self.0
    }
}

/// A time window during which the thruster fires at a specified throttle.
#[derive(Debug, Clone)]
pub struct BurnWindow {
    /// Start time [s] (inclusive).
    pub start: f64,
    /// End time [s] (exclusive).
    pub end: f64,
    /// Throttle level during this window \[0.0, 1.0\].
    pub throttle: f64,
}

impl BurnWindow {
    /// Create a full-throttle burn window.
    pub fn full(start: f64, end: f64) -> Self {
        Self {
            start,
            end,
            throttle: 1.0,
        }
    }
}

/// Fires during specified time windows, otherwise off.
pub struct ScheduledBurn {
    pub windows: Vec<BurnWindow>,
}

impl ThrustProfile for ScheduledBurn {
    fn throttle(&self, t: f64, _state: &SpacecraftState, _epoch: Option<&Epoch>) -> f64 {
        for w in &self.windows {
            if t >= w.start && t < w.end {
                return w.throttle;
            }
        }
        0.0
    }
}

// ---------------------------------------------------------------------------
// Thruster actuator + LoadModel
// ---------------------------------------------------------------------------

/// A thruster mounted on the spacecraft body.
///
/// Produces thrust force in a fixed body-frame direction, with optional
/// torque from centre-of-thrust offset, and mass depletion via Isp.
///
/// The firing logic is delegated to a [`ThrustProfile`], enabling clean
/// separation of actuator physics from control/scheduling concerns.
pub struct Thruster {
    /// Maximum thrust [N].
    thrust_n: f64,
    /// Specific impulse [s].
    isp_s: f64,
    /// Thrust direction in body frame (unit vector).
    direction_body: Vector3<f64>,
    /// Thruster position offset from CoM [m, body frame].
    offset_body: Vector3<f64>,
    /// Dry mass [kg] — failsafe floor (thrust ceases when mass ≤ dry_mass).
    dry_mass: f64,
    /// Control/scheduling logic.
    profile: Box<dyn ThrustProfile>,
}

impl Thruster {
    /// Create a thruster with the given maximum thrust, specific impulse, and
    /// body-frame direction.
    ///
    /// Defaults: offset = 0 (CoM), profile = full throttle, dry_mass = 0.
    ///
    /// # Panics
    /// Panics if `direction_body` is zero-length.
    pub fn new(thrust_n: f64, isp_s: f64, direction_body: Vector3<f64>) -> Self {
        let dir = direction_body.normalize();
        assert!(dir.magnitude() > 0.5, "Thrust direction must be non-zero");
        Self {
            thrust_n,
            isp_s,
            direction_body: dir,
            offset_body: Vector3::zeros(),
            dry_mass: 0.0,
            profile: Box::new(ConstantThrottle(1.0)),
        }
    }

    /// Set the thruster offset from the spacecraft centre of mass [m, body frame].
    pub fn with_offset(mut self, offset: Vector3<f64>) -> Self {
        self.offset_body = offset;
        self
    }

    /// Set the dry mass floor [kg].
    ///
    /// When `state.mass ≤ dry_mass`, the thruster produces zero output
    /// regardless of the profile.  This is a physical failsafe to prevent
    /// `F/m` singularity when propellant is exhausted.
    pub fn with_dry_mass(mut self, dry_mass: f64) -> Self {
        self.dry_mass = dry_mass;
        self
    }

    /// Set the thrust profile (control/scheduling logic).
    pub fn with_profile(mut self, profile: Box<dyn ThrustProfile>) -> Self {
        self.profile = profile;
        self
    }
}

impl Thruster {
    /// Compute thruster loads from SpacecraftState.
    pub(crate) fn loads(
        &self,
        t: f64,
        state: &SpacecraftState,
        epoch: Option<&Epoch>,
    ) -> ExternalLoads {
        // Failsafe: propellant exhausted
        if state.mass <= self.dry_mass {
            return ExternalLoads::zeros();
        }

        let throttle = self.profile.throttle(t, state, epoch).clamp(0.0, 1.0);
        if throttle < 1e-15 {
            return ExternalLoads::zeros();
        }

        // Force in body frame [N]
        let f_body_n = self.thrust_n * throttle * self.direction_body;

        // Torque from offset [N·m]
        let torque_body = self.offset_body.cross(&f_body_n);

        // Acceleration: body → inertial [km/s²]
        // F [N] / mass [kg] = [m/s²], / 1000 = [km/s²]
        let a_body = arika::frame::Vec3::from_raw(f_body_n / state.mass / 1000.0);
        let a_inertial = state.attitude.rotation_to_eci().transform(&a_body);

        // Mass flow rate [kg/s]
        let mass_rate = -(self.thrust_n * throttle) / (self.isp_s * G0);

        ExternalLoads {
            acceleration_inertial: a_inertial,
            torque_body: arika::frame::Vec3::from_raw(torque_body),
            mass_rate,
        }
    }
}

impl<S: HasAttitude + HasOrbit<Frame = arika::frame::SimpleEci> + HasMass> Model<S> for Thruster {
    fn name(&self) -> &str {
        "thruster"
    }

    fn eval(&self, t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        // Construct SpacecraftState from capabilities for ThrustProfile compatibility
        let sc_state = SpacecraftState {
            orbit: state.orbit().clone(),
            attitude: state.attitude().clone(),
            mass: state.mass(),
        };
        self.loads(t, &sc_state, epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;
    use std::f64::consts::FRAC_PI_2;

    fn sample_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    fn state_with_mass(mass: f64) -> SpacecraftState {
        SpacecraftState {
            mass,
            ..sample_state()
        }
    }

    // Quaternion for 90° rotation about Z: body +X → inertial +Y
    fn rotated_90z_state() -> SpacecraftState {
        let half = FRAC_PI_2 / 2.0;
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(half.cos(), 0.0, 0.0, half.sin()),
                angular_velocity: Vector3::zeros(),
            },
            mass: 500.0,
        }
    }

    // ======== ThrustProfile tests ========

    #[test]
    fn constant_throttle_value() {
        let p = ConstantThrottle(0.7);
        assert!((p.throttle(0.0, &sample_state(), None) - 0.7).abs() < 1e-15);
    }

    #[test]
    fn scheduled_burn_inside_window() {
        let p = ScheduledBurn {
            windows: vec![BurnWindow {
                start: 10.0,
                end: 20.0,
                throttle: 0.8,
            }],
        };
        assert!((p.throttle(15.0, &sample_state(), None) - 0.8).abs() < 1e-15);
    }

    #[test]
    fn scheduled_burn_outside_window() {
        let p = ScheduledBurn {
            windows: vec![BurnWindow::full(10.0, 20.0)],
        };
        assert_eq!(p.throttle(5.0, &sample_state(), None), 0.0);
        assert_eq!(p.throttle(25.0, &sample_state(), None), 0.0);
    }

    #[test]
    fn scheduled_burn_boundary() {
        let p = ScheduledBurn {
            windows: vec![BurnWindow::full(10.0, 20.0)],
        };
        // start is inclusive
        assert_eq!(p.throttle(10.0, &sample_state(), None), 1.0);
        // end is exclusive
        assert_eq!(p.throttle(20.0, &sample_state(), None), 0.0);
    }

    #[test]
    fn scheduled_burn_multiple_windows() {
        let p = ScheduledBurn {
            windows: vec![
                BurnWindow {
                    start: 0.0,
                    end: 5.0,
                    throttle: 0.5,
                },
                BurnWindow::full(10.0, 15.0),
            ],
        };
        assert!((p.throttle(3.0, &sample_state(), None) - 0.5).abs() < 1e-15);
        assert_eq!(p.throttle(12.0, &sample_state(), None), 1.0);
        assert_eq!(p.throttle(7.0, &sample_state(), None), 0.0);
    }

    // ======== Thruster construction tests ========

    #[test]
    fn new_normalizes_direction() {
        let t = Thruster::new(1.0, 300.0, Vector3::new(3.0, 0.0, 0.0));
        assert!((t.direction_body - Vector3::new(1.0, 0.0, 0.0)).magnitude() < 1e-15);
    }

    #[test]
    #[should_panic(expected = "Thrust direction must be non-zero")]
    fn new_zero_direction_panics() {
        let _t = Thruster::new(1.0, 300.0, Vector3::zeros());
    }

    #[test]
    fn default_profile_full_throttle() {
        let t = Thruster::new(1.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &sample_state(), None);
        // Should fire (default is ConstantThrottle(1.0))
        assert!(loads.acceleration_inertial.magnitude() > 0.0);
    }

    #[test]
    fn builder_with_offset_profile_dry_mass() {
        let t = Thruster::new(10.0, 300.0, Vector3::x())
            .with_offset(Vector3::new(0.0, 1.0, 0.0))
            .with_dry_mass(100.0)
            .with_profile(Box::new(ConstantThrottle(0.5)));
        assert_eq!(t.offset_body, Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(t.dry_mass, 100.0);
    }

    // ======== LoadModel tests (analytical) ========

    #[test]
    fn acceleration_magnitude() {
        // 1 N on 500 kg: a = 1/(500*1000) = 2e-6 km/s²
        let t = Thruster::new(1.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &sample_state(), None);
        let expected = 1.0 / (500.0 * 1000.0);
        assert!(
            (loads.acceleration_inertial.magnitude() - expected).abs() < 1e-15,
            "got {}, expected {}",
            loads.acceleration_inertial.magnitude(),
            expected
        );
    }

    #[test]
    fn acceleration_direction_identity() {
        // Identity attitude: body +X = inertial +X
        let t = Thruster::new(1.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &sample_state(), None);
        let a = loads.acceleration_inertial.into_inner();
        assert!(a[0] > 0.0);
        assert!(a[1].abs() < 1e-15);
        assert!(a[2].abs() < 1e-15);
    }

    #[test]
    fn acceleration_direction_rotated_90z() {
        // 90° about Z: body +X → inertial +Y
        let t = Thruster::new(1.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &rotated_90z_state(), None);
        let a = loads.acceleration_inertial.into_inner();
        assert!(a[0].abs() < 1e-10, "expected ~0 x-component, got {}", a[0]);
        assert!(a[1] > 0.0, "expected positive y-component, got {}", a[1]);
        assert!(a[2].abs() < 1e-15);
    }

    #[test]
    fn torque_from_offset() {
        // Offset [0, 1, 0] m, force along +X: τ = [0,1,0] × [F,0,0] = [0,0,-F]
        let thrust = 10.0;
        let t = Thruster::new(thrust, 300.0, Vector3::x()).with_offset(Vector3::new(0.0, 1.0, 0.0));
        let loads = t.loads(0.0, &sample_state(), None);
        let tb = loads.torque_body.into_inner();
        assert!((tb[0]).abs() < 1e-15, "τx should be 0");
        assert!((tb[1]).abs() < 1e-15, "τy should be 0");
        assert!(
            (tb[2] - (-thrust)).abs() < 1e-12,
            "τz should be -F={}, got {}",
            -thrust,
            tb[2]
        );
    }

    #[test]
    fn torque_zero_at_com() {
        let t = Thruster::new(10.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &sample_state(), None);
        assert!(loads.torque_body.magnitude() < 1e-15);
    }

    #[test]
    fn mass_rate_value() {
        // F=1N, Isp=300s: dm/dt = -1/(300*9.80665) = -3.4038e-4 kg/s
        let t = Thruster::new(1.0, 300.0, Vector3::x());
        let loads = t.loads(0.0, &sample_state(), None);
        let expected = -1.0 / (300.0 * G0);
        assert!(
            (loads.mass_rate - expected).abs() < 1e-12,
            "got {}, expected {}",
            loads.mass_rate,
            expected
        );
    }

    #[test]
    fn zero_when_not_firing() {
        let t = Thruster::new(1.0, 300.0, Vector3::x()).with_profile(Box::new(ScheduledBurn {
            windows: vec![BurnWindow::full(100.0, 200.0)],
        }));
        let loads = t.loads(0.0, &sample_state(), None);
        assert_eq!(loads.acceleration_inertial, arika::frame::Vec3::zeros());
        assert_eq!(loads.torque_body, arika::frame::Vec3::zeros());
        assert_eq!(loads.mass_rate, 0.0);
    }

    #[test]
    fn zero_when_propellant_exhausted() {
        // mass=100, dry_mass=100 → should not fire
        let t = Thruster::new(1.0, 300.0, Vector3::x()).with_dry_mass(100.0);
        let loads = t.loads(0.0, &state_with_mass(100.0), None);
        assert_eq!(loads.mass_rate, 0.0);
        assert_eq!(loads.acceleration_inertial, arika::frame::Vec3::zeros());
    }

    #[test]
    fn throttle_clamped_above_one() {
        let t =
            Thruster::new(1.0, 300.0, Vector3::x()).with_profile(Box::new(ConstantThrottle(1.5)));
        let loads = t.loads(0.0, &sample_state(), None);
        // Should be clamped to 1.0: same as full throttle
        let t_full = Thruster::new(1.0, 300.0, Vector3::x());
        let loads_full = t_full.loads(0.0, &sample_state(), None);
        assert!(
            (loads.acceleration_inertial - loads_full.acceleration_inertial).magnitude() < 1e-15
        );
        assert!((loads.mass_rate - loads_full.mass_rate).abs() < 1e-15);
    }

    #[test]
    fn partial_throttle() {
        let t_full = Thruster::new(10.0, 300.0, Vector3::x());
        let t_half =
            Thruster::new(10.0, 300.0, Vector3::x()).with_profile(Box::new(ConstantThrottle(0.5)));
        let state = sample_state();
        let loads_full = t_full.loads(0.0, &state, None);
        let loads_half = t_half.loads(0.0, &state, None);

        // Half throttle → half acceleration, half mass_rate
        assert!(
            (loads_half.acceleration_inertial - loads_full.acceleration_inertial * 0.5).magnitude()
                < 1e-15
        );
        assert!((loads_half.mass_rate - loads_full.mass_rate * 0.5).abs() < 1e-15);
    }

    // ======== Integration test: dynamics uses mass_rate ========

    #[test]
    fn dynamics_uses_mass_rate() {
        use crate::orbital::gravity::PointMass;
        use arika::earth::MU as MU_EARTH;
        use nalgebra::Matrix3;
        use utsuroi::DynamicalSystem;

        use super::super::SpacecraftDynamics;

        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_model(Thruster::new(10.0, 300.0, Vector3::x()));

        let state = sample_state();
        let d = dyn_sc.derivatives(0.0, &state.into());

        // mass derivative should be negative (propellant consumption)
        assert!(
            d.plant.mass < 0.0,
            "mass_rate should be negative, got {}",
            d.plant.mass
        );
        let expected = -10.0 / (300.0 * G0);
        assert!(
            (d.plant.mass - expected).abs() < 1e-12,
            "got {}, expected {}",
            d.plant.mass,
            expected
        );
    }
}
