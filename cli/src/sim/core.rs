use std::collections::HashMap;

use std::ops::ControlFlow;

use orts::OrbitalState;
use orts::group::HasPosition;
use orts::orbital::OrbitalSystem;
use orts::orbital::gravity::GravityField;
use orts::orbital::kepler::KeplerianElements;
use orts::perturbations::ThirdBodyGravity;
use orts::record::entity_path::EntityPath;
use orts::setup::{SatelliteParams, build_spacecraft_dynamics};
use orts::spacecraft::SpacecraftDynamics;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::AttitudeConfig;
use crate::satellite::{OrbitSpec, SatelliteSpec};
use crate::sim::params::SimParams;

/// Attitude telemetry payload for WebSocket protocol.
///
/// Encapsulates quaternion and angular velocity as a single unit to prevent
/// half-populated states. The `source` field distinguishes how the attitude
/// was produced (propagated dynamics vs. derived).
#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct AttitudePayload {
    /// Body-to-inertial quaternion [w, x, y, z] (Hamilton scalar-first).
    pub quaternion_wxyz: [f64; 4],
    /// Angular velocity in body frame [rad/s].
    pub angular_velocity_body: [f64; 3],
    /// How this attitude was produced.
    pub source: AttitudeSource,
    /// Reaction wheel angular momentum [N·m·s] per wheel (if RW is present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rw_momentum: Option<Vec<f64>>,
}

/// One model's torque on the body [N·m].
///
/// A list rather than a map keyed by name, because
/// [`Model::name`](orts::model::Model::name) is not unique: two models of one
/// spacecraft can answer the same name, and a map would keep whichever came
/// last. The order is the order the models are evaluated in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub struct ModelTorque {
    /// The model's own name, as `Model::name` gives it.
    ///
    /// Not renamed the way `accelerations` renames a panel model to the force
    /// it computes: a torque is reported per model, and which model produced it
    /// is the thing a reader is checking.
    pub model: String,
    /// Torque in the body frame [N·m].
    pub torque_body_nm: [f64; 3],
}

/// How the attitude data was produced.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum AttitudeSource {
    #[serde(rename = "propagated")]
    Propagated,
}

/// Viewer marker shape a satellite is rendered with when it has no 3D model.
///
/// Display concern only — carried from config through `SatelliteInfo` so a sim can
/// declare its preferred shape; the viewer may still override it per satellite.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[ts(export)]
pub enum MarkerShape {
    /// Featureless sphere (orientation not shown).
    #[serde(rename = "sphere")]
    Sphere,
    /// XYZ orientation cube (faces colored per body axis).
    #[serde(rename = "axes-cube")]
    AxesCube,
}

/// A single state snapshot used in history messages.
#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct HistoryState {
    #[ts(type = "string")]
    pub entity_path: EntityPath,
    pub t: f64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub raan: f64,
    pub argument_of_periapsis: f64,
    pub true_anomaly: f64,
    /// Pre-computed derived values for chart display.
    #[serde(default)]
    pub altitude: f64,
    #[serde(default)]
    pub specific_energy: f64,
    #[serde(default)]
    pub angular_momentum: f64,
    #[serde(default)]
    pub velocity_mag: f64,
    /// Per-force acceleration magnitudes [km/s²]: "gravity", "drag", "srp", etc.
    /// Omitted from the wire when empty.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[ts(as = "Option<_>", optional)]
    pub accelerations: HashMap<String, f64>,
    /// Per-model body torque [N·m], one entry per model.
    ///
    /// Every model appears, including one that only produces an acceleration:
    /// its entry is a measured zero, which is what a reader checking whether a
    /// disturbance is acting needs to see. Empty means the satellite has no
    /// models at all — an orbit-only satellite, or a sample taken before any
    /// were evaluated — and is omitted from the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<_>", optional)]
    pub torques: Vec<ModelTorque>,
    /// Attitude telemetry (present only when SpacecraftDynamics is used).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attitude: Option<AttitudePayload>,
}

/// What every model is doing to a spacecraft at one sample.
///
/// The two breakdowns travel together because they are read together and are
/// filled from the same dynamics: `accelerations` is a channel per physical
/// force, `torques` an entry per model. Bundling them keeps the constructors
/// below from growing another positional argument.
#[derive(Default, Clone, Debug)]
pub struct ModelLoads {
    /// Per-force acceleration magnitudes [km/s²].
    pub accelerations: HashMap<String, f64>,
    /// Per-model body torque [N·m].
    pub torques: Vec<ModelTorque>,
}

impl ModelLoads {
    /// Accelerations alone, for a satellite with no attitude to disturb.
    pub fn accelerations(accelerations: HashMap<String, f64>) -> Self {
        Self {
            accelerations,
            torques: Vec::new(),
        }
    }
}

/// Create a HistoryState from position/velocity, computing Keplerian elements and derived values.
#[allow(clippy::too_many_arguments)]
pub fn make_history_state(
    entity_path: EntityPath,
    t: f64,
    pos: &nalgebra::Vector3<f64>,
    vel: &nalgebra::Vector3<f64>,
    mu: f64,
    body_radius: f64,
    loads: ModelLoads,
    attitude: Option<AttitudePayload>,
) -> HistoryState {
    let ModelLoads {
        accelerations,
        torques,
    } = loads;
    let elements = KeplerianElements::from_state_vector(pos, vel, mu);
    let r_mag = pos.magnitude();
    let v_mag = vel.magnitude();
    let h = pos.cross(vel);
    HistoryState {
        entity_path,
        t,
        position: [pos.x, pos.y, pos.z],
        velocity: [vel.x, vel.y, vel.z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        altitude: r_mag - body_radius,
        specific_energy: v_mag * v_mag / 2.0 - mu / r_mag,
        angular_momentum: h.magnitude(),
        velocity_mag: v_mag,
        accelerations,
        torques,
        attitude,
    }
}

/// Downsample a list of states to at most `max_points`, always preserving first and last.
pub fn downsample_states(states: &[HistoryState], max_points: usize) -> Vec<HistoryState> {
    let n = states.len();
    if n <= max_points || max_points < 2 {
        return states.to_vec();
    }

    let mut result = Vec::with_capacity(max_points);
    result.push(states[0].clone());

    let interior = max_points - 2;
    for i in 1..=interior {
        let idx = i * (n - 1) / (interior + 1);
        result.push(states[idx].clone());
    }

    result.push(states[n - 1].clone());
    result
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

/// Both breakdowns for the wire, from one evaluation of every model.
///
/// Telemetry reports them at one sample, and evaluating every model twice for
/// that would double the panel models' shadow geometry, which is most of their
/// cost.
pub fn spacecraft_loads(
    dynamics: &orts::spacecraft::SpacecraftDynamics<Box<dyn orts::orbital::gravity::GravityField>>,
    t: f64,
    state: &orts::effector::AugmentedState<orts::spacecraft::SpacecraftState>,
) -> ModelLoads {
    let breakdown = dynamics.load_breakdown(t, state);
    ModelLoads {
        accelerations: breakdown
            .accelerations
            .into_iter()
            .map(|(name, mag)| (force_channel(name).to_string(), mag))
            .collect(),
        torques: breakdown
            .torques
            .into_iter()
            .map(|(model, torque)| {
                let torque = torque.into_inner();
                ModelTorque {
                    model: model.to_string(),
                    torque_body_nm: [torque.x, torque.y, torque.z],
                }
            })
            .collect(),
    }
}

/// Report a panel model under the name of the force it computes.
///
/// `accelerations` on the wire is a channel per physical force, not per model:
/// `PanelSrp` and the cannonball `SolarRadiationPressure` are two ways of
/// computing the same solar radiation pressure, and config rejects having both
/// on one satellite, so one channel serves either. The viewer's columns and its
/// four-term perturbation total are keyed on these names.
///
/// Which model produced it is carried by `SatelliteInfo::perturbations`, whose
/// business is model names.
pub fn force_channel(model_name: &str) -> &str {
    match model_name {
        "panel_drag" => "drag",
        "panel_srp" => "srp",
        other => other,
    }
}

/// Convert a SatelliteSpec to SatelliteParams for OrbitalSystem construction.
pub fn sat_params(spec: &SatelliteSpec) -> SatelliteParams {
    let has_bstar_drag = matches!(&spec.orbit, OrbitSpec::ElementSet { elements, .. } if elements.fields().bstar.abs() > 1e-15);
    SatelliteParams {
        has_drag: has_bstar_drag || spec.ballistic_coeff.is_some(),
        ballistic_coeff: spec.ballistic_coeff,
        srp_area_to_mass: spec.srp_area_to_mass,
        srp_cr: spec.srp_cr,
        disturbances: spec.disturbances,
        shape: spec.panels.clone(),
    }
}

/// Build the dynamics for one attitude satellite.
///
/// The single place any entry point turns a spec into `SpacecraftDynamics`, so
/// `orts run` and `orts serve` cannot end up with different model sets for the
/// same config. They could before: each added the gravity-gradient torque on
/// its own line, and a third such line sat in `run` after the other two moved
/// into `orts::setup`, so `run` evaluated the torque twice and ignored
/// `gravity_gradient = false`.
pub fn spacecraft_dynamics_for(
    spec: &SatelliteSpec,
    att: &AttitudeConfig,
    params: &SimParams,
    third_bodies: &[ThirdBodyGravity],
) -> Result<SpacecraftDynamics<Box<dyn GravityField>>, String> {
    build_spacecraft_dynamics(
        &params.body,
        params.central_gravity(),
        params.epoch,
        &sat_params(spec),
        third_bodies,
        att.inertia_matrix(),
        params.build_atmosphere_model(),
    )
    .map_err(|e| format!("solar force models: {e}"))
}

/// Terminate a satellite on surface impact or atmospheric entry.
///
/// Generic over the state so the orbit-only, spacecraft and controlled paths
/// share one termination rule: all three only need the position.
pub fn body_event_checker<S: HasPosition>(
    params: &SimParams,
) -> impl Fn(f64, &S) -> ControlFlow<String> + Send + 'static {
    let props = params.body.properties();
    let body_radius = props.radius;
    let atmosphere_altitude = props.atmosphere_altitude;
    move |_t: f64, state: &S| {
        let r = state.position().magnitude();
        if r < body_radius {
            ControlFlow::Break(format!("collision at {:.1} km altitude", r - body_radius))
        } else if let Some(atm_alt) = atmosphere_altitude {
            if r < body_radius + atm_alt {
                ControlFlow::Break(format!(
                    "atmospheric entry at {:.1} km altitude",
                    r - body_radius
                ))
            } else {
                ControlFlow::Continue(())
            }
        } else {
            ControlFlow::Continue(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orts::record::entity_path::EntityPath;

    const TEST_MU: f64 = 398600.4418;
    const TEST_BODY_RADIUS: f64 = 6378.137;

    #[test]
    fn history_state_has_entity_path() {
        let hs = make_history_state(
            EntityPath::parse("/world/sat/test-sat"),
            10.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            TEST_BODY_RADIUS,
            ModelLoads::default(),
            None,
        );
        assert_eq!(hs.entity_path, EntityPath::parse("/world/sat/test-sat"));
        assert!((hs.t - 10.0).abs() < 1e-9);
        assert!(hs.attitude.is_none());
    }

    #[test]
    fn history_state_entity_path_serialized() {
        let hs = make_history_state(
            EntityPath::parse("/world/sat/my-sat"),
            5.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            TEST_BODY_RADIUS,
            ModelLoads::default(),
            None,
        );
        let json = serde_json::to_string(&hs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["entity_path"], "/world/sat/my-sat");
        // attitude should be absent (skip_serializing_if)
        assert!(v.get("attitude").is_none());
    }

    #[test]
    fn attitude_payload_roundtrip() {
        let payload = AttitudePayload {
            quaternion_wxyz: [1.0, 0.0, 0.0, 0.0],
            angular_velocity_body: [0.01, -0.02, 0.03],
            source: AttitudeSource::Propagated,
            rw_momentum: None,
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
            rw_momentum: None,
        });
        let hs = make_history_state(
            EntityPath::parse("/world/sat/att-sat"),
            20.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            TEST_BODY_RADIUS,
            ModelLoads::default(),
            attitude,
        );
        assert!(hs.attitude.is_some());
        let att = hs.attitude.unwrap();
        assert!((att.quaternion_wxyz[0] - 0.707).abs() < 1e-9);

        // Serialization should include attitude
        let hs2 = make_history_state(
            EntityPath::parse("/world/sat/att-sat"),
            20.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            TEST_BODY_RADIUS,
            ModelLoads::default(),
            Some(AttitudePayload {
                quaternion_wxyz: [0.707, 0.0, 0.707, 0.0],
                angular_velocity_body: [0.0, 0.1, 0.0],
                source: AttitudeSource::Propagated,
                rw_momentum: None,
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

    /// The torque list travels as a list, because `Model::name` is not unique:
    /// two models answering one name would collapse into a single entry in a
    /// map, and the second one's torque would be the only one to arrive.
    #[test]
    fn two_models_of_one_name_both_reach_the_wire() {
        let state = HistoryState {
            torques: vec![
                ModelTorque {
                    model: "thruster".to_string(),
                    torque_body_nm: [1.0, 0.0, 0.0],
                },
                ModelTorque {
                    model: "thruster".to_string(),
                    torque_body_nm: [0.0, 2.0, 0.0],
                },
            ],
            ..make_history_state(
                EntityPath::parse("/world/sat/two"),
                0.0,
                &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
                &nalgebra::Vector3::new(0.0, 7.669, 0.0),
                TEST_MU,
                TEST_BODY_RADIUS,
                ModelLoads::default(),
                None,
            )
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: HistoryState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.torques.len(), 2, "both entries survive: {json}");
        assert_eq!(back.torques[1].torque_body_nm, [0.0, 2.0, 0.0]);
    }

    /// A sample with no torque leaves the field off the wire, so a reader can
    /// tell "no model produces one" from a measured zero.
    #[test]
    fn a_sample_without_torque_omits_the_field() {
        let state = make_history_state(
            EntityPath::parse("/world/sat/quiet"),
            0.0,
            &nalgebra::Vector3::new(6778.0, 0.0, 0.0),
            &nalgebra::Vector3::new(0.0, 7.669, 0.0),
            TEST_MU,
            TEST_BODY_RADIUS,
            ModelLoads::default(),
            None,
        );
        let json = serde_json::to_string(&state).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("torques").is_none(), "{json}");

        // And a zero torque a model did report is on the wire as zero.
        let measured = HistoryState {
            torques: vec![ModelTorque {
                model: "gravity_gradient".to_string(),
                torque_body_nm: [0.0, 0.0, 0.0],
            }],
            ..state
        };
        let json = serde_json::to_string(&measured).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["torques"][0]["model"], "gravity_gradient");
    }
}
