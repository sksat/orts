//! Zonal gravity (J2/J3/J4) as a frame-aware perturbation.
//!
//! Unlike the spherically symmetric central term
//! ([`PointMass`](crate::orbital::gravity::PointMass)), the zonal harmonics are
//! symmetric about the central body's *rotation pole*, so the acceleration
//! depends on the orientation of the integration frame relative to that pole.
//! [`ZonalGravity`] therefore takes the pole direction from [`EarthPoleBridge`]
//! instead of assuming the frame's Z axis is the pole.
//!
//! - `SimpleEci`: pole = `+Z`, reproducing the classic frame-Z formula (for a
//!   non-Earth body this assumes its state is expressed with `+Z` along that
//!   body's spin axis).
//! - `Gcrs`: pole = the IAU 2006 CIP, so J2 is evaluated about Earth's true pole
//!   (offset ~0.1° from GCRS Z by 2024), more accurate than `SimpleEci`.
//!
//! Add it to a system that already carries a central
//! [`PointMass`](crate::orbital::gravity::PointMass) gravity field — this model
//! contributes the oblateness perturbation only, not the point-mass term.

use std::marker::PhantomData;

use arika::epoch::{Epoch, Utc};
use nalgebra::Vector3;

use crate::environment::EarthPoleBridge;
use crate::model::ExternalLoads;
use crate::model::{HasOrbit, Model};

/// Zonal harmonics (J2, optional J3/J4) gravity perturbation about the
/// rotation pole supplied by [`EarthPoleBridge`].
pub struct ZonalGravity<F: EarthPoleBridge = arika::frame::SimpleEci> {
    /// Gravitational parameter of the central body [km³/s²].
    pub mu: f64,
    /// Equatorial radius of the central body [km].
    pub r_body: f64,
    /// J2 coefficient (dimensionless).
    pub j2: f64,
    /// J3 coefficient (dimensionless, optional).
    pub j3: Option<f64>,
    /// J4 coefficient (dimensionless, optional).
    pub j4: Option<f64>,
    // `fn() -> F` so the marker never imposes `Send`/`Sync` (or variance)
    // requirements on `F`; `ZonalGravity<F>` is unconditionally `Send + Sync`.
    _frame: PhantomData<fn() -> F>,
}

impl<F: EarthPoleBridge> ZonalGravity<F> {
    /// Create a zonal gravity perturbation for a central body.
    pub fn new(mu: f64, r_body: f64, j2: f64, j3: Option<f64>, j4: Option<f64>) -> Self {
        Self {
            mu,
            r_body,
            j2,
            j3,
            j4,
            _frame: PhantomData,
        }
    }

    /// Zonal perturbation acceleration [km/s²] about the pole direction in
    /// frame `F` at `utc`.
    ///
    /// Each zonal term is `A·r + B·p̂` where `p̂` is the pole unit vector,
    /// `ζ = r·p̂`, and `s² = (ζ/r)²`. For `p̂ = +Z` this reduces to the classic
    /// `position.z`-based formula (pinned by the characterization test).
    fn acceleration(&self, position: &Vector3<f64>, utc: &Epoch<Utc>) -> Vector3<f64> {
        let pole = F::earth_pole(utc).into_inner();
        let r = *position;

        let r2 = r.norm_squared();
        let r_mag = r2.sqrt();
        let r5 = r2 * r2 * r_mag;
        let zeta = r.dot(&pole); // component along the pole
        // ζ/r = cos(colatitude) = sin(geocentric latitude φ from the equator),
        // so s2 = sin²φ — the same argument as the classic J2 formula.
        let s2 = (zeta * zeta) / r2;
        let re2 = self.r_body * self.r_body;

        // J2: a = c2·(5s²−1)·r − 2·c2·ζ·p̂
        let c2 = 1.5 * self.j2 * self.mu * re2 / r5;
        let mut accel = c2 * (5.0 * s2 - 1.0) * r - (2.0 * c2 * zeta) * pole;

        let r7 = r5 * r2;

        // J3: a = A3·r + B3·p̂
        if let Some(j3) = self.j3 {
            let re3 = re2 * self.r_body;
            let c3 = 0.5 * j3 * self.mu * re3;
            let a3 = -5.0 * c3 * zeta / r7 * (3.0 - 7.0 * s2);
            let b3 = 3.0 * c3 * (1.0 - 5.0 * s2) / r5;
            accel += a3 * r + b3 * pole;
        }

        // J4: a = A4·r + B4·p̂
        if let Some(j4) = self.j4 {
            let re4 = re2 * re2;
            let s4 = s2 * s2;
            let c4 = j4 * self.mu * re4;
            let a4 = (15.0 / 8.0) * c4 / r7 * (1.0 - 14.0 * s2 + 21.0 * s4);
            let b4 = (5.0 / 2.0) * c4 * zeta / r7 * (3.0 - 7.0 * s2);
            accel += a4 * r + b4 * pole;
        }

        accel
    }
}

impl<F: EarthPoleBridge, S: HasOrbit<Frame = F>> Model<S, F> for ZonalGravity<F> {
    fn name(&self) -> &str {
        "zonal_gravity"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        // The pole is epoch-independent for `SimpleEci`; for `Gcrs` without an
        // epoch, fall back to J2000 (mirrors the drag model's convention).
        let dummy = Epoch::from_jd(2451545.0);
        let utc = epoch.unwrap_or(&dummy);
        ExternalLoads::acceleration(self.acceleration(state.orbit().position(), utc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::orbital::gravity::{GravityField, PointMass, ZonalHarmonics};
    use arika::earth::{J2 as J2_E, J3 as J3_E, J4 as J4_E, MU as MU_E, R as R_E};
    use arika::frame::SimpleEci;
    use proptest::prelude::*;

    fn legacy(j3: Option<f64>, j4: Option<f64>) -> ZonalHarmonics {
        ZonalHarmonics {
            r_body: R_E,
            j2: J2_E,
            j3,
            j4,
        }
    }

    fn zonal(j3: Option<f64>, j4: Option<f64>) -> ZonalGravity<SimpleEci> {
        ZonalGravity::new(MU_E, R_E, J2_E, j3, j4)
    }

    /// The legacy `ZonalHarmonics` returns point-mass + zonal; `ZonalGravity`
    /// returns the zonal perturbation only. So the reference is the difference.
    fn legacy_zonal_only(zh: &ZonalHarmonics, pos: &Vector3<f64>) -> Vector3<f64> {
        zh.acceleration(MU_E, pos) - PointMass.acceleration(MU_E, pos)
    }

    /// Characterization: with pole = +Z (`SimpleEci`), `ZonalGravity` must
    /// reproduce the legacy frame-Z `ZonalHarmonics` perturbation to f64
    /// round-off, across J2 / J3 / J4 variants and representative positions.
    #[test]
    fn matches_legacy_zonal_harmonics_for_simple_eci() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0); // ignored: SimpleEci pole = +Z
        let cases = [
            (None, None),
            (Some(J3_E), None),
            (None, Some(J4_E)),
            (Some(J3_E), Some(J4_E)),
        ];
        let positions = [
            Vector3::new(6778.0, 0.0, 0.0),         // equatorial
            Vector3::new(0.0, 0.0, 7000.0),         // polar
            Vector3::new(4000.0, -3000.0, 5000.0),  // generic
            Vector3::new(42164.0, 0.0, 0.0),        // GEO
            Vector3::new(-5000.0, 2000.0, -3000.0), // generic
        ];
        for (j3, j4) in cases {
            let zh = legacy(j3, j4);
            let zg = zonal(j3, j4);
            for p in positions {
                let a_ref = legacy_zonal_only(&zh, &p);
                let a_new = zg.acceleration(&p, &epoch);
                let rel = (a_new - a_ref).norm() / a_ref.norm().max(1e-30);
                assert!(
                    rel < 1e-12,
                    "j3={j3:?} j4={j4:?} pos={p:?}: rel err {rel:e} (ref={a_ref:?}, new={a_new:?})"
                );
            }
        }
    }

    /// Garbage in → garbage out: a non-finite or degenerate position must not
    /// be silently turned into an all-finite (wrong) acceleration (CLAUDE.md:
    /// pin NaN/∞ behaviour too). The exact NaN pattern differs from the legacy
    /// frame-Z formula — the pole generalization uses `ζ = r·p̂`, and `inf·0` in
    /// that dot product yields NaN where component extraction did not — which is
    /// fine: inf positions are non-physical, and finite inputs are pinned
    /// exactly by [`matches_legacy_zonal_harmonics_for_simple_eci`].
    #[test]
    fn non_finite_inputs_yield_non_finite_output() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let zg = zonal(Some(J3_E), Some(J4_E));
        for p in [
            Vector3::new(f64::NAN, 0.0, 7000.0),
            Vector3::new(7000.0, f64::INFINITY, 0.0),
            Vector3::new(0.0, 0.0, 0.0), // zero radius → 0/0
        ] {
            let a = zg.acceleration(&p, &epoch);
            assert!(
                !a.iter().all(|c| c.is_finite()),
                "non-finite/degenerate input {p:?} must not yield an all-finite acceleration, got {a:?}"
            );
        }
    }

    /// The `Model` impl wires `acceleration` into `ExternalLoads`.
    #[test]
    fn model_eval_matches_acceleration() {
        let zg = zonal(None, None);
        let state = OrbitalState::new(
            Vector3::new(6778.0, 100.0, 200.0),
            Vector3::new(0.0, 7.5, 0.0),
        );
        let loads = Model::<OrbitalState>::eval(&zg, 0.0, &state, None);
        let a = zg.acceleration(state.position(), &Epoch::from_jd(2451545.0));
        assert_eq!(loads.acceleration_inertial.into_inner(), a);
    }

    proptest! {
        /// Property generalization of the characterization test: for `SimpleEci`
        /// (pole = +Z), `ZonalGravity` equals the legacy frame-Z `ZonalHarmonics`
        /// perturbation at *any* position, for every J2/J3/J4 variant. Bounded
        /// in absolute terms scaled by the (non-vanishing) point-mass
        /// acceleration, so the J2 zero-crossing geometry (where the zonal term
        /// itself vanishes) doesn't blow up a relative error.
        #[test]
        fn zonal_gravity_equals_legacy_over_random_positions(
            x in -60000.0f64..60000.0,
            y in -60000.0f64..60000.0,
            z in -60000.0f64..60000.0,
            j3_on in any::<bool>(),
            j4_on in any::<bool>(),
        ) {
            let r = Vector3::new(x, y, z);
            prop_assume!(r.norm() > 100.0); // skip near-origin (formula is singular at r=0)

            let j3 = j3_on.then_some(J3_E);
            let j4 = j4_on.then_some(J4_E);
            let zh = legacy(j3, j4);
            let zg = zonal(j3, j4);

            let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0); // ignored: SimpleEci pole = +Z
            let a_ref = legacy_zonal_only(&zh, &r);
            let a_new = zg.acceleration(&r, &epoch);

            let pm = PointMass.acceleration(MU_E, &r).norm();
            let diff = (a_new - a_ref).norm();
            prop_assert!(
                diff <= 1e-9 * pm,
                "diff {diff:e} exceeds 1e-9·|pm| ({:e}) at {r:?}",
                1e-9 * pm
            );
        }
    }
}
