//! Magnetorquer (MTQ) assembly as a [`Model`].
//!
//! Models a set of magnetic torquers, each with its own axis and maximum
//! dipole moment. The assembly converts per-MTQ commanded moments into
//! a combined magnetic torque on the spacecraft body.
//!
//! The magnetic torque follows `τ = m × B` where `m` is the total
//! realized dipole moment vector and `B` is the local geomagnetic
//! field in the body frame.

use arika::epoch::Epoch;
use arika::frame::{self, Vec3};
use nalgebra::Vector3;
use tobari::magnetic::MagneticFieldModel;

use crate::magnetic;
use crate::model::{ExternalLoads, HasAttitude, HasOrbit, Model};

/// A single magnetic torquer with physical limits.
#[derive(Debug, Clone)]
pub struct Mtq {
    /// Axis in body frame (unit vector, normalized on construction).
    axis: Vector3<f64>,
    /// Maximum dipole moment magnitude [A·m²].
    pub max_moment: f64,
}

impl Mtq {
    /// Create a magnetic torquer with the given axis (will be normalized).
    ///
    /// # Panics
    /// Panics if `axis` is zero-length or `max_moment` is negative.
    pub fn new(axis: Vector3<f64>, max_moment: f64) -> Self {
        let norm = axis.magnitude();
        assert!(norm > 1e-15, "MTQ axis must be non-zero");
        assert!(
            max_moment >= 0.0,
            "max_moment must be non-negative, got {max_moment}"
        );
        Self {
            axis: axis / norm,
            max_moment,
        }
    }

    /// Get the axis unit vector.
    pub fn axis(&self) -> &Vector3<f64> {
        &self.axis
    }
}

/// MTQ assembly geometry and constraint logic (no magnetic field model).
///
/// This core struct handles per-MTQ clamping, moment allocation, and
/// torque computation without depending on any environment model.
/// It is designed to be unit-tested independently.
///
/// Allocation uses a precomputed pseudo-inverse matrix, supporting
/// non-orthogonal MTQ arrangements (e.g., skewed 4-coil).
#[derive(Debug, Clone)]
pub struct MtqAssemblyCore {
    mtqs: Vec<Mtq>,
    /// Allocation matrix (pseudo-inverse of axis matrix), `n×3`.
    alloc_pinv: nalgebra::DMatrix<f64>,
}

impl MtqAssemblyCore {
    /// Create an assembly from a list of MTQs.
    pub fn new(mtqs: Vec<Mtq>) -> Self {
        let axes: Vec<_> = mtqs.iter().map(|m| *m.axis()).collect();
        let alloc_pinv = super::reaction_wheel::build_allocation_pinv(&axes);
        Self { mtqs, alloc_pinv }
    }

    /// Standard 3-axis orthogonal arrangement with identical MTQs.
    pub fn three_axis(max_moment: f64) -> Self {
        Self::new(vec![
            Mtq::new(Vector3::x(), max_moment),
            Mtq::new(Vector3::y(), max_moment),
            Mtq::new(Vector3::z(), max_moment),
        ])
    }

    /// Access the MTQs.
    pub fn mtqs(&self) -> &[Mtq] {
        &self.mtqs
    }

    /// Number of MTQs in the assembly.
    pub fn num_mtqs(&self) -> usize {
        self.mtqs.len()
    }

    /// Clamp per-MTQ commanded moments and compute the realized total
    /// dipole moment vector in the body frame.
    ///
    /// Each commanded moment is clamped to `[-max_moment, max_moment]`
    /// for the corresponding MTQ, then projected onto its axis.
    ///
    /// # Panics
    /// Panics if `commanded.len() != self.num_mtqs()`.
    pub fn realized_moment(&self, commanded: &[f64]) -> Vector3<f64> {
        assert_eq!(
            commanded.len(),
            self.mtqs.len(),
            "commanded moments length ({}) != MTQ count ({})",
            commanded.len(),
            self.mtqs.len()
        );
        let mut total = Vector3::zeros();
        for (mtq, &cmd) in self.mtqs.iter().zip(commanded.iter()) {
            let clamped = cmd.clamp(-mtq.max_moment, mtq.max_moment);
            total += clamped * mtq.axis;
        }
        total
    }

    /// Compute the magnetic torque from per-MTQ commanded moments and
    /// the local magnetic field in the body frame.
    ///
    /// `τ = m_total × B_body`
    ///
    /// # Panics
    /// Panics if `commanded.len() != self.num_mtqs()`.
    pub fn torque(&self, commanded: &[f64], b_body: &Vector3<f64>) -> Vector3<f64> {
        self.realized_moment(commanded).cross(b_body)
    }

    /// Allocate a desired body-frame moment to per-MTQ moments.
    ///
    /// Uses the precomputed pseudo-inverse of the axis matrix for
    /// correct allocation in non-orthogonal layouts. Results are
    /// clamped to each MTQ's `[-max_moment, max_moment]`.
    ///
    /// When underactuated (fewer axes than 3), the least-squares
    /// approximation is returned (unrealizable components are dropped).
    pub fn allocate(&self, desired: &Vector3<f64>) -> Vec<f64> {
        // MTQ: u = pinv * desired (direct, no sign flip)
        let d = nalgebra::DVector::from_column_slice(desired.as_slice());
        let result = &self.alloc_pinv * d;
        // Clamp to per-MTQ limits
        result
            .iter()
            .zip(self.mtqs.iter())
            .map(|(&u, mtq)| u.clamp(-mtq.max_moment, mtq.max_moment))
            .collect()
    }
}

/// Per-MTQ command. Re-exported from `crate::plugin::command::MtqCommand` for convenience.
pub use crate::plugin::command::MtqCommand;

/// MTQ assembly with magnetic field model, usable as a [`Model<S>`]
/// in the ODE system.
///
/// The `command` field is `pub` so it can be updated between
/// integration segments (zero-order hold, set by plugin or host controller).
#[derive(Clone)]
pub struct MtqAssembly<F: MagneticFieldModel> {
    core: MtqAssemblyCore,
    /// Per-MTQ command (direct moments or normalized), updated between ODE segments.
    pub command: MtqCommand,
    /// Geomagnetic field model.
    field: F,
}

impl<F: MagneticFieldModel> MtqAssembly<F> {
    /// Create an assembly from a core and field model.
    pub fn new(core: MtqAssemblyCore, field: F) -> Self {
        let n = core.num_mtqs();
        Self {
            core,
            command: MtqCommand::Moments(vec![0.0; n]),
            field,
        }
    }

    /// Standard 3-axis orthogonal arrangement.
    pub fn three_axis(max_moment: f64, field: F) -> Self {
        Self::new(MtqAssemblyCore::three_axis(max_moment), field)
    }

    /// Access the core (geometry + constraint logic).
    pub fn core(&self) -> &MtqAssemblyCore {
        &self.core
    }

    /// Resolve the current command into per-MTQ dipole moments [A·m²].
    ///
    /// `MtqCommand::Moments` is used directly; `MtqCommand::NormalizedMoments`
    /// is clamped to `[-1, 1]` per element and scaled by each MTQ's
    /// `max_moment`.
    ///
    /// # Panics
    /// Panics if the command length does not match the number of MTQs.
    fn resolved_moments(&self) -> Vec<f64> {
        match &self.command {
            MtqCommand::Moments(m) => {
                assert_eq!(
                    m.len(),
                    self.core.num_mtqs(),
                    "MtqCommand::Moments length ({}) != MTQ count ({})",
                    m.len(),
                    self.core.num_mtqs()
                );
                m.clone()
            }
            MtqCommand::NormalizedMoments(u) => {
                assert_eq!(
                    u.len(),
                    self.core.num_mtqs(),
                    "MtqCommand::NormalizedMoments length ({}) != MTQ count ({})",
                    u.len(),
                    self.core.num_mtqs()
                );
                u.iter()
                    .zip(self.core.mtqs())
                    .map(|(&u, mtq)| u.clamp(-1.0, 1.0) * mtq.max_moment)
                    .collect()
            }
        }
    }
}

// TODO: Same SimpleEci constraint as CommandedMagnetorquer (magnetic::field_eci).
impl<F: MagneticFieldModel, S: HasAttitude + HasOrbit<Frame = frame::SimpleEci>> Model<S>
    for MtqAssembly<F>
{
    fn name(&self) -> &str {
        "mtq_assembly"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };
        let b_eci =
            magnetic::field_eci(&self.field, &state.orbit().position_eci(), epoch).into_inner();
        if b_eci.magnitude() < 1e-30 {
            return ExternalLoads::zeros();
        }
        let b_body = state
            .attitude()
            .rotation_to_body()
            .transform(&Vec3::<frame::SimpleEci>::from_raw(b_eci))
            .into_inner();
        let moments = self.resolved_moments();
        ExternalLoads::torque(self.core.torque(&moments, &b_body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use arika::epoch::Epoch;
    use arika::frame::Vec3 as FrameVec3;
    use nalgebra::Vector4;
    use tobari::magnetic::TiltedDipole;

    fn test_epoch() -> Epoch {
        Epoch::j2000()
    }

    struct TestState {
        attitude: AttitudeState,
        orbit: OrbitalState,
    }

    impl HasAttitude for TestState {
        fn attitude(&self) -> &AttitudeState {
            &self.attitude
        }
    }

    impl HasOrbit for TestState {
        type Frame = frame::SimpleEci;
        fn orbit(&self) -> &OrbitalState<frame::SimpleEci> {
            &self.orbit
        }
    }

    // ── Core tests ──

    #[test]
    fn three_axis_creates_three_mtqs() {
        let core = MtqAssemblyCore::three_axis(1.0);
        assert_eq!(core.num_mtqs(), 3);
    }

    #[test]
    fn zero_command_gives_zero_moment() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let m = core.realized_moment(&[0.0, 0.0, 0.0]);
        assert!(m.magnitude() < 1e-15);
    }

    #[test]
    fn zero_command_gives_zero_torque() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let b = Vector3::new(1e-5, 2e-5, 3e-5);
        let tau = core.torque(&[0.0, 0.0, 0.0], &b);
        assert!(tau.magnitude() < 1e-30);
    }

    #[test]
    fn single_axis_moment() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let m = core.realized_moment(&[0.5, 0.0, 0.0]);
        assert!((m.x - 0.5).abs() < 1e-15);
        assert!(m.y.abs() < 1e-15);
        assert!(m.z.abs() < 1e-15);
    }

    #[test]
    fn clamping() {
        let core = MtqAssemblyCore::three_axis(0.5);
        // Command exceeds max
        let m = core.realized_moment(&[10.0, -10.0, 0.3]);
        assert!((m.x - 0.5).abs() < 1e-15);
        assert!((m.y - (-0.5)).abs() < 1e-15);
        assert!((m.z - 0.3).abs() < 1e-15);
    }

    #[test]
    fn torque_is_m_cross_b() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let b = Vector3::new(0.0, 0.0, 1e-5);
        // m = [1, 0, 0], B = [0, 0, 1e-5] → τ = m × B = [0, -1e-5, 0]
        let tau = core.torque(&[1.0, 0.0, 0.0], &b);
        assert!(tau.x.abs() < 1e-20);
        assert!((tau.y - (-1e-5)).abs() < 1e-20);
        assert!(tau.z.abs() < 1e-20);
    }

    #[test]
    fn allocate_orthogonal() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let desired = Vector3::new(0.3, -0.5, 0.7);
        let allocated = core.allocate(&desired);
        assert_eq!(allocated.len(), 3);
        assert!((allocated[0] - 0.3).abs() < 1e-15);
        assert!((allocated[1] - (-0.5)).abs() < 1e-15);
        assert!((allocated[2] - 0.7).abs() < 1e-15);
    }

    #[test]
    fn allocate_clamps() {
        let core = MtqAssemblyCore::three_axis(0.5);
        let desired = Vector3::new(10.0, -10.0, 0.3);
        let allocated = core.allocate(&desired);
        assert!((allocated[0] - 0.5).abs() < 1e-15);
        assert!((allocated[1] - (-0.5)).abs() < 1e-15);
        assert!((allocated[2] - 0.3).abs() < 1e-15);
    }

    #[test]
    fn allocate_roundtrip_orthogonal() {
        let core = MtqAssemblyCore::three_axis(1.0);
        let desired = Vector3::new(0.3, -0.5, 0.7);
        let allocated = core.allocate(&desired);
        let realized = core.realized_moment(&allocated);
        assert!(
            (realized - desired).magnitude() < 1e-12,
            "roundtrip error: {:.3e}",
            (realized - desired).magnitude()
        );
    }

    #[test]
    fn allocate_skewed_4mtq_roundtrip() {
        // 4 MTQs in a skewed configuration (overactuated)
        let angle = std::f64::consts::FRAC_PI_4;
        let sin = angle.sin();
        let cos = angle.cos();
        let core = MtqAssemblyCore::new(vec![
            Mtq::new(Vector3::new(sin, 0.0, cos), 1.0),
            Mtq::new(Vector3::new(0.0, sin, cos), 1.0),
            Mtq::new(Vector3::new(-sin, 0.0, cos), 1.0),
            Mtq::new(Vector3::new(0.0, -sin, cos), 1.0),
        ]);
        let desired = Vector3::new(0.1, -0.2, 0.3);
        let allocated = core.allocate(&desired);
        assert_eq!(allocated.len(), 4);
        let realized = core.realized_moment(&allocated);
        assert!(
            (realized - desired).magnitude() < 1e-12,
            "skewed 4-MTQ roundtrip error: {:.3e}",
            (realized - desired).magnitude()
        );
    }

    #[test]
    fn allocate_2axis_mtq_drops_unrealizable() {
        // 2 MTQs on X/Y only (underactuated)
        let core = MtqAssemblyCore::new(vec![
            Mtq::new(Vector3::x(), 1.0),
            Mtq::new(Vector3::y(), 1.0),
        ]);
        let desired = Vector3::new(0.3, -0.5, 0.7);
        let allocated = core.allocate(&desired);
        assert_eq!(allocated.len(), 2);
        let realized = core.realized_moment(&allocated);
        // X and Y realized, Z dropped
        assert!((realized.x - 0.3).abs() < 1e-12);
        assert!((realized.y - (-0.5)).abs() < 1e-12);
        assert!(realized.z.abs() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "MTQ axis must be non-zero")]
    fn zero_axis_panics() {
        Mtq::new(Vector3::zeros(), 1.0);
    }

    #[test]
    #[should_panic(expected = "max_moment must be non-negative")]
    fn negative_max_moment_panics() {
        Mtq::new(Vector3::x(), -1.0);
    }

    #[test]
    #[should_panic(expected = "commanded moments length")]
    fn wrong_length_panics() {
        let core = MtqAssemblyCore::three_axis(1.0);
        core.realized_moment(&[0.0, 0.0]); // 2 instead of 3
    }

    // ── Assembly (Model) tests ──

    #[test]
    fn assembly_zero_command_zero_torque() {
        let assembly = MtqAssembly::three_axis(1.0, TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = assembly.eval(0.0, &state, Some(&epoch));
        assert!(loads.torque_body.magnitude() < 1e-20);
    }

    #[test]
    fn assembly_nonzero_command_produces_torque() {
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![1.0, 0.0, 0.0]);
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = assembly.eval(0.0, &state, Some(&epoch));
        // With a non-zero magnetic field and non-zero moment, torque should be non-zero
        assert!(loads.torque_body.magnitude() > 1e-15);
    }

    #[test]
    fn assembly_no_epoch_returns_zero() {
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![1.0, 1.0, 1.0]);
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let loads = assembly.eval(0.0, &state, None);
        assert!(loads.torque_body.magnitude() < 1e-30);
    }

    #[test]
    fn assembly_no_acceleration_or_mass_rate() {
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![1.0, 0.5, -0.3]);
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = assembly.eval(0.0, &state, Some(&epoch));
        assert!(loads.acceleration_inertial.magnitude() < 1e-15);
        assert!(loads.mass_rate.abs() < 1e-15);
    }

    #[test]
    fn assembly_clamping_bounds_torque() {
        let max_m = 0.001;
        let mut assembly = MtqAssembly::three_axis(max_m, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![100.0, 100.0, 100.0]);
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = assembly.eval(0.0, &state, Some(&epoch));

        // Torque should be bounded by clamped moment magnitude × B magnitude
        let b = magnetic::field_eci(
            &TiltedDipole::earth(),
            &FrameVec3::<frame::SimpleEci>::new(7000.0, 0.0, 0.0),
            &epoch,
        )
        .magnitude();
        let max_torque = 3.0_f64.sqrt() * max_m * b;
        assert!(
            loads.torque_body.magnitude() <= max_torque * 1.01,
            "Torque should be bounded: |tau|={:.6e}, bound={max_torque:.6e}",
            loads.torque_body.magnitude()
        );
    }

    #[test]
    fn assembly_moments_variant_produces_same_result() {
        // Sanity: MtqCommand::Moments([0.5, 0, 0]) should match the
        // pre-variant behaviour of setting commanded_moments directly.
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![0.5, 0.0, 0.0]);
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads_moments = assembly.eval(0.0, &state, Some(&epoch));

        // Also compute expected value directly from the core
        let b_eci = magnetic::field_eci(
            &TiltedDipole::earth(),
            &FrameVec3::<frame::SimpleEci>::new(7000.0, 0.0, 0.0),
            &epoch,
        )
        .into_inner();
        let b_body = state
            .attitude
            .rotation_to_body()
            .transform(&FrameVec3::<frame::SimpleEci>::from_raw(b_eci))
            .into_inner();
        let expected = assembly.core.torque(&[0.5, 0.0, 0.0], &b_body);
        let got = loads_moments.torque_body.into_inner();
        assert!((got - expected).magnitude() < 1e-20);
    }

    #[test]
    fn assembly_normalized_moments_scales_by_max_moment() {
        // NormalizedMoments([0.5, 0, 0]) with max_moment=10.0
        // should produce the same torque as Moments([5.0, 0, 0]).
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();

        let mut assembly_norm = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly_norm.command = MtqCommand::NormalizedMoments(vec![0.5, 0.0, 0.0]);
        let loads_norm = assembly_norm.eval(0.0, &state, Some(&epoch));

        let mut assembly_direct = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly_direct.command = MtqCommand::Moments(vec![5.0, 0.0, 0.0]);
        let loads_direct = assembly_direct.eval(0.0, &state, Some(&epoch));

        let diff = loads_norm.torque_body.into_inner() - loads_direct.torque_body.into_inner();
        assert!(
            diff.magnitude() < 1e-18,
            "normalized and direct should match: norm={:?}, direct={:?}",
            loads_norm.torque_body,
            loads_direct.torque_body
        );
    }

    #[test]
    fn assembly_normalized_clamps_to_unit() {
        // NormalizedMoments with values outside [-1, 1] should be clamped.
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();

        let mut assembly_clamped = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly_clamped.command = MtqCommand::NormalizedMoments(vec![100.0, -100.0, 0.3]);
        let loads_clamped = assembly_clamped.eval(0.0, &state, Some(&epoch));

        let mut assembly_ref = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        // 100 clamped to 1, -100 clamped to -1, 0.3 unchanged, each × 10.0
        assembly_ref.command = MtqCommand::Moments(vec![10.0, -10.0, 3.0]);
        let loads_ref = assembly_ref.eval(0.0, &state, Some(&epoch));

        let diff = loads_clamped.torque_body.into_inner() - loads_ref.torque_body.into_inner();
        assert!(diff.magnitude() < 1e-18);
    }

    #[test]
    #[should_panic(expected = "MtqCommand::Moments length")]
    fn assembly_moments_wrong_length_panics() {
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::Moments(vec![1.0, 0.0]); // 2 instead of 3
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let _ = assembly.eval(0.0, &state, Some(&epoch));
    }

    #[test]
    #[should_panic(expected = "MtqCommand::NormalizedMoments length")]
    fn assembly_normalized_moments_wrong_length_panics() {
        let mut assembly = MtqAssembly::three_axis(10.0, TiltedDipole::earth());
        assembly.command = MtqCommand::NormalizedMoments(vec![0.5, 0.5]); // 2 instead of 3
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let _ = assembly.eval(0.0, &state, Some(&epoch));
    }
}
