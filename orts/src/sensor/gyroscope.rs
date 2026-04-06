//! Rate gyroscope sensor.
//!
//! Returns the spacecraft's angular velocity in the body frame,
//! optionally with noise models applied.

use kaname::epoch::Epoch;
use nalgebra::Vector3;

use super::noise::NoiseModel;
use crate::SpacecraftState;

/// Three-axis rate gyroscope.
///
/// The ideal measurement is `state.attitude.angular_velocity`, which
/// is already expressed in the body frame \[rad/s\]. Noise models are
/// added via the builder-style [`Self::with_noise`] method.
///
/// The `epoch` parameter is unused but accepted for signature
/// consistency with [`super::Magnetometer`].
pub struct Gyroscope {
    noise: Vec<Box<dyn NoiseModel>>,
}

impl Gyroscope {
    /// Create an ideal gyroscope (no noise).
    pub fn new() -> Self {
        Self { noise: Vec::new() }
    }

    /// Add a noise model. Multiple calls chain in order.
    ///
    /// ```ignore
    /// let gyro = Gyroscope::new()
    ///     .with_noise(GaussianNoise::isotropic(1e-4, 42))
    ///     .with_noise(BiasRandomWalk::isotropic(1e-5, dt, 99));
    /// ```
    pub fn with_noise(mut self, noise: impl NoiseModel + 'static) -> Self {
        self.noise.push(Box::new(noise));
        self
    }

    /// Measure the angular velocity in the body frame \[rad/s\].
    pub fn measure(&mut self, state: &SpacecraftState, _epoch: &Epoch) -> Vector3<f64> {
        let mut omega = state.attitude.angular_velocity;
        for n in &mut self.noise {
            omega = n.apply(omega);
        }
        omega
    }
}

impl Default for Gyroscope {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;
    use crate::sensor::noise::{BiasRandomWalk, GaussianNoise};
    use nalgebra::Vector4;

    fn make_state(omega: Vector3<f64>) -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: omega,
            },
            mass: 50.0,
        }
    }

    #[test]
    fn ideal_gyroscope_returns_true_angular_velocity() {
        let mut gyro = Gyroscope::new();
        let omega = Vector3::new(0.1, 0.05, -0.03);
        let state = make_state(omega);
        let epoch = Epoch::j2000();
        assert_eq!(gyro.measure(&state, &epoch), omega);
    }

    #[test]
    fn noisy_gyroscope_differs_from_true() {
        let mut gyro = Gyroscope::new().with_noise(GaussianNoise::isotropic(1e-3, 42));
        let omega = Vector3::new(0.1, 0.05, -0.03);
        let state = make_state(omega);
        let epoch = Epoch::j2000();
        let measured = gyro.measure(&state, &epoch);
        assert!((measured - omega).magnitude() > 0.0);
        assert!((measured - omega).magnitude() < 0.1);
    }

    #[test]
    fn chained_noise_models() {
        let mut gyro = Gyroscope::new()
            .with_noise(GaussianNoise::isotropic(1e-4, 42))
            .with_noise(BiasRandomWalk::isotropic(1e-5, 1.0, 99));
        let omega = Vector3::new(0.1, 0.05, -0.03);
        let state = make_state(omega);
        let epoch = Epoch::j2000();
        let m1 = gyro.measure(&state, &epoch);
        let m2 = gyro.measure(&state, &epoch);
        // Bias drift accumulates, so consecutive measurements differ.
        assert_ne!(m1, m2);
    }

    #[test]
    fn noisy_gyroscope_is_deterministic() {
        let omega = Vector3::new(0.1, 0.05, -0.03);
        let state = make_state(omega);
        let epoch = Epoch::j2000();
        let mut g1 = Gyroscope::new().with_noise(GaussianNoise::isotropic(1e-3, 42));
        let mut g2 = Gyroscope::new().with_noise(GaussianNoise::isotropic(1e-3, 42));
        assert_eq!(g1.measure(&state, &epoch), g2.measure(&state, &epoch));
    }
}
