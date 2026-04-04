use std::collections::HashMap;

use orts::record::entity_path::EntityPath;
use serde::{Deserialize, Serialize};

use crate::config::{SatelliteConfig, SimConfig};
use crate::satellite::SatelliteInfo;
use crate::sim::core::HistoryState;

/// Client-to-server WebSocket message.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "query_range")]
    QueryRange {
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        entity_path: Option<EntityPath>,
    },
    /// Start a simulation from idle state.
    #[serde(rename = "start_simulation")]
    StartSimulation { config: SimConfig },
    /// Add a satellite to a running simulation.
    #[serde(rename = "add_satellite")]
    AddSatellite {
        #[serde(flatten)]
        satellite: SatelliteConfig,
    },
    /// Pause a running simulation.
    #[serde(rename = "pause_simulation")]
    PauseSimulation,
    /// Resume a paused simulation.
    #[serde(rename = "resume_simulation")]
    ResumeSimulation,
    /// Terminate the simulation and return to idle.
    #[serde(rename = "terminate_simulation")]
    TerminateSimulation,
}

/// Server-to-client WebSocket message.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Simulation metadata sent once when a client connects.
    #[serde(rename = "info")]
    Info {
        mu: f64,
        dt: f64,
        output_interval: f64,
        stream_interval: f64,
        central_body: String,
        central_body_radius: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        epoch_jd: Option<f64>,
        satellites: Vec<SatelliteInfo>,
    },
    /// A single simulation state snapshot.
    #[serde(rename = "state")]
    State {
        entity_path: EntityPath,
        t: f64,
        position: [f64; 3],
        velocity: [f64; 3],
        semi_major_axis: f64,
        eccentricity: f64,
        inclination: f64,
        raan: f64,
        argument_of_periapsis: f64,
        true_anomaly: f64,
        /// Pre-computed derived values for chart display (avoids client-side recomputation).
        altitude: f64,
        specific_energy: f64,
        angular_momentum: f64,
        velocity_mag: f64,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        accelerations: HashMap<String, f64>,
        /// Attitude telemetry (present only when SpacecraftDynamics is used).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attitude: Option<crate::sim::core::AttitudePayload>,
    },
    /// Bounded history overview sent on connect.
    ///
    /// The server is deliberately time-range-agnostic: it ships a fixed
    /// downsampled overview of the full simulation (capped at
    /// `OVERVIEW_MAX_POINTS_PER_ENTITY` per satellite) regardless of how
    /// long it has been running. Clients that need higher-resolution data
    /// for a specific display window pull it via a follow-up
    /// [`ClientMessage::QueryRange`] request. See
    /// [`HistoryBuffer::overview`](crate::commands::serve::history::HistoryBuffer::overview)
    /// for the incrementally-maintained cache that makes the handshake
    /// O(1) in sim duration.
    #[serde(rename = "history")]
    History { states: Vec<HistoryState> },
    /// Response to a client query_range request.
    #[serde(rename = "query_range_response")]
    QueryRangeResponse {
        t_min: f64,
        t_max: f64,
        states: Vec<HistoryState>,
    },
    /// Notification that a satellite's simulation has terminated.
    #[serde(rename = "simulation_terminated")]
    SimulationTerminated {
        entity_path: EntityPath,
        t: f64,
        reason: String,
    },
    /// Server status (sent on connect when idle).
    #[serde(rename = "status")]
    Status { state: String },
    /// Confirmation that a satellite was added.
    #[serde(rename = "satellite_added")]
    SatelliteAdded { satellite: SatelliteInfo, t: f64 },
    /// Notification that high-resolution textures are now available for a body.
    #[serde(rename = "textures_ready")]
    TexturesReady { body: String },
    /// Error response.
    #[serde(rename = "error")]
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::core::make_history_state;
    use orts::record::entity_path::EntityPath;

    const TEST_MU: f64 = 398600.4418;
    const TEST_BODY_RADIUS: f64 = 6378.137;

    fn make_state(t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        make_history_state(
            EntityPath::parse("/world/sat/default"),
            t,
            &pos,
            &vel,
            TEST_MU,
            TEST_BODY_RADIUS,
            HashMap::new(),
            None,
        )
    }

    #[test]
    fn history_message_serialization() {
        let msg = WsMessage::History {
            states: vec![make_state(0.0), make_state(10.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history");
        let states = v["states"].as_array().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0]["t"], 0.0);
        assert_eq!(states[0]["position"].as_array().unwrap().len(), 3);
        assert_eq!(states[0]["velocity"].as_array().unwrap().len(), 3);
        assert_eq!(states[1]["t"], 10.0);
    }

    #[test]
    fn history_message_empty_states() {
        let msg = WsMessage::History { states: vec![] };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history");
        assert_eq!(v["states"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn client_message_query_range_deserialize() {
        let json = r#"{"type":"query_range","t_min":10.0,"t_max":50.0,"max_points":100}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange {
                t_min,
                t_max,
                max_points,
                entity_path,
            } => {
                assert!((t_min - 10.0).abs() < 1e-9);
                assert!((t_max - 50.0).abs() < 1e-9);
                assert_eq!(max_points, Some(100));
                assert_eq!(entity_path, None);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn client_message_query_range_without_max_points() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange {
                max_points,
                entity_path,
                ..
            } => {
                assert_eq!(max_points, None);
                assert_eq!(entity_path, None);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn client_message_query_range_with_entity_path() {
        let json =
            r#"{"type":"query_range","t_min":0.0,"t_max":100.0,"entity_path":"/world/sat/iss"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { entity_path, .. } => {
                assert_eq!(entity_path, Some(EntityPath::parse("/world/sat/iss")));
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn query_range_response_serialization() {
        let msg = WsMessage::QueryRangeResponse {
            t_min: 10.0,
            t_max: 50.0,
            states: vec![make_state(20.0), make_state(30.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "query_range_response");
        assert_eq!(v["t_min"], 10.0);
        assert_eq!(v["t_max"], 50.0);
        assert_eq!(v["states"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn info_message_has_satellites_array() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![
                SatelliteInfo {
                    id: "sso".to_string(),
                    name: Some("SSO 800km".to_string()),
                    altitude: 800.0,
                    period: 6052.5,
                    perturbations: vec![],
                },
                SatelliteInfo {
                    id: "iss".to_string(),
                    name: Some("ISS (ZARYA)".to_string()),
                    altitude: 420.0,
                    period: 5560.0,
                    perturbations: vec![],
                },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        let sats = v["satellites"].as_array().unwrap();
        assert_eq!(sats.len(), 2);
        assert_eq!(sats[0]["id"], "sso");
        assert_eq!(sats[0]["name"], "SSO 800km");
        assert_eq!(sats[1]["id"], "iss");
        assert!(v.get("altitude").is_none());
        assert!(v.get("period").is_none());
        assert!(v.get("satellite_name").is_none());
    }

    #[test]
    fn info_message_with_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![SatelliteInfo {
                id: "default".to_string(),
                name: None,
                altitude: 400.0,
                period: 5554.0,
                perturbations: vec![],
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        assert_eq!(v["epoch_jd"], 2460390.0);
        assert!(v["satellites"].is_array());
    }

    #[test]
    fn info_message_without_epoch() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: None,
            satellites: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "info");
        assert!(v.get("epoch_jd").is_none());
    }

    #[test]
    fn info_message_with_satellite_info() {
        let msg = WsMessage::Info {
            mu: 398600.4418,
            dt: 10.0,
            output_interval: 10.0,
            stream_interval: 10.0,
            central_body: "earth".to_string(),
            central_body_radius: 6378.137,
            epoch_jd: Some(2460390.0),
            satellites: vec![SatelliteInfo {
                id: "iss".to_string(),
                name: Some("ISS (ZARYA)".to_string()),
                altitude: 420.0,
                period: 5560.0,
                perturbations: vec![],
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["satellites"][0]["name"], "ISS (ZARYA)");
        assert_eq!(v["satellites"][0]["id"], "iss");
    }

    #[test]
    fn client_message_pause_simulation() {
        let json = r#"{"type":"pause_simulation"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::PauseSimulation));
    }

    #[test]
    fn client_message_resume_simulation() {
        let json = r#"{"type":"resume_simulation"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::ResumeSimulation));
    }

    #[test]
    fn client_message_terminate_simulation() {
        let json = r#"{"type":"terminate_simulation"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::TerminateSimulation));
    }

    #[test]
    fn state_message_has_entity_path() {
        let msg = WsMessage::State {
            entity_path: EntityPath::parse("/world/sat/iss"),
            t: 100.0,
            position: [6778.0, 0.0, 0.0],
            velocity: [0.0, 7.669, 0.0],
            semi_major_axis: 6778.0,
            eccentricity: 0.001,
            inclination: 0.9,
            raan: 1.2,
            argument_of_periapsis: 0.5,
            true_anomaly: 2.1,
            altitude: 399.863,
            specific_energy: -0.1,
            angular_momentum: 51988.882,
            velocity_mag: 7.669,
            accelerations: HashMap::new(),
            attitude: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "state");
        assert_eq!(v["entity_path"], "/world/sat/iss");
        // attitude should be absent when None
        assert!(v.get("attitude").is_none());
    }

    #[test]
    fn textures_ready_serialization() {
        let msg = WsMessage::TexturesReady {
            body: "earth".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "textures_ready");
        assert_eq!(v["body"], "earth");
    }

    #[test]
    fn state_message_with_attitude() {
        use crate::sim::core::{AttitudePayload, AttitudeSource};
        let msg = WsMessage::State {
            entity_path: EntityPath::parse("/world/sat/sat1"),
            t: 50.0,
            position: [6778.0, 0.0, 0.0],
            velocity: [0.0, 7.669, 0.0],
            semi_major_axis: 6778.0,
            eccentricity: 0.0,
            inclination: 0.0,
            raan: 0.0,
            argument_of_periapsis: 0.0,
            true_anomaly: 0.0,
            altitude: 399.863,
            specific_energy: -0.1,
            angular_momentum: 51988.882,
            velocity_mag: 7.669,
            accelerations: HashMap::new(),
            attitude: Some(AttitudePayload {
                quaternion_wxyz: [0.707, 0.0, 0.707, 0.0],
                angular_velocity_body: [0.0, 0.01, 0.0],
                source: AttitudeSource::Propagated,
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "state");
        assert!(v.get("attitude").is_some());
        assert_eq!(v["attitude"]["source"], "propagated");
        let q = v["attitude"]["quaternion_wxyz"].as_array().unwrap();
        assert_eq!(q.len(), 4);
    }
}
