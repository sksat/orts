//! Star tracker (STT) sensor.
//!
//! Measures the spacecraft's attitude quaternion. The ideal sensor
//! returns the true quaternion; noisy versions apply a small random
//! rotation perturbation to model pointing error.

use kaname::epoch::Epoch;
use nalgebra::{UnitQuaternion, Vector3, Vector4};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand_distr::Normal;

use crate::SpacecraftState;
use crate::plugin::tick_input::AttitudeBodyToInertial;

/// Star tracker sensor.
///
/// Returns the attitude quaternion (body→inertial, Hamilton scalar-first).
/// Noise is modeled as a small random rotation applied to the true
/// quaternion:
///
/// ```text
/// q_measured = q_true * δq(σ)
/// ```
///
/// where `δq` is a unit quaternion constructed from Gaussian-distributed
/// small-angle body-frame rotations with per-axis standard deviation `σ`
/// \[rad\]. Typical star tracker accuracy is 1–30 arcsec (5e-6 to 1.5e-4 rad).
pub struct StarTracker {
    sigma: Option<(Vector3<f64>, StdRng)>,
}

impl StarTracker {
    /// Create an ideal star tracker (zero noise).
    pub fn new() -> Self {
        Self { sigma: None }
    }

    /// Add pointing noise with per-axis standard deviation \[rad\].
    pub fn with_pointing_noise(self, sigma: Vector3<f64>, seed: u64) -> Self {
        Self {
            sigma: Some((sigma, StdRng::seed_from_u64(seed))),
        }
    }

    /// Add isotropic pointing noise \[rad\].
    pub fn with_pointing_noise_isotropic(self, sigma: f64, seed: u64) -> Self {
        self.with_pointing_noise(Vector3::new(sigma, sigma, sigma), seed)
    }

    /// Measure the attitude quaternion (body→inertial).
    pub fn measure(&mut self, state: &SpacecraftState, _epoch: &Epoch) -> AttitudeBodyToInertial {
        let q_true = UnitQuaternion::from_quaternion(state.attitude.orientation().into_inner());

        let q_measured = match &mut self.sigma {
            Some((sigma, rng)) => {
                let dx = rng.sample(Normal::new(0.0, sigma.x).unwrap());
                let dy = rng.sample(Normal::new(0.0, sigma.y).unwrap());
                let dz = rng.sample(Normal::new(0.0, sigma.z).unwrap());
                let delta = UnitQuaternion::from_scaled_axis(Vector3::new(dx, dy, dz));
                q_true * delta
            }
            None => q_true,
        };

        let q = q_measured.into_inner();
        AttitudeBodyToInertial::new(Vector4::new(q.w, q.i, q.j, q.k))
    }
}

impl Default for StarTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::AttitudeState;
    use crate::orbital::OrbitalState;

    fn make_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
            attitude: AttitudeState {
                quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                angular_velocity: Vector3::new(0.1, 0.05, -0.03),
            },
            mass: 50.0,
        }
    }

    #[test]
    fn ideal_returns_true_quaternion() {
        let mut stt = StarTracker::new();
        let state = make_state();
        let epoch = Epoch::j2000();
        let q = stt.measure(&state, &epoch);
        assert_eq!(q.into_inner(), state.attitude.quaternion);
    }

    #[test]
    fn noisy_differs_from_true() {
        // 10 arcsec ≈ 5e-5 rad
        let mut stt = StarTracker::new().with_pointing_noise_isotropic(5e-5, 42);
        let state = make_state();
        let epoch = Epoch::j2000();
        let q = stt.measure(&state, &epoch);
        assert_ne!(q.into_inner(), state.attitude.quaternion);
        // Should still be close to unit quaternion.
        let q = q.into_inner();
        let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((norm - 1.0).abs() < 1e-10);
    }

    #[test]
    fn noisy_is_deterministic() {
        let state = make_state();
        let epoch = Epoch::j2000();
        let mut s1 = StarTracker::new().with_pointing_noise_isotropic(5e-5, 42);
        let mut s2 = StarTracker::new().with_pointing_noise_isotropic(5e-5, 42);
        assert_eq!(s1.measure(&state, &epoch), s2.measure(&state, &epoch));
    }

    #[test]
    fn noise_magnitude_is_reasonable() {
        let sigma = 1e-4; // ~20 arcsec
        let mut stt = StarTracker::new().with_pointing_noise_isotropic(sigma, 42);
        let state = make_state();
        let epoch = Epoch::j2000();
        let n_samples = 1000;
        let mut max_angle = 0.0_f64;
        for _ in 0..n_samples {
            let q_meas = stt.measure(&state, &epoch).into_inner();
            let q_true = &state.attitude.quaternion;
            // Angular distance: 2 * arccos(|q_true · q_meas|)
            let dot = (q_true[0] * q_meas[0]
                + q_true[1] * q_meas[1]
                + q_true[2] * q_meas[2]
                + q_true[3] * q_meas[3])
                .abs();
            let angle = 2.0 * dot.min(1.0).acos();
            max_angle = max_angle.max(angle);
        }
        // With sigma=1e-4 rad, 3-axis, max angle should be < ~10*sigma
        assert!(
            max_angle < 10.0 * sigma,
            "max angle {max_angle:.3e} too large for sigma {sigma:.3e}"
        );
    }
}
