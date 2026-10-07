//! Magnetometer sensor.
//!
//! Transforms the geomagnetic field from the ECI frame to the
//! spacecraft body frame using the attitude quaternion, adds the field the
//! onboard MTQs produce at the sensor, then optionally applies noise models.

use std::sync::Arc;

use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::Epoch;
use arika::frame;
use nalgebra::Vector3;
use tobari::magnetic::MagneticFieldModel;

use super::noise::NoiseModel;
use super::onboard_field::{MtqCoupling, OnboardMagneticSources};
use crate::SpacecraftState;
use crate::magnetic;
use crate::model::HasAttitude;
use crate::plugin::tick_input::MagneticFieldBody;

/// Three-axis magnetometer.
///
/// Evaluates the host's geomagnetic field model at the spacecraft's
/// current ECI position and epoch, rotates the result into the
/// body frame via the attitude quaternion, and adds the spacecraft's own fields
/// at the sensor:
///
/// ```text
/// B_body = noise(R_bi · B_eci(r, epoch) + K · u + b_res)
/// ```
///
/// `K` is the sensor's [`MtqCoupling`] (none by default, i.e. zero) and `u` the
/// rods' realized moments from [`OnboardMagneticSources`]. `b_res` is the
/// sensor's residual field ([`Self::with_residual_field`], zero by default).
///
/// Noise models are added via the builder-style [`Self::with_noise`]
/// method and applied in the order they were added.
pub struct Magnetometer {
    field_model: Arc<dyn MagneticFieldModel>,
    mtq_coupling: Option<MtqCoupling>,
    residual_field: Option<Vector3<f64>>,
    noise: Vec<Box<dyn NoiseModel>>,
}

impl Magnetometer {
    /// Create an ideal magnetometer (no noise).
    pub fn new(field_model: Arc<dyn MagneticFieldModel>) -> Self {
        Self {
            field_model,
            mtq_coupling: None,
            residual_field: None,
            noise: Vec::new(),
        }
    }

    /// Couple the sensor to the MTQ rods: the reading then includes the field
    /// the rods produce at the sensor.
    pub fn with_mtq_coupling(mut self, coupling: MtqCoupling) -> Self {
        self.mtq_coupling = Some(coupling);
        self
    }

    /// Add a constant field [T, body frame] at the sensor: the field the
    /// spacecraft's own magnetization makes there (the hard-iron offset a
    /// calibration measures).
    ///
    /// It is given apart from the residual dipole of the torque model
    /// ([`ResidualDipoleTorque`](crate::attitude::ResidualDipoleTorque)): the
    /// magnetization is spread over the spacecraft, so the field at one point
    /// does not follow from its dipole moment.
    ///
    /// # Panics
    /// Panics if a component is non-finite.
    pub fn with_residual_field(mut self, field: Vector3<f64>) -> Self {
        assert!(
            field.iter().all(|v| v.is_finite()),
            "residual field must be finite, got {field:?}"
        );
        self.residual_field = Some(field);
        self
    }

    /// The sensor's residual field [T, body frame], if any.
    pub fn residual_field(&self) -> Option<&Vector3<f64>> {
        self.residual_field.as_ref()
    }

    /// The sensor's coupling to the MTQ rods, if any.
    pub fn mtq_coupling(&self) -> Option<&MtqCoupling> {
        self.mtq_coupling.as_ref()
    }

    /// Add a noise model. Multiple calls chain in order.
    ///
    /// ```ignore
    /// let mag = Magnetometer::new(field_model)
    ///     .with_noise(GaussianNoise::isotropic(1e-7, 42))
    ///     .with_noise(BiasRandomWalk::isotropic(1e-8, dt, 99));
    /// ```
    pub fn with_noise(mut self, noise: impl NoiseModel + 'static) -> Self {
        self.noise.push(Box::new(noise));
        self
    }

    /// Measure the magnetic field in the body frame, for a `SimpleEci` state.
    ///
    /// Thin wrapper over [`Self::measure_in_frame`] (which needs no EOP for
    /// `SimpleEci`).
    ///
    /// `t` is the sim time of the sample [s], which the noise models are keyed on.
    pub fn measure(
        &mut self,
        t: f64,
        state: &SpacecraftState,
        epoch: &Epoch,
        sources: &OnboardMagneticSources,
    ) -> MagneticFieldBody {
        self.measure_in_frame::<frame::SimpleEci>(
            t,
            state,
            &EarthOrientation::simple(*epoch),
            sources,
        )
    }

    /// Measure the magnetic field in the body frame for a state propagated in
    /// an arbitrary inertial frame `F`.
    ///
    /// The field is evaluated in `F` via [`magnetic::field_inertial`] — the
    /// ERA-only rotation for `SimpleEci`, the full IAU 2006 chain for `Gcrs`
    /// (whose `orientation` carries the EOP data) — and then rotated into the
    /// body frame, where the MTQs' field at the sensor is added before noise.
    ///
    /// # Panics
    /// Panics if the sensor has an MTQ coupling and `sources` carries no MTQ
    /// state, or a rod count different from the coupling's.
    pub fn measure_in_frame<F: EarthFixedTransform>(
        &mut self,
        t: f64,
        state: &SpacecraftState<F>,
        orientation: &EarthOrientation<'_, F>,
        sources: &OnboardMagneticSources,
    ) -> MagneticFieldBody {
        super::noise::keyed::check_sample_time(t);
        let b_inertial = magnetic::field_inertial::<F>(
            self.field_model.as_ref(),
            &state.orbit.position_vec(),
            orientation,
        );
        let b_body_typed = state.attitude_from_inertial().transform(&b_inertial);
        let mut b_body = b_body_typed.into_inner();
        if let Some(coupling) = &self.mtq_coupling {
            let rods = sources
                .mtq_rod_moments()
                .expect("magnetometer is coupled to MTQs, but the sources carry no MTQ state");
            b_body += coupling.field(rods);
        }
        if let Some(residual) = &self.residual_field {
            b_body += residual;
        }
        for n in &mut self.noise {
            b_body = n.apply(t, b_body);
        }
        MagneticFieldBody::new(arika::frame::Vec3::from_raw(b_body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use crate::sensor::noise::GaussianNoise;
    use nalgebra::{Vector3, Vector4};
    use tobari::magnetic::TiltedDipole;

    fn leo_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::zeros(),
            },
            mass: 50.0,
        }
    }

    #[test]
    fn ideal_magnetometer_returns_finite_nonzero_for_leo() {
        let mut mag = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        let state = leo_state();
        let epoch = Epoch::j2000();
        let b_body = mag
            .measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
            .into_inner();
        assert!(b_body.is_finite());
        let magnitude = b_body.magnitude();
        assert!(
            magnitude > 1e-5 && magnitude < 1e-4,
            "expected LEO-range B, got {magnitude:.3e} T"
        );
    }

    #[test]
    fn identity_quaternion_gives_same_as_eci() {
        let field_model = Arc::new(TiltedDipole::earth());
        let mut mag = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>);
        let state = leo_state();
        let epoch = Epoch::j2000();
        let b_body = mag
            .measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
            .into_inner();
        let b_eci = magnetic::field_eci(field_model.as_ref(), &state.orbit.position_eci(), &epoch);
        assert!((b_body.into_inner() - b_eci.into_inner()).magnitude() < 1e-15);
    }

    // Frame-generalization characterization (#151)

    fn snapshot_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
            attitude: AttitudeState::new(
                nalgebra::UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            mass: 50.0,
        }
    }

    /// Characterization: pinned pre-refactor `SimpleEci` body-frame field \[T\],
    /// so opening the sensor to a generic inertial frame cannot change it.
    #[test]
    fn simple_eci_measurement_snapshot() {
        let mut mag = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let got = mag
            .measure(
                0.0,
                &snapshot_state(),
                &epoch,
                &OnboardMagneticSources::none(),
            )
            .into_inner();
        let expected = nalgebra::Vector3::new(
            4.382433684690031e-6,
            3.059072261218701e-5,
            7.100082750661239e-6,
        );
        assert!(
            (got.into_inner() - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "SimpleEci magnetometer reading changed: {got:?}"
        );
    }

    /// **Discriminating test (#151)**: the same raw state read in `Gcrs` goes
    /// through the full IAU 2006 chain, so the reading matches a
    /// `field_inertial::<Gcrs>` reconstruction (bit-exact) and differs
    /// measurably from the `SimpleEci` reading.
    #[test]
    fn gcrs_measurement_uses_the_iau2006_field_chain() {
        use crate::test_support::zero_eop;

        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let simple = snapshot_state();
        let pos = *simple.orbit.position();
        let state = SpacecraftState::<frame::Gcrs> {
            orbit: OrbitalState::<frame::Gcrs>::new_in_frame(pos, *simple.orbit.velocity()),
            attitude: simple.attitude.clone(),
            mass: simple.mass,
        };

        let mut mag = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        let got = mag
            .measure_in_frame::<frame::Gcrs>(
                0.0,
                &state,
                &EarthOrientation::new(epoch, &zero_eop()),
                &OnboardMagneticSources::none(),
            )
            .into_inner()
            .into_inner();

        let b_gcrs = magnetic::field_inertial::<frame::Gcrs>(
            &TiltedDipole::earth(),
            &arika::frame::Vec3::from_raw(pos),
            &EarthOrientation::new(epoch, &zero_eop()),
        );
        let expected = state
            .attitude_from_inertial()
            .transform(&b_gcrs)
            .into_inner();
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "Gcrs magnetometer must use the Gcrs field: {got:?} vs {expected:?}"
        );

        let simple_eci = mag
            .measure(0.0, &simple, &epoch, &OnboardMagneticSources::none())
            .into_inner()
            .into_inner();
        assert!(
            (got - simple_eci).magnitude() > simple_eci.magnitude() * 1e-4,
            "Gcrs reading should differ from the SimpleEci reading"
        );
    }

    // Onboard MTQ field

    fn coupling() -> MtqCoupling {
        MtqCoupling::from_columns(vec![
            Vector3::new(2e-5, 1e-6, 0.0),
            Vector3::new(0.0, -1e-5, 3e-6),
            Vector3::new(-4e-6, 0.0, 5e-5),
        ])
    }

    fn mtq_on() -> OnboardMagneticSources {
        OnboardMagneticSources::none().with_mtq_rod_moments(vec![0.5, -1.0, 0.2])
    }

    /// The coupled reading is the uncoupled one plus `K · u`.
    #[test]
    fn coupled_reading_adds_the_rods_field() {
        let state = snapshot_state();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let mut plain = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        let mut coupled =
            Magnetometer::new(Arc::new(TiltedDipole::earth())).with_mtq_coupling(coupling());

        let earth = plain.measure(0.0, &state, &epoch, &mtq_on()).into_inner();
        let got = coupled.measure(0.0, &state, &epoch, &mtq_on()).into_inner();
        let expected = earth.into_inner() + coupling().field(&[0.5, -1.0, 0.2]);
        assert_eq!(got.into_inner(), expected);
        assert!(
            (got.into_inner() - earth.into_inner()).magnitude() > 1e-6,
            "the rods' field must move the reading"
        );
    }

    /// An uncoupled sensor reads the geomagnetic field whatever the MTQs do.
    #[test]
    fn uncoupled_reading_ignores_the_mtq_state() {
        let state = snapshot_state();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let mut mag = Magnetometer::new(Arc::new(TiltedDipole::earth()));
        assert_eq!(
            mag.measure(0.0, &state, &epoch, &mtq_on()),
            mag.measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
        );
    }

    /// With no ambient field the reading is the rods' field alone: the
    /// self-interference does not vanish with the geomagnetic field.
    #[test]
    fn coupled_reading_without_ambient_field_is_the_rods_field() {
        let mut mag =
            Magnetometer::new(Arc::new(tobari::magnetic::NoField)).with_mtq_coupling(coupling());
        let got = mag
            .measure(0.0, &leo_state(), &Epoch::j2000(), &mtq_on())
            .into_inner()
            .into_inner();
        assert_eq!(got, coupling().field(&[0.5, -1.0, 0.2]));
    }

    /// The residual field is added to the reading as it is, alongside the rods'
    /// field.
    #[test]
    fn residual_field_adds_to_the_reading() {
        let state = snapshot_state();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let b_res = Vector3::new(3e-7, -1e-7, 2e-7);
        let mut coupled =
            Magnetometer::new(Arc::new(TiltedDipole::earth())).with_mtq_coupling(coupling());
        let mut with_residual = Magnetometer::new(Arc::new(TiltedDipole::earth()))
            .with_mtq_coupling(coupling())
            .with_residual_field(b_res);
        let without = coupled.measure(&state, &epoch, &mtq_on()).into_inner();
        let got = with_residual
            .measure(&state, &epoch, &mtq_on())
            .into_inner();
        assert_eq!(got.into_inner(), without.into_inner() + b_res);
    }

    #[test]
    #[should_panic(expected = "residual field must be finite")]
    fn a_non_finite_residual_field_is_rejected() {
        let _ = Magnetometer::new(Arc::new(TiltedDipole::earth()))
            .with_residual_field(Vector3::new(0.0, f64::INFINITY, 0.0));
    }

    /// A coupled sensor refuses a sample without MTQ state rather than
    /// reading as if the rods were off.
    #[test]
    #[should_panic(expected = "the sources carry no MTQ state")]
    fn coupled_reading_requires_the_mtq_state() {
        let mut mag =
            Magnetometer::new(Arc::new(TiltedDipole::earth())).with_mtq_coupling(coupling());
        let _ = mag.measure(
            0.0,
            &leo_state(),
            &Epoch::j2000(),
            &OnboardMagneticSources::none(),
        );
    }

    #[test]
    fn noisy_magnetometer_differs_from_ideal() {
        let field_model = Arc::new(TiltedDipole::earth());
        let mut ideal = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>);
        let mut noisy = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>)
            .with_noise(GaussianNoise::isotropic(1e-6, 42));
        let state = leo_state();
        let epoch = Epoch::j2000();
        let b_ideal = ideal
            .measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
            .into_inner();
        let b_noisy = noisy
            .measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
            .into_inner();
        assert!(
            (b_ideal - b_noisy).magnitude() > 0.0,
            "noisy and ideal should differ"
        );
        assert!((b_ideal - b_noisy).magnitude() < 1e-4, "noise too large");
    }

    #[test]
    fn noisy_magnetometer_is_deterministic() {
        let field_model = Arc::new(TiltedDipole::earth());
        let mut m1 = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>)
            .with_noise(GaussianNoise::isotropic(1e-6, 42));
        let mut m2 = Magnetometer::new(Arc::clone(&field_model) as Arc<dyn MagneticFieldModel>)
            .with_noise(GaussianNoise::isotropic(1e-6, 42));
        let state = leo_state();
        let epoch = Epoch::j2000();
        assert_eq!(
            m1.measure(0.0, &state, &epoch, &OnboardMagneticSources::none()),
            m2.measure(0.0, &state, &epoch, &OnboardMagneticSources::none())
        );
    }
}
