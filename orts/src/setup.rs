use std::sync::Arc;

use nalgebra::Matrix3;
use tobari::gravity::SphericalHarmonicField;

use crate::orbital::gravity::{self, GravityField};
use crate::spacecraft::SpacecraftDynamics;
use arika::body::{BodyProperties, KnownBody};
use arika::epoch::Epoch;

use crate::attitude::CoupledGravityGradient;
use crate::orbital::OrbitalSystem;
use crate::perturbations::{
    AtmosphericDrag, SolarRadiationPressure, SphericalHarmonicGravity, ThirdBodyGravity,
    ZonalGravity,
};
use crate::spacecraft::{PanelDrag, PanelSrp, SpacecraftShape};

/// Which environmental disturbance torques to model.
///
/// These only reach the attitude equations, so they are ignored when building
/// an orbit-only system: there is no orientation for a torque to act on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisturbanceTorques {
    /// Gravity-gradient torque from the central body's field.
    pub gravity_gradient: bool,
}

impl Default for DisturbanceTorques {
    /// Gravity gradient on, which is what attitude propagation has always used.
    fn default() -> Self {
        Self {
            gravity_gradient: true,
        }
    }
}

/// Physical parameters of a satellite relevant to model construction.
pub struct SatelliteParams {
    /// Whether drag should be enabled (e.g., TLE has non-zero B* or explicit ballistic coeff).
    pub has_drag: bool,
    /// Ballistic coefficient Cd*A/(2m) [m²/kg].
    pub ballistic_coeff: Option<f64>,
    /// SRP cross-sectional area to mass ratio [m²/kg].
    pub srp_area_to_mass: Option<f64>,
    /// SRP radiation pressure coefficient.
    pub srp_cr: Option<f64>,
    /// Which environmental disturbance torques to model.
    pub disturbances: DisturbanceTorques,
    /// Flat-panel outer surface, when the satellite has one.
    ///
    /// Present means both SRP and drag come from the panels: the outer shape is
    /// one object, so modelling one force from panels and the other from an
    /// isotropic cross-section would describe two different spacecraft. Only
    /// reaches [`build_spacecraft_dynamics`] — panel forces need an attitude.
    pub shape: Option<SpacecraftShape>,
}

/// Central (point-mass) gravity field. Oblateness is added separately as a
/// [`ZonalGravity`] perturbation via [`build_zonal_gravity`], because the zonal
/// terms depend on the rotation-pole orientation in the integration frame and
/// are no longer frame-independent like the point-mass term.
fn build_gravity_field() -> Box<dyn GravityField> {
    Box::new(gravity::PointMass)
}

/// Zonal (J2/J3/J4) perturbation for the body, if it has an oblateness model.
///
/// These builders produce `SimpleEci` systems, where `EarthRotationPole` returns
/// the frame's `+Z` axis as the pole — the legacy frame-Z convention. So
/// `ZonalGravity` reproduces the previous frame-Z behaviour for every
/// body; for a non-Earth body that is correct only insofar as its state is
/// expressed in a frame whose `+Z` is that body's spin axis (the pre-existing
/// assumption, unchanged here). The Earth-specific CIP only enters for the
/// geocentric `Gcrs` frame, which is Earth-only by construction.
fn build_zonal_gravity(body: &KnownBody, mu: f64) -> Option<ZonalGravity> {
    let props = body.properties();
    props
        .j2
        .map(|j2| ZonalGravity::new(mu, props.radius, j2, props.j3, props.j4))
}

/// Check the preconditions a spherical-harmonic field puts on the system it
/// joins, so a misuse fails here with a reason instead of inside the first
/// `eval`.
///
/// - Earth only: the `SimpleEci` Earth-fixed transform rotates by Earth's
///   rotation angle, so a lunar or Martian field would be spun at the wrong
///   rate.
/// - An absolute epoch is required: the longitude-dependent terms have no
///   value without Earth's rotation angle (`SphericalHarmonicGravity::eval`
///   panics without one).
/// - `mu` must be the field's own GM: the point-mass term and the harmonic
///   terms are one model, so two GMs must not be mixed (DESIGN.md 設計規約;
///   WGS-84 vs EGM2008 is 7.5e-10, ~0.3 m/day along-track at LEO).
fn check_gravity_field_preconditions(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    field: &SphericalHarmonicField,
) {
    assert!(
        *body == KnownBody::Earth,
        "a spherical-harmonic gravity field is Earth-only (the SimpleEci Earth-fixed \
         transform rotates at Earth's rate), got {body:?}"
    );
    assert!(
        epoch.is_some(),
        "a spherical-harmonic gravity field needs an absolute epoch: its longitude-dependent \
         terms are fixed to the rotating Earth"
    );
    // Relative 1e-12: the CLI copies `field.gm()` into `mu` bit-for-bit; the
    // slack only absorbs a caller that round-tripped it through text, while
    // still catching the 7.5e-10 WGS-84-vs-EGM2008 mix-up.
    assert!(
        (mu - field.gm()).abs() <= 1e-12 * field.gm(),
        "mu = {mu} km³/s² differs from the gravity field's GM = {} km³/s²: use the field's GM \
         for the point-mass term (the point mass and the harmonics are one model)",
        field.gm()
    );
}

/// Return the default third-body perturbations for a given central body.
///
/// - For Earth: Sun + Moon
/// - For other bodies: Sun only
pub fn default_third_bodies(body: &KnownBody) -> Vec<ThirdBodyGravity> {
    let mut bodies = vec![ThirdBodyGravity::sun()];
    if *body == KnownBody::Earth {
        bodies.push(ThirdBodyGravity::moon());
    }
    bodies
}

/// SRP with the shadow model sized to the actual central body,
/// not the Earth default baked into `for_earth`.
fn build_srp(props: &BodyProperties, sat: &SatelliteParams, am: f64) -> SolarRadiationPressure {
    let mut srp = SolarRadiationPressure::for_earth(Some(am)).with_shadow_body(props.radius);
    if let Some(cr) = sat.srp_cr {
        srp = srp.with_cr(cr);
    }
    srp
}

/// Build an OrbitalSystem for the given body, automatically configuring gravity,
/// third-body perturbations, drag, and SRP based on the provided parameters.
///
/// Assumes a solar-system context. When `epoch` is provided, automatically adds
/// Sun (and Moon for Earth) third-body gravity perturbations.
///
/// Third-body perturbations are specified explicitly via the `third_bodies` parameter.
/// Use [`default_third_bodies`] to get the standard set for a given central body.
///
/// If `atmosphere` is provided and drag is enabled for Earth, it will be used as the
/// atmospheric density model. If `None`, the default exponential model is used.
///
/// `gravity_field` selects the oblateness model: `Some` installs the full
/// spherical-harmonic field ([`SphericalHarmonicGravity`], Earth only, needs
/// `epoch`, and `mu` must be the field's GM — see
/// [`check_gravity_field_preconditions`]); `None` installs the body's J2/J3/J4
/// [`ZonalGravity`]. Never both: each contains J2.
pub fn build_orbital_system(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteParams,
    third_bodies: &[ThirdBodyGravity],
    atmosphere: Option<Box<dyn tobari::AtmosphereModel>>,
    gravity_field: Option<Arc<SphericalHarmonicField>>,
) -> OrbitalSystem {
    let props = body.properties();
    let mut system = OrbitalSystem::new(mu, build_gravity_field()).with_body_radius(props.radius);

    // Oblateness: the full spherical-harmonic field when given, else J2/J3/J4.
    match gravity_field {
        Some(field) => {
            check_gravity_field_preconditions(body, mu, epoch, &field);
            system = system.with_model(SphericalHarmonicGravity::for_simple_eci(field));
        }
        None => {
            if let Some(zonal) = build_zonal_gravity(body, mu) {
                system = system.with_model(zonal);
            }
        }
    }

    // Third-body gravity (requires epoch for ephemeris)
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        for tb in third_bodies {
            system = system.with_model(tb.clone());
        }
    }

    // Atmospheric drag (Earth only)
    if *body == KnownBody::Earth && sat.has_drag {
        let drag = match atmosphere {
            Some(model) => AtmosphericDrag::for_earth(sat.ballistic_coeff).with_atmosphere(model),
            None => AtmosphericDrag::for_earth(sat.ballistic_coeff),
        };
        system = system.with_model(drag);
    }

    // Solar Radiation Pressure (requires epoch for Sun position)
    if epoch.is_some()
        && let Some(am) = sat.srp_area_to_mass
    {
        system = system.with_model(build_srp(&props, sat, am));
    }
    system
}

/// Build a SpacecraftDynamics for the given body, automatically configuring gravity,
/// third-body perturbations, drag, and SRP based on the provided parameters.
///
/// Assumes a solar-system context. When `epoch` is provided, automatically adds
/// the explicitly listed third-body gravity perturbations.
/// Use [`default_third_bodies`] to get the standard set for a given central body.
///
/// This mirrors [`build_orbital_system`] but produces a coupled orbit-attitude
/// system, so it is where the disturbance torques belong: an orbit-only system
/// has no orientation for one to act on. Forces and torques go in through the
/// same capability-based `Model<S>`, and callers do not register environmental
/// models themselves — duplicating that across the `run` and `serve` entry
/// points is how the two came to disagree about which models a config gets.
/// Actuators (RW, MTQ, thrusters) stay with the caller, since which ones a
/// spacecraft carries comes from its own hardware description.
// Eight independent knobs of one builder; bundling them into a struct would
// only move the same eight names one level down.
#[allow(clippy::too_many_arguments)]
pub fn build_spacecraft_dynamics(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteParams,
    third_bodies: &[ThirdBodyGravity],
    inertia: Matrix3<f64>,
    atmosphere: Option<Box<dyn tobari::AtmosphereModel>>,
    gravity_field: Option<Arc<SphericalHarmonicField>>,
) -> SpacecraftDynamics<Box<dyn GravityField>> {
    let props = body.properties();
    // Panels and the isotropic parameters describe the same two forces, so
    // taking both would mean dropping one without saying so. The CLI rejects
    // the combination in config; a library caller reaches here directly.
    if sat.shape.is_some() {
        assert!(
            sat.ballistic_coeff.is_none() && sat.srp_area_to_mass.is_none() && sat.srp_cr.is_none(),
            "a panelled shape and the isotropic drag/SRP parameters \
             (ballistic_coeff, srp_area_to_mass, srp_cr) describe the same forces: keep one"
        );
    }

    let mut system =
        SpacecraftDynamics::new(mu, build_gravity_field(), inertia).with_body_radius(props.radius);

    // Oblateness: the full spherical-harmonic field when given, else J2/J3/J4
    // (see `build_orbital_system`).
    match gravity_field {
        Some(field) => {
            check_gravity_field_preconditions(body, mu, epoch, &field);
            system = system.with_model(SphericalHarmonicGravity::for_simple_eci(field));
        }
        None => {
            if let Some(zonal) = build_zonal_gravity(body, mu) {
                system = system.with_model(zonal);
            }
        }
    }

    // Third-body gravity (requires epoch for ephemeris)
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        for tb in third_bodies {
            system = system.with_model(tb.clone());
        }
    }

    // Drag (Earth only). One decision, not two independent `if`s: `atmosphere`
    // is a by-value box, so only one of the two models can be handed it.
    if *body == KnownBody::Earth {
        match (&sat.shape, sat.has_drag) {
            (Some(shape), _) => {
                // Panels carry their own areas and drag coefficients, so
                // writing them is the opt-in; there is no ballistic
                // coefficient to gate on. `has_drag` is ignored rather than
                // asserted on, because it is also set by a TLE's B*, and the
                // panels then describe the same drag more precisely.
                let drag = PanelDrag::for_earth(shape.clone());
                let drag = match atmosphere {
                    Some(model) => drag.with_atmosphere(model),
                    None => drag,
                };
                system = system.with_model(drag);
            }
            (None, true) => {
                let drag = match atmosphere {
                    Some(model) => {
                        AtmosphericDrag::for_earth(sat.ballistic_coeff).with_atmosphere(model)
                    }
                    None => AtmosphericDrag::for_earth(sat.ballistic_coeff),
                };
                system = system.with_model(drag);
            }
            (None, false) => {}
        }
    }

    // Solar Radiation Pressure (requires epoch for Sun position)
    if epoch.is_some() {
        match (&sat.shape, sat.srp_area_to_mass) {
            (Some(shape), _) => {
                // The shadow is cast by whatever the spacecraft orbits, so the
                // Earth radius `for_earth` bakes in is only right for Earth.
                // TODO: route this arm through `build_srp` too so both SRP
                // paths size the shadow in one place.
                system = system
                    .with_model(PanelSrp::for_earth(shape.clone()).with_shadow_body(props.radius));
            }
            (None, Some(am)) => {
                system = system.with_model(build_srp(&props, sat, am));
            }
            (None, None) => {}
        }
    }

    // Disturbance torques. Loads compose additively and every model is
    // evaluated against the same state snapshot, so this order carries no
    // meaning.
    if sat.disturbances.gravity_gradient {
        system = system.with_model(CoupledGravityGradient::new(mu, inertia));
    }
    system
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_system_sets_body_radius() {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None, None);
        assert_eq!(system.body_radius, Some(body.properties().radius));
    }

    fn earth_sat(disturbances: DisturbanceTorques) -> SatelliteParams {
        SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances,
            shape: None,
        }
    }

    fn earth_dynamics(
        disturbances: DisturbanceTorques,
    ) -> SpacecraftDynamics<Box<dyn GravityField>> {
        let body = KnownBody::Earth;
        build_spacecraft_dynamics(
            &body,
            body.properties().mu,
            None,
            &earth_sat(disturbances),
            &[],
            Matrix3::identity(),
            None,
            None,
        )
    }

    fn one_panel_shape() -> SpacecraftShape {
        use crate::spacecraft::{PanelOptics, SurfacePanel};
        SpacecraftShape::panels(vec![
            SurfacePanel::at_com(
                4.0,
                nalgebra::Vector3::new(1.0, 0.0, 0.0),
                2.2,
                PanelOptics::new(0.2, 0.1),
            )
            .with_cp_offset(nalgebra::Vector3::new(0.0, 1.0, 0.0)),
        ])
    }

    fn earth_dynamics_with(
        shape: Option<SpacecraftShape>,
        has_drag: bool,
        srp_area_to_mass: Option<f64>,
    ) -> SpacecraftDynamics<Box<dyn GravityField>> {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag,
            ballistic_coeff: has_drag.then_some(0.01),
            srp_area_to_mass,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape,
        };
        build_spacecraft_dynamics(
            &body,
            body.properties().mu,
            Some(Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap()),
            &sat,
            &[],
            Matrix3::identity(),
            None,
            None,
        )
    }

    /// The shadow is cast by whatever the spacecraft orbits, so a Mars
    /// simulation must not eclipse against Earth's radius.
    ///
    /// The panel sits anti-sunward at a perpendicular offset of 5000 km from
    /// the body-Sun axis: Mars' disc (3396.2 km) does not reach that far, so
    /// the panel is lit, while Earth's (6378.137 km) does, so the same panel
    /// would be dark. Which radius the model was handed decides the answer.
    #[test]
    fn panel_srp_shadows_against_the_central_body() {
        use crate::attitude::AttitudeState;
        use crate::orbital::OrbitalState;
        use crate::spacecraft::SpacecraftState;

        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let sun_dir = arika::sun::sun_position_eci(&epoch.to_tdb())
            .into_inner()
            .normalize();
        // Any direction across the body-Sun axis serves as the offset.
        let across = sun_dir
            .cross(&nalgebra::Vector3::new(0.0, 0.0, 1.0))
            .normalize();
        let position = -sun_dir * 20_000.0 + across * 5_000.0;

        let srp_magnitude = |body: KnownBody| {
            let sat = SatelliteParams {
                has_drag: false,
                ballistic_coeff: None,
                srp_area_to_mass: None,
                srp_cr: None,
                disturbances: DisturbanceTorques::default(),
                shape: Some(one_panel_shape()),
            };
            let system = build_spacecraft_dynamics(
                &body,
                body.properties().mu,
                Some(epoch),
                &sat,
                &[],
                Matrix3::identity(),
                None,
                None,
            );
            let state = SpacecraftState {
                orbit: OrbitalState::new(position, nalgebra::Vector3::new(0.0, 3.0, 0.0)),
                attitude: AttitudeState::identity(),
                mass: 100.0,
            };
            system
                .model_breakdown(0.0, &state)
                .into_iter()
                .find(|(name, _)| *name == "panel_srp")
                .map(|(_, loads)| loads.acceleration_inertial.magnitude())
                .expect("panel_srp should be installed")
        };

        let mars_radius = KnownBody::Mars.properties().radius;
        let earth_radius = KnownBody::Earth.properties().radius;
        assert!(
            mars_radius < 5_000.0 && earth_radius > 5_000.0,
            "the offset has to sit between the two radii: mars {mars_radius}, earth {earth_radius}"
        );

        assert!(
            srp_magnitude(KnownBody::Mars) > 0.0,
            "Mars' disc does not reach 5000 km, so the panel is lit"
        );
        assert_eq!(
            srp_magnitude(KnownBody::Earth),
            0.0,
            "Earth's disc does reach it, so the same panel is dark"
        );
    }

    /// A caller who sets both is told, instead of having one silently dropped.
    ///
    /// The CLI rejects the combination while reading config, so this guards the
    /// library path: `build_spacecraft_dynamics` is public.
    #[test]
    #[should_panic(expected = "describe the same forces")]
    fn panels_alongside_an_isotropic_parameter_panics() {
        earth_dynamics_with(Some(one_panel_shape()), false, Some(0.02));
    }

    /// `has_drag` is not part of that guard. A TLE with a non-zero B* sets it,
    /// so a panelled satellite propagated from an element set arrives here with
    /// it on and no `ballistic_coeff` — the panels then describe the drag.
    #[test]
    fn panels_with_bstar_drag_still_install_the_panel_model() {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag: true,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: Some(one_panel_shape()),
        };
        let system = build_spacecraft_dynamics(
            &body,
            body.properties().mu,
            Some(Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap()),
            &sat,
            &[],
            Matrix3::identity(),
            None,
            None,
        );
        let names = system.model_names();
        assert!(names.contains(&"panel_drag"), "models: {names:?}");
        assert!(!names.contains(&"drag"), "models: {names:?}");
    }

    /// Panels drive both forces, and the isotropic models step aside.
    #[test]
    fn panels_install_the_panel_force_models() {
        let system = earth_dynamics_with(Some(one_panel_shape()), false, None);
        let names = system.model_names();
        assert!(names.contains(&"panel_drag"), "models: {names:?}");
        assert!(names.contains(&"panel_srp"), "models: {names:?}");
        assert!(!names.contains(&"drag"), "models: {names:?}");
        assert!(!names.contains(&"srp"), "models: {names:?}");
    }

    /// Without panels the isotropic path is untouched.
    #[test]
    fn without_panels_the_isotropic_models_stay() {
        let system = earth_dynamics_with(None, true, Some(0.02));
        let names = system.model_names();
        assert!(names.contains(&"drag"), "models: {names:?}");
        assert!(names.contains(&"srp"), "models: {names:?}");
        assert!(!names.contains(&"panel_drag"), "models: {names:?}");
        assert!(!names.contains(&"panel_srp"), "models: {names:?}");
    }

    /// Panel forces need an attitude, so an orbit-only system gets none even
    /// when a shape is supplied.
    #[test]
    fn orbital_system_never_installs_a_panel_model() {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: Some(one_panel_shape()),
        };
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap()),
            &sat,
            &[],
            None,
            None,
        );
        let names = system.model_names();
        assert!(!names.contains(&"panel_drag"), "models: {names:?}");
        assert!(!names.contains(&"panel_srp"), "models: {names:?}");
    }

    #[test]
    fn spacecraft_dynamics_installs_gravity_gradient_by_default() {
        let system = earth_dynamics(DisturbanceTorques::default());
        assert!(system.model_names().contains(&"gravity_gradient"));
    }

    /// Exactly once, not merely present. A caller that added the torque itself
    /// on top of the builder is how `orts run` came to evaluate it twice.
    #[test]
    fn spacecraft_dynamics_installs_gravity_gradient_exactly_once() {
        let system = earth_dynamics(DisturbanceTorques::default());
        let count = system
            .model_names()
            .iter()
            .filter(|n| **n == "gravity_gradient")
            .count();
        assert_eq!(count, 1, "models: {:?}", system.model_names());
    }

    #[test]
    fn spacecraft_dynamics_omits_a_disabled_gravity_gradient() {
        let system = earth_dynamics(DisturbanceTorques {
            gravity_gradient: false,
        });
        assert!(!system.model_names().contains(&"gravity_gradient"));
    }

    /// An orbit-only system has no orientation, so a torque there would be
    /// summed into a channel `OrbitalSystem` discards.
    #[test]
    fn orbital_system_never_installs_a_torque() {
        let body = KnownBody::Earth;
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            None,
            &earth_sat(DisturbanceTorques::default()),
            &[],
            None,
            None,
        );
        assert!(!system.model_names().contains(&"gravity_gradient"));
    }

    #[test]
    fn build_system_with_drag() {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag: true,
            ballistic_coeff: Some(0.01),
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None, None);
        assert!(system.model_names().contains(&"drag"));
    }

    #[test]
    fn build_system_no_drag_when_not_earth() {
        let body = KnownBody::Mars;
        let sat = SatelliteParams {
            has_drag: true,
            ballistic_coeff: Some(0.01),
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None, None);
        assert!(!system.model_names().contains(&"drag"));
    }

    #[test]
    fn build_system_with_epoch_adds_third_body() {
        let body = KnownBody::Earth;
        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let third_bodies = default_third_bodies(&body);
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
            None,
        );
        let names = system.model_names();
        assert!(names.contains(&"third_body_sun"));
        assert!(names.contains(&"third_body_moon"));
    }

    #[test]
    fn build_system_with_srp() {
        let body = KnownBody::Earth;
        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: Some(0.02),
            srp_cr: Some(1.8),
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let third_bodies = default_third_bodies(&body);
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
            None,
        );
        assert!(system.model_names().contains(&"srp"));
    }

    #[test]
    fn build_system_no_third_bodies_when_empty() {
        let body = KnownBody::Earth;
        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        // Explicitly pass empty third bodies
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &[],
            None,
            None,
        );
        let names = system.model_names();
        assert!(!names.contains(&"third_body_sun"));
        assert!(!names.contains(&"third_body_moon"));
    }
    #[test]
    fn srp_shadow_radius_matches_central_body() {
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let moon = KnownBody::Moon.properties();
        let srp = build_srp(&moon, &sat, 0.02);
        assert_eq!(srp.shadow_body_radius, Some(moon.radius));
        let earth = KnownBody::Earth.properties();
        let srp = build_srp(&earth, &sat, 0.02);
        assert_eq!(srp.shadow_body_radius, Some(earth.radius));
    }

    // --- spherical-harmonic gravity field --------------------------------

    /// A degree-4 field carrying exactly Earth's J2/J3/J4 (C̄n0 = −Jn/√(2n+1)),
    /// with the given GM.
    fn earth_field(mu: f64) -> Arc<SphericalHarmonicField> {
        let props = KnownBody::Earth.properties();
        let coeffs = [
            (2, 0, -props.j2.unwrap() / 5.0f64.sqrt(), 0.0),
            (3, 0, -props.j3.unwrap() / 7.0f64.sqrt(), 0.0),
            (4, 0, -props.j4.unwrap() / 9.0f64.sqrt(), 0.0),
        ];
        Arc::new(
            SphericalHarmonicField::from_normalized_coefficients(mu, props.radius, 4, &coeffs)
                .unwrap(),
        )
    }

    fn epoch() -> Option<Epoch> {
        Some(Epoch::from_gregorian(2026, 1, 1, 0, 0, 0.0))
    }

    /// With a field, the harmonic model replaces the zonal one — never both,
    /// since each contains J2.
    #[test]
    fn gravity_field_replaces_zonal_gravity_in_orbital_system() {
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        let with_field =
            build_orbital_system(&body, mu, epoch(), &sat, &[], None, Some(earth_field(mu)));
        let names = with_field.model_names();
        assert!(names.contains(&"spherical_harmonic_gravity"), "{names:?}");
        assert!(!names.contains(&"zonal_gravity"), "{names:?}");

        let without = build_orbital_system(&body, mu, epoch(), &sat, &[], None, None);
        let names = without.model_names();
        assert!(names.contains(&"zonal_gravity"), "{names:?}");
        assert!(!names.contains(&"spherical_harmonic_gravity"), "{names:?}");
    }

    #[test]
    fn gravity_field_replaces_zonal_gravity_in_spacecraft_dynamics() {
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        let dynamics = build_spacecraft_dynamics(
            &body,
            mu,
            epoch(),
            &sat,
            &[],
            Matrix3::identity(),
            None,
            Some(earth_field(mu)),
        );
        let names = dynamics.model_names();
        assert!(names.contains(&"spherical_harmonic_gravity"), "{names:?}");
        assert!(!names.contains(&"zonal_gravity"), "{names:?}");
    }

    /// The field carries exactly Earth's J2/J3/J4, so at a fixed instant the
    /// field-backed system and the zonal one must agree on the oblateness
    /// acceleration (the two paths share only the Jn constants).
    #[test]
    fn gravity_field_and_zonal_agree_on_j2_acceleration() {
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        let with_field =
            build_orbital_system(&body, mu, epoch(), &sat, &[], None, Some(earth_field(mu)));
        let zonal = build_orbital_system(&body, mu, epoch(), &sat, &[], None, None);
        let state = crate::OrbitalState::new(
            nalgebra::Vector3::new(4000.0, -3000.0, 5000.0),
            nalgebra::Vector3::new(0.0, 7.5, 0.0),
        );
        let pick = |sys: &OrbitalSystem, name: &str| {
            sys.acceleration_breakdown(0.0, &state)
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, a)| a)
                .unwrap()
        };
        let a_field = pick(&with_field, "spherical_harmonic_gravity");
        let a_zonal = pick(&zonal, "zonal_gravity");
        assert!(
            (a_field - a_zonal).abs() <= 1e-12 * a_zonal,
            "{a_field} vs {a_zonal}"
        );
    }

    #[test]
    #[should_panic(expected = "Earth-only")]
    fn gravity_field_rejects_non_earth_body() {
        let body = KnownBody::Moon;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        let _ = build_orbital_system(&body, mu, epoch(), &sat, &[], None, Some(earth_field(mu)));
    }

    #[test]
    #[should_panic(expected = "needs an absolute epoch")]
    fn gravity_field_rejects_missing_epoch() {
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        let _ = build_orbital_system(&body, mu, None, &sat, &[], None, Some(earth_field(mu)));
    }

    #[test]
    #[should_panic(expected = "differs from the gravity field's GM")]
    fn gravity_field_rejects_mismatched_mu() {
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let sat = earth_sat(DisturbanceTorques::default());
        // The field carries EGM2008's GM; the system is handed WGS-84's.
        let field = earth_field(398600.4415);
        let _ = build_orbital_system(&body, mu, epoch(), &sat, &[], None, Some(field));
    }
}
