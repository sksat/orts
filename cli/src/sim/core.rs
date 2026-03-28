use std::collections::HashMap;

use orts::OrbitalState;
use orts::orbital::OrbitalSystem;
use orts::orbital::kepler::KeplerianElements;
use orts::setup::SatelliteParams;
use serde::{Deserialize, Serialize};

use crate::satellite::{OrbitSpec, SatelliteSpec};

/// Attitude telemetry payload for WebSocket protocol.
///
/// Encapsulates quaternion and angular velocity as a single unit to prevent
/// half-populated states. The `source` field distinguishes how the attitude
/// was produced (propagated dynamics vs. derived).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AttitudePayload {
    /// Body-to-inertial quaternion [w, x, y, z] (Hamilton scalar-first).
    pub quaternion_wxyz: [f64; 4],
    /// Angular velocity in body frame [rad/s].
    pub angular_velocity_body: [f64; 3],
    /// How this attitude was produced.
    pub source: AttitudeSource,
}

/// How the attitude data was produced.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AttitudeSource {
    #[serde(rename = "propagated")]
    Propagated,
}

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
    /// Attitude telemetry (present only when SpacecraftDynamics is used).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attitude: Option<AttitudePayload>,
}

/// Create a HistoryState from position/velocity, computing Keplerian elements.
pub fn make_history_state(
    satellite_id: &str,
    t: f64,
    pos: &nalgebra::Vector3<f64>,
    vel: &nalgebra::Vector3<f64>,
    mu: f64,
    accelerations: HashMap<String, f64>,
    attitude: Option<AttitudePayload>,
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
        attitude,
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

/// Compute acceleration breakdown from a SpacecraftDynamics system.
///
/// Uses [`SpacecraftDynamics::acceleration_breakdown`], mirroring
/// the output format of [`accel_breakdown`] for protocol compatibility.
pub fn spacecraft_accel_breakdown(
    dynamics: &orts::spacecraft::SpacecraftDynamics<Box<dyn orts::orbital::gravity::GravityField>>,
    t: f64,
    state: &orts::spacecraft::SpacecraftState,
) -> HashMap<String, f64> {
    dynamics
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
            None,
        );
        assert_eq!(hs.satellite_id, "test-sat");
        assert!((hs.t - 10.0).abs() < 1e-9);
        assert!(hs.attitude.is_none());
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
            None,
        );
        let json = serde_json::to_string(&hs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellite_id"], "my-sat");
        // attitude should be absent (skip_serializing_if)
        assert!(v.get("attitude").is_none());
    }

    #[test]
    fn attitude_payload_roundtrip() {
        let payload = AttitudePayload {
            quaternion_wxyz: [1.0, 0.0, 0.0, 0.0],
            angular_velocity_body: [0.01, -0.02, 0.03],
            source: AttitudeSource::Propagated,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: AttitudePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.quaternion_wxyz, payload.quaternion_wxyz);
        assert_eq!(
            deserialized.angular_velocity_body,
            payload.angular_velocity_body
        );
        assert_eq!(deserialized.source, AttitudeSource::Propagated);
    }

    #[test]
    fn history_state_with_attitude() {
        let attitude = Some(AttitudePayload {
            quaternion_wxyz: [0.707, 0.0, 0.707, 0.0],
            angular_velocity_body: [0.0, 0.1, 0.0],
            source: AttitudeSource::Propagated,
        });
        let hs = make_history_state(
            "att-sat",
            20.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            HashMap::new(),
            attitude,
        );
        assert!(hs.attitude.is_some());
        let att = hs.attitude.unwrap();
        assert!((att.quaternion_wxyz[0] - 0.707).abs() < 1e-9);

        // Serialization should include attitude
        let hs2 = make_history_state(
            "att-sat",
            20.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            HashMap::new(),
            Some(AttitudePayload {
                quaternion_wxyz: [0.707, 0.0, 0.707, 0.0],
                angular_velocity_body: [0.0, 0.1, 0.0],
                source: AttitudeSource::Propagated,
            }),
        );
        let json = serde_json::to_string(&hs2).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("attitude").is_some());
        assert_eq!(v["attitude"]["source"], "propagated");
    }

    #[test]
    fn attitude_payload_deserialize_from_json() {
        let json = r#"{"quaternion_wxyz":[1,0,0,0],"angular_velocity_body":[0,0,0],"source":"propagated"}"#;
        let payload: AttitudePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.quaternion_wxyz, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(payload.source, AttitudeSource::Propagated);
    }
}
