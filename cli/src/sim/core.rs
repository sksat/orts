use std::collections::HashMap;
use std::sync::Arc;

use kaname::epoch::Epoch;
use orts_integrator::State;
use orts_orbits::{body::KnownBody, drag::AtmosphericDrag, gravity, kepler::KeplerianElements, orbital_system::OrbitalSystem, srp::SolarRadiationPressure, third_body::ThirdBodyGravity};
use serde::{Deserialize, Serialize};

use crate::cli::AtmosphereChoice;
use crate::satellite::{OrbitSpec, SatelliteSpec};

/// A single state snapshot used in history messages.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryState {
    pub satellite_id: String,
    pub t: f64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub raan: f64,
    pub argument_of_periapsis: f64,
    pub true_anomaly: f64,
    /// Per-force acceleration magnitudes [km/s²]: "gravity", "drag", "srp", etc.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub accelerations: HashMap<String, f64>,
}

/// Create a HistoryState from position/velocity, computing Keplerian elements.
pub fn make_history_state(
    satellite_id: &str,
    t: f64,
    pos: &nalgebra::Vector3<f64>,
    vel: &nalgebra::Vector3<f64>,
    mu: f64,
    accelerations: HashMap<String, f64>,
) -> HistoryState {
    let elements = KeplerianElements::from_state_vector(pos, vel, mu);
    HistoryState {
        satellite_id: satellite_id.to_string(),
        t,
        position: [pos.x, pos.y, pos.z],
        velocity: [vel.x, vel.y, vel.z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        accelerations,
    }
}

/// Compute acceleration breakdown as a HashMap from an OrbitalSystem.
pub fn accel_breakdown(system: &OrbitalSystem, t: f64, state: &State) -> HashMap<String, f64> {
    system
        .acceleration_breakdown(t, state)
        .into_iter()
        .map(|(name, mag)| (name.to_string(), mag))
        .collect()
}

/// Build an OrbitalSystem for the given body, using ZonalHarmonics if available.
///
/// When `epoch` is provided, epoch-dependent perturbations (third-body gravity)
/// are automatically enabled. When the satellite has a TLE with non-zero B*,
/// atmospheric drag is added (Earth only).
#[allow(clippy::too_many_arguments)]
pub fn build_orbital_system(
    body: &KnownBody,
    mu: f64,
    epoch: Option<Epoch>,
    sat: &SatelliteSpec,
    atmosphere: AtmosphereChoice,
    f107: f64,
    ap: f64,
    space_weather: Option<&Arc<tobari::CssiSpaceWeather>>,
) -> OrbitalSystem {
    let props = body.properties();
    let gravity_field: Box<dyn gravity::GravityField> = match props.j2 {
        Some(j2) => Box::new(gravity::ZonalHarmonics {
            r_body: props.radius,
            j2,
            j3: props.j3,
            j4: props.j4,
        }),
        None => Box::new(gravity::PointMass),
    };
    let mut system = OrbitalSystem::new(mu, gravity_field)
        .with_body_radius(props.radius);

    // Set epoch for time-dependent perturbations
    if let Some(epoch) = epoch {
        system = system.with_epoch(epoch);

        // Third-body gravity: Sun (always), Moon (Earth only)
        system = system.with_perturbation(Box::new(ThirdBodyGravity::sun()));
        if *body == KnownBody::Earth {
            system = system.with_perturbation(Box::new(ThirdBodyGravity::moon()));
        }
    }

    // Atmospheric drag (Earth only)
    // Enable when: TLE has non-zero B* (implies drag-relevant orbit), or user provides ballistic-coeff
    if *body == KnownBody::Earth {
        let has_tle_drag = matches!(&sat.orbit, OrbitSpec::Tle { tle_data, .. } if tle_data.bstar.abs() > 1e-15);
        if has_tle_drag || sat.ballistic_coeff.is_some() {
            let drag = match atmosphere {
                AtmosphereChoice::Exponential => {
                    AtmosphericDrag::for_earth(sat.ballistic_coeff)
                }
                AtmosphereChoice::HarrisPriester => {
                    AtmosphericDrag::for_earth(sat.ballistic_coeff)
                        .with_atmosphere(Box::new(
                            tobari::HarrisPriester::new(),
                        ))
                }
                AtmosphereChoice::Nrlmsise00 => {
                    let provider: Box<dyn tobari::SpaceWeatherProvider> = match space_weather {
                        Some(cssi) => Box::new((**cssi).clone()),
                        None => Box::new(tobari::ConstantWeather::new(f107, ap)),
                    };
                    AtmosphericDrag::for_earth(sat.ballistic_coeff)
                        .with_atmosphere(Box::new(
                            tobari::Nrlmsise00::new(provider),
                        ))
                }
            };
            system = system.with_perturbation(Box::new(drag));
        }
    }

    // Solar Radiation Pressure (requires epoch for Sun position)
    if epoch.is_some()
        && let Some(am) = sat.srp_area_to_mass
    {
        let mut srp = SolarRadiationPressure::for_earth(Some(am));
        if let Some(cr) = sat.srp_cr {
            srp = srp.with_cr(cr);
        }
        system = system.with_perturbation(Box::new(srp));
    }

    system
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satellite::parse_sat_spec;

    const TEST_MU: f64 = 398600.4418;

    #[test]
    fn history_state_has_satellite_id() {
        let hs = make_history_state("test-sat", 10.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU, HashMap::new());
        assert_eq!(hs.satellite_id, "test-sat");
        assert!((hs.t - 10.0).abs() < 1e-9);
    }

    #[test]
    fn history_state_satellite_id_serialized() {
        let hs = make_history_state("my-sat", 5.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU, HashMap::new());
        let json = serde_json::to_string(&hs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellite_id"], "my-sat");
    }

    #[test]
    fn build_orbital_system_sets_body_radius() {
        let body = KnownBody::Earth;
        let spec = parse_sat_spec("altitude=400", body);
        let system = build_orbital_system(&body, body.properties().mu, None, &spec, AtmosphereChoice::Exponential, 150.0, 15.0, None);
        assert_eq!(system.body_radius, Some(body.properties().radius));
    }
}
