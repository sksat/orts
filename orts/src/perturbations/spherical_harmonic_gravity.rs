//! Full spherical-harmonic gravity as a frame-aware perturbation.
//!
//! [`SphericalHarmonicGravity`] wraps a [`SphericalHarmonicField`] (a fully
//! normalized `C̄nm, S̄nm` set such as EGM96 / EGM2008 / EIGEN-6C4, loaded
//! from an ICGEM `.gfc` file) and evaluates its non-central acceleration in
//! the propagation frame `F`.
//!
//! The tesseral and sectorial terms (`m ≥ 1`) are fixed to the rotating
//! Earth, so unlike [`ZonalGravity`](super::ZonalGravity) — which only needs
//! the pole *direction* — this model needs the full Earth-fixed rotation at
//! each instant. That is what the [`EarthFixedTransform`] bound provides:
//!
//! - `SimpleEci`: ERA-only Z rotation, no EOP (the approximate,
//!   visualization-grade path — longitude is off by the missing precession /
//!   nutation / polar motion, ~0.1–0.3°);
//! - `Gcrs`: the full IAU 2006 CIO chain into ITRS, **polar motion included**,
//!   driven by the EOP provider held in `eop` — the path for metre-class
//!   work.
//!
//! # Composition rules
//!
//! - Add it to a system that already carries the central
//!   [`PointMass`](crate::orbital::gravity::PointMass) term: the field
//!   contributes degree ≥ 2 only.
//! - **Not together with `ZonalGravity`**: both contain J2/J3/J4, so the
//!   oblateness would be counted twice. Install one or the other.
//! - Use the field's own [`gm()`](SphericalHarmonicField::gm) for the point
//!   mass. A field's GM differs from WGS-84's `398600.4418` by ~3e-7 relative;
//!   left inconsistent, that alone drifts a LEO orbit by ~100 m/day
//!   along-track, which defeats the purpose of a 70×70 field.
//!
//! # Epoch
//!
//! Earth's rotation angle is meaningless without an absolute epoch, so
//! [`Model::eval`] **panics** if the system has no `epoch_0`. The other
//! frame-aware models fall back to J2000 there; for a longitude-dependent
//! field that fallback would silently rotate the whole geoid to a wrong
//! angle, and there is no useful approximation to offer instead.

use std::sync::Arc;

use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::{Epoch, Utc};
use arika::frame::{self, Vec3};
use nalgebra::Vector3;
use tobari::gravity::SphericalHarmonicField;

use crate::model::{ExternalLoads, HasFrame, HasOrbit, Model};

/// Non-central spherical-harmonic gravity of a
/// [`SphericalHarmonicField`], evaluated through frame `F`'s Earth-fixed
/// transform.
pub struct SphericalHarmonicGravity<F: EarthFixedTransform = frame::SimpleEci> {
    /// The coefficient set (shared: several satellites can hold the same
    /// 70×70 field without copying its ~20 k coefficients each).
    pub field: Arc<SphericalHarmonicField>,
    /// EOP storage for the frame's Earth-fixed transform. `()` for
    /// `SimpleEci`.
    pub eop: F::EopStorage,
}

impl SphericalHarmonicGravity<frame::SimpleEci> {
    /// The ERA-only `SimpleEci` path, which needs no EOP data.
    pub fn for_simple_eci(field: Arc<SphericalHarmonicField>) -> Self {
        Self { field, eop: () }
    }
}

impl<F: EarthFixedTransform> SphericalHarmonicGravity<F> {
    /// Evaluate `field` in frame `F` using `eop` for the Earth-fixed
    /// rotation.
    pub fn new(field: Arc<SphericalHarmonicField>, eop: F::EopStorage) -> Self {
        Self { field, eop }
    }

    /// Non-central acceleration [km/s²] in frame `F` at `position` [km] and
    /// `utc`: rotate into `F::Fixed`, evaluate, rotate back.
    fn acceleration(&self, position: &Vector3<f64>, utc: &Epoch<Utc>) -> Vector3<f64> {
        let orientation = EarthOrientation::new(*utc, &self.eop);
        let fixed_to_inertial = F::fixed_to_inertial(&orientation);
        let pos_fixed = fixed_to_inertial
            .inverse()
            .transform(&Vec3::<F>::from_raw(*position));
        let a_fixed = self.field.acceleration_ecef(&pos_fixed.into_inner());
        fixed_to_inertial
            .transform(&Vec3::<F::Fixed>::from_raw(a_fixed))
            .into_inner()
    }
}

impl<F: EarthFixedTransform, S: HasFrame<Frame = F> + HasOrbit> Model<S>
    for SphericalHarmonicGravity<F>
{
    fn name(&self) -> &str {
        "spherical_harmonic_gravity"
    }

    /// # Panics
    ///
    /// If `epoch` is `None` — see the module docs.
    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        let utc = epoch.expect(
            "SphericalHarmonicGravity needs an absolute epoch (set `epoch_0` on the system): \
             the longitude-dependent terms are fixed to the rotating Earth and have no \
             meaningful value without Earth's rotation angle",
        );
        ExternalLoads::acceleration(self.acceleration(state.orbit().position(), utc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::perturbations::ZonalGravity;
    use crate::test_support::zero_eop;
    use arika::earth::eop::{NutationCorrections, PolarMotion, Ut1Offset};
    use arika::earth::{GcrsEopStorage, J2 as J2_E, J3 as J3_E, J4 as J4_E, MU as MU_E, R as R_E};

    /// J2..J4 as a fully normalized zonal field: C̄n0 = −Jn / √(2n+1).
    fn zonal_field() -> Arc<SphericalHarmonicField> {
        let coeffs = [
            (2, 0, -J2_E / 5.0f64.sqrt(), 0.0),
            (3, 0, -J3_E / 7.0f64.sqrt(), 0.0),
            (4, 0, -J4_E / 9.0f64.sqrt(), 0.0),
        ];
        Arc::new(
            SphericalHarmonicField::from_normalized_coefficients(MU_E, R_E, 4, &coeffs).unwrap(),
        )
    }

    /// A C̄22-only field: its potential is largest along the body-fixed ±x
    /// axis, so the ECI direction of that axis is observable.
    fn c22_field() -> Arc<SphericalHarmonicField> {
        Arc::new(
            SphericalHarmonicField::from_normalized_coefficients(
                MU_E,
                R_E,
                2,
                &[(2, 2, 1e-6, 0.0)],
            )
            .unwrap(),
        )
    }

    fn positions() -> [Vector3<f64>; 5] {
        [
            Vector3::new(6948.0, 0.0, 0.0),
            Vector3::new(4000.0, -3000.0, 5000.0),
            Vector3::new(-5000.0, 2000.0, -3000.0),
            Vector3::new(0.0, 0.0, 7000.0),
            Vector3::new(42164.0, 0.0, 100.0),
        ]
    }

    fn rel_err(got: &Vector3<f64>, want: &Vector3<f64>) -> f64 {
        (got - want).norm() / want.norm()
    }

    /// Zonal terms are symmetric about the pole, so a zonal-only field must
    /// reproduce `ZonalGravity` whatever Earth's rotation angle: for
    /// `SimpleEci` (pole = +Z, ERA-only rotation about Z) to round-off. This
    /// ties the field's normalization to the J2/J3/J4 constants the rest of
    /// orts uses.
    #[test]
    fn zonal_only_field_matches_zonal_gravity_in_simple_eci() {
        let sh = SphericalHarmonicGravity::for_simple_eci(zonal_field());
        let zonal = ZonalGravity::<frame::SimpleEci>::new(MU_E, R_E, J2_E, Some(J3_E), Some(J4_E));
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        for p in positions() {
            let want = zonal.acceleration(&p, &epoch);
            let got = sh.acceleration(&p, &epoch);
            assert!(rel_err(&got, &want) < 1e-12, "{p:?}: {got:?} vs {want:?}");
        }
    }

    /// Same in `Gcrs` with zero EOP: the ITRS pole is then exactly the model
    /// CIP that `ZonalGravity<Gcrs>` uses, so the two agree to round-off.
    /// A wrong rotation direction (or a missing inverse) would tilt the pole by
    /// twice the ~0.1° CIP offset and fail this by ~1e-3.
    #[test]
    fn zonal_only_field_matches_zonal_gravity_in_gcrs_with_zero_eop() {
        let sh = SphericalHarmonicGravity::<frame::Gcrs>::new(zonal_field(), zero_eop());
        let zonal = ZonalGravity::<frame::Gcrs>::new(MU_E, R_E, J2_E, Some(J3_E), Some(J4_E));
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        for p in positions() {
            let want = zonal.acceleration(&p, &epoch);
            let got = sh.acceleration(&p, &epoch);
            assert!(rel_err(&got, &want) < 1e-11, "{p:?}: {got:?} vs {want:?}");
        }
    }

    /// The body-fixed x axis of the field sits at ECI longitude +ERA. Along
    /// that axis the C̄22 term's gradient is purely radial (it is the term's
    /// symmetry axis); 45° away it is purely tangential. A rotation applied
    /// with the wrong sign puts the axis at −ERA instead — distinguishable
    /// from +ERA (mod the term's 90° symmetry) exactly when ERA ≢ 0 mod 45°,
    /// so the epoch is nudged to ERA ≡ 22.5° mod 45°, where the −ERA
    /// direction is a purely tangential one.
    #[test]
    fn simple_eci_rotates_the_field_by_plus_era() {
        let sh = SphericalHarmonicGravity::for_simple_eci(c22_field());
        let base = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let era0 = base.to_ut1_naive().era().to_degrees().rem_euclid(45.0);
        // ERA advances 360° per sidereal day ≈ 1° per 239.34 s.
        let shift_s = (22.5 - era0).rem_euclid(45.0) * 239.34;
        let epoch = base.add_si_seconds(shift_s);
        let era = epoch.to_ut1_naive().era();
        let residual = (era.to_degrees().rem_euclid(45.0) - 22.5).abs();
        assert!(
            residual < 0.1,
            "epoch nudge failed: ERA mod 45° = {}",
            22.5 + residual
        );
        let r = 7000.0;

        let tangential_fraction = |lon: f64| {
            let p = Vector3::new(r * lon.cos(), r * lon.sin(), 0.0);
            let a = sh.acceleration(&p, &epoch);
            let radial = a.dot(&p) / r;
            (a - radial * p / r).norm() / a.norm()
        };
        assert!(
            tangential_fraction(era) < 1e-9,
            "on-axis: {}",
            tangential_fraction(era)
        );
        assert!(tangential_fraction(era + 0.25 * std::f64::consts::PI) > 0.99);
        // The opposite sign convention puts the axis at −ERA, which for this
        // epoch is 45° (mod 90°) from the true axis: purely tangential.
        assert!(
            tangential_fraction(-era) > 0.99,
            "field appears rotated by −ERA"
        );
    }

    /// Non-zero EOP must reach the transform: with polar motion switched on,
    /// the ITRS pole tilts away from the CIP and the zonal-only field no longer
    /// agrees with `ZonalGravity<Gcrs>` — by about the polar-motion angle
    /// times the J2 gradient, which is what a model that quietly used zero EOP
    /// would miss.
    #[test]
    fn gcrs_uses_the_supplied_eop() {
        struct BigPolarMotion;
        impl Ut1Offset for BigPolarMotion {
            fn dut1(&self, _: f64) -> f64 {
                0.0
            }
        }
        impl PolarMotion for BigPolarMotion {
            // 100″ (the trait's unit is arcseconds) — far beyond real polar
            // motion, so the effect is unambiguous.
            fn x_pole(&self, _: f64) -> f64 {
                100.0
            }
            fn y_pole(&self, _: f64) -> f64 {
                0.0
            }
        }
        impl NutationCorrections for BigPolarMotion {
            fn dx(&self, _: f64) -> f64 {
                0.0
            }
            fn dy(&self, _: f64) -> f64 {
                0.0
            }
        }
        let with_pm = SphericalHarmonicGravity::<frame::Gcrs>::new(
            zonal_field(),
            GcrsEopStorage::new(BigPolarMotion),
        );
        let without = SphericalHarmonicGravity::<frame::Gcrs>::new(zonal_field(), zero_eop());
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let p = Vector3::new(4000.0, -3000.0, 5000.0);
        let rel = rel_err(
            &with_pm.acceleration(&p, &epoch),
            &without.acceleration(&p, &epoch),
        );
        // 100″ ≈ 4.8e-4 rad; the J2 acceleration changes by that order.
        assert!((1e-4..1e-2).contains(&rel), "rel change {rel:e}");
    }

    #[test]
    fn model_eval_matches_acceleration() {
        let sh = SphericalHarmonicGravity::for_simple_eci(c22_field());
        let state = OrbitalState::new(
            Vector3::new(6778.0, 100.0, 200.0),
            Vector3::new(0.0, 7.5, 0.0),
        );
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let loads = Model::<OrbitalState>::eval(&sh, 0.0, &state, Some(&epoch));
        assert_eq!(
            loads.acceleration_inertial.into_inner(),
            sh.acceleration(state.position(), &epoch)
        );
        assert_eq!(
            Model::<OrbitalState>::name(&sh),
            "spherical_harmonic_gravity"
        );
    }

    #[test]
    #[should_panic(expected = "needs an absolute epoch")]
    fn eval_without_epoch_panics() {
        let sh = SphericalHarmonicGravity::for_simple_eci(c22_field());
        let state = OrbitalState::new(
            Vector3::new(6778.0, 100.0, 200.0),
            Vector3::new(0.0, 7.5, 0.0),
        );
        let _ = Model::<OrbitalState>::eval(&sh, 0.0, &state, None);
    }

    #[test]
    fn non_finite_position_yields_non_finite_acceleration() {
        let sh = SphericalHarmonicGravity::for_simple_eci(c22_field());
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let a = sh.acceleration(&Vector3::new(f64::NAN, 0.0, 7000.0), &epoch);
        assert!(!a.iter().all(|c| c.is_finite()), "{a:?}");
    }
}
