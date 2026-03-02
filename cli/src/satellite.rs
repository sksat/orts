use orts_datamodel::entity_path::EntityPath;
use orts_integrator::State;
use orts_orbits::{body::KnownBody, kepler::KeplerianElements, tle::Tle};
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
    /// From a TLE (parsed into Keplerian elements).
    Tle { tle_data: Tle, elements: KeplerianElements },
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
}

impl SatelliteSpec {
    pub fn initial_state(&self, mu: f64) -> State {
        match &self.orbit {
            OrbitSpec::Circular { r0, inclination, raan, .. } => {
                let elements = KeplerianElements {
                    semi_major_axis: *r0,
                    eccentricity: 0.0,
                    inclination: *inclination,
                    raan: *raan,
                    argument_of_periapsis: 0.0,
                    true_anomaly: 0.0,
                };
                let (pos, vel) = elements.to_state_vector(mu);
                State { position: pos, velocity: vel }
            }
            OrbitSpec::Tle { elements, .. } => {
                let (pos, vel) = elements.to_state_vector(mu);
                State { position: pos, velocity: vel }
            }
        }
    }

    /// Altitude for display purposes.
    pub fn altitude(&self, body: &KnownBody) -> f64 {
        match &self.orbit {
            OrbitSpec::Circular { altitude, .. } => *altitude,
            OrbitSpec::Tle { elements, .. } => {
                let perigee_r = elements.semi_major_axis * (1.0 - elements.eccentricity);
                perigee_r - body.properties().radius
            }
        }
    }

    pub fn entity_path(&self) -> EntityPath {
        EntityPath::parse(&format!("/world/sat/{}", self.id))
    }
}

/// Satellite info sent in the WebSocket info message.
#[derive(Serialize, Clone, Debug)]
pub struct SatelliteInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub altitude: f64,
    pub period: f64,
    /// Names of active perturbation force models (e.g. "drag", "srp", "third_body_sun").
    pub perturbations: Vec<String>,
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
                "altitude" => altitude = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid altitude: {value}"))),
                "inclination" => inclination = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid inclination: {value}"))),
                "raan" => raan = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid raan: {value}"))),
                "norad-id" => norad_id = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid norad-id: {value}"))),
                "tle-line1" => tle_line1 = Some(value.trim().to_string()),
                "tle-line2" => tle_line2 = Some(value.trim().to_string()),
                "ballistic-coeff" => ballistic_coeff = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid ballistic-coeff: {value}"))),
                "srp-area-to-mass" => srp_area_to_mass = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid srp-area-to-mass: {value}"))),
                "srp-cr" => srp_cr = Some(value.trim().parse().unwrap_or_else(|_| panic!("Invalid srp-cr: {value}"))),
                k => panic!("Unknown satellite spec key: {k}"),
            }
        }
    }

    // Determine orbit
    let (orbit, period, derived_name) = if let Some(norad) = norad_id {
        let tle = fetch_tle_by_norad_id(norad);
        let elements = tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let tle_name = tle.name.clone();
        (OrbitSpec::Tle { tle_data: tle, elements }, period, tle_name)
    } else if let (Some(l1), Some(l2)) = (tle_line1, tle_line2) {
        let text = format!("{l1}\n{l2}");
        let tle = Tle::parse(&text).unwrap_or_else(|e| panic!("Failed to parse TLE in --sat: {e}"));
        let elements = tle.to_keplerian_elements(mu);
        let period = elements.period(mu);
        let tle_name = tle.name.clone();
        (OrbitSpec::Tle { tle_data: tle, elements }, period, tle_name)
    } else {
        let alt = altitude.unwrap_or(400.0);
        let r0 = body.properties().radius + alt;
        let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
        let inc = inclination.unwrap_or(0.0).to_radians();
        let ra = raan.unwrap_or(0.0).to_radians();
        (OrbitSpec::Circular { altitude: alt, r0, inclination: inc, raan: ra }, period, None)
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
    }
}

pub fn parse_body(s: &str) -> KnownBody {
    match s {
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
        _ => panic!("Unknown body: {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sat_spec_circular_altitude() {
        let spec = parse_sat_spec("altitude=800,id=sso", KnownBody::Earth);
        assert_eq!(spec.id, "sso");
        assert!(matches!(spec.orbit, OrbitSpec::Circular { altitude, .. } if (altitude - 800.0).abs() < 1e-9));
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
        assert!(matches!(spec.orbit, OrbitSpec::Tle { .. }));
    }

    #[test]
    fn satellite_spec_initial_state_circular() {
        let spec = parse_sat_spec("altitude=400,id=test", KnownBody::Earth);
        let mu = KnownBody::Earth.properties().mu;
        let state = spec.initial_state(mu);
        let r = state.position.magnitude();
        let expected_r = 6378.137 + 400.0;
        assert!((r - expected_r).abs() < 1e-6, "r = {r}, expected {expected_r}");
    }

    #[test]
    fn satellite_spec_initial_state_inclined() {
        let mu = KnownBody::Earth.properties().mu;
        let spec = parse_sat_spec("altitude=800,inclination=98.6,id=sso-test", KnownBody::Earth);
        let state = spec.initial_state(mu);

        let r = state.position.magnitude();
        let expected_r = 6378.137 + 800.0;
        assert!((r - expected_r).abs() < 1e-6, "r = {r}, expected {expected_r}");

        let v = state.velocity.magnitude();
        let expected_v = (mu / expected_r).sqrt();
        assert!((v - expected_v).abs() < 1e-6, "v = {v}, expected {expected_v}");

        let h = state.position.cross(&state.velocity);
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
        let spec = parse_sat_spec("altitude=400,inclination=51.6,raan=90,id=iss-like", KnownBody::Earth);
        let state = spec.initial_state(mu);

        let h = state.position.cross(&state.velocity);
        let i = (h[2] / h.magnitude()).acos();
        assert!(
            (i - 51.6_f64.to_radians()).abs() < 1e-10,
            "inclination = {:.4}°, expected 51.6°",
            i.to_degrees()
        );

        let k = nalgebra::Vector3::new(0.0, 0.0, 1.0);
        let n = k.cross(&h);
        let raan = n[1].atan2(n[0]);
        let raan = if raan < 0.0 { raan + 2.0 * std::f64::consts::PI } else { raan };
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
        let state = spec.initial_state(mu);
        assert!(
            state.position[2].abs() < 1e-10,
            "equatorial orbit should have z ≈ 0, got {}",
            state.position[2]
        );
    }

    #[test]
    fn satellite_spec_entity_path() {
        let spec = parse_sat_spec("altitude=400,id=my-sat", KnownBody::Earth);
        let path = spec.entity_path();
        assert_eq!(path.to_string(), "/world/sat/my-sat");
    }
}
