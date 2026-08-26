//! Conversion helpers between the plugin-layer Rust types
//! (`orts::plugin::*`) and the WIT-generated types (either
//! [`super::sync_bindings`] for the sync backend or
//! [`super::async_bindings`] for the fiber-based async backend).
//!
//! The sync and async bindgen invocations produce **separate** Rust
//! types (`sync_bindings::orts::plugin::types::Vec3` vs
//! `async_bindings::orts::plugin::types::Vec3`), so we cannot share
//! conversion functions directly. Instead, this file declares the
//! conversion logic once as a `macro_rules!` and expands it into a
//! `sync` submodule (always) and an `r#async` submodule (when the
//! `plugin-wasm-async` feature is enabled). Each expansion operates
//! on its own `wit` type path while sharing the same implementation.

/// Expand `tick_input_to_wit` and `command_from_wit` for a given
/// `wit` types module path. Used to generate identical conversion
/// code against the sync and async bindgen outputs.
macro_rules! impl_convert {
    ($wit_mod:path) => {
        use $wit_mod as wit;

        use nalgebra::Vector3;

        use $crate::SpacecraftState;
        use $crate::attitude::AttitudeState;
        use $crate::orbital::OrbitalState;
        use $crate::plugin::message::{Message, NamedValue, NodeId, Outbound, Payload, Value};
        use $crate::plugin::tick_input::{ActuatorTelemetry, Sensors, SunSensorOutput, TickInput};
        use $crate::plugin::{Command, MtqCommand, PluginError, RwCommand, ThrusterCommand};

        // host -> guest (TickInput)

        /// Convert a host `TickInput<'_>` to the WIT `tick-input` record.
        pub fn tick_input_to_wit(obs: &TickInput<'_>) -> wit::TickInput {
            wit::TickInput {
                t: obs.t,
                spacecraft: spacecraft_to_wit(obs.spacecraft),
                epoch: obs.epoch.map(epoch_to_wit),
                sensors: sensor_readings_to_wit(obs.sensors),
                actuators: actuator_telemetry_to_wit(obs.actuators),
            }
        }

        fn actuator_telemetry_to_wit(a: &ActuatorTelemetry) -> wit::ActuatorTelemetry {
            wit::ActuatorTelemetry {
                rw: a.rw.as_ref().map(|rw| wit::RwTelemetry {
                    momentum: rw.momentum.clone(),
                    speeds: rw.speeds.clone(),
                    realized_torques: rw.realized_torques.clone(),
                }),
            }
        }

        fn spacecraft_to_wit(s: &SpacecraftState) -> wit::SpacecraftState {
            wit::SpacecraftState {
                orbit: orbital_to_wit(&s.orbit),
                attitude: attitude_to_wit(&s.attitude),
                mass: s.mass,
            }
        }

        fn orbital_to_wit(o: &OrbitalState) -> wit::OrbitalState {
            let pos = o.position_eci();
            let vel = o.velocity();
            wit::OrbitalState {
                position: wit::PositionEciKm {
                    x: pos.x(),
                    y: pos.y(),
                    z: pos.z(),
                },
                velocity: wit::VelocityEciKms {
                    x: vel.x,
                    y: vel.y,
                    z: vel.z,
                },
            }
        }

        fn attitude_to_wit(a: &AttitudeState) -> wit::AttitudeState {
            wit::AttitudeState {
                // Hamilton scalar-first: (w, x, y, z) matches both
                // nalgebra Vector4 order and WIT `quat` field order.
                orientation: wit::Quat {
                    w: a.quaternion[0],
                    x: a.quaternion[1],
                    y: a.quaternion[2],
                    z: a.quaternion[3],
                },
                angular_velocity: vec3_to_wit(&a.angular_velocity),
            }
        }

        fn epoch_to_wit(e: &arika::epoch::Epoch) -> wit::Epoch {
            wit::Epoch {
                julian_date: e.jd(),
            }
        }

        fn sensor_readings_to_wit(s: &Sensors) -> wit::Sensors {
            wit::Sensors {
                magnetometers: s
                    .magnetometers
                    .iter()
                    .map(|m| {
                        let v = m.into_inner().into_inner();
                        wit::MagneticFieldBody {
                            x: v.x,
                            y: v.y,
                            z: v.z,
                        }
                    })
                    .collect(),
                gyroscopes: s
                    .gyroscopes
                    .iter()
                    .map(|g| {
                        let v = g.into_inner().into_inner();
                        wit::AngularVelocityBody {
                            x: v.x,
                            y: v.y,
                            z: v.z,
                        }
                    })
                    .collect(),
                star_trackers: s
                    .star_trackers
                    .iter()
                    .map(|a| {
                        let q = a.into_inner();
                        wit::AttitudeBodyToInertial {
                            w: q[0],
                            x: q[1],
                            y: q[2],
                            z: q[3],
                        }
                    })
                    .collect(),
                sun_sensors: s
                    .sun_sensors
                    .iter()
                    .map(|o| match o {
                        SunSensorOutput::Fine {
                            direction,
                            illumination,
                        } => {
                            let wit_dir = direction.map(|d| {
                                let v = d.into_inner().into_inner();
                                wit::SunDirectionBody {
                                    x: v.x,
                                    y: v.y,
                                    z: v.z,
                                }
                            });
                            wit::SunSensorOutput::Fine(wit::SunFineOutput {
                                direction: wit_dir,
                                illumination: *illumination,
                            })
                        }
                        SunSensorOutput::Coarse(val) => wit::SunSensorOutput::Coarse(*val),
                    })
                    .collect(),
            }
        }

        fn vec3_to_wit(v: &Vector3<f64>) -> wit::Vec3 {
            wit::Vec3 {
                x: v.x,
                y: v.y,
                z: v.z,
            }
        }

        // guest -> host (Command)

        /// Convert a WIT `command` record to the plugin-layer `Command` struct.
        ///
        /// Returns `PluginError::BadCommand` if any numeric field is NaN / Inf.
        pub fn command_from_wit(cmd: wit::Command) -> Result<Command, PluginError> {
            let mtq = cmd.mtq.map(|mtq_cmd| match mtq_cmd {
                wit::MtqCommand::Moments(m) => MtqCommand::Moments(m),
                wit::MtqCommand::NormalizedMoments(n) => MtqCommand::NormalizedMoments(n),
            });
            let rw = cmd.rw.map(|rw_cmd| match rw_cmd {
                wit::RwCommand::Speeds(s) => RwCommand::Speeds(s),
                wit::RwCommand::Torques(t) => RwCommand::Torques(t),
            });
            let thruster = cmd.thruster.map(|thr_cmd| match thr_cmd {
                wit::ThrusterCommand::Throttles(t) => ThrusterCommand::Throttles(t),
            });
            let result = Command { mtq, rw, thruster };
            if !result.is_finite() {
                return Err(PluginError::BadCommand(format!("{result:?}")));
            }
            Ok(result)
        }

        // ───────────────────── messaging (msg-io) ─────────────────────
        //
        // transport 層は payload を解釈しないので、Command のような
        // NaN ガードは行わない（メッセージは ODE に入らない opaque データ）。

        fn node_id_to_wit(n: NodeId) -> wit::NodeId {
            match n {
                NodeId::Ground => wit::NodeId::Ground,
                NodeId::Satellite(id) => wit::NodeId::Satellite(id),
            }
        }

        fn node_id_from_wit(n: wit::NodeId) -> NodeId {
            match n {
                wit::NodeId::Ground => NodeId::Ground,
                wit::NodeId::Satellite(id) => NodeId::Satellite(id),
            }
        }

        fn value_to_wit(v: Value) -> wit::Value {
            match v {
                Value::Boolean(b) => wit::Value::Boolean(b),
                Value::Integer(i) => wit::Value::Integer(i),
                Value::Number(n) => wit::Value::Number(n),
                Value::Text(s) => wit::Value::Text(s),
                Value::Bytes(b) => wit::Value::Bytes(b),
            }
        }

        fn value_from_wit(v: wit::Value) -> Value {
            match v {
                wit::Value::Boolean(b) => Value::Boolean(b),
                wit::Value::Integer(i) => Value::Integer(i),
                wit::Value::Number(n) => Value::Number(n),
                wit::Value::Text(s) => Value::Text(s),
                wit::Value::Bytes(b) => Value::Bytes(b),
            }
        }

        fn payload_to_wit(p: Payload) -> wit::Payload {
            match p {
                Payload::KeyValue(kvs) => wit::Payload::KeyValue(
                    kvs.into_iter()
                        .map(|kv| wit::NamedValue {
                            name: kv.name,
                            value: value_to_wit(kv.value),
                        })
                        .collect(),
                ),
                Payload::Binary(b) => wit::Payload::Binary(b),
                Payload::Json(s) => wit::Payload::Json(s),
            }
        }

        fn payload_from_wit(p: wit::Payload) -> Payload {
            match p {
                wit::Payload::KeyValue(kvs) => Payload::KeyValue(
                    kvs.into_iter()
                        .map(|kv| NamedValue {
                            name: kv.name,
                            value: value_from_wit(kv.value),
                        })
                        .collect(),
                ),
                wit::Payload::Binary(b) => Payload::Binary(b),
                wit::Payload::Json(s) => Payload::Json(s),
            }
        }

        /// Convert a host [`Message`] to the WIT `message` record
        /// (host → guest inbox, delivered via `recv-batch`).
        pub fn message_to_wit(m: Message) -> wit::Message {
            wit::Message {
                src: node_id_to_wit(m.src),
                dst: node_id_to_wit(m.dst),
                kind: m.kind,
                payload: payload_to_wit(m.payload),
            }
        }

        /// Convert a WIT `outbound` record to the host [`Outbound`]
        /// (guest → host, captured from `send-message`).
        pub fn outbound_from_wit(o: wit::Outbound) -> Outbound {
            Outbound {
                dst: node_id_from_wit(o.dst),
                kind: o.kind,
                payload: payload_from_wit(o.payload),
            }
        }
    };
}

/// Conversions targeting the sync bindgen output in
/// [`super::sync_bindings`]. Used by [`super::sync_controller`].
pub mod sync {
    impl_convert!(super::super::sync_bindings::orts::plugin::types);

    #[cfg(test)]
    mod tests {
        use super::*;
        use nalgebra::{Vector3, Vector4};

        fn make_spacecraft() -> SpacecraftState {
            SpacecraftState {
                orbit: OrbitalState::new(
                    Vector3::new(7000.0, 0.0, 0.0),
                    Vector3::new(0.0, 7.5, 0.0),
                ),
                attitude: AttitudeState {
                    quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
                    angular_velocity: Vector3::new(0.1, 0.05, -0.03),
                },
                mass: 50.0,
            }
        }

        #[test]
        fn observation_roundtrip_preserves_values() {
            let spacecraft = make_spacecraft();
            let epoch = arika::epoch::Epoch::j2000();
            use crate::plugin::tick_input::{
                AngularVelocityBody, AttitudeBodyToInertial, MagneticFieldBody,
            };
            use arika::frame::{Body, Vec3};
            let sensors = Sensors {
                magnetometers: vec![MagneticFieldBody::new(Vec3::<Body>::new(1e-5, 2e-5, -3e-5))],
                gyroscopes: vec![AngularVelocityBody::new(Vec3::<Body>::new(
                    0.1, 0.05, -0.03,
                ))],
                star_trackers: vec![AttitudeBodyToInertial::new(Vector4::new(
                    1.0, 0.0, 0.0, 0.0,
                ))],
                sun_sensors: vec![],
            };
            let actuators = ActuatorTelemetry::default();
            let obs = TickInput {
                t: 42.0,
                epoch: Some(&epoch),
                sensors: &sensors,
                actuators: &actuators,
                spacecraft: &spacecraft,
            };
            let wit_obs = tick_input_to_wit(&obs);
            assert_eq!(wit_obs.t, 42.0);
            assert_eq!(wit_obs.spacecraft.mass, 50.0);
            assert_eq!(wit_obs.spacecraft.orbit.position.x, 7000.0);
            assert_eq!(wit_obs.spacecraft.attitude.orientation.w, 1.0);
            assert_eq!(wit_obs.spacecraft.attitude.angular_velocity.x, 0.1);
            let wit_epoch = wit_obs.epoch.expect("epoch must be Some");
            assert_eq!(wit_epoch.julian_date, epoch.jd());
            // Sensor fields (now lists).
            assert_eq!(wit_obs.sensors.magnetometers.len(), 1);
            assert_eq!(wit_obs.sensors.magnetometers[0].x, 1e-5);
            assert_eq!(wit_obs.sensors.gyroscopes.len(), 1);
            assert_eq!(wit_obs.sensors.gyroscopes[0].x, 0.1);
            assert_eq!(wit_obs.sensors.star_trackers.len(), 1);
            assert_eq!(wit_obs.sensors.star_trackers[0].w, 1.0);
        }

        /// Characterization: the v0 WIT attitude payload carries the four
        /// quaternion components in `(w, x, y, z)` order. Pinned with four
        /// distinct non-zero components, which the identity quaternion of
        /// `observation_roundtrip_preserves_values` cannot discriminate.
        #[test]
        fn star_tracker_wit_payload_component_order() {
            use crate::plugin::tick_input::AttitudeBodyToInertial;
            let spacecraft = make_spacecraft();
            let q = nalgebra::UnitQuaternion::from_axis_angle(
                &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                0.7,
            );
            let sensors = Sensors {
                star_trackers: vec![AttitudeBodyToInertial::new(Vector4::new(
                    q.w, q.i, q.j, q.k,
                ))],
                ..Sensors::empty()
            };
            let actuators = ActuatorTelemetry::default();
            let obs = TickInput {
                t: 0.0,
                epoch: None,
                sensors: &sensors,
                actuators: &actuators,
                spacecraft: &spacecraft,
            };
            let wit_obs = tick_input_to_wit(&obs);
            let got = &wit_obs.sensors.star_trackers[0];
            // The boundary is a pure copy, so the order is pinned exactly.
            assert_eq!([got.w, got.x, got.y, got.z], [q.w, q.i, q.j, q.k]);
            // Pinned literals so a re-ordering of `q`'s own components would
            // still be caught (relative: the fixture goes through libm trig).
            let expected = Vector4::new(
                0.9393727128473789,
                0.10391372781674944,
                -0.17318954636124906,
                0.27710327417799857,
            );
            let actual = Vector4::new(got.w, got.x, got.y, got.z);
            assert!(
                (actual - expected).magnitude() <= 1e-12 * expected.magnitude(),
                "v0 WIT attitude payload changed: {actual:?}"
            );
        }

        #[test]
        fn observation_empty_sensors() {
            let spacecraft = make_spacecraft();
            let sensors = Sensors::empty();
            let actuators = ActuatorTelemetry::default();
            let obs = TickInput {
                t: 0.0,
                spacecraft: &spacecraft,
                epoch: None,
                sensors: &sensors,
                actuators: &actuators,
            };
            let wit_obs = tick_input_to_wit(&obs);
            assert!(wit_obs.sensors.magnetometers.is_empty());
            assert!(wit_obs.sensors.gyroscopes.is_empty());
            assert!(wit_obs.sensors.star_trackers.is_empty());
        }

        #[test]
        fn command_roundtrip_mtq_moments() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::Moments(vec![1.0, -2.0, 0.5])),
                rw: None,
                thruster: None,
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, Some(MtqCommand::Moments(vec![1.0, -2.0, 0.5])));
            assert_eq!(cmd.rw, None);
            assert_eq!(cmd.thruster, None);
        }

        #[test]
        fn command_roundtrip_mtq_normalized_moments() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::NormalizedMoments(vec![0.5, -1.0, 0.25])),
                rw: None,
                thruster: None,
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(
                cmd.mtq,
                Some(MtqCommand::NormalizedMoments(vec![0.5, -1.0, 0.25]))
            );
            assert_eq!(cmd.rw, None);
        }

        #[test]
        fn command_roundtrip_rw_torques() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: Some(wit::RwCommand::Torques(vec![0.01, -0.02, 0.03])),
                thruster: None,
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, None);
            assert_eq!(cmd.rw, Some(RwCommand::Torques(vec![0.01, -0.02, 0.03])));
        }

        #[test]
        fn command_roundtrip_rw_speeds() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: Some(wit::RwCommand::Speeds(vec![10.0, -5.0, 0.0])),
                thruster: None,
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, None);
            assert_eq!(cmd.rw, Some(RwCommand::Speeds(vec![10.0, -5.0, 0.0])));
        }

        #[test]
        fn command_roundtrip_thruster_throttles() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: None,
                thruster: Some(wit::ThrusterCommand::Throttles(vec![0.5, 1.0, 0.0])),
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, None);
            assert_eq!(cmd.rw, None);
            assert_eq!(
                cmd.thruster,
                Some(ThrusterCommand::Throttles(vec![0.5, 1.0, 0.0]))
            );
        }

        #[test]
        fn command_from_wit_rejects_nan() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::Moments(vec![1.0, f64::NAN, 0.0])),
                rw: None,
                thruster: None,
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn command_from_wit_rejects_nan_normalized() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::NormalizedMoments(vec![0.5, f64::NAN, 0.0])),
                rw: None,
                thruster: None,
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn command_from_wit_rejects_nan_rw() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: Some(wit::RwCommand::Torques(vec![f64::INFINITY, 0.0, 0.0])),
                thruster: None,
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn command_from_wit_rejects_nan_thruster() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: None,
                thruster: Some(wit::ThrusterCommand::Throttles(vec![f64::NAN, 0.5])),
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn message_to_wit_preserves_fields() {
            use crate::plugin::message::{Message, NodeId, Payload, Value};
            let m = Message {
                src: NodeId::Ground,
                dst: NodeId::Satellite(7),
                kind: "orts.cmd.set-mode.v1".into(),
                payload: Payload::key_value([("mode", Value::Text("nadir".into()))]),
            };
            let w = message_to_wit(m);
            assert!(matches!(w.src, wit::NodeId::Ground));
            assert!(matches!(w.dst, wit::NodeId::Satellite(7)));
            assert_eq!(w.kind, "orts.cmd.set-mode.v1");
            match &w.payload {
                wit::Payload::KeyValue(kvs) => {
                    assert_eq!(kvs[0].name, "mode");
                    assert!(matches!(&kvs[0].value, wit::Value::Text(s) if s == "nadir"));
                }
                _ => panic!("expected KeyValue"),
            }
        }

        #[test]
        fn outbound_from_wit_preserves_fields() {
            use crate::plugin::message::{NodeId, Value};
            let w = wit::Outbound {
                dst: wit::NodeId::Ground,
                kind: "orts.tlm.mode.v1".to_string(),
                payload: wit::Payload::KeyValue(vec![wit::NamedValue {
                    name: "mode".to_string(),
                    value: wit::Value::Text("detumble".to_string()),
                }]),
            };
            let o = outbound_from_wit(w);
            assert_eq!(o.dst, NodeId::Ground);
            assert_eq!(o.kind, "orts.tlm.mode.v1");
            assert_eq!(
                o.payload.get("mode").and_then(Value::as_text),
                Some("detumble")
            );
        }

        #[test]
        fn value_roundtrip_through_wit() {
            use crate::plugin::message::Value;
            for v in [
                Value::Boolean(true),
                Value::Integer(-9),
                Value::Number(2.5),
                Value::Text("x".into()),
                Value::Bytes(vec![1, 2, 3]),
            ] {
                let back = value_from_wit(value_to_wit(v.clone()));
                assert_eq!(back, v);
            }
        }

        #[test]
        fn quat_ordering_is_scalar_first() {
            let att = AttitudeState {
                quaternion: Vector4::new(0.9, 0.1, 0.2, 0.3), // w=0.9, x=0.1, y=0.2, z=0.3
                angular_velocity: Vector3::zeros(),
            };
            let wit_att = attitude_to_wit(&att);
            assert_eq!(wit_att.orientation.w, 0.9);
            assert_eq!(wit_att.orientation.x, 0.1);
            assert_eq!(wit_att.orientation.y, 0.2);
            assert_eq!(wit_att.orientation.z, 0.3);
        }
    }
}

/// Conversions targeting the async bindgen output in
/// [`super::async_bindings`]. Used by the fiber-based async backend.
#[cfg(feature = "plugin-wasm-async")]
pub mod r#async {
    impl_convert!(super::super::async_bindings::orts::plugin::types);
}
