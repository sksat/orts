//! Sensor noise models.
//!
//! [`NoiseModel`] trait defines the interface for injecting noise into
//! sensor measurements. Implementations are composed into sensor structs
//! (e.g. `Magnetometer`, `Gyroscope`) via an optional field.
//!
//! All noise models must be deterministic given the same seed — this is
//! critical for simulation reproducibility and oracle tests.

use nalgebra::Vector3;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand_distr::Normal;

/// Noise model that transforms a true measurement into a noisy one.
///
/// Implementations hold internal state (e.g. an RNG, bias drift state)
/// and are called once per tick. `Send` is required so sensors can be
/// used in per-satellite worker threads.
pub trait NoiseModel: Send {
    /// Apply noise to a true 3-axis measurement, returning the noisy value.
    fn apply(&mut self, true_value: Vector3<f64>) -> Vector3<f64>;
}

/// Additive Gaussian white noise.
///
/// Each axis gets independent zero-mean Gaussian noise with the
/// configured standard deviation (sigma). The RNG is seeded for
/// reproducibility.
///
/// ```text
/// noisy = true_value + N(0, sigma)
/// ```
pub struct GaussianNoise {
    dist_x: Normal<f64>,
    dist_y: Normal<f64>,
    dist_z: Normal<f64>,
    rng: StdRng,
}

impl GaussianNoise {
    /// Create a Gaussian noise model with per-axis standard deviations.
    ///
    /// `sigma` contains the 1-sigma standard deviation for each axis.
    /// `seed` ensures deterministic reproducibility.
    pub fn new(sigma: Vector3<f64>, seed: u64) -> Self {
        Self {
            dist_x: Normal::new(0.0, sigma.x).expect("sigma.x must be non-negative"),
            dist_y: Normal::new(0.0, sigma.y).expect("sigma.y must be non-negative"),
            dist_z: Normal::new(0.0, sigma.z).expect("sigma.z must be non-negative"),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// Create with the same sigma for all three axes.
    pub fn isotropic(sigma: f64, seed: u64) -> Self {
        Self::new(Vector3::new(sigma, sigma, sigma), seed)
    }
}

impl NoiseModel for GaussianNoise {
    fn apply(&mut self, true_value: Vector3<f64>) -> Vector3<f64> {
        Vector3::new(
            true_value.x + self.rng.sample(self.dist_x),
            true_value.y + self.rng.sample(self.dist_y),
            true_value.z + self.rng.sample(self.dist_z),
        )
    }
}

/// Bias random walk (Wiener process on the bias vector).
///
/// Models a slowly drifting bias that accumulates over time. At each
/// tick, the bias is updated by a Gaussian step:
///
/// ```text
/// bias(t+dt) = bias(t) + N(0, sigma_drift * sqrt(dt))
/// noisy = true_value + bias
/// ```
///
/// This is a standard gyroscope bias instability model. The drift
/// rate `sigma_drift` has units of \[measurement unit / sqrt(s)\].
pub struct BiasRandomWalk {
    bias: Vector3<f64>,
    dist_x: Normal<f64>,
    dist_y: Normal<f64>,
    dist_z: Normal<f64>,
    rng: StdRng,
}

impl BiasRandomWalk {
    /// Create a bias random walk model.
    ///
    /// - `sigma_drift`: drift rate per axis \[unit / sqrt(s)\]
    /// - `dt`: time step between sensor evaluations \[s\]
    /// - `seed`: RNG seed for reproducibility
    pub fn new(sigma_drift: Vector3<f64>, dt: f64, seed: u64) -> Self {
        assert!(dt > 0.0, "dt must be positive");
        let scale = dt.sqrt();
        Self {
            bias: Vector3::zeros(),
            dist_x: Normal::new(0.0, sigma_drift.x * scale)
                .expect("sigma_drift.x must be non-negative"),
            dist_y: Normal::new(0.0, sigma_drift.y * scale)
                .expect("sigma_drift.y must be non-negative"),
            dist_z: Normal::new(0.0, sigma_drift.z * scale)
                .expect("sigma_drift.z must be non-negative"),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// Create with isotropic drift rate.
    pub fn isotropic(sigma_drift: f64, dt: f64, seed: u64) -> Self {
        Self::new(
            Vector3::new(sigma_drift, sigma_drift, sigma_drift),
            dt,
            seed,
        )
    }
}

impl NoiseModel for BiasRandomWalk {
    fn apply(&mut self, true_value: Vector3<f64>) -> Vector3<f64> {
        let step = Vector3::new(
            self.rng.sample(self.dist_x),
            self.rng.sample(self.dist_y),
            self.rng.sample(self.dist_z),
        );
        self.bias += step;
        true_value + self.bias
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_noise_is_deterministic() {
        let mut n1 = GaussianNoise::isotropic(1e-5, 42);
        let mut n2 = GaussianNoise::isotropic(1e-5, 42);
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert_eq!(n1.apply(v), n2.apply(v));
    }

    #[test]
    fn gaussian_noise_different_seeds_differ() {
        let mut n1 = GaussianNoise::isotropic(1e-3, 42);
        let mut n2 = GaussianNoise::isotropic(1e-3, 99);
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert_ne!(n1.apply(v), n2.apply(v));
    }

    #[test]
    fn gaussian_noise_zero_sigma_is_identity() {
        let mut n = GaussianNoise::isotropic(0.0, 42);
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert_eq!(n.apply(v), v);
    }

    #[test]
    fn bias_random_walk_accumulates() {
        let mut brw = BiasRandomWalk::isotropic(1e-4, 1.0, 42);
        let v = Vector3::zeros();
        let first = brw.apply(v);
        let second = brw.apply(v);
        // Bias should accumulate: second call adds to existing bias.
        // They should differ (with high probability).
        assert_ne!(first, second);
    }

    #[test]
    fn bias_random_walk_is_deterministic() {
        let mut b1 = BiasRandomWalk::isotropic(1e-4, 1.0, 42);
        let mut b2 = BiasRandomWalk::isotropic(1e-4, 1.0, 42);
        let v = Vector3::new(1.0, 2.0, 3.0);
        assert_eq!(b1.apply(v), b2.apply(v));
        assert_eq!(b1.apply(v), b2.apply(v));
    }

    #[test]
    fn gaussian_noise_magnitude_is_reasonable() {
        let sigma = 1e-5;
        let mut n = GaussianNoise::isotropic(sigma, 42);
        let v = Vector3::zeros();
        // Run many samples and check the standard deviation is roughly correct.
        let n_samples = 10_000;
        let mut sum_sq = 0.0;
        for _ in 0..n_samples {
            let noisy = n.apply(v);
            sum_sq += noisy.magnitude_squared();
        }
        // Expected variance per axis = sigma^2, so total magnitude^2 ~ 3*sigma^2.
        let empirical_rms = (sum_sq / n_samples as f64).sqrt();
        let expected_rms = (3.0_f64).sqrt() * sigma;
        assert!(
            (empirical_rms - expected_rms).abs() < 0.5 * expected_rms,
            "empirical RMS {empirical_rms:.3e} too far from expected {expected_rms:.3e}"
        );
    }
}
