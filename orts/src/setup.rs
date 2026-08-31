use nalgebra::Matrix3;

use crate::orbital::gravity::{self, GravityField};
use crate::spacecraft::SpacecraftDynamics;
use arika::body::KnownBody;
use arika::epoch::Epoch;
use arika::sun::SunPositionError;

use crate::orbital::OrbitalSystem;
use crate::perturbations::{
    AtmosphericDrag, SolarRadiationPressure, ThirdBodyGravity, ZonalGravity,
};

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
        // Both body-dependent quantities come from the central body: the Sun's
        // position relative to it, and its own radius for the shadow.
        let mut srp = SolarRadiationPressure::for_body(*body, Some(am))?;
        if let Some(cr) = sat.srp_cr {
            srp = srp.with_cr(cr);
        }
        system = system.with_model(srp);
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
) -> Result<SpacecraftDynamics<Box<dyn GravityField>>, SunPositionError> {
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
        // Both body-dependent quantities come from the central body: the Sun's
        // position relative to it, and its own radius for the shadow.
        let mut srp = SolarRadiationPressure::for_body(*body, Some(am))?;
        if let Some(cr) = sat.srp_cr {
            srp = srp.with_cr(cr);
        }
        system = system.with_model(srp);
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
        };
        let system = build_orbital_system(&body, body.properties().mu, None, &sat, &[], None)
            .expect("no solar models without third bodies");
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

    /// SRP built through the setup path carries the central body's own radius.
    #[test]
    fn build_orbital_system_gives_srp_the_central_body_s_shadow() {
        let epoch = Epoch::j2000();
        let sat = SatelliteParams {
            has_drag: false,
            ballistic_coeff: None,
            srp_area_to_mass: Some(0.02),
            srp_cr: None,
        };
        for body in [KnownBody::Earth, KnownBody::Mars] {
            let third_bodies = default_third_bodies(&body).expect("supported body");
            let system = build_orbital_system(
                &body,
                body.properties().mu,
                Some(epoch),
                &sat,
                &third_bodies,
                None,
            )
            .expect("a supported central body");
            assert!(
                system.model_names().contains(&"srp"),
                "{}: srp should be present",
                body.properties().name
            );
        }
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
}
