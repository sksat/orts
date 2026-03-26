use nalgebra::Matrix3;

use crate::orbital::gravity::{self, GravityField};
use crate::spacecraft::SpacecraftDynamics;
use kaname::body::KnownBody;
use kaname::epoch::Epoch;

use crate::orbital::OrbitalSystem;
use crate::perturbations::{AtmosphericDrag, SolarRadiationPressure, ThirdBodyGravity};

/// Physical parameters of a satellite relevant to force model construction.
pub struct SatelliteParams {
    /// Whether drag should be enabled (e.g., TLE has non-zero B* or explicit ballistic coeff).
    pub has_drag: bool,
    /// Ballistic coefficient Cd*A/(2m) [m²/kg].
    pub ballistic_coeff: Option<f64>,
    /// SRP cross-sectional area to mass ratio [m²/kg].
    pub srp_area_to_mass: Option<f64>,
    /// SRP radiation pressure coefficient.
    pub srp_cr: Option<f64>,
}

/// Build a gravity field model for the given body.
fn build_gravity_field(body: &KnownBody) -> Box<dyn GravityField> {
    let props = body.properties();
    match props.j2 {
        Some(j2) => Box::new(gravity::ZonalHarmonics {
            r_body: props.radius,
            j2,
            j3: props.j3,
            j4: props.j4,
        }),
        None => Box::new(gravity::PointMass),
    }
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
    let gravity_field = build_gravity_field(body);
    let mut system = OrbitalSystem::new(mu, gravity_field).with_body_radius(props.radius);

    // Third-body gravity (requires epoch for ephemeris)
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        for tb in third_bodies {
            system = system.with_model(ThirdBodyGravity {
                name: tb.name,
                mu_body: tb.mu_body,
                body_position_fn: tb.body_position_fn,
            });
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
/// This mirrors [`build_orbital_system`] but produces a coupled orbit-attitude system.
/// Force-only models (drag, SRP, third-body) are added via capability-based `Model<S>`.
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
    let gravity_field = build_gravity_field(body);
    let mut system =
        SpacecraftDynamics::new(mu, gravity_field, inertia).with_body_radius(props.radius);

    // Third-body gravity (requires epoch for ephemeris)
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        for tb in third_bodies {
            system = system.with_model(ThirdBodyGravity {
                name: tb.name,
                mu_body: tb.mu_body,
                body_position_fn: tb.body_position_fn,
            });
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
        };
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None);
        assert_eq!(system.body_radius, Some(body.properties().radius));
    }

    #[test]
    fn build_system_with_drag() {
        let body = KnownBody::Earth;
        let sat = SatelliteParams {
            has_drag: true,
            ballistic_coeff: Some(0.01),
            srp_area_to_mass: None,
            srp_cr: None,
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
        };
        // Explicitly pass empty third bodies
        let system =
            build_orbital_system(&body, body.properties().mu, Some(epoch), &sat, &[], None);
        let names = system.model_names();
        assert!(!names.contains(&"third_body_sun"));
        assert!(!names.contains(&"third_body_moon"));
    }
}
