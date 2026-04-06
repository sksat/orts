//! Magnetometer sensor.
//!
//! Transforms the geomagnetic field from the ECI frame to the
//! spacecraft body frame using the attitude quaternion, then
//! optionally applies noise models.

use std::sync::Arc;

use kaname::epoch::Epoch;
use tobari::magnetic::MagneticFieldModel;

use super::noise::NoiseModel;
use crate::SpacecraftState;
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

    /// Measure the magnetic field in the body frame.
    pub fn measure(&mut self, state: &SpacecraftState, epoch: &Epoch) -> MagneticFieldBody {
        let b_eci = self
            .field_model
            .field_eci(&state.orbit.position_eci(), epoch);
        let mut b_body = state.attitude.inertial_to_body() * b_eci;
        for n in &mut self.noise {
            b_body = n.apply(b_body);
        }
        MagneticFieldBody::new(b_body)
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
        assert!(b_body.iter().all(|x| x.is_finite()));
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
        let b_eci = field_model.field_eci(&state.orbit.position_eci(), &epoch);
        assert!((b_body - b_eci).magnitude() < 1e-15);
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
