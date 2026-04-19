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
        use $crate::plugin::tick_input::{ActuatorTelemetry, Sensors, SunSensorOutput, TickInput};
        use $crate::plugin::{Command, MtqCommand, PluginError, RwCommand};

        // ───────────────────── host -> guest (TickInput) ─────────────────────

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

        // ───────────────────── guest -> host (Command) ─────────────────────

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
            let result = Command { mtq, rw };
            if !result.is_finite() {
                return Err(PluginError::BadCommand(format!("{result:?}")));
            }
            Ok(result)
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
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, Some(MtqCommand::Moments(vec![1.0, -2.0, 0.5])));
            assert_eq!(cmd.rw, None);
        }

        #[test]
        fn command_roundtrip_mtq_normalized_moments() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::NormalizedMoments(vec![0.5, -1.0, 0.25])),
                rw: None,
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
            };
            let cmd = command_from_wit(wit_cmd).unwrap();
            assert_eq!(cmd.mtq, None);
            assert_eq!(cmd.rw, Some(RwCommand::Speeds(vec![10.0, -5.0, 0.0])));
        }

        #[test]
        fn command_from_wit_rejects_nan() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::Moments(vec![1.0, f64::NAN, 0.0])),
                rw: None,
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn command_from_wit_rejects_nan_normalized() {
            let wit_cmd = wit::Command {
                mtq: Some(wit::MtqCommand::NormalizedMoments(vec![0.5, f64::NAN, 0.0])),
                rw: None,
            };
            assert!(command_from_wit(wit_cmd).is_err());
        }

        #[test]
        fn command_from_wit_rejects_nan_rw() {
            let wit_cmd = wit::Command {
                mtq: None,
                rw: Some(wit::RwCommand::Torques(vec![f64::INFINITY, 0.0, 0.0])),
            };
            assert!(command_from_wit(wit_cmd).is_err());
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
