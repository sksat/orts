//! Sensor noise models.
//!
//! [`NoiseModel`] trait defines the interface for injecting noise into
//! sensor measurements. Implementations are composed into sensor structs
//! (e.g. `Magnetometer`, `Gyroscope`) via an optional field.
//!
//! A noise model's value is a function of its seed and the sim time it is
//! asked about, whatever order or how often it is asked (DESIGN.md
//! "センサの noise は seed と時刻で決める"), so a reading can be evaluated again
//! later and come out the same.

use nalgebra::Vector3;

/// Noise model that transforms a true measurement into a noisy one.
///
/// The result depends on the model's configuration, the sim time `t` [s] and
/// `true_value` only: calling it again for the same `t`, in any order and at
/// any interval, gives the same value. `&mut self` is for caches, which do not
/// change what is returned. `Send` is required so sensors can be used in
/// per-satellite worker threads.
pub trait NoiseModel: Send {
    /// Apply noise to a true 3-axis measurement taken at sim time `t` [s].
    ///
    /// # Panics
    /// Implementations panic on a non-finite `t`.
    fn apply(&mut self, t: f64, true_value: Vector3<f64>) -> Vector3<f64>;
}

/// Deterministic random numbers drawn from a key.
///
/// SplitMix64 rather than a seeded `rand` generator, whose stream is not
/// stable across `rand` releases: the values here are part of the noise
/// models' contract.
pub(crate) mod keyed {
    /// One SplitMix64 output for `x`.
    fn mix(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }

    /// Hash the key parts in order, each through a full mixing round, so that
    /// parts in different positions do not cancel.
    fn hash(parts: &[u64]) -> u64 {
        parts.iter().fold(0, |h, &p| mix(h ^ p))
    }

    /// A uniform number in the open interval (0, 1), from the top 52 bits.
    ///
    /// The midpoints of a 52-bit grid: with 53 bits the top midpoint,
    /// `1 - 2^-54`, is not representable and rounds to `1.0`, which would give
    /// Box–Muller a zero radius.
    pub(crate) fn uniform_open(x: u64) -> f64 {
        // 2^-52: one step of the 52-bit grid; the half step keeps 0 and 1 out.
        const STEP: f64 = 1.0 / (1u64 << 52) as f64;
        ((x >> 12) as f64 + 0.5) * STEP
    }

    /// Refuse a sample time a noise model could not be keyed on, for the
    /// sensors that have no noise model to refuse it themselves.
    ///
    /// # Panics
    /// Panics on a non-finite `t`.
    pub(crate) fn check_sample_time(t: f64) {
        time_bits(t);
    }

    /// The bit pattern of a sim time used as a key, with `-0.0` read as `0.0`.
    ///
    /// # Panics
    /// Panics on a non-finite `t`.
    pub(crate) fn time_bits(t: f64) -> u64 {
        assert!(t.is_finite(), "noise sim time must be finite, got {t}");
        if t == 0.0 {
            0.0f64.to_bits()
        } else {
            t.to_bits()
        }
    }

    /// A standard normal number for the key, by Box–Muller.
    pub(crate) fn standard_normal(parts: &[u64]) -> f64 {
        let h = hash(parts);
        // Two lanes of the same key: the two uniforms Box–Muller needs.
        let u1 = uniform_open(mix(h ^ 1));
        let u2 = uniform_open(mix(h ^ 2));
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// Tags that keep the models' keys apart, so two models with one seed do not
/// draw the same numbers.
const GAUSSIAN_TAG: u64 = 0x6761_7573_7369_616E; // "gaussian"
const RANDOM_WALK_TAG: u64 = 0x7261_6E64_7761_6C6B; // "randwalk"

/// Additive Gaussian white noise.
///
/// Each axis gets independent zero-mean Gaussian noise with the configured
/// standard deviation (sigma), drawn from the seed and the sim time: the same
/// time gives the same noise, and two different times independent noise.
///
/// ```text
/// noisy = true_value + N(0, sigma)
/// ```
pub struct GaussianNoise {
    sigma: Vector3<f64>,
    seed: u64,
}

impl GaussianNoise {
    /// Create a Gaussian noise model with per-axis standard deviations.
    ///
    /// # Panics
    /// Panics if a component of `sigma` is negative or non-finite.
    pub fn new(sigma: Vector3<f64>, seed: u64) -> Self {
        assert!(
            sigma.iter().all(|s| s.is_finite() && *s >= 0.0),
            "sigma must be non-negative and finite, got {sigma:?}"
        );
        Self { sigma, seed }
    }

    /// Create with the same sigma for all three axes.
    pub fn isotropic(sigma: f64, seed: u64) -> Self {
        Self::new(Vector3::new(sigma, sigma, sigma), seed)
    }
}

impl NoiseModel for GaussianNoise {
    fn apply(&mut self, t: f64, true_value: Vector3<f64>) -> Vector3<f64> {
        let bits = keyed::time_bits(t);
        let n = |axis: u64| keyed::standard_normal(&[GAUSSIAN_TAG, self.seed, bits, axis]);
        true_value + self.sigma.component_mul(&Vector3::new(n(0), n(1), n(2)))
    }
}

/// Bias random walk (Wiener process on the bias vector).
///
/// Models a slowly drifting bias, piecewise constant on a grid `t_k = k·dt`
/// anchored at `t = 0`: the bias steps at each grid point and holds until the
/// next, so it is right-continuous.
///
/// ```text
/// bias(t) = W(floor(t / dt)),  W(k) - W(k-1) ~ N(0, sigma_drift² · dt) independent
/// noisy = true_value + bias(t)
/// ```
///
/// `W` at a grid point is drawn directly, by Lévy's midpoint construction of
/// Brownian motion: the end of `[0, 2^LEVELS]` is drawn first, then the
/// midpoint of each interval given its ends (a Brownian bridge), descending
/// to the point asked for. Each draw is keyed on the seed and its interval, so
/// the value at any time is the same however the walk is queried, and costs
/// `LEVELS` draws per axis wherever the time lies. The values are those of a
/// Wiener process at the grid points, not an approximation of one.
///
/// The bias is zero before the first grid point after `t = 0`, negative times
/// included. This is a standard gyroscope bias instability model. The drift
/// rate `sigma_drift` has units of \[measurement unit / sqrt(s)\].
pub struct BiasRandomWalk {
    step_sigma: Vector3<f64>,
    dt: f64,
    seed: u64,
}

/// Depth of the random walk's midpoint construction: the grid covers
/// `2^LEVELS` steps, 8900 years at a 1 ms step. Rounding grows with the root
/// value, `2^(LEVELS/2)` steps' standard deviation, so the error at a grid
/// point stays near `2^(LEVELS/2) · ε ≈ 2e-9` of one step's.
const LEVELS: u32 = 48;

/// Kinds of draw in the random walk's construction, so the root interval's end
/// and its midpoint, keyed on the same interval, are independent.
const ROOT_END: u64 = 0;
const MIDPOINT: u64 = 1;

impl BiasRandomWalk {
    /// Create a bias random walk model.
    ///
    /// - `sigma_drift`: drift rate per axis \[unit / sqrt(s)\]
    /// - `dt`: grid step of the walk \[s\]
    /// - `seed`: RNG seed for reproducibility
    ///
    /// # Panics
    /// Panics if `dt` is not positive and finite, or a component of
    /// `sigma_drift` is negative or non-finite.
    pub fn new(sigma_drift: Vector3<f64>, dt: f64, seed: u64) -> Self {
        assert!(
            dt.is_finite() && dt > 0.0,
            "dt must be positive and finite, got {dt}"
        );
        assert!(
            sigma_drift.iter().all(|s| s.is_finite() && *s >= 0.0),
            "sigma_drift must be non-negative and finite, got {sigma_drift:?}"
        );
        Self {
            step_sigma: sigma_drift * dt.sqrt(),
            dt,
            seed,
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

    /// The index of the last grid point at or before `t`, `0` before the first.
    ///
    /// `floor(t / dt)`, corrected by one either way where the division rounds
    /// across an integer, so a time computed as `k · dt` lands on `k`.
    ///
    /// # Panics
    /// Panics if `t` lies beyond the `2^LEVELS` steps the grid covers.
    fn grid_index(&self, t: f64) -> u64 {
        if t < self.dt {
            return 0;
        }
        let end = (1u64 << LEVELS) as f64;
        let mut k = (t / self.dt).floor();
        assert!(
            k < end,
            "t = {t} s is beyond the random walk's 2^{LEVELS} steps of {} s",
            self.dt
        );
        if (k + 1.0) * self.dt <= t {
            k += 1.0;
        } else if k * self.dt > t {
            k -= 1.0;
        }
        k as u64
    }

    /// A standard normal vector for one draw of the construction: the end of
    /// the root interval (`ROOT_END`) or the midpoint of `[a, b]` (`MIDPOINT`).
    fn draw(&self, kind: u64, a: u64, b: u64) -> Vector3<f64> {
        let n = |axis: u64| keyed::standard_normal(&[RANDOM_WALK_TAG, self.seed, kind, a, b, axis]);
        Vector3::new(n(0), n(1), n(2))
    }

    /// `W(k)`, by descending from `[0, 2^LEVELS]` to `k`.
    fn walk_at(&self, k: u64) -> Vector3<f64> {
        let (mut a, mut b) = (0u64, 1u64 << LEVELS);
        let (mut wa, mut wb) = (
            Vector3::zeros(),
            self.step_sigma.component_mul(&self.draw(ROOT_END, a, b)) * (b as f64).sqrt(),
        );
        while k != a && k != b {
            let m = a + (b - a) / 2;
            // The midpoint of a Brownian bridge over `b - a` steps: the mean of
            // its ends, with variance `(b - a) / 4` steps.
            let wm = (wa + wb) / 2.0
                + self.step_sigma.component_mul(&self.draw(MIDPOINT, a, b))
                    * ((b - a) as f64).sqrt()
                    / 2.0;
            if k < m {
                (b, wb) = (m, wm);
            } else {
                (a, wa) = (m, wm);
            }
        }
        if k == a { wa } else { wb }
    }
}

impl NoiseModel for BiasRandomWalk {
    fn apply(&mut self, t: f64, true_value: Vector3<f64>) -> Vector3<f64> {
        keyed::time_bits(t);
        true_value + self.walk_at(self.grid_index(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Vector3<f64> = Vector3::new(1.0, 2.0, 3.0);

    /// The uniform stays inside (0, 1) at both ends of the hash's range, so
    /// Box–Muller never takes `ln` of 0 or a zero radius from 1.
    #[test]
    fn keyed_uniform_is_strictly_inside_the_unit_interval() {
        let (low, high) = (keyed::uniform_open(0), keyed::uniform_open(u64::MAX));
        assert!(low > 0.0, "{low}");
        assert!(high < 1.0, "{high}");
    }

    #[test]
    fn gaussian_noise_is_a_function_of_the_time() {
        let mut n = GaussianNoise::isotropic(1e-5, 42);
        let first = n.apply(3.0, V);
        let _ = n.apply(4.0, V);
        let _ = n.apply(1.0, V);
        assert_eq!(n.apply(3.0, V), first, "the same time reads the same");
        // A fresh model asked only about t = 3 agrees with the one asked about
        // other times in between.
        assert_eq!(GaussianNoise::isotropic(1e-5, 42).apply(3.0, V), first);
    }

    #[test]
    fn gaussian_noise_differs_between_times_and_seeds() {
        let mut n = GaussianNoise::isotropic(1e-3, 42);
        assert_ne!(n.apply(1.0, V), n.apply(2.0, V));
        assert_ne!(
            n.apply(1.0, V),
            GaussianNoise::isotropic(1e-3, 99).apply(1.0, V)
        );
    }

    #[test]
    fn gaussian_noise_axes_differ() {
        let noisy = GaussianNoise::isotropic(1.0, 42).apply(5.0, Vector3::zeros());
        assert!(noisy.x != noisy.y && noisy.y != noisy.z, "{noisy:?}");
    }

    #[test]
    fn gaussian_noise_zero_sigma_is_identity() {
        assert_eq!(GaussianNoise::isotropic(0.0, 42).apply(1.0, V), V);
    }

    #[test]
    fn negative_zero_time_reads_as_zero() {
        let mut n = GaussianNoise::isotropic(1.0, 42);
        assert_eq!(n.apply(-0.0, V), n.apply(0.0, V));
    }

    #[test]
    #[should_panic(expected = "noise sim time must be finite")]
    fn gaussian_noise_rejects_a_nan_time() {
        let _ = GaussianNoise::isotropic(1.0, 42).apply(f64::NAN, V);
    }

    #[test]
    #[should_panic(expected = "noise sim time must be finite")]
    fn bias_random_walk_rejects_an_infinite_time() {
        let _ = BiasRandomWalk::isotropic(1.0, 1.0, 42).apply(f64::INFINITY, V);
    }

    #[test]
    #[should_panic(expected = "sigma must be non-negative and finite")]
    fn gaussian_noise_rejects_a_negative_sigma() {
        let _ = GaussianNoise::isotropic(-1.0, 42);
    }

    /// The sample mean and standard deviation over many times match sigma, and
    /// neighbouring times are uncorrelated.
    #[test]
    fn gaussian_noise_has_the_configured_distribution() {
        let sigma = 1e-5;
        let mut n = GaussianNoise::new(Vector3::new(sigma, 2.0 * sigma, 0.5 * sigma), 7);
        let samples: Vec<Vector3<f64>> = (0..20_000)
            .map(|i| n.apply(i as f64 * 0.1, Vector3::zeros()))
            .collect();
        let count = samples.len() as f64;
        for (axis, s) in [(0, sigma), (1, 2.0 * sigma), (2, 0.5 * sigma)] {
            let xs: Vec<f64> = samples.iter().map(|v| v[axis]).collect();
            let mean = xs.iter().sum::<f64>() / count;
            let sd = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / count).sqrt();
            // 4 standard errors of the mean and of the standard deviation.
            assert!(
                mean.abs() < 4.0 * s / count.sqrt(),
                "axis {axis} mean {mean:e}"
            );
            assert!(
                (sd - s).abs() < 4.0 * s / (2.0 * count).sqrt(),
                "axis {axis} sd {sd:e} vs {s:e}"
            );
            let lag1 = xs
                .windows(2)
                .map(|w| (w[0] - mean) * (w[1] - mean))
                .sum::<f64>()
                / (count - 1.0)
                / (sd * sd);
            assert!(
                lag1.abs() < 4.0 / count.sqrt(),
                "axis {axis} lag-1 correlation {lag1}"
            );
        }
    }

    #[test]
    fn bias_random_walk_is_zero_before_the_first_grid_point() {
        let mut b = BiasRandomWalk::isotropic(1.0, 1.0, 42);
        assert_eq!(b.apply(0.0, V), V);
        assert_eq!(b.apply(0.999, V), V);
        assert_eq!(b.apply(-5.0, V), V);
        assert_ne!(b.apply(1.0, V), V, "the first step lands at t = dt");
    }

    /// The bias holds between grid points and steps at them.
    #[test]
    fn bias_random_walk_is_piecewise_constant_on_its_grid() {
        let mut b = BiasRandomWalk::isotropic(1.0, 0.5, 42);
        assert_eq!(b.apply(1.0, V), b.apply(1.49, V));
        assert_ne!(b.apply(1.49, V), b.apply(1.5, V));
    }

    /// The same time reads the same however the walk was queried before:
    /// forward, after jumping back, or from a fresh model.
    #[test]
    fn bias_random_walk_is_a_function_of_the_time() {
        let mut forward = BiasRandomWalk::isotropic(1e-3, 0.1, 42);
        let mut at = Vec::new();
        for i in 1..=50 {
            at.push(forward.apply(i as f64 * 0.1, Vector3::zeros()));
        }
        let mut jumping = BiasRandomWalk::isotropic(1e-3, 0.1, 42);
        assert_eq!(jumping.apply(5.0, Vector3::zeros()), at[49]);
        assert_eq!(
            jumping.apply(2.0, Vector3::zeros()),
            at[19],
            "after going back"
        );
        assert_eq!(
            jumping.apply(3.05, Vector3::zeros()),
            at[29],
            "between grid points"
        );
    }

    /// A time far along the grid costs no more than a near one and reads the
    /// same whatever was read before it.
    #[test]
    fn bias_random_walk_reads_a_far_time_directly() {
        let far = (1u64 << 47) as f64 + 3.0;
        let mut b = BiasRandomWalk::isotropic(1e-3, 1.0, 42);
        let at_far = b.apply(far, Vector3::zeros());
        let _ = b.apply(5.0, Vector3::zeros());
        assert_eq!(b.apply(far, Vector3::zeros()), at_far);
        assert!(at_far.iter().all(|v| v.is_finite()));
    }

    #[test]
    #[should_panic(expected = "beyond the random walk")]
    fn bias_random_walk_refuses_a_time_beyond_its_grid() {
        let _ =
            BiasRandomWalk::isotropic(1.0, 1.0, 42).apply((1u64 << 48) as f64, Vector3::zeros());
    }

    /// A step so small that `t / dt` overflows is refused rather than walked.
    #[test]
    #[should_panic(expected = "beyond the random walk")]
    fn bias_random_walk_refuses_an_unrepresentable_grid_index() {
        let _ = BiasRandomWalk::isotropic(1.0, f64::MIN_POSITIVE, 42).apply(1.0, Vector3::zeros());
    }

    /// The increments are N(0, sigma_drift² · dt) and independent of their
    /// neighbours, checked across many seeds at a point inside a dyadic
    /// interval and at one on each side of a boundary of the construction.
    #[test]
    fn bias_random_walk_increments_are_independent_steps() {
        let (sigma, dt) = (0.3, 0.25);
        let step_var = sigma * sigma * dt;
        let seeds = 4_000u64;
        for k in [7u64, 1 << 20] {
            let (mut sum_sq, mut sum_cross) = (0.0, 0.0);
            for seed in 0..seeds {
                let b = BiasRandomWalk::isotropic(sigma, dt, seed);
                let w = |i: u64| b.walk_at(i).x;
                let (d0, d1) = (w(k) - w(k - 1), w(k + 1) - w(k));
                sum_sq += d0 * d0;
                sum_cross += d0 * d1;
            }
            let n = seeds as f64;
            let var = sum_sq / n;
            let corr = sum_cross / n / step_var;
            assert!(
                (var / step_var - 1.0).abs() < 4.0 * (2.0 / n).sqrt(),
                "k = {k}: increment variance {var} vs {step_var}"
            );
            assert!(
                corr.abs() < 4.0 / n.sqrt(),
                "k = {k}: neighbour correlation {corr}"
            );
        }
    }

    /// Times computed as `k · dt` land on grid point `k`, where the division
    /// would round below it.
    #[test]
    fn bias_random_walk_grid_index_lands_on_multiples_of_dt() {
        let dt = 0.1;
        let b = BiasRandomWalk::isotropic(1.0, dt, 42);
        for k in 1..=10_000u64 {
            assert_eq!(b.grid_index(k as f64 * dt), k, "k = {k}");
        }
    }

    /// The steps have variance sigma_drift² · dt, so the bias after n steps has
    /// variance n · sigma_drift² · dt; checked across many seeds.
    #[test]
    fn bias_random_walk_has_the_configured_drift() {
        let (sigma, dt) = (0.2, 0.5);
        // A short walk, and one half way along the construction's grid, where
        // the root interval's end and its first midpoint meet.
        for steps in [40u64, 1 << 47] {
            let finals: Vec<f64> = (0..4_000)
                .map(|seed| BiasRandomWalk::isotropic(sigma, dt, seed).walk_at(steps).x)
                .collect();
            let count = finals.len() as f64;
            let var = finals.iter().map(|x| x * x).sum::<f64>() / count;
            let expected = steps as f64 * sigma * sigma * dt;
            // The sample variance of a normal has relative standard error sqrt(2/n).
            assert!(
                (var / expected - 1.0).abs() < 4.0 * (2.0 / count).sqrt(),
                "{steps} steps: variance {var} vs {expected}"
            );
        }
    }

    #[test]
    fn two_models_with_one_seed_draw_different_numbers() {
        let g = GaussianNoise::isotropic(1.0, 42).apply(1.0, Vector3::zeros());
        let b = BiasRandomWalk::isotropic(1.0, 1.0, 42).apply(1.0, Vector3::zeros());
        assert_ne!(g, b);
    }
}
