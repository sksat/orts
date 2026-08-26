use crate::OrbitalState;
use arika::epoch::Epoch;
use arika::frame::{self, Body, Rotation};
use nalgebra::{Matrix3, UnitQuaternion, Vector3};

/// A target attitude reference that provides desired orientation and angular velocity.
///
/// Implementations define different pointing strategies (inertial hold, nadir pointing, etc.).
///
/// The type parameter `F` is the inertial frame of the observed orbital state
/// (default `SimpleEci`). A reference whose geometry is built from the state
/// vectors alone ([`NadirPointing`]) implements it for every `F`. One that
/// holds a direction of its own carries that direction's frame in its type and
/// implements the trait only for that frame — [`InertialPointing<F>`](InertialPointing) holds a
/// [`Rotation<Body, F>`](Rotation); a Sun-pointing or ground-target reference
/// would work the same way.
pub trait AttitudeReference<F: frame::Eci = frame::SimpleEci>: Send + Sync {
    /// Compute the target orientation and angular velocity at time `t`.
    ///
    /// Returns `(q_target, omega_target)` where:
    /// - `q_target` is the desired body→`F` rotation. The frame is part of the
    ///   type: the same physical attitude has different quaternion components
    ///   in different inertial frames, so an untagged quaternion could not say
    ///   which one it meant.
    /// - `omega_target` is the desired angular velocity in the *target* body
    ///   frame [rad/s] — a frame of its own, distinct from the spacecraft's
    ///   current body frame, so it is left untagged here.
    fn target(
        &self,
        t: f64,
        orbit: &OrbitalState<F>,
        epoch: Option<&Epoch>,
    ) -> (Rotation<Body, F>, Vector3<f64>);
}

/// Inertial pointing: hold a fixed orientation in the inertial frame `F`.
///
/// The target is a [`Rotation<Body, F>`](Rotation), so the frame it was
/// expressed in travels with it: the same `InertialPointing` value cannot be
/// used to steer a system integrated in another inertial frame, where those
/// quaternion components would denote a different physical attitude (the frames
/// differ by precession/nutation, ~0.1° at the pole by 2024).
pub struct InertialPointing<F = frame::SimpleEci> {
    pub target_q: Rotation<Body, F>,
}

// Frame-generic now that the target names its own frame: the impl is only
// reachable for the `F` the target was built in.
impl<F: frame::Eci> AttitudeReference<F> for InertialPointing<F> {
    fn target(
        &self,
        _t: f64,
        _orbit: &OrbitalState<F>,
        _epoch: Option<&Epoch>,
    ) -> (Rotation<Body, F>, Vector3<f64>) {
        (self.target_q, Vector3::zeros())
    }
}

/// Nadir pointing: align the body Z-axis with nadir (toward Earth center).
///
/// LVLH (Local Vertical Local Horizontal) frame definition:
/// - Z_lvlh = -r/|r| (nadir direction)
/// - Y_lvlh = -(r × v)/|r × v| (negative orbit normal)
/// - X_lvlh = Y_lvlh × Z_lvlh (approximately along velocity for circular orbits)
///
/// The target angular velocity in the LVLH body frame is `[0, -n, 0]` where
/// `n = |r × v| / r²` is the instantaneous angular rate.
pub struct NadirPointing;

// The LVLH triad is built from the position and velocity of the state itself,
// so the reference is valid in whichever inertial frame `F` they are given in.
impl<F: frame::Eci> AttitudeReference<F> for NadirPointing {
    fn target(
        &self,
        _t: f64,
        orbit: &OrbitalState<F>,
        _epoch: Option<&Epoch>,
    ) -> (Rotation<Body, F>, Vector3<f64>) {
        let r = *orbit.position();
        let v = *orbit.velocity();
        let r_mag = r.magnitude();

        // Angular momentum vector
        let h = r.cross(&v);
        let h_mag = h.magnitude();

        // LVLH frame axes in inertial coordinates
        let z_lvlh = -r / r_mag; // nadir
        let y_lvlh = -h / h_mag; // negative orbit normal
        let x_lvlh = y_lvlh.cross(&z_lvlh); // ~along velocity

        // Rotation matrix from LVLH body to inertial: columns are LVLH axes in inertial frame
        let r_lvlh_to_inertial = Matrix3::from_columns(&[x_lvlh, y_lvlh, z_lvlh]);
        let q_target = UnitQuaternion::from_rotation_matrix(
            &nalgebra::Rotation3::from_matrix_unchecked(r_lvlh_to_inertial),
        );

        // Instantaneous angular rate: n = |h| / r²
        let n = h_mag / (r_mag * r_mag);

        // Angular velocity of LVLH frame in LVLH body frame: [0, -n, 0]
        let omega_target = Vector3::new(0.0, -n, 0.0);

        (Rotation::<Body, F>::from_raw(q_target), omega_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A target orientation with all four components distinct and non-zero, so
    /// a component-wise snapshot cannot pass by symmetry.
    fn nontrivial_target() -> UnitQuaternion<f64> {
        UnitQuaternion::from_axis_angle(
            &nalgebra::Unit::new_normalize(Vector3::new(-0.2, 0.7, 0.4)),
            1.1,
        )
    }

    fn snapshot_orbit() -> OrbitalState {
        OrbitalState::new(
            Vector3::new(4000.0, -5000.0, 2500.0),
            Vector3::new(1.0, 2.0, 7.0),
        )
    }

    /// Characterization: the `InertialPointing` target is handed back
    /// component-for-component (no renormalization, no frame rotation), and the
    /// target rate is exactly zero. Pins the values so that typing the target
    /// by its inertial frame cannot change them.
    #[test]
    fn inertial_pointing_target_components_snapshot() {
        let q = nontrivial_target();
        let ref_point = InertialPointing {
            target_q: Rotation::<Body, frame::SimpleEci>::from_raw(q),
        };
        let (q_out, omega_out) = ref_point.target(0.0, &snapshot_orbit(), None);
        let q_out = q_out.into_inner();

        // (w, x, y, z), Hamilton scalar-first.
        let expected = nalgebra::Vector4::new(
            0.8525245220595057,
            -0.12584829590370833,
            0.44046903566297907,
            0.25169659180741666,
        );
        let got = nalgebra::Vector4::new(q_out.w, q_out.i, q_out.j, q_out.k);
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude(),
            "InertialPointing target changed: {got:?}"
        );
        // A rate of exactly zero is the definition of an inertial hold, not a
        // near-zero numerical outcome, so assert it exactly.
        assert_eq!(omega_out, Vector3::zeros());
    }

    /// Characterization: `NadirPointing` in `SimpleEci`, pinned component-wise
    /// on an eccentric, out-of-plane state (so every component is non-zero).
    #[test]
    fn nadir_pointing_target_components_snapshot() {
        let (q_out, omega_out) = NadirPointing.target(0.0, &snapshot_orbit(), None);
        let q_out = q_out.into_inner();
        let got = nalgebra::Vector4::new(q_out.w, q_out.i, q_out.j, q_out.k);

        // Sign of a quaternion is a gauge freedom; compare through the
        // rotation it denotes by pinning the LVLH axes instead.
        let m = q_out.to_rotation_matrix();
        let r = snapshot_orbit();
        let z_body = m * Vector3::new(0.0, 0.0, 1.0);
        let expected_z = -r.position().normalize();
        assert!(
            (z_body - expected_z).magnitude() <= 1e-12 * expected_z.magnitude(),
            "nadir axis changed: {z_body:?}"
        );
        let h = r.position().cross(r.velocity());
        let y_body = m * Vector3::new(0.0, 1.0, 0.0);
        let expected_y = -h.normalize();
        assert!(
            (y_body - expected_y).magnitude() <= 1e-12 * expected_y.magnitude(),
            "orbit-normal axis changed: {y_body:?}"
        );
        // The rate is the instantaneous |h|/r², non-zero for this state.
        let n = h.magnitude() / r.position().magnitude_squared();
        assert!(
            n > 1e-4,
            "fixture must produce a non-zero rate, got {n:.3e}"
        );
        let expected_omega = Vector3::new(0.0, -n, 0.0);
        assert!(
            (omega_out - expected_omega).magnitude() <= 1e-12 * expected_omega.magnitude(),
            "nadir rate changed: {omega_out:?}"
        );
        assert!(got.iter().all(|c| c.is_finite()));
    }

    /// Characterization: a non-finite attitude target is passed through rather
    /// than sanitized or panicked on. Pins the behavior of the (predicate-free)
    /// reference path for `NaN` / `±∞` inputs.
    #[test]
    fn non_finite_states_do_not_panic() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            // Inertial hold: a non-finite target normalizes to NaN.
            let q = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(bad, 0.0, 0.0, 1.0));
            let (q_out, omega_out) = InertialPointing {
                target_q: Rotation::<Body, frame::SimpleEci>::from_raw(q),
            }
            .target(0.0, &snapshot_orbit(), None);
            let q_out = q_out.into_inner();
            assert!(q_out.w.is_nan(), "expected NaN scalar part for {bad}");
            assert_eq!(omega_out, Vector3::zeros());

            // Nadir pointing: a non-finite state propagates to a NaN target.
            let orbit = OrbitalState::new(
                Vector3::new(bad, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            );
            let (q_nadir, omega_nadir) = NadirPointing.target(0.0, &orbit, None);
            let q_nadir = q_nadir.into_inner();
            assert!(
                !q_nadir.w.is_finite()
                    || !q_nadir.i.is_finite()
                    || !q_nadir.j.is_finite()
                    || !q_nadir.k.is_finite(),
                "expected a non-finite nadir target for {bad}, got {q_nadir:?}"
            );
            assert!(
                omega_nadir[1].is_nan(),
                "expected NaN rate for {bad}, got {omega_nadir:?}"
            );
        }
    }

    #[test]
    fn inertial_pointing_returns_fixed_target() {
        let axis = nalgebra::Unit::new_normalize(Vector3::new(0.0, 0.0, 1.0));
        let q = UnitQuaternion::from_axis_angle(&axis, PI / 4.0);
        let ref_point = InertialPointing {
            target_q: Rotation::<Body, frame::SimpleEci>::from_raw(q),
        };

        let orbit = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));

        let (q_out, omega_out) = ref_point.target(0.0, &orbit, None);
        assert!((q_out.into_inner().angle() - PI / 4.0).abs() < 1e-14);
        assert!(omega_out.magnitude() < 1e-15);
    }

    #[test]
    fn nadir_z_axis_points_toward_earth() {
        let orbit = OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0));

        let nadir = NadirPointing;
        let (q_target, _omega) = nadir.target(0.0, &orbit, None);

        // The body Z-axis in inertial frame should point nadir (toward -r)
        let r_mat = q_target.into_inner().to_rotation_matrix();
        let z_body_inertial = r_mat * Vector3::new(0.0, 0.0, 1.0);

        let r_hat = orbit.position().normalize();
        // Z_lvlh = -r/|r|, so body Z should be -r_hat
        let expected = -r_hat;
        let error = (z_body_inertial - expected).magnitude();
        assert!(
            error < 1e-14,
            "Body Z should point nadir, error: {error:.2e}"
        );
    }

    #[test]
    fn nadir_omega_target_circular_orbit() {
        let mu: f64 = 398600.4418;
        let r = 7000.0;
        let v_circ = (mu / r).sqrt();

        let orbit = OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v_circ, 0.0));

        let nadir = NadirPointing;
        let (_q_target, omega_target) = nadir.target(0.0, &orbit, None);

        // n = |h| / r² = r*v / r² = v/r for circular orbit
        let n_expected = v_circ / r;

        // omega_target should be [0, -n, 0]
        assert!(
            omega_target[0].abs() < 1e-15,
            "omega_x should be 0, got {}",
            omega_target[0]
        );
        assert!(
            (omega_target[1] + n_expected).abs() < 1e-12,
            "omega_y should be -{n_expected}, got {}",
            omega_target[1]
        );
        assert!(
            omega_target[2].abs() < 1e-15,
            "omega_z should be 0, got {}",
            omega_target[2]
        );
    }

    #[test]
    fn nadir_orthonormal_frame() {
        // Verify the LVLH frame is orthonormal
        let orbit = OrbitalState::new(
            Vector3::new(5000.0, 3000.0, 1000.0),
            Vector3::new(-1.0, 6.0, 2.0),
        );

        let nadir = NadirPointing;
        let (q_target, _) = nadir.target(0.0, &orbit, None);

        // The rotation matrix should be orthonormal
        let r_mat = q_target.into_inner().to_rotation_matrix();
        let m = r_mat.matrix();
        let identity = m.transpose() * m;

        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (identity[(i, j)] - expected).abs() < 1e-14,
                    "R^T R[{i},{j}] = {}, expected {expected}",
                    identity[(i, j)]
                );
            }
        }
    }
}
