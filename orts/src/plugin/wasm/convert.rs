//! Conversion helpers between the plugin-layer Rust types
//! (`orts::plugin::*`) and the WIT-generated types
//! (`orts::plugin::wasm::bindings::orts::plugin::types::*`).
//!
//! These functions are the only place where `nalgebra` types touch
//! WIT-generated types. Keeping the conversion in one module avoids
//! scattering `Vector3::new(v.x, v.y, v.z)` across the codebase and
//! makes it easy to audit precision / ordering when the WIT record
//! layout changes.

use kaname::frame::{Body, Vec3};
use nalgebra::Vector3;

use super::bindings::orts::plugin::types as wit;

use crate::SpacecraftState;
use crate::attitude::AttitudeState;
use crate::orbital::OrbitalState;
use crate::plugin::tick_input::{Sensors, TickInput};
use crate::plugin::{Command, PluginError};

// ───────────────────── host -> guest (TickInput) ─────────────────────

/// Convert a host `TickInput<'_>` to the WIT `tick-input` record.
pub fn tick_input_to_wit(obs: &TickInput<'_>) -> wit::TickInput {
    wit::TickInput {
        t: obs.t,
        spacecraft: spacecraft_to_wit(obs.spacecraft),
        epoch: obs.epoch.map(epoch_to_wit),
        sensors: sensor_readings_to_wit(obs.sensors),
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

fn epoch_to_wit(e: &kaname::epoch::Epoch) -> wit::Epoch {
    wit::Epoch {
        julian_date: e.jd(),
    }
}

fn sensor_readings_to_wit(s: &Sensors) -> wit::Sensors {
    wit::Sensors {
        magnetometer: s.magnetometer.map(|m| {
            let v = m.into_inner().into_inner();
            wit::MagneticFieldBody {
                x: v.x,
                y: v.y,
                z: v.z,
            }
        }),
        gyroscope: s.gyroscope.map(|g| {
            let v = g.into_inner().into_inner();
            wit::AngularVelocityBody {
                x: v.x,
                y: v.y,
                z: v.z,
            }
        }),
        star_tracker: s.star_tracker.map(|a| {
            let q = a.into_inner();
            wit::AttitudeBodyToInertial {
                w: q[0],
                x: q[1],
                y: q[2],
                z: q[3],
            }
        }),
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
    let result = Command {
        magnetic_moment: cmd
            .magnetic_moment
            .map(|v| Vec3::<Body>::new(v.x, v.y, v.z)),
        rw_torque: cmd.rw_torque.map(|v| Vec3::<Body>::new(v.x, v.y, v.z)),
    };
    if !result.is_finite() {
        return Err(PluginError::BadCommand(format!("{result:?}")));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector4;

    fn make_spacecraft() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
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
        let epoch = kaname::epoch::Epoch::j2000();
        use crate::plugin::tick_input::{
            AngularVelocityBody, AttitudeBodyToInertial, MagneticFieldBody,
        };
        let sensors = Sensors {
            magnetometer: Some(MagneticFieldBody::new(Vec3::new(1e-5, 2e-5, -3e-5))),
            gyroscope: Some(AngularVelocityBody::new(Vec3::new(0.1, 0.05, -0.03))),
            star_tracker: Some(AttitudeBodyToInertial::new(Vector4::new(
                1.0, 0.0, 0.0, 0.0,
            ))),
        };
        let obs = TickInput {
            t: 42.0,
            epoch: Some(&epoch),
            sensors: &sensors,
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
        // Sensor fields.
        let b = wit_obs.sensors.magnetometer.expect("B must be Some");
        assert_eq!(b.x, 1e-5);
        let omega = wit_obs.sensors.gyroscope.expect("omega must be Some");
        assert_eq!(omega.x, 0.1);
        let att = wit_obs.sensors.star_tracker.expect("att must be Some");
        assert_eq!(att.w, 1.0);
    }

    #[test]
    fn observation_empty_sensors() {
        let spacecraft = make_spacecraft();
        let sensors = Sensors::empty();
        let obs = TickInput {
            t: 0.0,
            spacecraft: &spacecraft,
            epoch: None,
            sensors: &sensors,
        };
        let wit_obs = tick_input_to_wit(&obs);
        assert!(wit_obs.sensors.magnetometer.is_none());
        assert!(wit_obs.sensors.gyroscope.is_none());
        assert!(wit_obs.sensors.star_tracker.is_none());
    }

    #[test]
    fn command_roundtrip_magnetic_moment() {
        let wit_cmd = wit::Command {
            magnetic_moment: Some(wit::CommandedMagneticMoment {
                x: 1.0,
                y: -2.0,
                z: 0.5,
            }),
            rw_torque: None,
        };
        let cmd = command_from_wit(wit_cmd).unwrap();
        assert_eq!(cmd.magnetic_moment, Some(Vec3::<Body>::new(1.0, -2.0, 0.5)));
        assert_eq!(cmd.rw_torque, None);
    }

    #[test]
    fn command_roundtrip_rw_torque() {
        let wit_cmd = wit::Command {
            magnetic_moment: None,
            rw_torque: Some(wit::CommandedRwTorque {
                x: 0.01,
                y: -0.02,
                z: 0.03,
            }),
        };
        let cmd = command_from_wit(wit_cmd).unwrap();
        assert_eq!(cmd.magnetic_moment, None);
        assert_eq!(cmd.rw_torque, Some(Vec3::<Body>::new(0.01, -0.02, 0.03)));
    }

    #[test]
    fn command_from_wit_rejects_nan() {
        let wit_cmd = wit::Command {
            magnetic_moment: Some(wit::CommandedMagneticMoment {
                x: 1.0,
                y: f64::NAN,
                z: 0.0,
            }),
            rw_torque: None,
        };
        assert!(command_from_wit(wit_cmd).is_err());
    }

    #[test]
    fn command_from_wit_rejects_nan_rw() {
        let wit_cmd = wit::Command {
            magnetic_moment: None,
            rw_torque: Some(wit::CommandedRwTorque {
                x: f64::INFINITY,
                y: 0.0,
                z: 0.0,
            }),
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
