use nalgebra::Matrix3;

use crate::orbital::gravity::{self, GravityField};
use crate::spacecraft::SpacecraftDynamics;
use arika::body::KnownBody;
use arika::epoch::Epoch;

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
/// - For Earth: Sun + Moon
/// - For other bodies: Sun only
pub fn default_third_bodies(body: &KnownBody) -> Vec<ThirdBodyGravity> {
    let mut bodies = vec![ThirdBodyGravity::sun()];
    if *body == KnownBody::Earth {
        bodies.push(ThirdBodyGravity::moon());
    }
    bodies
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
) -> OrbitalSystem {
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
        let mut srp = SolarRadiationPressure::for_earth(Some(am));
        if let Some(cr) = sat.srp_cr {
            srp = srp.with_cr(cr);
        }
        system = system.with_model(srp);
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
pub fn build_spacecraft_dynamics(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteParams,
    third_bodies: &[ThirdBodyGravity],
    inertia: Matrix3<f64>,
    atmosphere: Option<Box<dyn tobari::AtmosphereModel>>,
) -> SpacecraftDynamics<Box<dyn GravityField>> {
    let props = body.properties();
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
                // coefficient to gate on.
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
                // TODO: size the shadow to the central body once #385 lands.
                // It replaces `for_earth` with a `build_srp` helper for the
                // cannonball arm below; this arm should go through the same
                // helper rather than growing a second copy of the fix.
                system = system.with_model(PanelSrp::for_earth(shape.clone()));
            }
            (None, Some(am)) => {
                let mut srp = SolarRadiationPressure::for_earth(Some(am));
                if let Some(cr) = sat.srp_cr {
                    srp = srp.with_cr(cr);
                }
                system = system.with_model(srp);
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None);
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None);
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
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None);
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
        let system =
            build_orbital_system(&body, body.properties().mu, Some(epoch), &sat, &[], None);
        let names = system.model_names();
        assert!(!names.contains(&"third_body_sun"));
        assert!(!names.contains(&"third_body_moon"));
    }
}
