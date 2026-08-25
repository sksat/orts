//! Magnetometer sensor.
//!
//! Transforms the geomagnetic field from the ECI frame to the
//! spacecraft body frame using the attitude quaternion, then
//! optionally applies noise models.

use std::sync::Arc;

use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::Epoch;
use arika::frame;
use tobari::magnetic::MagneticFieldModel;

use super::noise::NoiseModel;
use crate::SpacecraftState;
use crate::magnetic;
use crate::plugin::tick_input::MagneticFieldBody;

/// Three-axis magnetometer.
///
/// Evaluates the host's geomagnetic field model at the spacecraft's
/// current ECI position and epoch, then rotates the result into the
/// body frame via the attitude quaternion:
///
/// ```text
/// B_body = noise(R_bi · B_eci(r, epoch))
/// ```
///
/// Noise models are added via the builder-style [`Self::with_noise`]
/// method and applied in the order they were added.
pub struct Magnetometer {
    field_model: Arc<dyn MagneticFieldModel>,
    noise: Vec<Box<dyn NoiseModel>>,
}

impl Magnetometer {
    /// Create an ideal magnetometer (no noise).
    pub fn new(field_model: Arc<dyn MagneticFieldModel>) -> Self {
        Self {
            field_model,
            noise: Vec::new(),
        }
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
    pub fn measure(&mut self, state: &SpacecraftState, epoch: &Epoch) -> MagneticFieldBody {
        self.measure_in_frame::<frame::SimpleEci>(state, &EarthOrientation::simple(*epoch))
    }

    /// Measure the magnetic field in the body frame for a state propagated in
    /// an arbitrary inertial frame `F`.
    ///
    /// The field is evaluated in `F` via [`magnetic::field_inertial`] — the
    /// ERA-only rotation for `SimpleEci`, the full IAU 2006 chain for `Gcrs`
    /// (whose `orientation` carries the EOP data) — and then rotated into the
    /// body frame.
    pub fn measure_in_frame<F: EarthFixedTransform>(
        &mut self,
        state: &SpacecraftState<F>,
        orientation: &EarthOrientation<'_, F>,
    ) -> MagneticFieldBody {
        let b_inertial = magnetic::field_inertial::<F>(
            self.field_model.as_ref(),
            &state.orbit.position_vec(),
            orientation,
        );
        let b_body_typed = state
            .attitude
            .rotation_from_inertial::<F>()
            .transform(&b_inertial);
        let mut b_body = b_body_typed.into_inner();
        for n in &mut self.noise {
            b_body = n.apply(b_body);
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
        let b_body = mag.measure(&state, &epoch).into_inner();
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
        let b_body = mag.measure(&state, &epoch).into_inner();
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
        let got = mag.measure(&snapshot_state(), &epoch).into_inner();
        let expected = nalgebra::Vector3::new(
            -4.382433684690031e-6,
            -3.059072261218701e-5,
            -7.100082750661239e-6,
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
            .measure_in_frame::<frame::Gcrs>(&state, &EarthOrientation::new(epoch, &zero_eop()))
            .into_inner()
            .into_inner();

        let b_gcrs = magnetic::field_inertial::<frame::Gcrs>(
            &TiltedDipole::earth(),
            &arika::frame::Vec3::from_raw(pos),
            &EarthOrientation::new(epoch, &zero_eop()),
        );
        let expected = state
            .attitude
            .rotation_from_inertial::<frame::Gcrs>()
            .transform(&b_gcrs)
            .into_inner();
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "Gcrs magnetometer must use the Gcrs field: {got:?} vs {expected:?}"
        );

        let simple_eci = mag.measure(&simple, &epoch).into_inner().into_inner();
        assert!(
            (got - simple_eci).magnitude() > simple_eci.magnitude() * 1e-4,
            "Gcrs reading should differ from the SimpleEci reading"
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
        let b_ideal = ideal.measure(&state, &epoch).into_inner();
        let b_noisy = noisy.measure(&state, &epoch).into_inner();
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
        assert_eq!(m1.measure(&state, &epoch), m2.measure(&state, &epoch));
    }
}
