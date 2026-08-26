//! Star tracker (STT) sensor.
//!
//! Measures the spacecraft's attitude quaternion. The ideal sensor
//! returns the true quaternion; noisy versions apply a small random
//! rotation perturbation to model pointing error.

use arika::epoch::Epoch;
use arika::frame::{Body, Rotation};
use nalgebra::{UnitQuaternion, Vector3};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
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

    /// Measure the attitude quaternion (body→inertial), for a `SimpleEci` state.
    pub fn measure(&mut self, state: &SpacecraftState, epoch: &Epoch) -> AttitudeBodyToInertial {
        self.measure_in_frame::<arika::frame::SimpleEci>(state, epoch)
    }

    /// Measure the attitude quaternion (body→inertial) for a state propagated in
    /// an arbitrary inertial frame `F`.
    ///
    /// The reading is the body→`F` rotation, and its components depend on `F`:
    /// the same physical attitude has different quaternion components in
    /// `SimpleEci` and in `Gcrs`. The returned
    /// [`AttitudeBodyToInertial<F>`](AttitudeBodyToInertial) carries that frame
    /// in its type, so a `Gcrs` reading cannot be consumed as `SimpleEci`.
    ///
    /// The *components* are the same either way — [`crate::attitude::AttitudeState`]
    /// stores an untyped quaternion, and this re-tags it with the frame the
    /// state was propagated in. That is exactly why the tag has to travel with
    /// the value: the numbers alone do not say which frame they belong to.
    pub fn measure_in_frame<F: arika::frame::Eci>(
        &mut self,
        state: &SpacecraftState<F>,
        _epoch: &Epoch,
    ) -> AttitudeBodyToInertial<F> {
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

        AttitudeBodyToInertial::new(Rotation::<Body, F>::from_raw(q_measured))
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
    use nalgebra::Vector4;

    /// The four components `(w, x, y, z)` of a reading, for value assertions.
    /// Available in any frame: the point of the frame tag is that the caller
    /// must name a frame to *interpret* these numbers, not that it cannot read
    /// them.
    fn components<F>(a: &AttitudeBodyToInertial<F>) -> Vector4<f64> {
        let q = a.inner().inner();
        Vector4::new(q.w, q.i, q.j, q.k)
    }

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
        assert_eq!(components(&q), state.attitude.quaternion);
    }

    #[test]
    fn noisy_differs_from_true() {
        // 10 arcsec ≈ 5e-5 rad
        let mut stt = StarTracker::new().with_pointing_noise_isotropic(5e-5, 42);
        let state = make_state();
        let epoch = Epoch::j2000();
        let q = stt.measure(&state, &epoch);
        assert_ne!(components(&q), state.attitude.quaternion);
        // Should still be close to unit quaternion.
        let q = components(&q);
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

    // Characterization for the frame-typed attitude reading (#332)

    /// A state whose attitude has four distinct, non-zero quaternion
    /// components, so a component-wise snapshot cannot pass by symmetry.
    fn nontrivial_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
            attitude: AttitudeState::new(
                UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            mass: 50.0,
        }
    }

    fn assert_close(got: Vector4<f64>, expected: Vector4<f64>, what: &str) {
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude(),
            "{what} changed: {got:?}"
        );
    }

    /// Characterization: the ideal reading is the state quaternion, component
    /// for component, in `[w, x, y, z]` order.
    #[test]
    fn ideal_measurement_components_snapshot() {
        let mut stt = StarTracker::new();
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let got = components(&stt.measure(&nontrivial_state(), &epoch));
        assert_close(
            got,
            Vector4::new(
                0.9393727128473789,
                0.10391372781674944,
                -0.17318954636124906,
                0.27710327417799857,
            ),
            "ideal star tracker reading",
        );
    }

    /// Characterization: the noise draw itself — RNG stream, per-axis σ order,
    /// and the `q_true * δq` composition order — is pinned numerically, for
    /// both the isotropic and the anisotropic constructor.
    #[test]
    fn noisy_measurement_components_snapshot() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        let mut isotropic = StarTracker::new().with_pointing_noise_isotropic(5e-5, 42);
        assert_close(
            components(&isotropic.measure(&nontrivial_state(), &epoch)),
            Vector4::new(
                0.9393712890258086,
                0.10391330045435347,
                -0.17318662555980863,
                0.277110086553847,
            ),
            "isotropic noisy star tracker reading",
        );

        let mut anisotropic =
            StarTracker::new().with_pointing_noise(Vector3::new(1e-5, 5e-5, 2e-4), 7);
        assert_close(
            components(&anisotropic.measure(&nontrivial_state(), &epoch)),
            Vector4::new(
                0.939390464493437,
                0.10392513965336554,
                -0.1731971454146844,
                0.2770340581716219,
            ),
            "anisotropic noisy star tracker reading",
        );
    }

    /// Characterization: `measure_in_frame` is a pure re-tag — the components
    /// are the state quaternion's, whatever inertial frame is named, because
    /// [`AttitudeState`] stores an untyped quaternion. This is precisely why the
    /// reading must carry its frame in the type: the numbers alone say nothing.
    #[test]
    fn measurement_components_are_identical_in_every_frame() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let state = nontrivial_state();

        let simple = components(
            &StarTracker::new().measure_in_frame::<arika::frame::SimpleEci>(&state, &epoch),
        );
        let gcrs_state = SpacecraftState::<arika::frame::Gcrs> {
            orbit: OrbitalState::<arika::frame::Gcrs>::new_in_frame(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
            attitude: state.attitude.clone(),
            mass: state.mass,
        };
        let gcrs = components(
            &StarTracker::new().measure_in_frame::<arika::frame::Gcrs>(&gcrs_state, &epoch),
        );
        assert_eq!(simple, gcrs);
    }

    /// Characterization: a non-finite attitude yields a non-finite reading
    /// rather than a panic, on both the ideal and the noisy path.
    #[test]
    fn non_finite_attitude_yields_non_finite_reading() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let state = SpacecraftState {
                orbit: OrbitalState::new(
                    Vector3::new(7000.0, 0.0, 0.0),
                    Vector3::new(0.0, 7.5, 0.0),
                ),
                attitude: AttitudeState {
                    quaternion: Vector4::new(bad, 0.0, 0.0, 1.0),
                    angular_velocity: Vector3::zeros(),
                },
                mass: 50.0,
            };
            let ideal = components(&StarTracker::new().measure(&state, &epoch));
            assert!(
                ideal.iter().all(|c| c.is_nan()),
                "expected an all-NaN ideal reading for {bad}, got {ideal:?}"
            );
            let noisy = components(
                &StarTracker::new()
                    .with_pointing_noise_isotropic(5e-5, 42)
                    .measure(&state, &epoch),
            );
            assert!(
                noisy.iter().all(|c| c.is_nan()),
                "expected an all-NaN noisy reading for {bad}, got {noisy:?}"
            );
        }
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
            let q_meas = components(&stt.measure(&state, &epoch));
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
