use std::sync::Arc;

use arika::epoch::Epoch;
use arika::frame::{self, Vec3};
use nalgebra::Vector3;

use arika::frame::Eci;

use crate::model::ExternalLoads;
use crate::model::{HasOrbit, Model};

/// Type alias for a position function: `Epoch -> ECI position [km]`.
///
/// Stored as an `Arc<dyn Fn>` so the struct is cheaply cloneable and can hold
/// closures that capture state (e.g., an interpolated ephemeris table).
pub type BodyPositionFn = Arc<dyn Fn(&Epoch) -> Vec3<frame::Gcrs> + Send + Sync>;

/// Third-body gravitational perturbation.
///
/// Computes the gravitational acceleration on a satellite due to a third body
/// (e.g., Sun or Moon) using the standard perturbation formula:
///
/// a = μ_3 * [(r_body - r_sat)/|r_body - r_sat|³ - r_body/|r_body|³]
///
/// where r_body is the position of the third body relative to the central body,
/// and r_sat is the satellite position relative to the central body.
///
/// Use the `::sun()` / `::moon()` constructors for standard bodies, or
/// `::custom()` to supply an arbitrary position closure (e.g., a tabulated
/// ephemeris source).
#[derive(Clone)]
pub struct ThirdBodyGravity {
    /// Human-readable name (e.g., "third_body_sun", "third_body_moon")
    pub name: &'static str,
    /// Gravitational parameter of the third body [km³/s²]
    pub mu_body: f64,
    /// Closure returning the third body position in ECI [km] at a given epoch.
    body_position_fn: BodyPositionFn,
}

impl ThirdBodyGravity {
    /// Create a Sun third-body perturbation (uses Meeus analytical ephemeris).
    pub fn sun() -> Self {
        Self {
            name: "third_body_sun",
            mu_body: arika::sun::MU,
            body_position_fn: Arc::new(arika::sun::sun_position_eci),
        }
    }

    /// Create a Moon third-body perturbation (uses Meeus analytical ephemeris).
    ///
    /// μ_Moon is sourced from [`arika::moon::MU`].
    pub fn moon() -> Self {
        Self {
            name: "third_body_moon",
            mu_body: arika::moon::MU,
            body_position_fn: Arc::new(arika::moon::moon_position_eci),
        }
    }

    /// Create a Moon third-body perturbation from any [`arika::moon::MoonEphemeris`]
    /// implementation.
    ///
    /// Use this to swap in a higher-accuracy Moon ephemeris (e.g. a tabulated
    /// JPL Horizons source) while keeping the same force-model wiring.
    ///
    /// Thanks to the blanket `impl<T: MoonEphemeris + ?Sized> MoonEphemeris for
    /// Arc<T>` in [`arika::moon`], this constructor accepts both owned
    /// implementations (`MeeusMoonEphemeris`) *and* shared trait objects
    /// (`Arc<dyn MoonEphemeris>`), so a single ephemeris can be fanned out to
    /// the integrator's force model and to any auxiliary targeting helpers.
    ///
    /// μ_Moon is fixed to [`arika::moon::MU`] and is **not** derived
    /// from the supplied ephemeris. If a non-standard μ is needed, use
    /// [`ThirdBodyGravity::custom`] directly.
    pub fn moon_with_ephemeris<E>(ephem: E) -> Self
    where
        E: arika::moon::MoonEphemeris + 'static,
    {
        let ephem = Arc::new(ephem);
        Self {
            name: "third_body_moon",
            mu_body: arika::moon::MU,
            body_position_fn: Arc::new(move |epoch| ephem.position_eci(epoch)),
        }
    }

    /// Create a custom third-body perturbation with an arbitrary position
    /// function.
    ///
    /// Use this for bodies not covered by `::sun()` / `::moon()`, or to supply
    /// a higher-accuracy ephemeris source (e.g., a precomputed table).
    pub fn custom<F>(name: &'static str, mu_body: f64, position_fn: F) -> Self
    where
        F: Fn(&Epoch) -> Vec3<frame::Gcrs> + Send + Sync + 'static,
    {
        Self {
            name,
            mu_body,
            body_position_fn: Arc::new(position_fn),
        }
    }
}

impl ThirdBodyGravity {
    /// Compute third-body gravitational acceleration [km/s²].
    ///
    /// The tidal formula is pure vector arithmetic on raw `Vector3<f64>`.
    /// The body position closure returns `Vec3<Gcrs>` whose raw inner
    /// value is numerically equal to any other ECI frame at Meeus
    /// precision, so this function is frame-independent.
    pub(crate) fn acceleration(
        &self,
        sat_position: &Vector3<f64>,
        epoch: Option<&Epoch>,
    ) -> Vector3<f64> {
        let epoch = match epoch {
            Some(e) => e,
            None => return Vector3::zeros(),
        };

        let r_body = (self.body_position_fn)(epoch).into_inner();

        let r_sat_to_body = r_body - sat_position;
        let d = r_sat_to_body.magnitude();
        let r_body_mag = r_body.magnitude();

        // a = μ₃ * [(r_body - r_sat)/d³ - r_body/R³]
        self.mu_body
            * (r_sat_to_body / (d * d * d) - r_body / (r_body_mag * r_body_mag * r_body_mag))
    }
}

impl<F: Eci, S: HasOrbit<Frame = F>> Model<S, F> for ThirdBodyGravity {
    fn name(&self) -> &str {
        self.name
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        ExternalLoads::acceleration(self.acceleration(state.orbit().position(), epoch))
    }
}

// Static assertion that `ThirdBodyGravity` can cross thread boundaries.
// This is required so `OrbitalSystem` remains `Send + Sync` when it contains
// third-body models, which allows the integrator to be used from a worker
// thread (e.g. the WebSocket serve mode in orts-cli).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ThirdBodyGravity>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use arika::earth::{MU as MU_EARTH, R as R_EARTH};
    use nalgebra::vector;

    fn iss_state() -> OrbitalState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0])
    }

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    #[test]
    fn sun_perturbation_order_of_magnitude() {
        let tb = ThirdBodyGravity::sun();
        let state = iss_state();
        let epoch = test_epoch();

        let a = tb.acceleration(state.position(), Some(&epoch));
        let a_mag = a.magnitude();

        // Sun tidal acceleration on LEO satellite:
        // a ≈ 2*μ_sun*r_sat / d_sun³ ≈ 2*1.327e11*6778 / (1.5e8)³ ≈ 5e-10 km/s²
        assert!(
            a_mag > 1e-11 && a_mag < 1e-8,
            "Sun perturbation should be ~5e-10 km/s², got {a_mag:.6e}"
        );
    }

    #[test]
    fn moon_perturbation_order_of_magnitude() {
        let tb = ThirdBodyGravity::moon();
        let state = iss_state();
        let epoch = test_epoch();

        let a = tb.acceleration(state.position(), Some(&epoch));
        let a_mag = a.magnitude();

        // Moon tidal acceleration on LEO satellite:
        // a ≈ 2*μ_moon*r_sat / d_moon³ ≈ 2*4903*6778 / (3.84e5)³ ≈ 1.2e-9 km/s²
        assert!(
            a_mag > 1e-11 && a_mag < 1e-7,
            "Moon perturbation should be ~1e-9 km/s², got {a_mag:.6e}"
        );
    }

    #[test]
    fn no_epoch_returns_zero() {
        let tb = ThirdBodyGravity::sun();
        let state = iss_state();

        let a = tb.acceleration(state.position(), None);
        assert_eq!(
            a,
            Vector3::zeros(),
            "No epoch should give zero acceleration"
        );
    }

    #[test]
    fn perturbation_much_smaller_than_central_gravity() {
        let tb_sun = ThirdBodyGravity::sun();
        let tb_moon = ThirdBodyGravity::moon();
        let state = iss_state();
        let epoch = test_epoch();

        let a_sun = tb_sun
            .acceleration(state.position(), Some(&epoch))
            .magnitude();
        let a_moon = tb_moon
            .acceleration(state.position(), Some(&epoch))
            .magnitude();

        // Central body gravity: μ/r² ≈ 398600/6778² ≈ 8.7e-3 km/s²
        let r = state.position().magnitude();
        let a_central = MU_EARTH / (r * r);

        // Third-body should be ~6-7 orders of magnitude smaller
        assert!(
            a_sun < a_central * 1e-4,
            "Sun perturbation ({a_sun:.6e}) should be << central gravity ({a_central:.6e})"
        );
        assert!(
            a_moon < a_central * 1e-4,
            "Moon perturbation ({a_moon:.6e}) should be << central gravity ({a_central:.6e})"
        );
    }

    #[test]
    fn sun_perturbation_varies_with_epoch() {
        // Tidal force has 180° symmetry, so compare 90°-apart epochs (March vs June).
        // Place satellite on Y-axis:
        // - March: Sun near +X → satellite perpendicular → tidal compression along Y
        // - June: Sun near +Y → satellite along Sun axis → tidal stretching along Y
        // These give opposite Y-acceleration directions.
        let tb = ThirdBodyGravity::sun();
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let state = OrbitalState::new(vector![0.0, r, 0.0], vector![-v, 0.0, 0.0]);

        let epoch1 = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let epoch2 = Epoch::from_gregorian(2024, 6, 20, 12, 0, 0.0);

        let a1 = tb.acceleration(state.position(), Some(&epoch1));
        let a2 = tb.acceleration(state.position(), Some(&epoch2));

        // Direction should be very different (perpendicular vs parallel to Sun)
        let cos_angle = a1.normalize().dot(&a2.normalize());
        assert!(
            cos_angle < 0.5,
            "Sun perturbation should differ between March and June, cos={cos_angle:.3}"
        );
    }

    #[test]
    fn third_body_is_clone() {
        // Ensure `ThirdBodyGravity` is cheaply cloneable (Arc-backed).
        let tb = ThirdBodyGravity::moon();
        let tb2 = tb.clone();
        assert_eq!(tb.name, tb2.name);
        assert_eq!(tb.mu_body, tb2.mu_body);
        // Clone should produce the same acceleration from the same state/epoch.
        let state = iss_state();
        let epoch = test_epoch();
        let a1 = tb.acceleration(state.position(), Some(&epoch));
        let a2 = tb2.acceleration(state.position(), Some(&epoch));
        assert_eq!(a1, a2);
    }

    #[test]
    fn moon_constructor_uses_mu_moon_constant() {
        // Regression guard: the Moon μ in `ThirdBodyGravity::moon()` must come
        // from `arika::moon::MU` so there is one authoritative
        // value. If this test fails, someone reintroduced a hardcoded literal
        // and the two can drift.
        let tb = ThirdBodyGravity::moon();
        assert_eq!(tb.mu_body, arika::moon::MU);

        let tb_trait = ThirdBodyGravity::moon_with_ephemeris(arika::moon::MeeusMoonEphemeris);
        assert_eq!(tb_trait.mu_body, arika::moon::MU);
    }

    #[test]
    fn moon_with_ephemeris_accepts_arc_dyn_moon_ephemeris() {
        // Regression guard for the blanket `impl<T> MoonEphemeris for Arc<T>`.
        // Without the blanket impl, `Arc<dyn MoonEphemeris>` does not satisfy
        // `E: MoonEphemeris` and this constructor call would fail to compile.
        // apollo11/main.rs (and the upcoming artemis1 example) relies on this
        // shape to share one ephemeris between the integrator and targeters.
        use arika::moon::{MeeusMoonEphemeris, MoonEphemeris};
        let shared: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let tb = ThirdBodyGravity::moon_with_ephemeris(Arc::clone(&shared));
        let state = iss_state();
        let epoch = test_epoch();
        let a = tb.acceleration(state.position(), Some(&epoch));
        // Should produce the same acceleration as `::moon()` for the same
        // underlying Meeus source.
        let a_ref = ThirdBodyGravity::moon().acceleration(state.position(), Some(&epoch));
        assert_eq!(a, a_ref);
    }

    #[test]
    fn custom_third_body_uses_supplied_closure() {
        // Build a custom third body at a fixed position and verify the
        // acceleration matches the analytic tidal formula.
        let fake_body_pos = Vec3::<frame::Gcrs>::new(1.0e6, 0.0, 0.0);
        let fake_mu = 1.0e5;
        let tb = ThirdBodyGravity::custom("fake", fake_mu, move |_epoch| fake_body_pos);
        let state = iss_state();
        let epoch = test_epoch();

        let a = tb.acceleration(state.position(), Some(&epoch));

        // Expected: μ_body * [(r_body - r_sat)/|r_body - r_sat|³ - r_body/|r_body|³]
        let fake_body_raw = fake_body_pos.into_inner();
        let r_sat_to_body = fake_body_raw - *state.position();
        let d = r_sat_to_body.magnitude();
        let r_body_mag = fake_body_raw.magnitude();
        let expected = fake_mu
            * (r_sat_to_body / (d * d * d)
                - fake_body_raw / (r_body_mag * r_body_mag * r_body_mag));
        let err = (a - expected).magnitude();
        assert!(
            err < 1e-15,
            "custom body acceleration mismatch: err={err:e}"
        );
        assert_eq!(tb.name, "fake");
    }

    #[test]
    fn moon_with_ephemeris_uses_supplied_ephemeris() {
        use arika::moon::{MeeusMoonEphemeris, MoonEphemeris};

        // `::moon_with_ephemeris(MeeusMoonEphemeris)` should produce the same
        // acceleration as `::moon()` (both delegate to the Meeus analytical model).
        let tb_default = ThirdBodyGravity::moon();
        let tb_trait = ThirdBodyGravity::moon_with_ephemeris(MeeusMoonEphemeris);
        let state = iss_state();
        let epoch = test_epoch();

        let a_default = tb_default.acceleration(state.position(), Some(&epoch));
        let a_trait = tb_trait.acceleration(state.position(), Some(&epoch));

        // They come from the same underlying Meeus data, so they should be
        // bit-identical.
        assert_eq!(a_default, a_trait);

        // The name and μ should also match.
        assert_eq!(tb_trait.name, "third_body_moon");
        assert_eq!(tb_trait.mu_body, 4902.800066);

        // Sanity check: the MoonEphemeris trait method returns a finite vector.
        let _ = MeeusMoonEphemeris.velocity_eci(&epoch);
    }

    #[test]
    fn moon_with_ephemeris_respects_custom_source() {
        use arika::moon::MoonEphemeris;

        // Build a fake Moon ephemeris that always returns a fixed position.
        // This simulates what a tabulated (Horizons-backed) source would do.
        struct FakeMoonEphem;
        impl MoonEphemeris for FakeMoonEphem {
            fn position_eci(&self, _epoch: &Epoch) -> Vec3<frame::Gcrs> {
                Vec3::new(400_000.0, 0.0, 0.0)
            }
            fn name(&self) -> &str {
                "fake"
            }
        }

        let tb = ThirdBodyGravity::moon_with_ephemeris(FakeMoonEphem);
        let state = iss_state();
        let epoch = test_epoch();
        let a = tb.acceleration(state.position(), Some(&epoch));

        // Compute the expected tidal acceleration analytically.
        let r_body = vector![400_000.0_f64, 0.0, 0.0];
        let r_sat_to_body = r_body - *state.position();
        let d = r_sat_to_body.magnitude();
        let r_body_mag = r_body.magnitude();
        let expected = 4902.800066
            * (r_sat_to_body / (d * d * d) - r_body / (r_body_mag * r_body_mag * r_body_mag));
        let err = (a - expected).magnitude();
        assert!(
            err < 1e-15,
            "Expected moon_with_ephemeris to use the fake source, err={err:e}"
        );
    }

    #[test]
    fn custom_third_body_closure_can_capture_state() {
        // Captured-state closures are the whole point of the `Arc<dyn Fn>`
        // refactor — verify that a closure capturing a `Vec` works.
        let positions = vec![
            Vec3::<frame::Gcrs>::new(1.0e6, 0.0, 0.0),
            Vec3::<frame::Gcrs>::new(0.0, 1.0e6, 0.0),
            Vec3::<frame::Gcrs>::new(0.0, 0.0, 1.0e6),
        ];
        // Move the Vec into the closure; the closure returns the first entry.
        let tb = ThirdBodyGravity::custom("captured", 1.0e5, move |_epoch| positions[0]);
        let state = iss_state();
        let epoch = test_epoch();
        let a = tb.acceleration(state.position(), Some(&epoch));
        assert!(a.magnitude() > 0.0);
    }

    #[test]
    fn geo_larger_perturbation_than_leo() {
        // GEO is farther from Earth center → third-body perturbation is relatively more significant
        let tb_moon = ThirdBodyGravity::moon();
        let epoch = test_epoch();

        let leo_state = iss_state();
        let geo_r = 42164.0; // GEO radius
        let geo_v = (MU_EARTH / geo_r).sqrt();
        let geo_state = OrbitalState::new(vector![geo_r, 0.0, 0.0], vector![0.0, geo_v, 0.0]);

        let a_leo = tb_moon
            .acceleration(leo_state.position(), Some(&epoch))
            .magnitude();
        let a_geo = tb_moon
            .acceleration(geo_state.position(), Some(&epoch))
            .magnitude();

        // At GEO, satellite is closer to Moon (shorter range) → larger perturbation
        // Also the "indirect" term is larger relative to "direct" term
        // The absolute perturbation may not always be larger, but relative to central gravity it is
        let a_central_leo = MU_EARTH / leo_state.position().magnitude_squared();
        let a_central_geo = MU_EARTH / geo_state.position().magnitude_squared();

        let ratio_leo = a_leo / a_central_leo;
        let ratio_geo = a_geo / a_central_geo;

        assert!(
            ratio_geo > ratio_leo,
            "Moon perturbation ratio at GEO ({ratio_geo:.6e}) should be > LEO ({ratio_leo:.6e})"
        );
    }
}
