use arika::eclipse::{self, SUN_RADIUS_KM, ShadowModel};
use arika::epoch::Epoch;
use arika::sun;
use nalgebra::Vector3;

use crate::perturbations::SOLAR_RADIATION_PRESSURE;
use arika::earth::R as R_EARTH;
use arika::earth::transform::EphemerisFrameBridge;

use crate::model::{HasAttitude, HasFrame, HasMass, HasOrbit, Model};

use super::{ExternalLoads, SpacecraftShape, SurfacePanel};

/// Radiation-pressure force on one flat panel [N, body frame].
///
/// `s_body` is the unit vector from the spacecraft toward the Sun in the body
/// frame; `pressure` [N/m²] is the solar radiation pressure already scaled for
/// heliocentric distance and illumination fraction. Returns zero for a panel
/// facing away from the Sun.
///
/// `panel.normal` must be unit length; every constructor of a model holding
/// panels enforces that, so this runs without re-checking it per stage.
///
/// Split out as a pure function so the force law can be checked against
/// closed-form values and for rotational equivariance without the Sun
/// ephemeris, which an inertial rotation of the state does not carry along.
fn panel_force(panel: &SurfacePanel, s_body: &Vector3<f64>, pressure: f64) -> Vector3<f64> {
    let cos_theta = panel.normal.dot(s_body);
    if cos_theta <= 0.0 {
        return Vector3::zeros();
    }

    let optics = panel.optics;
    let along_sun = optics.absorptivity() + optics.diffuse();
    let along_normal = 2.0 * (optics.specular() * cos_theta + optics.diffuse() / 3.0);

    -pressure * panel.area * cos_theta * (along_sun * s_body + along_normal * panel.normal)
}

/// Attitude-dependent solar radiation pressure model using flat surface panels.
///
/// Implements [`Model`] to produce both translational acceleration and
/// SRP torque from per-panel radiation forces.  For the [`SpacecraftShape::Sphere`]
/// variant, the `cr` and `area` are read from the shape itself.
///
/// Per-panel force, from the panel's [`PanelOptics`]:
///
/// ```text
/// F = -P·A·cosθ · [ (α + ρ_d)·ŝ  +  2·(ρ_s·cosθ + ρ_d/3)·n̂ ]
/// ```
///
/// with `cosθ = n̂·ŝ`, absorbed fraction `α`, specular `ρ_s`, diffuse `ρ_d`, and
/// `α + ρ_s + ρ_d = 1`. Absorption and diffuse *incidence* push along the Sun
/// line; specular reflection and diffuse *re-emission* push along the panel
/// normal. Face-on this collapses to `Cr = 1 + ρ_s + 2ρ_d/3`, recovering the
/// textbook 1 for a black panel, 2 for a mirror and 5/3 for a Lambertian one.
///
/// The torque is `Σ r_cp × F_panel`, so a panel whose centre of pressure sits
/// off the centre of mass produces an attitude disturbance.
///
/// [`PanelOptics`]: super::PanelOptics
pub struct PanelSrp {
    shape: SpacecraftShape,
    /// Central body radius for the eclipse model [km].
    /// `None` disables the eclipse test, so the spacecraft is never in a
    /// body's shadow. Panel-to-panel occlusion is separate and always applies.
    shadow_body_radius: Option<f64>,
    /// Shadow model to use (default: Cylindrical).
    shadow_model: ShadowModel,
    /// Where the Sun is, relative to the body this spacecraft orbits.
    ///
    /// The panel force points along the satellite-to-Sun line, so this decides
    /// its direction. Geocentric by default; [`Self::for_body`] replaces it,
    /// the same way [`crate::perturbations::SolarRadiationPressure`] does.
    sun_position_fn: crate::perturbations::SunPositionFn,
}

impl PanelSrp {
    /// Create a panel-based (attitude-dependent) SRP model from surface panels,
    /// with the geocentric Sun and no shadow — see [`Self::new`].
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn panels(panels: Vec<super::SurfacePanel>) -> Self {
        Self::new(SpacecraftShape::panels(panels))
    }

    /// Create an SRP model for Earth orbit with cylindrical Earth shadow.
    ///
    /// For the [`SpacecraftShape::Sphere`] variant, `cr` and `area` come from
    /// the shape. For [`SpacecraftShape::Panels`], each panel carries its own
    /// area and [`PanelOptics`].
    ///
    /// [`PanelOptics`]: super::PanelOptics
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn for_earth(shape: SpacecraftShape) -> Self {
        shape.assert_normals_are_unit();
        shape.assert_outlines_are_consistent();
        Self {
            shape,
            shadow_body_radius: Some(R_EARTH),
            shadow_model: ShadowModel::Cylindrical,
            sun_position_fn: std::sync::Arc::new(sun::sun_position_eci),
        }
    }

    /// Create a panel SRP model for orbit about `body`.
    ///
    /// Takes both body-dependent quantities from that body: the Sun's position
    /// relative to it, and its own radius for the shadow. The panel force points
    /// along the satellite-to-Sun line and its torque follows the panel's angle
    /// to that line, so a geocentric Sun direction about another body turns both
    /// — from Mars in 2026 that direction is up to 176° off.
    ///
    /// Orbiting the Sun itself works: the Sun sits at the origin, so the
    /// satellite-to-Sun vector is `-r_sat`, and there is nothing to cast a
    /// shadow.
    ///
    /// Fails only for a central body with no Sun ephemeris (Uranus, Neptune).
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn for_body(
        body: arika::body::KnownBody,
        shape: SpacecraftShape,
    ) -> Result<Self, arika::sun::SunPositionError> {
        shape.assert_normals_are_unit();
        shape.assert_outlines_are_consistent();
        if body == arika::body::KnownBody::Sun {
            return Ok(Self {
                shape,
                shadow_body_radius: None,
                shadow_model: ShadowModel::Cylindrical,
                sun_position_fn: std::sync::Arc::new(|_| {
                    arika::frame::Vec3::from_raw(Vector3::zeros())
                }),
            });
        }
        // Probe now so an unsupported body fails here rather than inside the
        // integrator, where the closure cannot report it.
        sun::sun_position_from_body(body, &Epoch::j2000().to_tdb())?;
        Ok(Self {
            shape,
            shadow_body_radius: Some(body.properties().radius),
            shadow_model: ShadowModel::Cylindrical,
            sun_position_fn: std::sync::Arc::new(move |epoch: &Epoch<arika::epoch::Tdb>| {
                sun::sun_position_from_body(body, epoch)
                    .expect("the same body was accepted at construction")
            }),
        })
    }

    /// Create an SRP model with the geocentric Sun and no central-body eclipse.
    ///
    /// The Sun direction is Earth's, so about another body the force points
    /// where the Sun is not — from Mars in 2026 up to 176° off. For a
    /// shadow-free model about `body`, take
    /// [`Self::for_body`]`(body, shape)?.`[`without_shadow()`] instead.
    ///
    /// [`without_shadow()`]: Self::without_shadow
    ///
    /// Panels still shade each other. That is a property of the shape rather
    /// than of an orbit, so it is not something either constructor turns off:
    /// an outlined panel that another one completely covers contributes nothing
    /// here either. Complete cover by one panel is the whole of what is found —
    /// a panel half in shadow produces its full force, as does one that two
    /// others cover between them.
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn new(shape: SpacecraftShape) -> Self {
        shape.assert_normals_are_unit();
        shape.assert_outlines_are_consistent();
        Self {
            shape,
            shadow_body_radius: None,
            shadow_model: ShadowModel::Cylindrical,
            sun_position_fn: std::sync::Arc::new(sun::sun_position_eci),
        }
    }

    /// Drop the shadow, keeping the Sun direction this model was built with
    /// (builder pattern).
    pub fn without_shadow(mut self) -> Self {
        self.shadow_body_radius = None;
        self
    }

    /// Set or override the shadow body radius (builder pattern).
    pub fn with_shadow_body(mut self, radius: f64) -> Self {
        self.shadow_body_radius = Some(radius);
        self
    }

    /// Set the shadow model (builder pattern).
    pub fn with_shadow_model(mut self, model: ShadowModel) -> Self {
        self.shadow_model = model;
        self
    }
}

impl PanelSrp {
    /// Compute SRP loads from full state (using capability trait methods).
    pub(crate) fn loads_from_state<F: EphemerisFrameBridge>(
        &self,
        orbit: &crate::OrbitalState<F>,
        body_to_inertial: arika::frame::Rotation<arika::frame::Body, F>,
        mass: f64,
        epoch: Option<&Epoch>,
    ) -> ExternalLoads<F> {
        let epoch = match epoch {
            Some(e) => e,
            None => return ExternalLoads::zeros(),
        };

        // Rotate the GCRS Sun ephemeris into the integration frame `F` (identity
        // for GCRS-aligned frames; see `EphemerisFrameBridge`). Keep the typed
        // `Vec3<F>` in scope and borrow `.inner()` only at raw-API boundaries.
        let sun_f =
            F::ephemeris_rotation(epoch).transform(&(self.sun_position_fn)(&epoch.to_tdb()));
        let sat_to_sun = sun_f.inner() - orbit.position();
        let r_sun = sat_to_sun.magnitude();
        let s_hat = sat_to_sun / r_sun;

        // Shadow check using arika::eclipse
        let illum = if let Some(body_r) = self.shadow_body_radius {
            let v = eclipse::illumination_central(
                orbit.position(),
                sun_f.inner(),
                body_r,
                SUN_RADIUS_KM,
                self.shadow_model,
            );
            if v <= 0.0 {
                return ExternalLoads::zeros();
            }
            v
        } else {
            1.0
        };

        let distance_ratio = sun::AU_KM / r_sun;
        // Scale base pressure by illumination for penumbra support
        let base_pressure = SOLAR_RADIATION_PRESSURE * distance_ratio * distance_ratio * illum; // [N/m²]

        match &self.shape {
            SpacecraftShape::Sphere { area, cr, .. } => {
                // a = -base_pressure * Cr * (A/m) * ŝ  [m/s²]
                // Divide by 1000 to convert to km/s²
                let a_mag = base_pressure * cr * area / mass / 1000.0;
                ExternalLoads {
                    acceleration_inertial: arika::frame::Vec3::from_raw(-a_mag * s_hat),
                    torque_body: arika::frame::Vec3::zeros(),
                    mass_rate: 0.0,
                }
            }
            SpacecraftShape::Panels(panels) => {
                // Transform Sun direction to body frame
                let s_body = body_to_inertial
                    .inverse()
                    .transform(&arika::frame::Vec3::<F>::from_raw(s_hat))
                    .into_inner();

                let mut total_force_body = Vector3::zeros(); // [N]
                let mut total_torque_body = Vector3::zeros(); // [N·m]

                for panel in panels {
                    // Facing first: a back-facing panel produces nothing either
                    // way, and the occlusion scan is O(N) per panel.
                    if panel.normal.dot(&s_body) <= 0.0 {
                        continue;
                    }
                    // A panel standing in the Sun's way keeps this one dark.
                    // Panels without an outline never do, so an area-only fleet
                    // is unaffected.
                    if crate::spacecraft::surface::is_fully_occluded(panel, panels, &s_body) {
                        continue;
                    }
                    let force = panel_force(panel, &s_body, base_pressure); // [N]

                    total_force_body += force;
                    total_torque_body += panel.cp_offset.cross(&force);
                }

                // a_body [m/s²] → a_inertial [km/s²]
                let a_body = arika::frame::Vec3::from_raw(total_force_body / mass);
                let a_inertial = body_to_inertial.transform(&a_body) / 1000.0;

                ExternalLoads {
                    acceleration_inertial: a_inertial,
                    torque_body: arika::frame::Vec3::from_raw(total_torque_body),
                    mass_rate: 0.0,
                }
            }
        }
    }
}

// Frame-correct panel SRP: the Sun ephemeris (`Vec3<Gcrs>`) is rotated into the
// integration frame `F` via `EphemerisFrameBridge`, and the attitude's
// frame-generic `rotation_{to,from}_inertial::<F>` carry vectors between `F` and
// the body frame — so panel SRP is valid for any such frame (identity for
// GCRS-aligned `SimpleEci`/`Gcrs`). A frame without an `EphemerisFrameBridge`
// impl (e.g. `Teme`) is rejected at compile time. See #191.
impl<F: EphemerisFrameBridge, S: HasFrame<Frame = F> + HasAttitude + HasOrbit + HasMass> Model<S>
    for PanelSrp
{
    fn name(&self) -> &str {
        "panel_srp"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        self.loads_from_state(
            state.orbit(),
            state.attitude_to_inertial(),
            state.mass(),
            epoch,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrbitalState;
    use crate::SpacecraftState;
    use crate::attitude::AttitudeState;
    use crate::perturbations::SolarRadiationPressure;
    use crate::spacecraft::{PanelOptics, SurfacePanel};
    use arika::earth::MU as MU_EARTH;
    use nalgebra::{Vector4, vector};

    fn test_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn iss_state() -> SpacecraftState {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        SpacecraftState {
            orbit: OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v, 0.0]),
            attitude: AttitudeState::identity(),
            mass: 1000.0,
        }
    }

    /// Unit vector from the [`iss_state`] satellite toward the Sun, in the
    /// inertial frame — and, at identity attitude, in the body frame too.
    /// Aligning a panel normal with it makes cosθ exactly 1, so a face-on
    /// oracle is exact instead of "≈ 1". The geocentric Sun direction is not a
    /// substitute: the satellite sits 6771 km off the geocentre.
    fn sat_to_sun_unit(epoch: &Epoch) -> Vector3<f64> {
        let sun = sun::sun_position_eci(&epoch.to_tdb()).into_inner();
        (sun - iss_state().orbit.position()).normalize()
    }

    fn quat_from_axis_angle(axis: Vector3<f64>, angle: f64) -> Vector4<f64> {
        let q =
            nalgebra::UnitQuaternion::from_axis_angle(&nalgebra::Unit::new_normalize(axis), angle);
        Vector4::new(q.w, q.i, q.j, q.k)
    }

    // Basic

    #[test]
    fn panel_srp_name() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        assert_eq!(Model::<SpacecraftState>::name(&srp), "panel_srp");
    }

    #[test]
    fn no_epoch_returns_zero() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let loads = srp.eval(0.0, &iss_state(), None);
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    #[test]
    #[should_panic(expected = "normal must be unit length")]
    fn for_earth_rejects_a_non_unit_normal() {
        // `SpacecraftShape::Panels` is a public variant, so a shape can reach
        // the model without passing through `SpacecraftShape::panels`.
        let shape = SpacecraftShape::Panels(vec![SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 2.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: None,
        }]);
        PanelSrp::for_earth(shape);
    }

    #[test]
    fn for_earth_defaults() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
    }

    // Sphere

    #[test]
    fn sphere_nonzero_srp() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert!(loads.acceleration_inertial.magnitude() > 0.0);
    }

    #[test]
    fn sphere_zero_torque() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    #[test]
    fn sphere_attitude_independent() {
        let srp = PanelSrp::for_earth(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();

        let s1 = iss_state();
        let mut s2 = iss_state();
        s2.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 1.2);

        let l1 = srp.eval(0.0, &s1, Some(&epoch));
        let l2 = srp.eval(0.0, &s2, Some(&epoch));

        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "Sphere SRP should be attitude-independent"
        );
    }

    #[test]
    fn sphere_away_from_sun() {
        let srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        let sun_dir = sun::sun_direction_eci(&epoch.to_tdb()).into_inner();
        let cos_angle = loads
            .acceleration_inertial
            .into_inner()
            .normalize()
            .dot(&sun_dir);
        assert!(
            cos_angle < -0.5,
            "SRP should point away from Sun, cos_angle={cos_angle:.3}"
        );
    }

    #[test]
    fn sphere_matches_scalar_srp() {
        let epoch = test_epoch();
        let state = iss_state(); // mass = 1000.0

        // Sphere: area=20.0, cr=1.5 → area_to_mass = 20.0/1000.0 = 0.02
        let panel_srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5));

        let scalar_srp = SolarRadiationPressure::for_earth(Some(0.02))
            .with_cr(1.5)
            .without_shadow();

        let panel_loads = panel_srp.eval(0.0, &state, Some(&epoch));
        let scalar_a = scalar_srp.acceleration(state.orbit.position(), Some(&epoch));

        let rel_err = (panel_loads.acceleration_inertial.into_inner() - scalar_a).magnitude()
            / scalar_a.magnitude();
        assert!(
            rel_err < 1e-10,
            "PanelSrp sphere should match SolarRadiationPressure: rel_err={rel_err:.3e}"
        );
    }

    // Per-panel force law, against closed-form values.
    //
    // These drive `panel_force` directly. Going through `eval` would tie every
    // oracle to the Sun ephemeris, and would make the equivariance check
    // impossible: rotating the state does not carry the Sun along with it.

    /// Solar radiation pressure at 1 AU [N/m²]. Any positive value works — the
    /// oracles are linear in it — so use the real one rather than a round number.
    const TEST_PRESSURE: f64 = SOLAR_RADIATION_PRESSURE;
    /// Incidence well away from both face-on and edge-on, where a `cosθ` law
    /// and a `cos²θ` law are clearly distinguishable.
    const OBLIQUE_ANGLE: f64 = 0.6; // rad ≈ 34°

    /// Sun direction tilted `angle` from `+X` within the XY plane, so `+X` as a
    /// panel normal sees `cosθ = cos(angle)`.
    fn sun_tilted_in_xy(angle: f64) -> Vector3<f64> {
        Vector3::new(angle.cos(), angle.sin(), 0.0)
    }

    #[test]
    fn absorber_oblique_matches_pure_anti_sun() {
        // A black panel re-emits nothing, so the entire force is absorbed
        // photon momentum: along −ŝ, scaled by the projected area A·cosθ.
        let area = 4.0;
        let panel = SurfacePanel::at_com(
            area,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let s_body = sun_tilted_in_xy(OBLIQUE_ANGLE);

        let f = panel_force(&panel, &s_body, TEST_PRESSURE);
        let expected = -TEST_PRESSURE * area * OBLIQUE_ANGLE.cos() * s_body;

        let err = (f - expected).magnitude() / expected.magnitude();
        assert!(
            err < 1e-14,
            "black panel: expected {expected:?}, got {f:?}, rel_err={err:.3e}"
        );
    }

    #[test]
    fn specular_oblique_is_normal_directed() {
        // A mirror reflects the incident momentum about the normal, so the net
        // force is purely along −n̂ and follows cos²θ. This is the term an
        // anti-Sun-only force law drops entirely.
        let area = 4.0;
        let panel = SurfacePanel::at_com(
            area,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(1.0, 0.0),
        );
        let s_body = sun_tilted_in_xy(OBLIQUE_ANGLE);

        let f = panel_force(&panel, &s_body, TEST_PRESSURE);
        let cos = OBLIQUE_ANGLE.cos();
        let expected = -2.0 * TEST_PRESSURE * area * cos * cos * panel.normal;

        let err = (f - expected).magnitude() / expected.magnitude();
        assert!(
            err < 1e-14,
            "mirror: expected {expected:?}, got {f:?}, rel_err={err:.3e}"
        );
    }

    #[test]
    fn diffuse_face_on_matches_lambertian() {
        // Face-on, a Lambertian panel gives Cr = 1 + 2/3: the incident momentum
        // contributes 1, its isotropic re-emission a further 2/3.
        let area = 4.0;
        let normal = Vector3::new(1.0, 0.0, 0.0);
        let panel = SurfacePanel::at_com(area, normal, 2.2, PanelOptics::new(0.0, 1.0));

        let f = panel_force(&panel, &normal, TEST_PRESSURE);
        let expected = -(1.0 + 2.0 / 3.0) * TEST_PRESSURE * area * normal;

        let err = (f - expected).magnitude() / expected.magnitude();
        assert!(
            err < 1e-14,
            "Lambertian: expected {expected:?}, got {f:?}, rel_err={err:.3e}"
        );
    }

    #[test]
    fn mixed_optics_component_oracle() {
        // A solar-array-like surface at oblique incidence, checked one component
        // at a time. Projecting onto a direction perpendicular to n̂ isolates the
        // Sun-line coefficient and vice versa, so this cannot be satisfied by a
        // force of the right magnitude pointing the wrong way.
        let (specular, diffuse) = (0.2, 0.1);
        let area = 4.0;
        let panel = SurfacePanel::at_com(
            area,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(specular, diffuse),
        );
        let s_body = sun_tilted_in_xy(OBLIQUE_ANGLE);
        let normal = panel.normal;

        let f = panel_force(&panel, &s_body, TEST_PRESSURE);

        let (cos, sin) = (OBLIQUE_ANGLE.cos(), OBLIQUE_ANGLE.sin());
        let scale = -TEST_PRESSURE * area * cos;

        // Perpendicular within the ŝ–n̂ plane to one of the two directions.
        let perp_to_normal = (s_body - normal * cos).normalize();
        let perp_to_sun = (normal - s_body * cos).normalize();
        // ŝ·perp_to_normal = n̂·perp_to_sun = sinθ, so each projection is the
        // corresponding coefficient times sinθ.
        let sun_coeff = f.dot(&perp_to_normal) / sin;
        let normal_coeff = f.dot(&perp_to_sun) / sin;

        let expected_sun = scale * (1.0 - specular); // α + ρ_d = 1 − ρ_s
        let expected_normal = scale * 2.0 * (specular * cos + diffuse / 3.0);
        assert!(
            (sun_coeff - expected_sun).abs() < expected_sun.abs() * 1e-12,
            "Sun-line coefficient: expected {expected_sun:.6e}, got {sun_coeff:.6e}"
        );
        assert!(
            (normal_coeff - expected_normal).abs() < expected_normal.abs() * 1e-12,
            "normal coefficient: expected {expected_normal:.6e}, got {normal_coeff:.6e}"
        );

        // Nothing out of the ŝ–n̂ plane.
        let out_of_plane = f.dot(&s_body.cross(&normal).normalize());
        assert!(
            out_of_plane.abs() < f.magnitude() * 1e-14,
            "force should lie in the ŝ–n̂ plane, out-of-plane={out_of_plane:.3e}"
        );
    }

    #[test]
    fn edge_on_and_backside_are_zero() {
        let panel = SurfacePanel::at_com(
            4.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.4, 0.3),
        );

        for (label, s_body) in [
            ("edge-on", Vector3::new(0.0, 1.0, 0.0)),
            ("backside", Vector3::new(-1.0, 0.0, 0.0)),
        ] {
            let f = panel_force(&panel, &s_body, TEST_PRESSURE);
            assert_eq!(f, Vector3::zeros(), "{label} panel should feel no force");
        }
    }

    /// Over a full turn of the Sun direction, the plate has exactly one lit
    /// face, and its force is the one-sided law for whichever side is lit.
    ///
    /// The single-direction tests below say the pair works somewhere. This says
    /// it works everywhere, which is the claim `back_face` exists to make: no
    /// attitude leaves the plate dark, none lights both faces (which would
    /// double the force), and the lit face's own optics are the ones used.
    #[test]
    fn a_plate_has_exactly_one_lit_face_at_every_sun_direction() {
        let cells = PanelOptics::new(0.1, 0.2);
        let substrate = PanelOptics::new(0.05, 0.4);
        let front = SurfacePanel::at_com(4.0, Vector3::new(1.0, 0.0, 0.0), 2.2, cells)
            .with_cp_offset(Vector3::new(0.0, 1.5, 0.0));
        let back = front.back_face(substrate);

        // Enough steps to land near edge-on without landing exactly on it, and
        // an odd count so the two exact edge-on angles are never sampled.
        const STEPS: usize = 721;
        let mut lit_front = 0;
        let mut lit_back = 0;
        for i in 0..STEPS {
            let angle = std::f64::consts::TAU * (i as f64) / (STEPS as f64);
            let s_body = sun_tilted_in_xy(angle);

            let f_front = panel_force(&front, &s_body, TEST_PRESSURE);
            let f_back = panel_force(&back, &s_body, TEST_PRESSURE);
            let (front_lit, back_lit) = (f_front != Vector3::zeros(), f_back != Vector3::zeros());

            assert!(
                !(front_lit && back_lit),
                "both faces lit at {angle} rad, which would double the force"
            );
            assert!(
                front_lit || back_lit,
                "neither face lit at {angle} rad, so the plate has a dead zone"
            );

            // `angle` is measured from the front's normal, so the front owns
            // the half-turn around it and the back owns the other. Counting
            // alone would also pass with the two roles swapped, or with the
            // boundary somewhere other than a quarter turn.
            let front_faces_the_sun = angle.cos() > 0.0;
            assert_eq!(
                front_lit,
                front_faces_the_sun,
                "at {} deg the lit face is the wrong one: front_lit={front_lit}",
                angle.to_degrees()
            );

            if front_lit {
                lit_front += 1;
            } else {
                lit_back += 1;
            }

            // The lit face's force is the one-sided law with that face's own
            // optics, so the pair cannot be silently sharing the front's.
            // The optics come from the test, not from the panel: reading
            // `lit.optics` would make the oracle agree with a back face that
            // silently kept the front's surface.
            let (lit, force, optics) = if front_lit {
                (&front, f_front, cells)
            } else {
                (&back, f_back, substrate)
            };
            let cos_theta = lit.normal.dot(&s_body);
            let along_sun = optics.absorptivity() + optics.diffuse();
            let along_normal = 2.0 * (optics.specular() * cos_theta + optics.diffuse() / 3.0);
            let expected = -TEST_PRESSURE
                * lit.area
                * cos_theta
                * (along_sun * s_body + along_normal * lit.normal);
            assert!(
                (force - expected).magnitude() < 1e-18,
                "at {angle} rad: {force:?} vs {expected:?}"
            );
        }
        assert_eq!(
            lit_front + lit_back,
            STEPS,
            "every sample lit exactly one face"
        );
        // Half the turn each, to within the one sample that the odd count
        // leaves unpaired across the two boundaries.
        assert!(
            lit_front.abs_diff(lit_back) <= 1,
            "the two faces split the turn evenly: {lit_front} / {lit_back}"
        );
    }

    /// Exactly edge-on, the pair is dark rather than doubled.
    ///
    /// `cos θ` is +0.0 for one face and -0.0 for the other, and both are
    /// dropped by `cos_theta <= 0.0`. A zero-thickness plate seen edge-on has
    /// no projected area, so no force is the right answer.
    #[test]
    fn a_plate_seen_edge_on_feels_nothing_from_either_face() {
        let front = SurfacePanel::at_com(
            4.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let back = front.back_face(PanelOptics::new(0.05, 0.4));

        let edge_on = Vector3::new(0.0, 1.0, 0.0);
        assert_eq!(front.normal.dot(&edge_on), 0.0);
        assert_eq!(back.normal.dot(&edge_on), 0.0);
        assert_eq!(
            panel_force(&front, &edge_on, TEST_PRESSURE),
            Vector3::zeros()
        );
        assert_eq!(
            panel_force(&back, &edge_on, TEST_PRESSURE),
            Vector3::zeros()
        );
    }

    /// A panel standing in the Sun's way leaves the one behind it dark.
    ///
    /// Both plates face +x with the Sun on +x; the near one is bigger and sits
    /// closer to the Sun, so it covers the far one completely. Without occlusion
    /// both would produce force, which is the whole of #392.
    fn shaded_pair(near_half: f64) -> SpacecraftShape {
        let far = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let near = SurfacePanel::rectangle(
            [near_half, near_half],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        )
        .with_cp_offset(Vector3::new(2.0, 0.0, 0.0));
        SpacecraftShape::panels(vec![far, near])
    }

    fn panel_srp_force(shape: SpacecraftShape, s_body: &Vector3<f64>) -> Vector3<f64> {
        let SpacecraftShape::Panels(panels) = shape else {
            panic!("expected panels");
        };
        let mut total = Vector3::zeros();
        for panel in &panels {
            if crate::spacecraft::surface::is_fully_occluded(panel, &panels, s_body) {
                continue;
            }
            total += panel_force(panel, s_body, TEST_PRESSURE);
        }
        total
    }

    /// The shaded panel is excluded by `PanelSrp::eval`, not just by the
    /// occlusion test called on its own.
    ///
    /// The tests below reach `is_fully_occluded` through a helper, so deleting
    /// the call inside `eval` would leave them green. This one goes through the
    /// model and checks the torque as well as the acceleration: the hidden
    /// panel has its own `cp_offset`, so a force wrongly attributed to it shows
    /// up there even when the magnitudes happen to be close.
    #[test]
    fn eval_excludes_a_shaded_panel_from_both_the_force_and_the_torque() {
        let epoch = test_epoch();
        let state = iss_state();
        // Sun direction in the body frame, so the plates can be aimed at it.
        let s_body = sat_to_sun_unit(&epoch);

        let optics = PanelOptics::new(0.1, 0.2);
        let in_plane = s_body.cross(&Vector3::new(0.0, 0.0, 1.0)).normalize();
        // Behind the near plate, and off-centre so it would torque if it were lit.
        let hidden = SurfacePanel::rectangle([0.4, 0.4], in_plane, s_body, 2.2, optics)
            .with_cp_offset(in_plane * 1.5);
        let near = SurfacePanel::rectangle([1.2, 1.2], in_plane, s_body, 2.2, optics)
            .with_cp_offset(in_plane * 1.5 + s_body * 2.0);

        let pair = PanelSrp::for_earth(SpacecraftShape::panels(vec![hidden.clone(), near.clone()]));
        let alone = PanelSrp::for_earth(SpacecraftShape::panels(vec![near]));
        let hidden_only = PanelSrp::for_earth(SpacecraftShape::panels(vec![hidden]));

        let pair = pair.eval(0.0, &state, Some(&epoch));
        let alone = alone.eval(0.0, &state, Some(&epoch));
        let hidden_only = hidden_only.eval(0.0, &state, Some(&epoch));

        assert!(
            hidden_only.acceleration_inertial.magnitude() > 0.0,
            "the hidden plate is lit when nothing is in front of it"
        );
        assert!(
            hidden_only.torque_body.into_inner().magnitude() > 0.0,
            "and it torques, so its exclusion is visible in the torque"
        );

        let (a_pair, a_alone) = (
            pair.acceleration_inertial.into_inner(),
            alone.acceleration_inertial.into_inner(),
        );
        assert!(
            (a_pair - a_alone).magnitude() / a_alone.magnitude() < 1e-12,
            "the pair's acceleration must be the near plate's alone: {a_pair:?} vs {a_alone:?}"
        );
        let (t_pair, t_alone) = (
            pair.torque_body.into_inner(),
            alone.torque_body.into_inner(),
        );
        assert!(
            (t_pair - t_alone).magnitude() / t_alone.magnitude() < 1e-12,
            "and its torque likewise: {t_pair:?} vs {t_alone:?}"
        );
    }

    /// An outline written through a struct literal is checked where the panels
    /// enter a model, since `rectangle`'s asserts can be bypassed.
    ///
    /// `SpacecraftShape::Panels` is written as the variant rather than through
    /// `SpacecraftShape::panels`, which validates on its own: going through the
    /// constructor would panic there and leave the model's own check untested.
    #[test]
    #[should_panic(expected = "perpendicular to the normal")]
    fn a_struct_literal_outline_with_a_parallel_axis_is_rejected() {
        let normal = Vector3::new(1.0, 0.0, 0.0);
        let panel = SurfacePanel {
            area: 4.0,
            normal,
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: Some(crate::spacecraft::PanelOutline::Rectangle {
                half_extent: [1.0, 1.0],
                in_plane_x: normal, // parallel, so both in-plane axes degenerate
            }),
        };
        let _ = PanelSrp::for_earth(SpacecraftShape::Panels(vec![panel]));
    }

    /// `for_body` carries the same invariant, and it is the constructor
    /// `orts::setup` reaches for, so config input arrives through it.
    #[test]
    #[should_panic(expected = "does not match the outline")]
    fn for_body_rejects_an_outline_inconsistent_with_the_area() {
        let panel = SurfacePanel {
            area: 99.0, // the outline says 4.0
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: Some(crate::spacecraft::PanelOutline::Rectangle {
                half_extent: [1.0, 1.0],
                in_plane_x: Vector3::new(0.0, 1.0, 0.0),
            }),
        };
        let _ = PanelSrp::for_body(
            arika::body::KnownBody::Earth,
            SpacecraftShape::Panels(vec![panel]),
        );
    }

    /// And `SpacecraftShape::panels` still establishes it, which is what lets
    /// the tests above write the variant directly.
    #[test]
    #[should_panic(expected = "does not match the outline")]
    fn the_panels_constructor_rejects_an_inconsistent_outline() {
        let panel = SurfacePanel {
            area: 99.0,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: Some(crate::spacecraft::PanelOutline::Rectangle {
                half_extent: [1.0, 1.0],
                in_plane_x: Vector3::new(0.0, 1.0, 0.0),
            }),
        };
        let _ = SpacecraftShape::panels(vec![panel]);
    }

    /// An infinite stored area passes a relative comparison against itself.
    #[test]
    #[should_panic(expected = "area must be positive and finite")]
    fn a_struct_literal_with_an_infinite_area_is_rejected() {
        let panel = SurfacePanel {
            area: f64::INFINITY,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: Some(crate::spacecraft::PanelOutline::Rectangle {
                half_extent: [1.0, 1.0],
                in_plane_x: Vector3::new(0.0, 1.0, 0.0),
            }),
        };
        let _ = PanelSrp::for_earth(SpacecraftShape::Panels(vec![panel]));
    }

    /// And an area that disagrees with the outline it claims.
    #[test]
    #[should_panic(expected = "does not match the outline")]
    fn a_struct_literal_outline_inconsistent_with_the_area_is_rejected() {
        let panel = SurfacePanel {
            area: 99.0, // the outline says 4.0
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: Some(crate::spacecraft::PanelOutline::Rectangle {
                half_extent: [1.0, 1.0],
                in_plane_x: Vector3::new(0.0, 1.0, 0.0),
            }),
        };
        let _ = PanelSrp::for_earth(SpacecraftShape::Panels(vec![panel]));
    }

    #[test]
    fn a_panel_behind_a_larger_one_feels_nothing() {
        let sun = Vector3::new(1.0, 0.0, 0.0);
        // The near plate is 2 m on a side against the far one's 2 m, and sits
        // between it and the Sun, so it covers it exactly.
        let covered = panel_srp_force(shaded_pair(1.0), &sun);
        let one_panel = panel_force(
            &SurfacePanel::rectangle(
                [1.0, 1.0],
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                2.2,
                PanelOptics::new(0.1, 0.2),
            )
            .with_cp_offset(Vector3::new(2.0, 0.0, 0.0)),
            &sun,
            TEST_PRESSURE,
        );
        assert!(
            (covered - one_panel).magnitude() < 1e-18,
            "only the near plate should contribute: {covered:?} vs {one_panel:?}"
        );
    }

    #[test]
    fn a_panel_only_partly_covered_still_feels_the_sun() {
        let sun = Vector3::new(1.0, 0.0, 0.0);
        // Half the width, so it cannot cover the far plate's corners.
        let force = panel_srp_force(shaded_pair(0.5), &sun);
        let both = {
            let SpacecraftShape::Panels(panels) = shaded_pair(0.5) else {
                panic!("expected panels");
            };
            panels
                .iter()
                .map(|p| panel_force(p, &sun, TEST_PRESSURE))
                .sum::<Vector3<f64>>()
        };
        assert!(
            (force - both).magnitude() < 1e-18,
            "partial cover is not occlusion here: {force:?} vs {both:?}"
        );
    }

    /// Which plate shields which follows the incoming direction, not the order
    /// they were written in.
    #[test]
    fn the_shielding_role_follows_the_incoming_direction() {
        let SpacecraftShape::Panels(panels) = shaded_pair(1.0) else {
            panic!("expected panels");
        };
        let occluded = |sun: Vector3<f64>| {
            (
                crate::spacecraft::surface::is_fully_occluded(&panels[0], &panels, &sun),
                crate::spacecraft::surface::is_fully_occluded(&panels[1], &panels, &sun),
            )
        };

        // Sun on +x: the plate at x = 2 stands in front of the one at the origin.
        assert_eq!(occluded(Vector3::new(1.0, 0.0, 0.0)), (true, false));
        // Sun on -x: the roles swap, because the origin plate is now the near one.
        assert_eq!(occluded(Vector3::new(-1.0, 0.0, 0.0)), (false, true));
        // Along the plates: neither stands in the other's way.
        assert_eq!(occluded(Vector3::new(0.0, 1.0, 0.0)), (false, false));
    }

    /// A panel with no outline is invisible to the shadow test, both ways.
    #[test]
    fn an_outlineless_panel_neither_shades_nor_is_shaded() {
        let sun = Vector3::new(1.0, 0.0, 0.0);
        let bare = SurfacePanel::at_com(
            4.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let big = SurfacePanel::rectangle(
            [4.0, 4.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        )
        .with_cp_offset(Vector3::new(2.0, 0.0, 0.0));

        let panels = vec![bare.clone(), big.clone()];
        assert!(
            !crate::spacecraft::surface::is_fully_occluded(&panels[0], &panels, &sun),
            "a panel with no outline cannot be shaded"
        );

        // And the other way round: an outlineless panel in front shades nothing.
        let far = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let bare_near = SurfacePanel::at_com(
            64.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        )
        .with_cp_offset(Vector3::new(2.0, 0.0, 0.0));
        let panels = vec![far, bare_near];
        assert!(
            !crate::spacecraft::surface::is_fully_occluded(&panels[0], &panels, &sun),
            "a panel with no outline cannot shade"
        );
    }

    /// The two faces of one plate share a plane, so neither shades the other.
    #[test]
    fn the_two_faces_of_a_plate_do_not_shade_each_other() {
        let front = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let back = front.back_face(PanelOptics::new(0.05, 0.4));
        let panels = vec![front, back];
        for sun in [Vector3::new(1.0, 0.0, 0.0), Vector3::new(-1.0, 0.0, 0.0)] {
            for (i, panel) in panels.iter().enumerate() {
                assert!(
                    !crate::spacecraft::surface::is_fully_occluded(panel, &panels, &sun),
                    "coincident faces must not shade each other (sun {sun:?}, panel {i})"
                );
            }
        }
    }

    /// The back face inherits the outline, since it is the same plate.
    #[test]
    fn back_face_carries_the_outline() {
        let front = SurfacePanel::rectangle(
            [1.0, 2.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.1, 0.2),
        );
        let back = front.back_face(PanelOptics::absorber());
        assert_eq!(back.outline, front.outline);
        assert_eq!(back.area, front.area);
    }

    /// The back face is what covers the attitudes the front cannot.
    ///
    /// A thin plate written as one panel produces nothing whenever the Sun is
    /// behind it, which is half of the attitudes a spinning spacecraft sees.
    /// With both faces present, exactly one of them is lit at any oblique
    /// incidence, and it is the one facing the Sun.
    #[test]
    fn a_thin_plate_needs_both_faces_to_cover_every_attitude() {
        let cells = PanelOptics::new(0.1, 0.2);
        let substrate = PanelOptics::new(0.05, 0.4);
        let front = SurfacePanel::at_com(4.0, Vector3::new(1.0, 0.0, 0.0), 2.2, cells)
            .with_cp_offset(Vector3::new(0.0, 1.5, 0.0));
        let back = front.back_face(substrate);

        // Oblique, not face-on: the claim is about every attitude, and the
        // normal term the force law adds is the one that depends on cos θ.
        let from_front = Vector3::new(0.6, 0.8, 0.0).normalize();
        let from_back = -from_front;
        assert!(
            front.normal.dot(&from_front) < 0.99,
            "the test direction has to be off the normal to exercise cos θ"
        );

        // One face lit, the other dark, whichever side the Sun is on.
        assert!(panel_force(&front, &from_front, TEST_PRESSURE).magnitude() > 0.0);
        assert_eq!(
            panel_force(&back, &from_front, TEST_PRESSURE),
            Vector3::zeros()
        );
        assert_eq!(
            panel_force(&front, &from_back, TEST_PRESSURE),
            Vector3::zeros()
        );
        assert!(panel_force(&back, &from_back, TEST_PRESSURE).magnitude() > 0.0);

        // The two sides differ in more than sign: their optics differ, so the
        // same illumination geometry gives different force magnitudes.
        let f_front = panel_force(&front, &from_front, TEST_PRESSURE);
        let f_back = panel_force(&back, &from_back, TEST_PRESSURE);
        assert!(
            (f_front.magnitude() - f_back.magnitude()).abs() > 1e-12,
            "front {} vs back {}",
            f_front.magnitude(),
            f_back.magnitude()
        );

        // Both act through the same point, so the torque flips with the force.
        let tau_front = front.cp_offset.cross(&f_front);
        let tau_back = back.cp_offset.cross(&f_back);
        assert!(
            tau_front.dot(&tau_back) < 0.0,
            "the two sides are pushed opposite ways, so their torques oppose: \
             {tau_front:?} vs {tau_back:?}"
        );
    }

    #[test]
    fn panel_force_equivariance_under_body_rotation() {
        // Rotating the panel and the Sun direction together must rotate the
        // force by the same amount: the law may not favour any body axis.
        use nalgebra::{Unit, UnitQuaternion};

        let panel = SurfacePanel::at_com(
            4.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.2, 0.1),
        );
        let s_body = sun_tilted_in_xy(OBLIQUE_ANGLE);
        let f = panel_force(&panel, &s_body, TEST_PRESSURE);

        // An axis off every coordinate axis, so a per-axis mistake cannot hide.
        let rot =
            UnitQuaternion::from_axis_angle(&Unit::new_normalize(Vector3::new(1.0, 2.0, 3.0)), 0.7);
        let rotated = SurfacePanel {
            normal: rot * panel.normal,
            ..panel.clone()
        };
        let f_rotated = panel_force(&rotated, &(rot * s_body), TEST_PRESSURE);

        let err = (f_rotated - rot * f).magnitude() / f.magnitude();
        assert!(
            err < 1e-14,
            "force should be equivariant, rel_err={err:.3e}"
        );
    }

    /// Orekit cross-validation of the per-panel force.
    ///
    /// The closed-form tests above are independent of the implementation but
    /// not of the formula: a shared error in the flat-plate law, or a
    /// misreading of what the coefficients mean, would satisfy them. Orekit's
    /// paneled radiation model is a separate implementation of the same
    /// physics, so agreeing with it pins the formula and the convention.
    ///
    /// Fixture from `tools/generate_orekit_panel_srp_fixtures.py`, which drives
    /// Orekit's `RadiationSensitive::radiationPressureAcceleration` on a
    /// one-panel spacecraft. That needs no propagator and no attitude provider,
    /// which is why a force oracle is available where a torque one is not:
    /// Orekit's paneled model returns an acceleration only. The torque this
    /// model builds from the force is pinned by the exact cross-product tests.
    #[test]
    fn orekit_panel_force_reference() {
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            specular: f64,
            diffuse: f64,
            incidence_deg: f64,
            force_body_n: [f64; 3],
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            pressure_n_m2: f64,
            area_m2: f64,
            panel_normal_body: [f64; 3],
            cases: Vec<Case>,
        }

        let raw = include_str!("../../tests/fixtures/orekit_panel_srp_reference.json");
        let fx: Fixture = serde_json::from_str(raw).expect("fixture parses");
        assert!(fx.cases.len() >= 12, "expected the full case set");

        let normal = Vector3::from_row_slice(&fx.panel_normal_body);
        for case in &fx.cases {
            let optics = PanelOptics::new(case.specular, case.diffuse);
            let panel = SurfacePanel::at_com(fx.area_m2, normal, 2.2, optics);
            let th = case.incidence_deg.to_radians();
            let s_body = Vector3::new(th.sin(), 0.0, th.cos());

            let ours = panel_force(&panel, &s_body, fx.pressure_n_m2);
            let theirs = Vector3::from_row_slice(&case.force_body_n);

            let err = (ours - theirs).magnitude() / theirs.magnitude();
            assert!(
                err < 1e-12,
                "{}: orekit {:?}, ours {:?}, rel_err={:.3e}",
                case.name,
                theirs,
                ours,
                err
            );
        }
    }

    // Ideal single panel + single Sun direction

    #[test]
    fn single_panel_face_on_analytical() {
        // A single black panel facing the Sun at identity attitude.
        // At March equinox, Sun is roughly +X, satellite at +X.
        // Panel normal = +X in body frame, identity attitude → +X in inertial.
        // Default optics absorb everything, so F = -P * A * cos(θ) * ŝ with
        // cos(θ) ≈ 1 — no reflection term, hence no Cr factor.
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let state = iss_state(); // at +X, identity attitude

        let loads = srp.eval(0.0, &state, Some(&epoch));

        // Expected magnitude: P_sr * (AU/r_sun)^2 * A * cos(θ) / (mass * 1000)
        // cos(θ) ≈ 1 (panel faces Sun), r_sun ≈ AU
        let expected_a = SOLAR_RADIATION_PRESSURE * 10.0 / (1000.0 * 1000.0);
        let actual_a = loads.acceleration_inertial.magnitude();

        let rel_err = (actual_a - expected_a).abs() / expected_a;
        assert!(
            rel_err < 0.05,
            "Single panel face-on: expected ~{expected_a:.3e}, got {actual_a:.3e}, rel_err={rel_err:.3}"
        );

        // Direction should be away from Sun (roughly -X)
        let sun_dir = sun::sun_direction_eci(&epoch.to_tdb()).into_inner();
        assert!(
            loads
                .acceleration_inertial
                .into_inner()
                .normalize()
                .dot(&sun_dir)
                < -0.5
        );

        // No torque (panel at CoM)
        assert!(loads.torque_body.magnitude() < 1e-20);
    }

    #[test]
    fn single_panel_backface_zero() {
        // Panel normal = -X (facing away from Sun), should get zero force.
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(-1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let state = iss_state(); // at +X, Sun roughly at +X

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.acceleration_inertial.magnitude() < 1e-20,
            "Backface panel should produce zero SRP"
        );
    }

    // Panels — scaling

    #[test]
    fn panel_force_scales_with_area() {
        let epoch = test_epoch();
        let state = iss_state();

        let p1 = SurfacePanel::at_com(
            5.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let p2 = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );

        let l1 = PanelSrp::new(SpacecraftShape::panels(vec![p1])).eval(0.0, &state, Some(&epoch));
        let l2 = PanelSrp::new(SpacecraftShape::panels(vec![p2])).eval(0.0, &state, Some(&epoch));

        let ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "2x area should give 2x force, ratio={ratio}"
        );
    }

    #[test]
    fn panel_force_scales_with_reflectivity() {
        // Face-on, a mirror (Cr = 2) pushes exactly twice as hard as a black
        // panel (Cr = 1). Aligning the normal with the satellite-to-Sun vector
        // makes cosθ exactly 1, so the factor is 2 and not 2cosθ.
        let epoch = test_epoch();
        let state = iss_state();
        let normal = sat_to_sun_unit(&epoch);

        let absorber = SurfacePanel::at_com(10.0, normal, 2.2, PanelOptics::absorber());
        let mirror = SurfacePanel::at_com(10.0, normal, 2.2, PanelOptics::new(1.0, 0.0));

        let l1 =
            PanelSrp::new(SpacecraftShape::panels(vec![absorber])).eval(0.0, &state, Some(&epoch));
        let l2 =
            PanelSrp::new(SpacecraftShape::panels(vec![mirror])).eval(0.0, &state, Some(&epoch));

        let ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "A mirror should push 2x a black panel face-on, ratio={ratio}"
        );
    }

    // Attitude coupling

    #[test]
    fn panels_different_attitude_different_srp() {
        // Use a panel normal aligned with the actual Sun direction for a clean test.
        let epoch = test_epoch();
        let sun_dir = sun::sun_direction_eci(&epoch.to_tdb()).into_inner();

        let panel = SurfacePanel::at_com(10.0, sun_dir, 2.2, PanelOptics::absorber());
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));

        // Identity attitude: panel faces Sun → non-zero SRP
        let s1 = iss_state();
        let l1 = srp.eval(0.0, &s1, Some(&epoch));

        // Rotated 90° about Z: panel perpendicular to Sun → zero SRP
        let mut s2 = iss_state();
        s2.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2);
        let l2 = srp.eval(0.0, &s2, Some(&epoch));

        assert!(l1.acceleration_inertial.magnitude() > 1e-15);
        // After 90° rotation the cos(θ) is very small but might not be exactly zero
        // due to Sun direction having a small Z component. Check it's much smaller.
        assert!(
            l2.acceleration_inertial.magnitude() < l1.acceleration_inertial.magnitude() * 0.1,
            "90° rotation should drastically reduce SRP: face-on={:.3e}, rotated={:.3e}",
            l1.acceleration_inertial.magnitude(),
            l2.acceleration_inertial.magnitude()
        );
    }

    // Shadow

    #[test]
    fn panel_zero_in_shadow() {
        // Panel normal = +X (faces Sun from -X side), but satellite is behind
        // Earth in shadow → shadow function should zero out the force.
        // We rotate the body 180° about Z so that body +X points toward +X in inertial
        // (the Sun direction), ensuring the panel *would* receive SRP if sunlit.
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(-1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::for_earth(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();

        // Place satellite behind Earth (Sun is roughly +X at equinox)
        let mut state = iss_state();
        state.orbit = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        // Rotate 180° about Z so body -X → inertial +X (toward Sun)
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::PI);

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert_eq!(
            loads.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "Should be zero in shadow"
        );

        // Verify it *would* be non-zero without shadow (confirms we're testing shadow, not backface)
        let srp_no_shadow = PanelSrp::new(SpacecraftShape::panels(vec![SurfacePanel::at_com(
            10.0,
            Vector3::new(-1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        )]));
        let loads_no_shadow = srp_no_shadow.eval(0.0, &state, Some(&epoch));
        assert!(
            loads_no_shadow.acceleration_inertial.magnitude() > 0.0,
            "Without shadow, the same panel should receive SRP"
        );
    }

    #[test]
    fn no_shadow_body_always_sunlit() {
        // Place satellite behind Earth at -X with a Sun-facing panel.
        // With shadow_body_radius=None, the force should still be non-zero.
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(-1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel])); // no shadow

        let epoch = test_epoch();
        let mut state = iss_state();
        state.orbit = OrbitalState::new(
            vector![-(R_EARTH + 400.0), 0.0, 0.0],
            vector![0.0, -7.67, 0.0],
        );
        // Rotate 180° about Z so body -X → inertial +X (toward Sun)
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::PI);

        let loads = srp.eval(0.0, &state, Some(&epoch));
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Without shadow body, satellite behind Earth should still get SRP"
        );

        // And verify that with shadow it would be zero
        let srp_with_shadow =
            PanelSrp::for_earth(SpacecraftShape::panels(vec![SurfacePanel::at_com(
                10.0,
                Vector3::new(-1.0, 0.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            )]));
        let loads_shadow = srp_with_shadow.eval(0.0, &state, Some(&epoch));
        assert_eq!(
            loads_shadow.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "With shadow body, same satellite should be in shadow"
        );
    }

    // Frame-awareness (CIRS): the GCRS Sun ephemeris must be rotated into the
    // integration frame. SimpleEci/Gcrs use an identity rotation (behaviour
    // preserved by every test above); CIRS applies a real ~0.3° rotation.

    /// Re-derives the sphere-branch acceleration [km/s²] for a given Sun
    /// position, mirroring `loads_from_state` (no shadow → illumination = 1).
    fn sphere_accel(
        sat: &Vector3<f64>,
        sun_pos: &Vector3<f64>,
        area: f64,
        cr: f64,
        mass: f64,
    ) -> Vector3<f64> {
        let sat_to_sun = sun_pos - sat;
        let r_sun = sat_to_sun.magnitude();
        let s_hat = sat_to_sun / r_sun;
        let dr = sun::AU_KM / r_sun;
        let base_pressure = SOLAR_RADIATION_PRESSURE * dr * dr;
        let a_mag = base_pressure * cr * area / mass / 1000.0;
        -a_mag * s_hat
    }

    #[test]
    fn cirs_sphere_eval_rotates_the_sun_ephemeris() {
        use arika::frame::{Cirs, Gcrs, Rotation};

        // Sphere is attitude-independent, so this isolates the Sun rotation.
        let srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5)); // no shadow
        let epoch = test_epoch();
        let sat = vector![7000.0, 1000.0, 500.0];
        let orbit = OrbitalState::<Cirs>::new_in_frame(sat, vector![0.0, 7.5, 0.0]);
        let att = AttitudeState::identity();

        let a_cirs = *srp
            .loads_from_state(&orbit, att.rotation_tagged_as(), 1000.0, Some(&epoch))
            .acceleration_inertial
            .inner();

        let sun_gcrs = sun::sun_position_eci(&epoch.to_tdb());
        let sun_cirs = Rotation::<Gcrs, Cirs>::iau2006_model(&epoch.to_tt()).transform(&sun_gcrs);
        // `loads_from_state` recomputes the *identical* model rotation + formula,
        // so this is bit-exact (the difference is 0.0, not f64 noise). The tight
        // bound is intentional: it pins that CIRS uses the EOP-free model
        // rotation — an EOP-corrected (dX/dY) variant would shift ~5e-18.
        let expected = sphere_accel(&sat, sun_cirs.inner(), 20.0, 1.5, 1000.0);
        assert!(
            (a_cirs - expected).norm() < 1e-18,
            "CIRS eval must apply the GCRS→CIRS Sun-ephemeris rotation"
        );

        let raw = sphere_accel(&sat, sun_gcrs.inner(), 20.0, 1.5, 1000.0);
        assert!(
            (a_cirs - raw).norm() > raw.norm() * 1e-4,
            "CIRS eval should differ from the raw GCRS-aligned result"
        );
    }

    #[test]
    fn cirs_panel_eval_rotates_the_sun_ephemeris() {
        use arika::frame::Cirs;

        // A Sun-facing panel drives the longer panel code path
        // (s_hat → body → force → back to F via `rotation_*_inertial::<F>`).
        // With identity attitude those rotations are no-ops, so this pins the
        // ephemeris rotation reaching the panel formula — not the attitude
        // accessors, which are covered by SimpleEci behaviour-preservation and
        // compile-time typing.
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let sat = vector![7000.0, 1000.0, 500.0];
        let att = AttitudeState::identity();

        let cirs = OrbitalState::<Cirs>::new_in_frame(sat, vector![0.0, 7.5, 0.0]);
        let simple = OrbitalState::new(sat, vector![0.0, 7.5, 0.0]); // SimpleEci
        let a_cirs = *srp
            .loads_from_state(&cirs, att.rotation_tagged_as(), 1000.0, Some(&epoch))
            .acceleration_inertial
            .inner();
        let a_simple = *srp
            .loads_from_state(&simple, att.rotation_tagged_as(), 1000.0, Some(&epoch))
            .acceleration_inertial
            .inner();

        // Same raw position & attitude → the only numerical difference is the
        // rotated Sun ephemeris propagating through the panel formula. A non-zero
        // difference proves CIRS applies the GCRS→CIRS rotation (SimpleEci is
        // identity); identical results would mean the frame was ignored.
        assert!(a_simple.norm() > 0.0, "panel should receive SRP");
        assert!(
            (a_cirs - a_simple).norm() > a_simple.norm() * 1e-4,
            "CIRS panel eval should differ from the SimpleEci (identity) result"
        );
    }

    // Torque

    #[test]
    fn panels_cp_offset_produces_torque() {
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(0.0, 1.0, 0.0), // 1 m offset in +y
            outline: None,
        };
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        assert!(
            loads.torque_body.magnitude() > 0.0,
            "Offset CP should produce non-zero torque"
        );
    }

    #[test]
    fn panels_cp_at_com_zero_torque() {
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Panel at CoM should produce zero torque"
        );
    }

    #[test]
    fn torque_cross_product_direction() {
        // Panel normal +X, CP offset (0, 1, 0).
        // Force is along -X in body frame (away from Sun).
        // τ = r × F = (0,1,0) × (F_x,0,0) = (0*0-0*F_x, 0*F_x-1*0, ... ) → z-component
        // Actually: (0,1,0) × (F_x,0,0) = (1*0 - 0*0, 0*F_x - 0*0, 0*0 - 1*F_x) = (0, 0, -F_x)
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(1.0, 0.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(0.0, 1.0, 0.0),
            outline: None,
        };
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));

        // Torque should be primarily about z-axis, and negative
        // τ = (0,1,0) × F where F is mostly along -X → (0,0,-F_x) with F_x < 0
        // so z-component depends on the Sun direction body-frame projection
        assert!(
            loads.torque_body.z().abs() > loads.torque_body.x().abs(),
            "Torque should be primarily about z-axis"
        );
        assert!(
            loads.torque_body.z().abs() > loads.torque_body.y().abs(),
            "Torque should be primarily about z-axis"
        );
        // Force is in -ŝ direction (away from Sun). Sun ≈ +X at equinox,
        // so force ≈ -X in body. τ_z = r_x*F_y - r_y*F_x = 0 - 1*F_x.
        // F_x < 0, so τ_z = -F_x > 0.
        assert!(
            loads.torque_body.z() > 0.0,
            "Torque z-component should be positive: τ_z={:.3e}",
            loads.torque_body.z()
        );
    }

    /// Solar radiation pressure at the [`iss_state`] satellite for `epoch`,
    /// scaled for heliocentric distance. Rebuilt here so torque oracles do not
    /// borrow the model's own intermediate value.
    fn pressure_at_iss(epoch: &Epoch) -> f64 {
        let sun = sun::sun_position_eci(&epoch.to_tdb()).into_inner();
        let r_sun = (sun - iss_state().orbit.position()).magnitude();
        SOLAR_RADIATION_PRESSURE * (sun::AU_KM / r_sun).powi(2)
    }

    #[test]
    fn torque_exact_cross_product_out_of_plane() {
        // The centre of pressure sits off the ŝ–n̂ plane, where r × ŝ and r × n̂
        // point in genuinely different directions. A force law that pushes only
        // along −ŝ therefore gets the torque *direction* wrong, not merely its
        // magnitude — so this pins the torque against a full closed-form force
        // and then shows it is measurably off the anti-Sun-only answer.
        let epoch = test_epoch();
        let s_body = sat_to_sun_unit(&epoch); // identity attitude → body == inertial
        let area = 8.0;
        let optics = PanelOptics::new(0.3, 0.2);
        // Normal 45° between +X and +Z, so cosθ ≈ 0.7 against the near-+X Sun,
        // and an offset along +Y, perpendicular to that plane.
        let normal = Vector3::new(1.0, 0.0, 1.0).normalize();
        let cp_offset = Vector3::new(0.0, 1.0, 0.0);
        let panel = SurfacePanel {
            area,
            normal,
            cd: 2.2,
            optics,
            cp_offset,
            outline: None,
        };

        let loads = PanelSrp::new(SpacecraftShape::panels(vec![panel])).eval(
            0.0,
            &iss_state(),
            Some(&epoch),
        );
        let tau = loads.torque_body.into_inner();

        let pressure = pressure_at_iss(&epoch);
        let cos = normal.dot(&s_body);
        assert!(cos > 0.6, "panel should be well illuminated, cosθ={cos:.3}");
        let force = -pressure
            * area
            * cos
            * ((optics.absorptivity() + optics.diffuse()) * s_body
                + 2.0 * (optics.specular() * cos + optics.diffuse() / 3.0) * normal);
        let expected = cp_offset.cross(&force);

        let err = (tau - expected).magnitude() / expected.magnitude();
        assert!(
            err < 1e-12,
            "τ should equal r × F: expected {expected:?}, got {tau:?}, rel_err={err:.3e}"
        );

        // The superseded anti-Sun-only law, at the lumped Cr this panel replaced.
        let anti_sun_only = -pressure * area * cos * 1.5 * s_body;
        let tau_anti_sun_only = cp_offset.cross(&anti_sun_only);
        let alignment = tau.normalize().dot(&tau_anti_sun_only.normalize());
        assert!(
            alignment < 0.99,
            "an anti-Sun-only force gives a different torque direction; \
             alignment={alignment:.6} means the normal term is missing"
        );
    }

    #[test]
    fn torque_exact_under_non_identity_attitude() {
        // Under a real attitude the Sun direction must be rotated into the body
        // frame before the cross product, and the torque must stay in the body
        // frame. Computing either in the inertial frame breaks this.
        let epoch = test_epoch();
        let angle = 0.9;
        let mut state = iss_state();
        state.attitude.quaternion = quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), angle);

        let area = 6.0;
        let optics = PanelOptics::new(0.25, 0.15);
        let normal = Vector3::new(1.0, 0.0, 0.0);
        let cp_offset = Vector3::new(0.0, 0.0, 1.2);
        let panel = SurfacePanel {
            area,
            normal,
            cd: 2.2,
            optics,
            cp_offset,
            outline: None,
        };

        let loads =
            PanelSrp::new(SpacecraftShape::panels(vec![panel])).eval(0.0, &state, Some(&epoch));
        let tau = loads.torque_body.into_inner();

        // Rotate the Sun direction into the body frame by hand: a rotation of
        // the body by +angle about Z takes an inertial vector to the body frame
        // by −angle.
        let s_inertial = sat_to_sun_unit(&epoch);
        let s_body = Vector3::new(
            angle.cos() * s_inertial.x + angle.sin() * s_inertial.y,
            -angle.sin() * s_inertial.x + angle.cos() * s_inertial.y,
            s_inertial.z,
        );

        let pressure = pressure_at_iss(&epoch);
        let cos = normal.dot(&s_body);
        assert!(cos > 0.5, "panel should be well illuminated, cosθ={cos:.3}");
        let force = -pressure
            * area
            * cos
            * ((optics.absorptivity() + optics.diffuse()) * s_body
                + 2.0 * (optics.specular() * cos + optics.diffuse() / 3.0) * normal);
        let expected = cp_offset.cross(&force);

        let err = (tau - expected).magnitude() / expected.magnitude();
        assert!(
            err < 1e-12,
            "τ should equal r × F in the body frame: expected {expected:?}, \
             got {tau:?}, rel_err={err:.3e}"
        );
    }

    // Integration with SpacecraftDynamics

    #[test]
    fn panel_srp_integrable() {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::SpacecraftDynamics;
        use utsuroi::{DynamicalSystem, Integrator, Rk4};

        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, nalgebra::Matrix3::identity())
            .with_model(PanelSrp::new(SpacecraftShape::panels(vec![panel])))
            .with_epoch(test_epoch());

        let state = iss_state();
        let dy = dynamics.derivatives(0.0, &state.clone().into());
        assert!(dy.plant.orbit.position().magnitude().is_finite());

        // One RK4 step
        let next = Rk4.step(&dynamics, 0.0, &state.into(), 10.0);
        assert!(next.plant.orbit.position().magnitude().is_finite());
        assert!(next.plant.attitude.quaternion.magnitude() > 0.99);
    }

    #[test]
    fn panel_srp_plus_panel_drag_combined() {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::{PanelDrag, SpacecraftDynamics};
        use utsuroi::DynamicalSystem;

        let panels = vec![
            SurfacePanel::at_com(
                10.0,
                Vector3::new(1.0, 0.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            ),
            SurfacePanel::at_com(
                10.0,
                Vector3::new(0.0, -1.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            ),
        ];
        let shape = SpacecraftShape::panels(panels);

        let dynamics = SpacecraftDynamics::new(MU_EARTH, PointMass, nalgebra::Matrix3::identity())
            .with_model(PanelDrag::for_earth(shape.clone()))
            .with_model(PanelSrp::new(shape))
            .with_epoch(test_epoch());

        let state = iss_state();
        let dy = dynamics.derivatives(0.0, &state.into());
        assert!(dy.plant.orbit.position().magnitude().is_finite());
    }

    // Order of magnitude

    #[test]
    fn srp_order_of_magnitude_geo() {
        // GEO satellite: A=30m², m=2000kg, solar-array optics (ρ_s=0.2, ρ_d=0.1)
        // → face-on Cr = 1 + ρ_s + 2ρ_d/3 ≈ 1.27
        // |a| = P_sr * Cr * A/m / 1000 ≈ 4.54e-6 * 1.27 * 0.015 / 1000 ≈ 8.6e-11 km/s²
        let panel = SurfacePanel::at_com(
            30.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::new(0.2, 0.1),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));
        let epoch = test_epoch();

        let r_geo = 42164.0; // km
        let v_geo = (MU_EARTH / r_geo).sqrt();
        let state = SpacecraftState {
            orbit: OrbitalState::new(vector![r_geo, 0.0, 0.0], vector![0.0, v_geo, 0.0]),
            attitude: AttitudeState::identity(),
            mass: 2000.0,
        };

        let a_mag = srp
            .eval(0.0, &state, Some(&epoch))
            .acceleration_inertial
            .magnitude();
        assert!(
            a_mag > 1e-12 && a_mag < 1e-8,
            "GEO SRP should be ~1e-10 km/s², got {a_mag:.3e}"
        );

        // The decade bound above would pass for Cr = 1 or Cr = 2 alike, so hold
        // the result to the coefficient this panel's optics actually imply. The
        // panel faces +X and the Sun is near +X at the equinox, so cosθ is close
        // to 1 but not equal to it — hence the 5% band rather than an equality.
        let cr_face_on = 1.0 + 0.2 + 2.0 * 0.1 / 3.0;
        let sun = sun::sun_position_eci(&epoch.to_tdb()).into_inner();
        let r_sun = (sun - state.orbit.position()).magnitude();
        let expected = SOLAR_RADIATION_PRESSURE * (sun::AU_KM / r_sun).powi(2) * cr_face_on * 30.0
            / (2000.0 * 1000.0);
        let rel_err = (a_mag - expected).abs() / expected;
        assert!(
            rel_err < 0.05,
            "GEO SRP should follow Cr = {cr_face_on:.4}: expected ~{expected:.3e}, \
             got {a_mag:.3e}, rel_err={rel_err:.3}"
        );
    }

    // Tumbling (time-varying attitude)

    fn run_tumbling_srp_test(angular_velocity: Vector3<f64>) -> (f64, f64) {
        use crate::orbital::gravity::PointMass;
        use crate::spacecraft::SpacecraftDynamics;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, Rk4};

        // Asymmetric single panel: SRP depends on orientation
        let panel = SurfacePanel::at_com(
            20.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let epoch = test_epoch();
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia)
            .with_model(srp)
            .with_epoch(epoch);

        let mut state = iss_state();
        state.attitude.angular_velocity = angular_velocity;

        // Collect SRP magnitude at each step
        let mut magnitudes = Vec::new();
        let _ = Rk4.integrate(&dyn_sc, state.into(), 0.0, 60.0, 1.0, |t, s| {
            let loads = dyn_sc.model_breakdown(t, &s.plant);
            if let Some((_, el)) = loads.first() {
                magnitudes.push(el.acceleration_inertial.magnitude());
            }
        });

        let min = magnitudes.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = magnitudes.iter().cloned().fold(0.0_f64, f64::max);
        (min, max)
    }

    #[test]
    fn tumbling_slow_varying_srp() {
        // Slow tumble about Z (0.05 rad/s ≈ 3°/s): SRP magnitude should vary
        // as the panel rotates in and out of Sun illumination.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.0, 0.0, 0.05));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Slow tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn tumbling_fast_varying_srp() {
        // Fast tumble about Z (1.0 rad/s ≈ 57°/s): SRP should vary more rapidly,
        // with the panel spending time in both sunlit and shadowed orientations.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.0, 0.0, 1.0));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Fast tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn tumbling_multiaxis_varying_srp() {
        // Tumble about all three axes: the panel normal sweeps a complex path.
        let (min, max) = run_tumbling_srp_test(Vector3::new(0.1, 0.05, 0.2));
        assert!(max > 0.0, "Should have some non-zero SRP during tumble");
        assert!(
            min < max * 0.99,
            "Multi-axis tumble should cause varying SRP: min={min:.3e}, max={max:.3e}"
        );
    }

    // Empty panels

    #[test]
    fn empty_panels_returns_zeros() {
        let srp = PanelSrp::new(SpacecraftShape::panels(vec![]));
        let epoch = test_epoch();
        let loads = srp.eval(0.0, &iss_state(), Some(&epoch));
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
        assert_eq!(loads.torque_body.into_inner(), Vector3::zeros());
    }

    // Mass scaling

    #[test]
    fn panel_force_scales_inversely_with_mass() {
        let epoch = test_epoch();
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );

        let mut s1 = iss_state();
        s1.mass = 500.0;
        let mut s2 = iss_state();
        s2.mass = 1000.0;

        let l1 = PanelSrp::new(SpacecraftShape::panels(vec![panel.clone()])).eval(
            0.0,
            &s1,
            Some(&epoch),
        );
        let l2 = PanelSrp::new(SpacecraftShape::panels(vec![panel])).eval(0.0, &s2, Some(&epoch));

        let ratio = l1.acceleration_inertial.magnitude() / l2.acceleration_inertial.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "Half mass should give 2x acceleration, ratio={ratio}"
        );
    }

    // Mixed illumination (some panels face Sun, some don't)

    #[test]
    fn mixed_illumination_only_sunlit_panels_contribute() {
        let epoch = test_epoch();
        let state = iss_state();

        // Panel facing Sun (+X normal)
        let sunlit = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        // Panel facing away (-X normal) — backface, should not contribute
        let dark = SurfacePanel::at_com(
            10.0,
            Vector3::new(-1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );

        let l_single = PanelSrp::new(SpacecraftShape::panels(vec![sunlit.clone()])).eval(
            0.0,
            &state,
            Some(&epoch),
        );
        let l_mixed = PanelSrp::new(SpacecraftShape::panels(vec![sunlit, dark])).eval(
            0.0,
            &state,
            Some(&epoch),
        );

        // Adding a backface panel should not change the result
        let diff = (l_single.acceleration_inertial - l_mixed.acceleration_inertial).magnitude();
        assert!(
            diff < 1e-20,
            "Backface panel should not contribute: diff={diff:.3e}"
        );
    }

    // Builder

    #[test]
    fn with_shadow_body_builder() {
        let srp = PanelSrp::new(SpacecraftShape::sphere(20.0, 2.2, 1.5)).with_shadow_body(R_EARTH);
        assert_eq!(srp.shadow_body_radius, Some(R_EARTH));
    }

    // Cube (symmetric multi-panel)

    #[test]
    fn cube_srp_three_faces_illuminated() {
        // A cube has 6 faces; at most 3 face the Sun at any time.
        // For identity attitude and Sun ≈ +X, the +X face is fully illuminated,
        // while ±Y and ±Z faces get glancing illumination from the Sun's small
        // off-axis components. The -X, and the other back faces get zero.
        let cube = SpacecraftShape::cube(0.5, 2.2, PanelOptics::absorber()); // 1m cube, half_size=0.5
        let srp = PanelSrp::new(cube);
        let epoch = test_epoch();
        let state = iss_state();

        let loads = srp.eval(0.0, &state, Some(&epoch));

        // Should produce non-zero force
        assert!(loads.acceleration_inertial.magnitude() > 0.0);

        // For a symmetric cube, the CP offsets of opposite faces cancel for face-on,
        // but glancing faces have non-cancelling CP torques. Net torque should be small
        // but possibly non-zero due to Sun not being exactly +X.
        // Just verify it's finite and much smaller than force * typical offset.
        assert!(loads.torque_body.magnitude().is_finite());
    }

    // Proptest: cos(θ) scaling with panel tilt

    mod prop {
        use super::*;
        use proptest::prelude::*;

        /// Generate an angle in [0, π/2) so the panel always faces the Sun.
        fn angle_facing_sun() -> impl Strategy<Value = f64> {
            (0.01_f64..std::f64::consts::FRAC_PI_2 - 0.01).boxed()
        }

        proptest! {
            #[test]
            fn cos_theta_scaling(angle in angle_facing_sun()) {
                // Rotate the panel about Z by `angle` from the Sun direction.
                // Expected: force ∝ cos(angle) relative to face-on (angle=0).
                let epoch = test_epoch();

                // Face-on panel (normal = +X, Sun ≈ +X)
                let p_face_on = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2, PanelOptics::absorber());
                let l_face_on = PanelSrp::new(SpacecraftShape::panels(vec![p_face_on]))
                    .eval(0.0, &iss_state(), Some(&epoch));

                // Tilted panel: rotate spacecraft about Z by `angle`
                let mut state = iss_state();
                state.attitude.quaternion =
                    quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), angle);

                let panel = SurfacePanel::at_com(10.0, Vector3::new(1.0, 0.0, 0.0), 2.2, PanelOptics::absorber());
                let l_tilted = PanelSrp::new(SpacecraftShape::panels(vec![panel]))
                    .eval(0.0, &state, Some(&epoch));

                let face_on_mag = l_face_on.acceleration_inertial.magnitude();
                let tilted_mag = l_tilted.acceleration_inertial.magnitude();

                if face_on_mag > 1e-20 {
                    // The ratio should be approximately cos(angle), but the Sun
                    // direction is not exactly +X (it's approximately +X at equinox).
                    // So we compute the actual expected cos(θ) from the Sun direction.
                    let sun_dir = sun::sun_direction_eci(&epoch.to_tdb()).into_inner();
                    // At identity: panel normal in inertial = +X
                    // At rotated: panel normal in inertial = (cos(angle), sin(angle), 0)
                    let rotated_normal = Vector3::new(angle.cos(), angle.sin(), 0.0);

                    let cos_theta_face = sun_dir.dot(&Vector3::new(1.0, 0.0, 0.0)).max(0.0);
                    let cos_theta_tilt = sun_dir.dot(&rotated_normal).max(0.0);

                    if cos_theta_face > 1e-10 {
                        let expected_ratio = cos_theta_tilt / cos_theta_face;
                        let actual_ratio = tilted_mag / face_on_mag;
                        let err = (actual_ratio - expected_ratio).abs();
                        prop_assert!(
                            err < 0.01,
                            "cos(θ) scaling failed: angle={angle:.4}, expected_ratio={expected_ratio:.6}, actual_ratio={actual_ratio:.6}, err={err:.6}"
                        );
                    }
                }
            }

            /// A mirror's force is `-2·P·A·cos²θ·n̂` at every incidence.
            ///
            /// `cos_theta_scaling` above uses a black panel, whose reflection
            /// term is identically zero, so it constrains only the `cosθ`
            /// projected-area factor. Without this the `cos²θ` specular
            /// dependence is pinned at a single angle.
            #[test]
            fn specular_force_follows_cos_squared(angle in angle_facing_sun()) {
                let area = 4.0;
                let panel = SurfacePanel::at_com(area, Vector3::new(1.0, 0.0, 0.0), 2.2, PanelOptics::new(1.0, 0.0));
                let s_body = sun_tilted_in_xy(angle);

                let f = panel_force(&panel, &s_body, TEST_PRESSURE);
                let cos = angle.cos();
                let expected = -2.0 * TEST_PRESSURE * area * cos * cos * panel.normal;

                let err = (f - expected).magnitude() / expected.magnitude();
                prop_assert!(
                    err < 1e-13,
                    "mirror at angle={angle:.4}: expected {expected:?}, got {f:?}, rel_err={err:.3e}"
                );
            }
        }
    }

    /// One panel facing `normal`, with the optics the other fixtures here use.
    fn panel_facing(normal: Vector3<f64>) -> SpacecraftShape {
        SpacecraftShape::panels(vec![SurfacePanel::at_com(
            4.0,
            normal,
            2.2,
            PanelOptics::new(0.2, 0.1),
        )])
    }

    /// The inertial acceleration a model puts on a satellite at `position`.
    fn accel_at(model: &PanelSrp, position: Vector3<f64>, epoch: &Epoch) -> Vector3<f64> {
        let orbit =
            OrbitalState::<arika::frame::Gcrs>::new_in_frame(position, vector![0.0, 3.0, 0.0]);
        let att = AttitudeState::identity();
        *model
            .loads_from_state(&orbit, att.rotation_tagged_as(), 1000.0, Some(epoch))
            .acceleration_inertial
            .inner()
    }

    /// `for_body` on Earth is what `for_earth` always was.
    ///
    /// The two have to agree, or every Earth simulation would change the day a
    /// caller switched over.
    #[test]
    fn for_body_on_earth_is_for_earth() {
        let epoch = test_epoch();
        let position = iss_state().orbit.position().to_owned();
        let normal = sat_to_sun_unit(&epoch);

        let a = accel_at(&PanelSrp::for_earth(panel_facing(normal)), position, &epoch);
        let b = accel_at(
            &PanelSrp::for_body(arika::body::KnownBody::Earth, panel_facing(normal))
                .expect("Earth has a Sun ephemeris"),
            position,
            &epoch,
        );
        assert_eq!(a, b, "the same geometry, so bit-for-bit the same force");
        assert!(a.magnitude() > 0.0, "a lit panel is pushed: {a:?}");
    }

    /// Around Mars the force follows Mars' Sun direction, not Earth's.
    ///
    /// This is the error: from Mars the geocentric Sun direction is far off, so
    /// the force pointed somewhere the Sun was not. The panel faces Mars' Sun
    /// here, which an Earth-based direction would read as unlit or pushed the
    /// other way.
    #[test]
    fn for_body_on_mars_follows_mars_sun() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars = arika::body::KnownBody::Mars;
        let mars_sun = sun::sun_position_from_body(mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner()
            .normalize();
        let earth_sun = sun::sun_position_eci(&epoch.to_tdb())
            .into_inner()
            .normalize();
        let separation = mars_sun
            .dot(&earth_sun)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!(
            separation > 20.0,
            "the directions have to differ for this to test anything: {separation}°"
        );

        // Sunward of Mars, so nothing eclipses, with the panel facing its Sun.
        let position = mars_sun * 20_000.0;
        let a = accel_at(
            &PanelSrp::for_body(mars, panel_facing(mars_sun)).expect("Mars has a Sun ephemeris"),
            position,
            &epoch,
        );
        assert!(a.magnitude() > 0.0, "a lit panel is pushed: {a:?}");
        // The satellite sits sunward of Mars, so the Sun is further along
        // `mars_sun` and the light pushes back along it.
        let along = a.normalize().dot(&mars_sun);
        assert!(
            along < -0.9,
            "the push is anti-sunward, away from Mars' Sun: {along} (a = {a:?})"
        );
    }

    /// `without_shadow` drops the shadow and keeps the body's Sun.
    ///
    /// A shadow-free panel model used to be reachable only through `new`,
    /// which reads Earth's Sun. On 2026-03-20 Mars' Sun direction is 152.8°
    /// from Earth's, so a panel facing Mars' Sun is unlit through `new` and
    /// the force comes out exactly zero. The builder route keeps Mars' Sun and
    /// only the shadow goes away.
    #[test]
    fn without_shadow_keeps_the_body_sun() {
        let epoch = Epoch::from_gregorian(2026, 3, 20, 12, 0, 0.0);
        let mars = arika::body::KnownBody::Mars;
        let mars_sun = sun::sun_position_from_body(mars, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner()
            .normalize();

        // Directly behind Mars, inside its shadow cylinder.
        let position = -mars_sun * 20_000.0;
        let panel = panel_facing(mars_sun);

        let shaded = accel_at(
            &PanelSrp::for_body(mars, panel.clone()).expect("Mars has a Sun ephemeris"),
            position,
            &epoch,
        );
        assert_eq!(
            shaded,
            Vector3::zeros(),
            "Mars eclipses the satellite: {shaded:?}"
        );

        let lit = accel_at(
            &PanelSrp::for_body(mars, panel.clone())
                .expect("Mars has a Sun ephemeris")
                .without_shadow(),
            position,
            &epoch,
        );
        assert!(
            lit.magnitude() > 0.0,
            "with the shadow gone the panel is lit: {lit:?}"
        );
        // On the anti-sun axis with the panel square to the Sun, the push is
        // straight back along `-mars_sun`; measured -1.000000.
        let along = lit.normalize().dot(&mars_sun);
        assert!(
            along < -0.999,
            "the push is anti-sunward along Mars' Sun line: {along} (a = {lit:?})"
        );

        // The geocentric route reads the Sun 152.8° away, which leaves this
        // panel facing away from it.
        let geocentric = accel_at(&PanelSrp::new(panel), position, &epoch);
        assert_eq!(
            geocentric,
            Vector3::zeros(),
            "Earth's Sun direction leaves the panel unlit: {geocentric:?}"
        );
    }

    /// A body with no Sun ephemeris is refused at construction.
    #[test]
    fn for_body_refuses_a_body_with_no_sun_ephemeris() {
        assert!(
            PanelSrp::for_body(
                arika::body::KnownBody::Uranus,
                panel_facing(vector![1.0, 0.0, 0.0])
            )
            .is_err()
        );
    }

    /// About the Sun the force is radially outward, and nothing eclipses.
    #[test]
    fn for_body_on_the_sun_pushes_outward() {
        let epoch = test_epoch();
        // Heliocentric: the satellite's position is the Sun-to-satellite vector,
        // so the lit face is the one pointing back at the origin.
        let position = vector![1.5e8, 0.0, 0.0];
        let inward = -position.normalize();
        let a = accel_at(
            &PanelSrp::for_body(arika::body::KnownBody::Sun, panel_facing(inward))
                .expect("the Sun sits at the origin of its own frame"),
            position,
            &epoch,
        );
        let outward = a.normalize().dot(&position.normalize());
        assert!(outward > 0.9, "radially outward: {outward} (a = {a:?})");
    }
}
