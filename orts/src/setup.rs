use nalgebra::Matrix3;

use crate::orbital::gravity::{self, GravityField};
use crate::spacecraft::SpacecraftDynamics;
use arika::body::KnownBody;
use arika::epoch::Epoch;
use arika::sun::SunPositionError;

use crate::attitude::CoupledGravityGradient;
use crate::orbital::OrbitalSystem;
use crate::perturbations::{
    AtmosphericDrag, SolarRadiationPressure, ThirdBodyGravity, ZonalGravity,
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

/// Return the default third-body perturbations for a given central body.
///
/// - Earth: Sun + Moon
/// - Moon: Sun + Earth
/// - Sun: none — it is not a third body to its own orbiters
/// - Other bodies with an ephemeris: Sun
///
/// The Sun's position is taken relative to the central body. This used to be
/// the geocentric vector for every body, which from Mars in 2026 points up to
/// 176° away from the true direction and scales the tidal term by up to 3.8x.
///
/// Fails for a central body with no Sun ephemeris (Uranus, Neptune) rather than
/// returning a set built on a substituted vector.
pub fn default_third_bodies(body: &KnownBody) -> Result<Vec<ThirdBodyGravity>, SunPositionError> {
    if *body == KnownBody::Sun {
        return Ok(Vec::new());
    }
    let mut bodies = vec![ThirdBodyGravity::sun_from_body(*body)?];
    match body {
        KnownBody::Earth => bodies.push(ThirdBodyGravity::moon()),
        KnownBody::Moon => bodies.push(ThirdBodyGravity::earth_from_moon()),
        _ => {}
    }
    Ok(bodies)
}

/// SRP whose body-dependent quantities both come from the central body: the
/// Sun's position relative to it, and its own radius for the shadow. Neither
/// is the Earth default `for_earth` bakes in.
fn build_srp(
    body: KnownBody,
    sat: &SatelliteParams,
    am: f64,
) -> Result<SolarRadiationPressure, SunPositionError> {
    let mut srp = SolarRadiationPressure::for_body(body, Some(am))?;
    if let Some(cr) = sat.srp_cr {
        srp = srp.with_cr(cr);
    }
    Ok(srp)
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
pub fn build_orbital_system(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteParams,
    third_bodies: &[ThirdBodyGravity],
    atmosphere: Option<Box<dyn tobari::AtmosphereModel>>,
) -> Result<OrbitalSystem, SunPositionError> {
    let props = body.properties();
    let mut system = OrbitalSystem::new(mu, build_gravity_field()).with_body_radius(props.radius);

    // Oblateness (J2/J3/J4) as a pole-aware perturbation.
    if let Some(zonal) = build_zonal_gravity(body, mu) {
        system = system.with_model(zonal);
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
        system = system.with_model(build_srp(*body, sat, am)?);
    }

    Ok(system)
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
pub fn build_spacecraft_dynamics(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteParams,
    third_bodies: &[ThirdBodyGravity],
    inertia: Matrix3<f64>,
    atmosphere: Option<Box<dyn tobari::AtmosphereModel>>,
) -> Result<SpacecraftDynamics<Box<dyn GravityField>>, SunPositionError> {
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

    // Oblateness (J2/J3/J4) as a pole-aware perturbation.
    if let Some(zonal) = build_zonal_gravity(body, mu) {
        system = system.with_model(zonal);
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
                system = system.with_model(PanelSrp::for_body(*body, shape.clone())?);
            }
            (None, Some(am)) => {
                system = system.with_model(build_srp(*body, sat, am)?);
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

    Ok(system)
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None)
            .expect("no solar models without third bodies");
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
        )
        .expect("Earth has a Sun ephemeris")
    }

    /// One panel whose normal is `normal`, so a caller can point it at the Sun.
    fn sunward_panel_shape(normal: nalgebra::Vector3<f64>) -> SpacecraftShape {
        use crate::spacecraft::{PanelOptics, SurfacePanel};
        SpacecraftShape::panels(vec![
            SurfacePanel::at_com(4.0, normal, 2.2, PanelOptics::new(0.2, 0.1))
                .with_cp_offset(nalgebra::Vector3::new(0.0, 1.0, 0.0)),
        ])
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
        )
        .expect("Earth has a Sun ephemeris")
    }

    /// The shadow is cast by whatever the spacecraft orbits, so a Mars
    /// simulation must not eclipse against Earth's radius.
    ///
    /// The panel sits anti-sunward at a perpendicular offset of 5000 km from
    /// the body-Sun axis: Mars' disc (3396.2 km) does not reach that far, so
    /// the panel is lit, while Earth's (6378.137 km) does, so the same panel
    /// would be dark. Which radius the model was handed decides the answer.
    ///
    /// The position is built from each body's own Sun direction, because the
    /// model now takes that from the body too — placing the satellite behind
    /// Mars with the geocentric direction would put it somewhere else entirely
    /// and the radii would not be what the answer turned on.
    #[test]
    fn panel_srp_shadows_against_the_central_body() {
        use crate::attitude::AttitudeState;
        use crate::orbital::OrbitalState;
        use crate::spacecraft::SpacecraftState;

        let epoch = Epoch::from_iso8601("2024-03-20T12:00:00Z").unwrap();

        let srp_magnitude = |body: KnownBody| {
            let sun_dir = arika::sun::sun_position_from_body(body, &epoch.to_tdb())
                .expect("this body has a Sun ephemeris")
                .into_inner()
                .normalize();
            // Any direction across the body-Sun axis serves as the offset.
            let across = sun_dir
                .cross(&nalgebra::Vector3::new(0.0, 0.0, 1.0))
                .normalize();
            let position = -sun_dir * 20_000.0 + across * 5_000.0;
            // The panel faces the Sun, so only the shadow decides whether there
            // is a force. A fixed normal would answer for the angle instead: the
            // two Sun directions are far apart, and a panel edge-on or facing
            // away from one of them reads as dark without any eclipse.
            let sat = SatelliteParams {
                has_drag: false,
                ballistic_coeff: None,
                srp_area_to_mass: None,
                srp_cr: None,
                disturbances: DisturbanceTorques::default(),
                shape: Some(sunward_panel_shape(sun_dir)),
            };
            let system = build_spacecraft_dynamics(
                &body,
                body.properties().mu,
                Some(epoch),
                &sat,
                &[],
                Matrix3::identity(),
                None,
            )
            .expect("this body has a Sun ephemeris");
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
        )
        .expect("this body has a Sun ephemeris");
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
        )
        .expect("this body has a Sun ephemeris");
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
        )
        .expect("this body has a Sun ephemeris");
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None)
            .expect("no solar models without third bodies");
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None)
            .expect("no solar models without third bodies");
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
        let third_bodies = default_third_bodies(&body).expect("Earth is supported");
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
        )
        .expect("a supported central body");
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
        let third_bodies = default_third_bodies(&body).expect("Earth is supported");
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
        )
        .expect("a supported central body");
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
        let system =
            build_orbital_system(&body, body.properties().mu, Some(epoch), &sat, &[], None)
                .expect("a supported central body");
        let names = system.model_names();
        assert!(!names.contains(&"third_body_sun"));
        assert!(!names.contains(&"third_body_moon"));
    }

    // default_third_bodies per central body

    /// Earth keeps Sun + Moon.
    #[test]
    fn default_third_bodies_for_earth_is_sun_and_moon() {
        let names: Vec<&str> = default_third_bodies(&KnownBody::Earth)
            .expect("Earth")
            .iter()
            .map(|tb| tb.name)
            .collect();
        assert_eq!(names, ["third_body_sun", "third_body_moon"]);
    }

    /// The Moon gains Earth, which dominates its own third-body environment and
    /// was missing entirely.
    #[test]
    fn default_third_bodies_for_the_moon_includes_earth() {
        let names: Vec<&str> = default_third_bodies(&KnownBody::Moon)
            .expect("the Moon")
            .iter()
            .map(|tb| tb.name)
            .collect();
        assert_eq!(names, ["third_body_sun", "third_body_earth"]);
    }

    /// A planet gets the Sun only — and it is the planet-relative Sun.
    #[test]
    fn default_third_bodies_for_a_planet_is_the_sun_alone() {
        for body in [KnownBody::Mercury, KnownBody::Venus, KnownBody::Mars] {
            let names: Vec<&str> = default_third_bodies(&body)
                .expect("supported planet")
                .iter()
                .map(|tb| tb.name)
                .collect();
            assert_eq!(names, ["third_body_sun"], "{}", body.properties().name);
        }
    }

    /// The Sun is not a third body to something orbiting it.
    #[test]
    fn default_third_bodies_for_the_sun_is_empty() {
        assert!(
            default_third_bodies(&KnownBody::Sun)
                .expect("the Sun is not an error, it just has no third bodies")
                .is_empty()
        );
    }

    /// A central body with no Sun ephemeris is reported, not given a set built
    /// on a substituted vector.
    #[test]
    fn default_third_bodies_rejects_a_body_with_no_sun_ephemeris() {
        for body in [KnownBody::Uranus, KnownBody::Neptune] {
            assert!(
                default_third_bodies(&body).is_err(),
                "{} should be refused",
                body.properties().name
            );
        }
    }

    /// The third-body Sun built through the setup path is the body-relative one.
    ///
    /// `default_third_bodies` is checked by name elsewhere, and both wirings
    /// are named `third_body_sun`, so the name says nothing about which Sun
    /// vector is inside. Measured at J2000, 6000 km sunward of Mars: the
    /// body-relative term is 1.767e-10 km/s² and the geocentric one
    /// 2.633e-10, 148° away — so the magnitude alone separates them.
    #[test]
    fn build_orbital_system_gives_the_third_body_sun_the_body_relative_vector() {
        use crate::orbital::OrbitalState;
        let epoch = Epoch::j2000();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            // No SRP, so the breakdown carries the third-body term alone.
            srp_area_to_mass: None,
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let body = KnownBody::Mars;
        let third_bodies = default_third_bodies(&body).expect("Mars is supported");
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
        )
        .expect("Mars is supported");

        let sun_mars = arika::sun::sun_position_from_body(body, &epoch.to_tdb())
            .expect("Mars has a Sun ephemeris")
            .into_inner();
        let pos = sun_mars.normalize() * 6000.0;
        let state = OrbitalState::new(pos, nalgebra::Vector3::new(0.0, 3.0, 0.0));

        let wired = system
            .acceleration_breakdown(0.0, &state)
            .into_iter()
            .find(|(name, _)| *name == "third_body_sun")
            .expect("the third-body Sun is in the breakdown")
            .1;

        let body_relative = ThirdBodyGravity::sun_from_body(body)
            .expect("Mars has a Sun ephemeris")
            .acceleration(&pos, Some(&epoch))
            .norm();
        let geocentric = ThirdBodyGravity::sun()
            .acceleration(&pos, Some(&epoch))
            .norm();

        assert!(
            (wired - body_relative).abs() < body_relative * 1e-9,
            "setup wires the Mars-relative Sun: {wired:.6e} vs {body_relative:.6e}"
        );
        assert!(
            (wired - geocentric).abs() > body_relative * 0.4,
            "the geocentric wiring gives {geocentric:.6e}, which this has to \
             separate from {wired:.6e}"
        );
    }

    /// SRP built through the setup path carries the central body's own radius
    /// and its own Sun direction.
    ///
    /// Checked through the acceleration rather than the model list, so wiring
    /// `for_earth` back in would fail: the two differ both in direction (Mars
    /// sees the Sun elsewhere) and in where the shadow starts (Mars is 3396 km,
    /// Earth 6378).
    #[test]
    fn build_orbital_system_gives_srp_the_central_body_s_geometry() {
        use crate::orbital::OrbitalState;
        let epoch = Epoch::j2000();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: Some(0.02),
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let body = KnownBody::Mars;
        let third_bodies = default_third_bodies(&body).expect("Mars is supported");
        let system = build_orbital_system(
            &body,
            body.properties().mu,
            Some(epoch),
            &sat,
            &third_bodies,
            None,
        )
        .expect("Mars is supported");
        assert!(
            system.model_names().contains(&"srp"),
            "srp should be present"
        );

        // Somewhere Mars cannot eclipse: on the sunward side, well outside it.
        let sun_mars = arika::sun::sun_position_from_body(body, &epoch.to_tdb())
            .expect("Mars")
            .into_inner();
        let pos = sun_mars.normalize() * 6000.0;
        let state = OrbitalState::new(pos, nalgebra::Vector3::new(0.0, 3.0, 0.0));

        let srp = SolarRadiationPressure::for_body(body, Some(0.02)).expect("Mars");
        let expected = srp.acceleration(&pos, Some(&epoch));
        let earth_wired =
            SolarRadiationPressure::for_earth(Some(0.02)).acceleration(&pos, Some(&epoch));

        // The breakdown reports each model's magnitude by name.
        let srp_mag = system
            .acceleration_breakdown(0.0, &state)
            .into_iter()
            .find(|(name, _)| *name == "srp")
            .expect("srp is in the breakdown")
            .1;

        assert!(
            (srp_mag - expected.norm()).abs() < expected.norm() * 1e-9,
            "setup should wire the Mars-relative SRP: {srp_mag} vs {}",
            expected.norm()
        );
        // The Earth-wired magnitude differs because Mars is at a different
        // heliocentric distance, so the magnitude alone separates them.
        assert!(
            (srp_mag - earth_wired.norm()).abs() > expected.norm() * 0.1,
            "the Earth-wired magnitude {} should be plainly different from {srp_mag}",
            earth_wired.norm()
        );
    }

    /// An unsupported central body fails the whole build instead of quietly
    /// propagating with the Sun in the wrong place.
    #[test]
    fn build_orbital_system_rejects_a_body_with_no_sun_ephemeris() {
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: Some(0.02),
            srp_cr: None,
            disturbances: DisturbanceTorques::default(),
            shape: None,
        };
        let body = KnownBody::Uranus;
        assert!(default_third_bodies(&body).is_err());
        assert!(
            build_orbital_system(
                &body,
                body.properties().mu,
                Some(Epoch::j2000()),
                &sat,
                &[],
                None,
            )
            .is_err(),
            "SRP for Uranus should be refused"
        );
    }
    /// The shadow radius is the central body's, whichever body that is.
    ///
    /// From #385, kept as its own case: `build_srp` now takes the geometry
    /// from the body, so this pins the radius half of that.
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
        let srp = build_srp(KnownBody::Moon, &sat, 0.02).expect("the Moon has a Sun vector");
        assert_eq!(srp.shadow_body_radius, Some(moon.radius));
        let earth = KnownBody::Earth.properties();
        let srp = build_srp(KnownBody::Earth, &sat, 0.02).expect("Earth has a Sun vector");
        assert_eq!(srp.shadow_body_radius, Some(earth.radius));
    }
}
