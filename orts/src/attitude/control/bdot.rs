use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::Epoch;
use arika::frame;
use nalgebra::Vector3;
use tobari::magnetic::{MagneticFieldModel, TiltedDipole};

use crate::OrbitalState;
use crate::attitude::AttitudeState;
use crate::control::DiscreteController;
use crate::magnetic;
use crate::model::ExternalLoads;
use crate::model::{HasAttitude, HasFrame, HasOrbit, Model};
use crate::spacecraft::MtqAssemblyCore;

/// B-dot detumbling controller using cross-product dB/dt estimation.
///
/// Estimates the time-derivative of the magnetic field in the body frame as
/// dB_body/dt = -omega x B_body (valid when |omega| >> orbital angular rate),
/// then commands a magnetic moment m = -k * dB/dt to dissipate rotational
/// energy.
///
/// The resulting torque tau = m x B always opposes the component of angular
/// velocity perpendicular to the local magnetic field (provable via
/// Cauchy-Schwarz: omega . tau <= 0).
///
/// Uses [`MtqAssemblyCore`] for per-MTQ allocation and clamping, ensuring
/// consistency with the plugin-controlled `MtqAssembly` path.
///
/// When no epoch is available, returns zero loads (magnetic field models
/// require epoch for ECEF↔ECI rotation and secular variation).
///
/// `Fr` is the inertial frame the geomagnetic field is evaluated in
/// (`SimpleEci` = ERA-only ECEF rotation, `Gcrs` = full IAU 2006 chain, which
/// needs an EOP provider).
pub struct BdotCross<
    F: MagneticFieldModel = TiltedDipole,
    Fr: EarthFixedTransform = frame::SimpleEci,
> {
    /// Gain k > 0  [A*m^2*s/(rad*T)]
    gain: f64,
    /// MTQ assembly core for allocation + clamping.
    mtq: MtqAssemblyCore,
    /// Geomagnetic field model
    field: F,
    /// EOP storage for the frame adapter. `()` for `SimpleEci`.
    eop: Fr::EopStorage,
}

impl<F: MagneticFieldModel> BdotCross<F, frame::SimpleEci> {
    /// Create a new B-dot detumbler with custom field model, in the default
    /// `SimpleEci` frame.
    ///
    /// `max_moment` is per-axis maximum [A·m²] for a 3-axis MTQ.
    ///
    /// # Panics
    /// Panics if `gain` is negative or any component of `max_moment` is negative.
    pub fn new(gain: f64, max_moment: Vector3<f64>, field: F) -> Self {
        Self::new_in_frame(gain, max_moment, field, ())
    }
}

impl<F: MagneticFieldModel, Fr: EarthFixedTransform> BdotCross<F, Fr> {
    /// Create a B-dot detumbler that evaluates the field in an arbitrary
    /// inertial frame `Fr`, with that frame's EOP storage (`()` for `SimpleEci`).
    ///
    /// # Panics
    /// Panics if `gain` is negative or any component of `max_moment` is negative.
    pub fn new_in_frame(
        gain: f64,
        max_moment: Vector3<f64>,
        field: F,
        eop: Fr::EopStorage,
    ) -> Self {
        assert!(gain >= 0.0, "gain must be non-negative, got {gain}");
        assert!(
            max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0,
            "max_moment must be non-negative, got {max_moment:?}"
        );
        use crate::spacecraft::Mtq;
        let mtq = MtqAssemblyCore::new(vec![
            Mtq::new(Vector3::x(), max_moment[0]),
            Mtq::new(Vector3::y(), max_moment[1]),
            Mtq::new(Vector3::z(), max_moment[2]),
        ]);
        Self {
            gain,
            mtq,
            field,
            eop,
        }
    }
}

// Frame-generic: the field is evaluated in the state's own inertial frame `Fr`
// via `magnetic::field_inertial` and rotated to the body frame with the matching
// `Fr → Body` rotation. A frame without an `EarthFixedTransform` impl is
// rejected at compile time. See #151.
impl<
    F: MagneticFieldModel,
    Fr: EarthFixedTransform,
    S: HasFrame<Frame = Fr> + HasAttitude + HasOrbit,
> Model<S, Fr> for BdotCross<F, Fr>
{
    fn name(&self) -> &str {
        "bdot"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<Fr> {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };

        let att = state.attitude();
        let orbit = state.orbit();

        // 1. Compute B in the inertial frame (requires epoch for the ECEF rotation)
        let b_inertial = magnetic::field_inertial::<Fr>(
            &self.field,
            &orbit.position_vec(),
            &EarthOrientation::new(*epoch, &self.eop),
        );
        if b_inertial.magnitude() < 1e-30 {
            return ExternalLoads::zeros();
        }

        // 2. Transform to body frame
        let b_body = state
            .attitude_from_inertial()
            .transform(&b_inertial)
            .into_inner();

        // 3. Analytical approximation: dB_body/dt = -omega x B_body
        let omega = &att.angular_velocity;
        let db_body_dt = -omega.cross(&b_body);

        // 4. Desired magnetic moment: m = -k * dB/dt = k * (omega x B)
        let desired = -self.gain * db_body_dt;

        // 5. Allocate to per-MTQ + clamp, then compute torque
        let allocated = self.mtq.allocate(&desired);
        let tau = self.mtq.torque(&allocated, &b_body);

        ExternalLoads::torque(tau)
    }
}

/// Actuator model that applies a commanded magnetic moment as torque.
///
/// The `commanded_moment` is held constant (set externally between ODE segments).
/// Torque is computed as tau = m x B where B is the local geomagnetic field in the body frame.
///
/// When no epoch is available, returns zero loads.
///
/// `Fr` is the inertial frame the geomagnetic field is evaluated in, as for
/// [`BdotCross`].
pub struct CommandedMagnetorquer<
    F: MagneticFieldModel = TiltedDipole,
    Fr: EarthFixedTransform = frame::SimpleEci,
> {
    /// Current commanded magnetic moment \[A*m^2\] in body frame.
    pub commanded_moment: Vector3<f64>,
    /// Geomagnetic field model.
    field: F,
    /// EOP storage for the frame adapter. `()` for `SimpleEci`.
    eop: Fr::EopStorage,
}

impl<F: MagneticFieldModel> CommandedMagnetorquer<F, frame::SimpleEci> {
    /// Create a new magnetorquer actuator model in the default `SimpleEci` frame.
    pub fn new(commanded_moment: Vector3<f64>, field: F) -> Self {
        Self::new_in_frame(commanded_moment, field, ())
    }
}

impl<F: MagneticFieldModel, Fr: EarthFixedTransform> CommandedMagnetorquer<F, Fr> {
    /// Create a magnetorquer that evaluates the field in an arbitrary inertial
    /// frame `Fr`, with that frame's EOP storage (`()` for `SimpleEci`).
    pub fn new_in_frame(commanded_moment: Vector3<f64>, field: F, eop: Fr::EopStorage) -> Self {
        Self {
            commanded_moment,
            field,
            eop,
        }
    }
}

// Frame-generic, as `BdotCross` above.
impl<
    F: MagneticFieldModel,
    Fr: EarthFixedTransform,
    S: HasFrame<Frame = Fr> + HasAttitude + HasOrbit,
> Model<S, Fr> for CommandedMagnetorquer<F, Fr>
{
    fn name(&self) -> &str {
        "magnetorquer"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<Fr> {
        let Some(epoch) = epoch else {
            return ExternalLoads::zeros();
        };
        let b_inertial = magnetic::field_inertial::<Fr>(
            &self.field,
            &state.orbit().position_vec(),
            &EarthOrientation::new(*epoch, &self.eop),
        );
        if b_inertial.magnitude() < 1e-30 {
            return ExternalLoads::zeros();
        }
        let b_body = state
            .attitude_from_inertial()
            .transform(&b_inertial)
            .into_inner();
        ExternalLoads::torque(self.commanded_moment.cross(&b_body))
    }
}

/// B-dot controller using finite-difference dB/dt estimation.
///
/// Unlike [`BdotCross`] which estimates dB/dt via the cross-product
/// -omega x B_body, this controller measures the actual
/// magnetic field at each sample time and computes dB/dt via backward
/// finite difference. This is more realistic (flight software only sees
/// magnetometer readings) but introduces a one-sample delay and produces
/// zero command on the first call.
///
/// When no epoch is available, returns zero command.
///
/// `Fr` is the inertial frame the geomagnetic field is evaluated in, as for
/// [`BdotCross`].
pub struct BdotFiniteDiff<
    F: MagneticFieldModel = TiltedDipole,
    Fr: EarthFixedTransform = frame::SimpleEci,
> {
    gain: f64,
    /// MTQ assembly core for allocation + clamping.
    mtq: MtqAssemblyCore,
    field: F,
    sample_period: f64,
    prev_b_body: Option<Vector3<f64>>,
    prev_t: f64,
    /// EOP storage for the frame adapter. `()` for `SimpleEci`.
    eop: Fr::EopStorage,
}

impl<F: MagneticFieldModel> BdotFiniteDiff<F, frame::SimpleEci> {
    /// Create a new finite-difference B-dot controller in the default
    /// `SimpleEci` frame.
    ///
    /// `max_moment` is per-axis maximum [A·m²] for a 3-axis MTQ.
    ///
    /// # Panics
    /// Panics if `gain` is negative, any component of `max_moment` is negative,
    /// or `sample_period` is not positive.
    pub fn new(gain: f64, max_moment: Vector3<f64>, field: F, sample_period: f64) -> Self {
        Self::new_in_frame(gain, max_moment, field, sample_period, ())
    }
}

impl<F: MagneticFieldModel, Fr: EarthFixedTransform> BdotFiniteDiff<F, Fr> {
    /// Create a finite-difference B-dot controller that samples the field in an
    /// arbitrary inertial frame `Fr`, with that frame's EOP storage (`()` for
    /// `SimpleEci`).
    ///
    /// # Panics
    /// Panics if `gain` is negative, any component of `max_moment` is negative,
    /// or `sample_period` is not positive.
    pub fn new_in_frame(
        gain: f64,
        max_moment: Vector3<f64>,
        field: F,
        sample_period: f64,
        eop: Fr::EopStorage,
    ) -> Self {
        assert!(gain >= 0.0, "gain must be non-negative, got {gain}");
        assert!(
            max_moment[0] >= 0.0 && max_moment[1] >= 0.0 && max_moment[2] >= 0.0,
            "max_moment must be non-negative, got {max_moment:?}"
        );
        assert!(
            sample_period > 0.0,
            "sample_period must be positive, got {sample_period}"
        );
        use crate::spacecraft::Mtq;
        let mtq = MtqAssemblyCore::new(vec![
            Mtq::new(Vector3::x(), max_moment[0]),
            Mtq::new(Vector3::y(), max_moment[1]),
            Mtq::new(Vector3::z(), max_moment[2]),
        ]);
        Self {
            gain,
            mtq,
            field,
            sample_period,
            prev_b_body: None,
            prev_t: 0.0,
            eop,
        }
    }
}

impl<F: MagneticFieldModel, Fr: EarthFixedTransform> DiscreteController<Fr>
    for BdotFiniteDiff<F, Fr>
{
    type Command = Vector3<f64>;

    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn initial_command(&self) -> Vector3<f64> {
        Vector3::zeros()
    }

    fn update(
        &mut self,
        t: f64,
        attitude: &AttitudeState,
        orbit: &OrbitalState<Fr>,
        epoch: Option<&Epoch>,
    ) -> Vector3<f64> {
        let Some(epoch) = epoch else {
            return Vector3::zeros();
        };
        let b_inertial = magnetic::field_inertial::<Fr>(
            &self.field,
            &orbit.position_vec(),
            &EarthOrientation::new(*epoch, &self.eop),
        );
        if b_inertial.magnitude() < 1e-30 {
            return Vector3::zeros();
        }
        let b_body = attitude
            .rotation_tagged_as::<Fr>()
            .inverse()
            .transform(&b_inertial)
            .into_inner();

        let m_cmd = match self.prev_b_body {
            Some(prev_b) => {
                let dt = t - self.prev_t;
                if dt < 1e-15 {
                    return Vector3::zeros();
                }
                let db_dt = (b_body - prev_b) / dt;
                let desired = -self.gain * db_dt;
                // Use MtqAssemblyCore for consistent allocation + clamp
                let allocated = self.mtq.allocate(&desired);
                self.mtq.realized_moment(&allocated)
            }
            None => Vector3::zeros(),
        };

        self.prev_b_body = Some(b_body);
        self.prev_t = t;
        m_cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use arika::epoch::Epoch;
    use arika::frame::Vec3 as FrameVec3;
    use nalgebra::Vector4;

    fn test_epoch() -> Epoch {
        Epoch::j2000()
    }

    /// Combined state for testing (provides HasAttitude + HasOrbit).
    struct TestState {
        attitude: AttitudeState,
        orbit: OrbitalState,
    }

    impl HasAttitude for TestState {
        fn attitude(&self) -> &AttitudeState {
            &self.attitude
        }
    }

    impl HasFrame for TestState {
        type Frame = arika::frame::SimpleEci;
    }

    impl HasOrbit for TestState {
        fn orbit(&self) -> &OrbitalState<arika::frame::SimpleEci> {
            &self.orbit
        }
    }

    #[test]
    fn zero_omega_gives_zero_torque() {
        let ctrl = BdotCross::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState::identity(),
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Zero omega should give zero torque, got {:?}",
            loads.torque_body
        );
    }

    #[test]
    fn torque_opposes_omega_component() {
        let ctrl = BdotCross::new(1e4, Vector3::new(10.0, 10.0, 10.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.05),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        let dot = state
            .attitude
            .angular_velocity
            .dot(&loads.torque_body.into_inner());
        assert!(
            dot <= 0.0,
            "omega . tau should be <= 0 (Cauchy-Schwarz), got {dot:.6e}"
        );
    }

    #[test]
    fn no_acceleration_or_mass_rate() {
        let ctrl = BdotCross::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.0, 0.0),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        assert!(loads.acceleration_inertial.magnitude() < 1e-15);
        assert!(loads.mass_rate.abs() < 1e-15);
    }

    #[test]
    fn moment_clamping() {
        let max_m = 0.001;
        let ctrl = BdotCross::new(
            1e10,
            Vector3::new(max_m, max_m, max_m),
            TiltedDipole::earth(),
        );
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(1.0, 1.0, 1.0),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let epoch = test_epoch();
        let loads = ctrl.eval(0.0, &state, Some(&epoch));
        let b = magnetic::field_eci(
            &TiltedDipole::earth(),
            &FrameVec3::<frame::SimpleEci>::new(7000.0, 0.0, 0.0),
            &epoch,
        )
        .magnitude();
        let max_torque = 3.0_f64.sqrt() * max_m * b;
        assert!(
            loads.torque_body.magnitude() <= max_torque * 1.01,
            "Torque should be bounded by clamped moment: |tau|={:.6e}, bound={max_torque:.6e}",
            loads.torque_body.magnitude()
        );
    }

    // Frame-generalization characterization tests (#151)
    //
    // Pinned `SimpleEci` numbers at a fully 3D state, so opening these
    // controllers to a generic inertial frame `F` cannot change them.

    fn snapshot_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn snapshot_state() -> TestState {
        TestState {
            attitude: AttitudeState::new(
                nalgebra::UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
        }
    }

    /// Characterization: pinned pre-refactor `SimpleEci` B-dot torque \[N·m\].
    #[test]
    fn bdot_cross_simple_eci_torque_snapshot() {
        let ctrl = BdotCross::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let got = ctrl
            .eval(0.0, &snapshot_state(), Some(&snapshot_epoch()))
            .torque_body
            .into_inner();
        let expected = Vector3::new(
            -1.1609801859995682e-7,
            9.248855991956987e-8,
            -3.268271786896945e-7,
        );
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "SimpleEci BdotCross torque changed: {got:?}"
        );
    }

    /// Characterization: pinned pre-refactor `SimpleEci` magnetorquer torque \[N·m\].
    #[test]
    fn commanded_magnetorquer_simple_eci_torque_snapshot() {
        let actuator =
            CommandedMagnetorquer::new(Vector3::new(0.5, -0.2, 0.1), TiltedDipole::earth());
        let got = actuator
            .eval(0.0, &snapshot_state(), Some(&snapshot_epoch()))
            .torque_body
            .into_inner();
        let expected = Vector3::new(
            -4.479088811350949e-6,
            -3.1117980068616165e-6,
            1.617184804303151e-5,
        );
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "SimpleEci CommandedMagnetorquer torque changed: {got:?}"
        );
    }

    /// **Discriminating test (#151)**: propagating the same raw state in `Gcrs`
    /// evaluates the field through the full IAU 2006 chain, so the B-dot torque
    /// matches a `field_inertial::<Gcrs>` reconstruction (bit-exact) and differs
    /// measurably from the `SimpleEci` number.
    #[test]
    fn gcrs_bdot_cross_uses_the_iau2006_field_chain() {
        use crate::test_support::zero_eop;

        struct GcrsState {
            attitude: AttitudeState,
            orbit: OrbitalState<frame::Gcrs>,
        }
        impl HasAttitude for GcrsState {
            fn attitude(&self) -> &AttitudeState {
                &self.attitude
            }
        }
        impl HasFrame for GcrsState {
            type Frame = frame::Gcrs;
        }

        impl HasOrbit for GcrsState {
            fn orbit(&self) -> &OrbitalState<frame::Gcrs> {
                &self.orbit
            }
        }

        let epoch = snapshot_epoch();
        let pos = Vector3::new(4000.0, -5000.0, 2500.0);
        let simple = snapshot_state();
        let state = GcrsState {
            attitude: simple.attitude,
            orbit: OrbitalState::<frame::Gcrs>::new_in_frame(pos, Vector3::new(1.0, 2.0, 7.0)),
        };

        let gain = 1e4;
        let max_moment = Vector3::new(1.0, 1.0, 1.0);
        let ctrl = BdotCross::<TiltedDipole, frame::Gcrs>::new_in_frame(
            gain,
            max_moment,
            TiltedDipole::earth(),
            zero_eop(),
        );
        let got = ctrl
            .eval(0.0, &state, Some(&epoch))
            .torque_body
            .into_inner();

        // Reconstruct with the Gcrs field.
        let b_gcrs = magnetic::field_inertial::<frame::Gcrs>(
            &TiltedDipole::earth(),
            &FrameVec3::from_raw(pos),
            &EarthOrientation::new(epoch, &zero_eop()),
        );
        let b_body = state
            .attitude_from_inertial()
            .transform(&b_gcrs)
            .into_inner();
        let desired = gain * state.attitude.angular_velocity.cross(&b_body);
        let reference = BdotCross::new(gain, max_moment, TiltedDipole::earth());
        let expected = reference
            .mtq
            .torque(&reference.mtq.allocate(&desired), &b_body);
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "Gcrs BdotCross torque must use the Gcrs field: {got:?} vs {expected:?}"
        );

        let simple_eci = Vector3::new(
            -1.1609801859995682e-7,
            9.248855991956987e-8,
            -3.268271786896945e-7,
        );
        assert!(
            (got - simple_eci).magnitude() > simple_eci.magnitude() * 1e-4,
            "Gcrs torque should differ from the SimpleEci result"
        );
    }

    /// Characterization of the `|B| < 1e-30` guard on non-finite input: with a
    /// NaN position the comparison is false, so NaN propagates into the torque
    /// instead of being swallowed by the zero-load early return.
    #[test]
    fn bdot_cross_nan_position_propagates_nan() {
        let ctrl = BdotCross::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: snapshot_state().attitude,
            orbit: OrbitalState::new(
                Vector3::new(f64::NAN, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
        };
        let tau = ctrl
            .eval(0.0, &state, Some(&snapshot_epoch()))
            .torque_body
            .into_inner();
        assert!(
            tau.iter().all(|c| c.is_nan()),
            "NaN must propagate: {tau:?}"
        );
    }

    #[test]
    fn no_epoch_returns_zero_loads() {
        // Without epoch, magnetic field models cannot compute the field,
        // so the controller returns zero loads.
        let ctrl = BdotCross::new(1e4, Vector3::new(1.0, 1.0, 1.0), TiltedDipole::earth());
        let state = TestState {
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.2, 0.05),
            },
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::zeros()),
        };
        let loads = ctrl.eval(0.0, &state, None);
        assert!(
            loads.torque_body.magnitude() < 1e-30,
            "Without epoch, should return zero loads"
        );
    }
}
