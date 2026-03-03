use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use orts_sim::record::archetypes::OrbitalState;
use orts_sim::record::entity_path::EntityPath;
use orts_sim::record::recording::Recording;
use orts_sim::record::timeline::TimePoint;
use serde::{Deserialize, Serialize};

use crate::satellite::SatelliteInfo;
use crate::sim::core::{HistoryState, make_history_state};

/// Client-to-server WebSocket message.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "query_range")]
    QueryRange {
        t_min: f64,
        t_max: f64,
        max_points: Option<usize>,
        satellite_id: Option<String>,
    },
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
        satellite_id: String,
        t: f64,
        position: [f64; 3],
        velocity: [f64; 3],
        semi_major_axis: f64,
        eccentricity: f64,
        inclination: f64,
        raan: f64,
        argument_of_periapsis: f64,
        true_anomaly: f64,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        accelerations: HashMap<String, f64>,
    },
    /// Downsampled history overview sent on connect.
    #[serde(rename = "history")]
    History { states: Vec<HistoryState> },
    /// Full-resolution history chunk sent in background.
    #[serde(rename = "history_detail")]
    HistoryDetail { states: Vec<HistoryState> },
    /// Marker indicating all detail chunks have been sent.
    #[serde(rename = "history_detail_complete")]
    HistoryDetailComplete,
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
        satellite_id: String,
        t: f64,
        reason: String,
    },
}

/// Bounded buffer that accumulates history states and periodically flushes to .rrd segments.
pub struct HistoryBuffer {
    /// Recent states kept in memory.
    pub states: VecDeque<HistoryState>,
    /// Maximum number of states to keep in memory before flushing.
    pub capacity: usize,
    /// Directory for .rrd segment files.
    pub data_dir: PathBuf,
    /// Number of segment files written so far.
    pub segment_count: u32,
    /// Gravitational parameter (for computing Keplerian elements from loaded data).
    pub mu: f64,
}

impl HistoryBuffer {
    pub fn new(capacity: usize, data_dir: PathBuf, mu: f64) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
        HistoryBuffer {
            states: VecDeque::new(),
            capacity,
            data_dir,
            segment_count: 0,
            mu,
        }
    }

    /// Push a state into the buffer. Flushes to .rrd if capacity is exceeded.
    pub fn push(&mut self, state: HistoryState) {
        self.states.push_back(state);
        if self.states.len() > self.capacity {
            self.flush();
        }
    }

    /// Flush the oldest half of the buffer to a .rrd segment file.
    pub fn flush(&mut self) {
        let flush_count = self.states.len() / 2;
        if flush_count == 0 {
            return;
        }

        let to_flush: Vec<HistoryState> = self.states.drain(..flush_count).collect();

        let mut rec = Recording::new();

        for (i, hs) in to_flush.iter().enumerate() {
            let sat_path = EntityPath::parse(&format!("/world/sat/{}", hs.satellite_id));
            let tp = TimePoint::new()
                .with_sim_time(hs.t)
                .with_step(i as u64);
            let os = OrbitalState::new(
                nalgebra::Vector3::new(hs.position[0], hs.position[1], hs.position[2]),
                nalgebra::Vector3::new(hs.velocity[0], hs.velocity[1], hs.velocity[2]),
            );
            rec.log_orbital_state(&sat_path, &tp, &os);
        }

        let seg_path = self
            .data_dir
            .join(format!("seg_{:04}.rrd", self.segment_count));
        if let Err(e) =
            orts_sim::record::rerun_export::save_as_rrd(&rec, "orts", seg_path.to_str().unwrap())
        {
            eprintln!("Warning: failed to flush segment: {e}");
            return;
        }

        self.segment_count += 1;
    }

    /// Load all data: .rrd segments + in-memory buffer, sorted by time.
    pub fn load_all(&self) -> Vec<HistoryState> {
        let mut all = Vec::new();

        // Read .rrd segment files in order
        for i in 0..self.segment_count {
            let seg_path = self.data_dir.join(format!("seg_{i:04}.rrd"));
            match orts_sim::record::rerun_export::load_from_rrd(seg_path.to_str().unwrap()) {
                Ok(rows) => {
                    for row in rows {
                        let pos = nalgebra::Vector3::new(row.x, row.y, row.z);
                        let vel = nalgebra::Vector3::new(row.vx, row.vy, row.vz);
                        // Entity path info is not preserved in RrdRow; extract from entity_path if available
                        let sid = row.entity_path.as_deref()
                            .and_then(|p| p.rsplit('/').next())
                            .unwrap_or("default");
                        all.push(make_history_state(sid, row.t, &pos, &vel, self.mu, HashMap::new()));
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to read segment {i}: {e}");
                }
            }
        }

        // Append in-memory buffer
        all.extend(self.states.iter().cloned());

        // Sort by time for multi-satellite interleaving
        all.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        all
    }

    /// Query states within a time range, optionally downsampled.
    pub fn query_range(&self, t_min: f64, t_max: f64, max_points: Option<usize>) -> Vec<HistoryState> {
        let all = self.load_all();
        let filtered: Vec<HistoryState> = all
            .into_iter()
            .filter(|s| s.t >= t_min && s.t <= t_max)
            .collect();
        match max_points {
            Some(mp) => Self::downsample(&filtered, mp),
            None => filtered,
        }
    }

    /// Downsample a list of states to at most `max_points`, always preserving first and last.
    pub fn downsample(states: &[HistoryState], max_points: usize) -> Vec<HistoryState> {
        let n = states.len();
        if n <= max_points || max_points < 2 {
            return states.to_vec();
        }

        let mut result = Vec::with_capacity(max_points);
        result.push(states[0].clone());

        // Distribute remaining (max_points - 2) samples evenly across the interior
        let interior = max_points - 2;
        for i in 1..=interior {
            let idx = i * (n - 1) / (interior + 1);
            result.push(states[idx].clone());
        }

        result.push(states[n - 1].clone());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MU: f64 = 398600.4418;

    fn make_state(t: f64) -> HistoryState {
        let pos = nalgebra::Vector3::new(6778.0 + t, t * 0.1, 0.0);
        let vel = nalgebra::Vector3::new(0.0, 7.669, 0.0);
        make_history_state("default", t, &pos, &vel, TEST_MU, HashMap::new())
    }

    fn temp_data_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orts-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    fn cleanup_dir(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // --- HistoryBuffer tests ---

    #[test]
    fn buffer_push_and_read() {
        let dir = temp_data_dir("push-read");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        buf.push(make_state(0.0));
        buf.push(make_state(10.0));
        buf.push(make_state(20.0));

        let all = buf.load_all();
        assert_eq!(all.len(), 3);
        assert!((all[0].t - 0.0).abs() < 1e-9);
        assert!((all[1].t - 10.0).abs() < 1e-9);
        assert!((all[2].t - 20.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_flush_creates_segment() {
        let dir = temp_data_dir("flush-seg");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU);

        // Push 5 states → exceeds capacity of 4 → triggers flush
        for i in 0..5 {
            buf.push(make_state(i as f64 * 10.0));
        }

        assert_eq!(buf.segment_count, 1);
        assert!(dir.join("seg_0000.rrd").exists());
        // After flushing half (2), buffer should have 3 states
        assert_eq!(buf.states.len(), 3);

        cleanup_dir(&dir);
    }

    #[test]
    fn buffer_load_all_includes_flushed_and_buffered() {
        let dir = temp_data_dir("load-all");
        let mut buf = HistoryBuffer::new(4, dir.clone(), TEST_MU);

        for i in 0..8 {
            buf.push(make_state(i as f64 * 10.0));
        }

        // Should have flushed some segments
        assert!(buf.segment_count > 0);

        let all = buf.load_all();
        assert_eq!(all.len(), 8);

        // Verify all times are present (order may differ slightly due to .rrd roundtrip)
        let mut times: Vec<f64> = all.iter().map(|s| s.t).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for i in 0..8 {
            assert!(
                (times[i] - i as f64 * 10.0).abs() < 0.01,
                "times[{i}] = {}, expected {}",
                times[i],
                i as f64 * 10.0
            );
        }

        cleanup_dir(&dir);
    }

    // --- Downsample tests ---

    #[test]
    fn downsample_correctness() {
        let states: Vec<HistoryState> = (0..100).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);

        assert_eq!(ds.len(), 10);
        // First and last are preserved
        assert!((ds[0].t - 0.0).abs() < 1e-9);
        assert!((ds[9].t - 99.0).abs() < 1e-9);
    }

    #[test]
    fn downsample_preserves_all_when_small() {
        let states: Vec<HistoryState> = (0..5).map(|i| make_state(i as f64)).collect();
        let ds = HistoryBuffer::downsample(&states, 10);
        assert_eq!(ds.len(), 5);
    }

    #[test]
    fn downsample_performance() {
        let states: Vec<HistoryState> = (0..100_000).map(|i| make_state(i as f64)).collect();
        let start = std::time::Instant::now();
        let ds = HistoryBuffer::downsample(&states, 1000);
        let elapsed = start.elapsed();

        assert_eq!(ds.len(), 1000);
        assert!(
            elapsed.as_millis() < 10,
            "downsample took {}ms, expected <10ms",
            elapsed.as_millis()
        );
    }

    // --- Performance tests ---

    #[test]
    fn flush_performance() {
        let dir = temp_data_dir("flush-perf");
        let mut buf = HistoryBuffer::new(10_000, dir.clone(), TEST_MU);

        for i in 0..5000 {
            buf.states.push_back(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        buf.flush();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 2000,
            "flush took {}ms, expected <2000ms",
            elapsed.as_millis()
        );
        assert_eq!(buf.segment_count, 1);

        cleanup_dir(&dir);
    }

    #[test]
    fn load_all_performance() {
        let dir = temp_data_dir("load-perf");
        let mut buf = HistoryBuffer::new(2000, dir.clone(), TEST_MU);

        // Insert 10000 points, which will create multiple segments
        for i in 0..10_000 {
            buf.push(make_state(i as f64));
        }

        let start = std::time::Instant::now();
        let all = buf.load_all();
        let elapsed = start.elapsed();

        assert_eq!(all.len(), 10_000);
        assert!(
            elapsed.as_millis() < 2000,
            "load_all took {}ms, expected <2000ms",
            elapsed.as_millis()
        );

        cleanup_dir(&dir);
    }

    // --- WsMessage serialization tests ---

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
    fn history_detail_message_serialization() {
        let msg = WsMessage::HistoryDetail {
            states: vec![make_state(5.0)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history_detail");
        assert_eq!(v["states"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn history_detail_complete_serialization() {
        let msg = WsMessage::HistoryDetailComplete;
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "history_detail_complete");
        // Should not have a "states" field
        assert!(v.get("states").is_none());
    }

    // --- ClientMessage tests ---

    #[test]
    fn client_message_query_range_deserialize() {
        let json = r#"{"type":"query_range","t_min":10.0,"t_max":50.0,"max_points":100}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange {
                t_min,
                t_max,
                max_points,
                satellite_id,
            } => {
                assert!((t_min - 10.0).abs() < 1e-9);
                assert!((t_max - 50.0).abs() < 1e-9);
                assert_eq!(max_points, Some(100));
                assert_eq!(satellite_id, None);
            }
        }
    }

    #[test]
    fn client_message_query_range_without_max_points() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { max_points, satellite_id, .. } => {
                assert_eq!(max_points, None);
                assert_eq!(satellite_id, None);
            }
        }
    }

    #[test]
    fn client_message_query_range_with_satellite_id() {
        let json = r#"{"type":"query_range","t_min":0.0,"t_max":100.0,"satellite_id":"iss"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::QueryRange { satellite_id, .. } => {
                assert_eq!(satellite_id, Some("iss".to_string()));
            }
        }
    }

    // --- QueryRangeResponse serialization ---

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

    // --- query_range tests ---

    #[test]
    fn query_range_filters_by_time() {
        let dir = temp_data_dir("qr-filter");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(20.0, 60.0, None);
        assert!(result.len() >= 4, "should include t=20,30,40,50,60");
        for s in &result {
            assert!(s.t >= 20.0 && s.t <= 60.0, "t={} out of range", s.t);
        }

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_with_downsample() {
        let dir = temp_data_dir("qr-ds");
        let mut buf = HistoryBuffer::new(200, dir.clone(), TEST_MU);

        for i in 0..100 {
            buf.push(make_state(i as f64));
        }

        let result = buf.query_range(0.0, 99.0, Some(10));
        assert_eq!(result.len(), 10);
        // First and last preserved
        assert!((result[0].t - 0.0).abs() < 1e-9);
        assert!((result[9].t - 99.0).abs() < 1e-9);

        cleanup_dir(&dir);
    }

    #[test]
    fn query_range_empty_range() {
        let dir = temp_data_dir("qr-empty");
        let mut buf = HistoryBuffer::new(100, dir.clone(), TEST_MU);

        for i in 0..10 {
            buf.push(make_state(i as f64 * 10.0));
        }

        let result = buf.query_range(200.0, 300.0, None);
        assert!(result.is_empty());

        cleanup_dir(&dir);
    }

    // --- Info message tests ---

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
        // Should NOT have flat altitude/period/satellite_name fields
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
        // epoch_jd should be absent (skip_serializing_if)
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
    fn state_message_has_satellite_id() {
        let msg = WsMessage::State {
            satellite_id: "iss".to_string(),
            t: 100.0,
            position: [6778.0, 0.0, 0.0],
            velocity: [0.0, 7.669, 0.0],
            semi_major_axis: 6778.0,
            eccentricity: 0.001,
            inclination: 0.9,
            raan: 1.2,
            argument_of_periapsis: 0.5,
            true_anomaly: 2.1,
            accelerations: HashMap::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "state");
        assert_eq!(v["satellite_id"], "iss");
    }
}
