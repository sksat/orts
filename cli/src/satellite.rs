use arika::body::KnownBody;
use arika::elements::Sgp4Elements;
use arika::epoch::Epoch;
use arika::frame::{FrameTransform, SimpleEci, Teme};
use arika::sgp4::Sgp4Propagator;
use orts::OrbitalState;
use orts::orbital::kepler::KeplerianElements;
use orts::record::entity_path::EntityPath;
use serde::Serialize;

use crate::tle::fetch_tle_by_norad_id;

/// How the orbit was specified on the command line.
#[derive(Clone)]
pub enum OrbitSpec {
    /// Circular orbit from --altitude, with optional inclination and RAAN.
    Circular {
        altitude: f64,
        r0: f64,
        /// Orbital inclination in radians (0 = equatorial).
        inclination: f64,
        /// Right Ascension of Ascending Node in radians.
        raan: f64,
    },
    /// From a parsed element set — TLE or OMM input, both decode into the
    /// canonical [`Sgp4Elements`] record. The initial Cartesian state is
    /// produced by SGP4 propagation (not a two-body conversion), so only the
    /// raw mean elements are carried.
    Omm { omm: Sgp4Elements },
}

/// Per-satellite specification.
#[derive(Clone)]
pub struct SatelliteSpec {
    /// Unique identifier used in entity paths and WebSocket messages.
    pub id: String,
    /// Display name (from TLE or user-provided).
    pub name: Option<String>,
    /// Orbit specification.
    pub orbit: OrbitSpec,
    /// Orbital period for this satellite.
    pub period: f64,
    /// Explicit ballistic coefficient Cd*A/(2m) [m²/kg] for drag.
    pub ballistic_coeff: Option<f64>,
    /// SRP cross-sectional area to mass ratio [m²/kg].
    pub srp_area_to_mass: Option<f64>,
    /// SRP radiation pressure coefficient (default: 1.5).
    pub srp_cr: Option<f64>,
    /// Attitude dynamics configuration. When present, SpacecraftDynamics is used.
    pub attitude_config: Option<crate::config::AttitudeConfig>,
    /// Viewer marker shape hint (display only; carried through to SatelliteInfo).
    pub shape: Option<crate::sim::core::MarkerShape>,
    /// Plugin controller configuration (used in Step 3: controlled.rs).
    #[allow(dead_code)]
    pub controller_config: Option<crate::config::ControllerConfig>,
    /// Enabled sensors (used in Step 3: controlled.rs).
    #[allow(dead_code)]
    pub sensor_choices: Option<Vec<crate::config::SensorChoice>>,
    /// Reaction wheel configuration (used in Step 3: controlled.rs).
    #[allow(dead_code)]
    pub rw_config: Option<crate::config::ReactionWheelConfig>,
    /// MTQ configuration (used in Step 3: controlled.rs).
    #[allow(dead_code)]
    pub mtq_config: Option<crate::config::MtqConfig>,
    /// Thruster configuration (plugin-commanded thruster assembly).
    #[allow(dead_code)]
    pub thruster_config: Option<crate::config::ThrusterConfig>,
    /// Declared stream-io byte streams (kble bridge). Passed to the WASM
    /// controller at construction; exposed by `serve` as binary WS endpoints.
    pub streams: Vec<String>,
}

impl SatelliteSpec {
    /// Build the integrator's initial [`OrbitalState`] (frame `SimpleEci`).
    ///
    /// For an element set ([`OrbitSpec::Omm`]) the state comes from SGP4
    /// propagation to `epoch`, then a TEME→`SimpleEci` rotation — not the
    /// two-body conversion, which is tens of km off at epoch. `epoch` is the
    /// resolved simulation-start epoch; when it is `None` the element-set epoch
    /// is used (propagation Δt = 0), so a bare TLE simulates from its own epoch.
    pub fn initial_state(&self, mu: f64, epoch: Option<Epoch>) -> Result<OrbitalState, String> {
        match &self.orbit {
            OrbitSpec::Circular {
                r0,
                inclination,
                raan,
                ..
            } => {
                let elements = KeplerianElements {
                    semi_major_axis: *r0,
                    eccentricity: 0.0,
                    inclination: *inclination,
                    raan: *raan,
                    argument_of_periapsis: 0.0,
                    true_anomaly: 0.0,
                };
                let (pos, vel) = elements.to_state_vector(mu);
                Ok(OrbitalState::new(pos, vel))
            }
            OrbitSpec::Omm { omm } => {
                let target = epoch.unwrap_or(omm.epoch);
                let propagator = Sgp4Propagator::from_elements(omm).map_err(|e| {
                    format!(
                        "SGP4 propagator init failed for NORAD {}: {e}",
                        omm.norad_cat_id
                    )
                })?;
                let (r_teme, v_teme) = propagator.propagate(target).map_err(|e| {
                    format!(
                        "SGP4 propagation failed for NORAD {}: {e}",
                        omm.norad_cat_id
                    )
                })?;
                // SGP4 yields TEME; rotate the state (position+velocity, ω = 0)
                // into the integration frame `SimpleEci` at the target epoch.
                let (pos, vel) =
                    FrameTransform::<Teme, SimpleEci>::teme_to_simple_eci(&target.to_ut1_naive())
                        .transform_state(&r_teme, &v_teme);
                Ok(OrbitalState::new(pos.into_inner(), vel.into_inner()))
            }
        }
    }

    /// Altitude for display purposes.
    pub fn altitude(&self, body: &KnownBody) -> f64 {
        match &self.orbit {
            OrbitSpec::Circular { altitude, .. } => *altitude,
            OrbitSpec::Omm { omm } => {
                // Perigee altitude from the mean elements' two-body semi-major
                // axis (display only; the propagated state is SGP4).
                let perigee_r =
                    omm.semi_major_axis(body.properties().mu) * (1.0 - omm.eccentricity);
                perigee_r - body.properties().radius
            }
        }
    }

    pub fn entity_path(&self) -> EntityPath {
        EntityPath::parse(&format!("/world/sat/{}", self.id))
    }
}

/// Satellite info sent in the WebSocket info message.
#[derive(Serialize, Clone, Debug, ts_rs::TS)]
#[ts(export)]
pub struct SatelliteInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    pub altitude: f64,
    pub period: f64,
    /// Names of active perturbation force models (e.g. "drag", "srp", "third_body_sun").
    pub perturbations: Vec<String>,
    /// Sim-declared viewer marker shape hint (display only; the viewer may override it).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub shape: Option<crate::sim::core::MarkerShape>,
}

/// Orbital period [s] from the SGP4 mean motion (`2π / n`). Equal to the
/// two-body period of the mean semi-major axis, for display purposes.
pub(crate) fn orbital_period(omm: &Sgp4Elements) -> f64 {
    2.0 * std::f64::consts::PI / omm.mean_motion
}

/// Parse a satellite specification string (key=value,key=value).
pub fn parse_sat_spec(s: &str, body: KnownBody) -> SatelliteSpec {
    let mu = body.properties().mu;
    let mut id = String::new();
    let mut name: Option<String> = None;
    let mut altitude: Option<f64> = None;
    let mut inclination: Option<f64> = None;
    let mut raan: Option<f64> = None;
    let mut norad_id: Option<u32> = None;
    let mut tle_line1: Option<String> = None;
    let mut tle_line2: Option<String> = None;
    let mut ballistic_coeff: Option<f64> = None;
    let mut srp_area_to_mass: Option<f64> = None;
    let mut srp_cr: Option<f64> = None;

    for part in s.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            match key.trim() {
                "id" => id = value.trim().to_string(),
                "name" => name = Some(value.trim().to_string()),
                "altitude" => {
                    altitude = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid altitude: {value}")),
                    )
                }
                "inclination" => {
                    inclination = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid inclination: {value}")),
                    )
                }
                "raan" => {
                    raan = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid raan: {value}")),
                    )
                }
                "norad-id" => {
                    norad_id = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid norad-id: {value}")),
                    )
                }
                "tle-line1" => tle_line1 = Some(value.trim().to_string()),
                "tle-line2" => tle_line2 = Some(value.trim().to_string()),
                "ballistic-coeff" => {
                    ballistic_coeff = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid ballistic-coeff: {value}")),
                    )
                }
                "srp-area-to-mass" => {
                    srp_area_to_mass = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid srp-area-to-mass: {value}")),
                    )
                }
                "srp-cr" => {
                    srp_cr = Some(
                        value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("Invalid srp-cr: {value}")),
                    )
                }
                k => panic!("Unknown satellite spec key: {k}"),
            }
        }
    }

    // Determine orbit
    let (orbit, period, derived_name) = if let Some(norad) = norad_id {
        let parsed = fetch_tle_by_norad_id(norad);
        let omm = parsed.elements;
        let period = orbital_period(&omm);
        let obj_name = parsed.object_name.clone();
        (OrbitSpec::Omm { omm }, period, obj_name)
    } else if let (Some(l1), Some(l2)) = (tle_line1, tle_line2) {
        let text = format!("{l1}\n{l2}");
        let parsed = arika::tle::parse(&text)
            .unwrap_or_else(|e| panic!("Failed to parse TLE in --sat: {e}"));
        let omm = parsed.elements;
        let period = orbital_period(&omm);
        let obj_name = parsed.object_name.clone();
        (OrbitSpec::Omm { omm }, period, obj_name)
    } else {
        let alt = altitude.unwrap_or(400.0);
        let r0 = body.properties().radius + alt;
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        let inc = inclination.unwrap_or(0.0).to_radians();
        let ra = raan.unwrap_or(0.0).to_radians();
        (
            OrbitSpec::Circular {
                altitude: alt,
                r0,
                inclination: inc,
                raan: ra,
            },
            period,
            None,
        )
    };

    if id.is_empty() {
        id = "auto".to_string();
    }

    SatelliteSpec {
        id,
        name: name.or(derived_name),
        orbit,
        period,
        ballistic_coeff,
        srp_area_to_mass,
        srp_cr,
        attitude_config: None, // CLI --sat does not yet support attitude; use config file
        shape: None,
        controller_config: None,
        sensor_choices: None,
        rw_config: None,
        mtq_config: None,
        thruster_config: None,
        streams: Vec::new(),
    }
}

pub fn parse_body(s: &str) -> KnownBody {
    try_parse_body(s).unwrap_or_else(|| panic!("Unknown body: {s}"))
}

/// Non-panicking body lookup, sharing the same name table as [`parse_body`].
/// Used by config validation to reject an unknown body up front (with a clear
/// error) instead of panicking later when the params are built.
pub fn try_parse_body(s: &str) -> Option<KnownBody> {
    Some(match s {
        "sun" => KnownBody::Sun,
        "mercury" => KnownBody::Mercury,
        "venus" => KnownBody::Venus,
        "earth" => KnownBody::Earth,
        "moon" => KnownBody::Moon,
        "mars" => KnownBody::Mars,
        "jupiter" => KnownBody::Jupiter,
        "saturn" => KnownBody::Saturn,
        "uranus" => KnownBody::Uranus,
        "neptune" => KnownBody::Neptune,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sat_spec_circular_altitude() {
        let spec = parse_sat_spec("altitude=800,id=sso", KnownBody::Earth);
        assert_eq!(spec.id, "sso");
        assert!(
            matches!(spec.orbit, OrbitSpec::Circular { altitude, .. } if (altitude - 800.0).abs() < 1e-9)
        );
        assert!(spec.period > 0.0);
    }

    #[test]
    fn parse_sat_spec_default_id() {
        let spec = parse_sat_spec("altitude=600", KnownBody::Earth);
        assert!(!spec.id.is_empty());
    }

    #[test]
    fn parse_sat_spec_with_name() {
        let spec = parse_sat_spec("altitude=800,id=sso,name=SSO 800km", KnownBody::Earth);
        assert_eq!(spec.id, "sso");
        assert_eq!(spec.name.as_deref(), Some("SSO 800km"));
    }

    #[test]
    fn parse_sat_spec_tle_lines() {
        let spec = parse_sat_spec(
            "tle-line1=1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993,tle-line2=2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000,id=iss",
            KnownBody::Earth,
        );
        assert_eq!(spec.id, "iss");
        assert!(matches!(spec.orbit, OrbitSpec::Omm { .. }));
    }

    #[test]
    fn satellite_spec_initial_state_circular() {
        let spec = parse_sat_spec("altitude=400,id=test", KnownBody::Earth);
        let mu = KnownBody::Earth.properties().mu;
        let state = spec.initial_state(mu, None).unwrap();
        let r = state.position().magnitude();
        let expected_r = 6378.137 + 400.0;
        assert!(
            (r - expected_r).abs() < 1e-6,
            "r = {r}, expected {expected_r}"
        );
    }

    #[test]
    fn satellite_spec_initial_state_inclined() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec(
            "altitude=800,inclination=98.6,id=sso-test",
            KnownBody::Earth,
        );
        let state = spec.initial_state(mu, None).unwrap();

        let r = state.position().magnitude();
        let expected_r = 6378.137 + 800.0;
        assert!(
            (r - expected_r).abs() < 1e-6,
            "r = {r}, expected {expected_r}"
        );

        let v = state.velocity().magnitude();
        let expected_v = (mu / expected_r).sqrt();
        assert!(
            (v - expected_v).abs() < 1e-6,
            "v = {v}, expected {expected_v}"
        );

        let h = state.position().cross(state.velocity());
        let i = (h[2] / h.magnitude()).acos();
        let expected_i = 98.6_f64.to_radians();
        assert!(
            (i - expected_i).abs() < 1e-10,
            "inclination = {:.4}°, expected {:.4}°",
            i.to_degrees(),
            expected_i.to_degrees()
        );
    }

    #[test]
    fn satellite_spec_initial_state_inclined_with_raan() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec(
            "altitude=400,inclination=51.6,raan=90,id=iss-like",
            KnownBody::Earth,
        );
        let state = spec.initial_state(mu, None).unwrap();

        let h = state.position().cross(state.velocity());
        let i = (h[2] / h.magnitude()).acos();
        assert!(
            (i - 51.6_f64.to_radians()).abs() < 1e-10,
            "inclination = {:.4}°, expected 51.6°",
            i.to_degrees()
        );

        let k = nalgebra::Vector3::new(0.0, 0.0, 1.0);
        let n = k.cross(&h);
        let raan = n[1].atan2(n[0]);
        let raan = if raan < 0.0 {
            raan + 2.0 * std::f64::consts::PI
        } else {
            raan
        };
        assert!(
            (raan - 90.0_f64.to_radians()).abs() < 1e-10,
            "RAAN = {:.4}°, expected 90°",
            raan.to_degrees()
        );
    }

    #[test]
    fn satellite_spec_initial_state_equatorial_default() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec("altitude=400,id=test", KnownBody::Earth);
        let state = spec.initial_state(mu, None).unwrap();
        assert!(
            state.position()[2].abs() < 1e-10,
            "equatorial orbit should have z ≈ 0, got {}",
            state.position()[2]
        );
    }

    #[test]
    fn satellite_spec_entity_path() {
        let spec = parse_sat_spec("altitude=400,id=my-sat", KnownBody::Earth);
        let path = spec.entity_path();
        assert_eq!(path.to_string(), "/world/sat/my-sat");
    }

    #[test]
    fn omm_initial_state_is_sgp4_not_two_body() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec(
            "tle-line1=1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993,tle-line2=2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000,id=iss",
            KnownBody::Earth,
        );
        let OrbitSpec::Omm { omm } = &spec.orbit else {
            panic!("expected Omm orbit");
        };

        // No epoch → propagate to the element epoch (Δt = 0), then TEME→SimpleEci.
        let p = *spec.initial_state(mu, None).unwrap().position();

        // Must equal a direct SGP4 propagation + rotation (pins the wiring).
        let (r_teme, v_teme) = Sgp4Propagator::from_elements(omm)
            .unwrap()
            .propagate(omm.epoch)
            .unwrap();
        let (r_eci, _) =
            FrameTransform::<Teme, SimpleEci>::teme_to_simple_eci(&omm.epoch.to_ut1_naive())
                .transform_state(&r_teme, &v_teme);
        assert!(
            (p - r_eci.into_inner()).norm() < 1e-9,
            "initial_state must be the SGP4→SimpleEci state"
        );

        // …and differ from the old two-body seed (`to_keplerian_elements` is a
        // tens-of-km-off approximation): this is the behaviour PR4 fixes.
        let (r_two_body, _) = omm.to_keplerian_elements(mu).to_state_vector(mu);
        let diff = (p - r_two_body).norm();
        assert!(
            diff > 1.0,
            "SGP4 seed should differ from the two-body seed by ≫ 1 km, got {diff:.2} km"
        );

        // Sanity: still a plausible LEO state.
        assert!(
            (p.magnitude() - 6778.0).abs() < 200.0,
            "ISS radius ~6778 km, got {}",
            p.magnitude()
        );
    }
}
