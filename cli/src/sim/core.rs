use std::collections::HashMap;

use orts::OrbitalState;
use orts::orbital::OrbitalSystem;
use orts::orbital::kepler::KeplerianElements;
use orts::setup::SatelliteParams;
use serde::{Deserialize, Serialize};

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
pub fn accel_breakdown(
    system: &OrbitalSystem,
    t: f64,
    state: &OrbitalState,
) -> HashMap<String, f64> {
    system
        .acceleration_breakdown(t, state)
        .into_iter()
        .map(|(name, mag)| (name.to_string(), mag))
        .collect()
}

/// Convert a SatelliteSpec to SatelliteParams for OrbitalSystem construction.
pub fn sat_params(spec: &SatelliteSpec) -> SatelliteParams {
    let has_tle_drag =
        matches!(&spec.orbit, OrbitSpec::Tle { tle_data, .. } if tle_data.bstar.abs() > 1e-15);
    SatelliteParams {
        has_drag: has_tle_drag || spec.ballistic_coeff.is_some(),
        ballistic_coeff: spec.ballistic_coeff,
        srp_area_to_mass: spec.srp_area_to_mass,
        srp_cr: spec.srp_cr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MU: f64 = 398600.4418;

    #[test]
    fn history_state_has_satellite_id() {
        let hs = make_history_state(
            "test-sat",
            10.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            HashMap::new(),
        );
        assert_eq!(hs.satellite_id, "test-sat");
        assert!((hs.t - 10.0).abs() < 1e-9);
    }

    #[test]
    fn history_state_satellite_id_serialized() {
        let hs = make_history_state(
            "my-sat",
            5.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            HashMap::new(),
        );
        let json = serde_json::to_string(&hs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellite_id"], "my-sat");
    }
}
