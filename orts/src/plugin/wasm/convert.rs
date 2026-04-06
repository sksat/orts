//! Conversion helpers between the plugin-layer Rust types
//! (`orts::plugin::*`) and the WIT-generated types
//! (`orts::plugin::wasm::bindings::orts::plugin::types::*`).
//!
//! These functions are the only place where `nalgebra` types touch
//! WIT-generated types. Keeping the conversion in one module avoids
//! scattering `Vector3::new(v.x, v.y, v.z)` across the codebase and
//! makes it easy to audit precision / ordering when the WIT record
//! layout changes.

use nalgebra::Vector3;

use super::bindings::orts::plugin::types as wit;

use crate::SpacecraftState;
use crate::attitude::AttitudeState;
use crate::orbital::OrbitalState;
use crate::plugin::observation::{EnvSnapshot, Observation};
use crate::plugin::{Command, PluginError};

// ───────────────────── host → guest (Observation) ─────────────────────

/// Convert a host `Observation<'_>` to the WIT `observation` record.
pub fn observation_to_wit(obs: &Observation<'_>) -> wit::Observation {
    wit::Observation {
        t: obs.t,
        spacecraft: spacecraft_to_wit(obs.spacecraft),
        epoch: obs.epoch.map(epoch_to_wit),
        env: env_snapshot_to_wit(obs.env),
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
    // `position_eci()` / `velocity_eci()` return `Eci(Vector3<f64>)`
    // newtypes; unwrap via `.0`.
    wit::OrbitalState {
        position: vec3_to_wit(&o.position_eci().0),
        velocity: vec3_to_wit(o.velocity()),
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

fn env_snapshot_to_wit(_env: &EnvSnapshot) -> wit::EnvSnapshot {
    // Phase P1-b1: empty record → empty struct.
    wit::EnvSnapshot {}
}

fn vec3_to_wit(v: &Vector3<f64>) -> wit::Vec3 {
    wit::Vec3 {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

// ───────────────────── guest → host (Command) ─────────────────────

/// Convert a WIT `command` variant to the plugin-layer `Command` enum.
///
/// Returns `PluginError::BadCommand` if any numeric field is NaN / Inf.
pub fn command_from_wit(cmd: wit::Command) -> Result<Command, PluginError> {
    let result = match cmd {
        wit::Command::MagneticMoment(v) => Command::MagneticMoment(Vector3::new(v.x, v.y, v.z)),
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
        let env = EnvSnapshot::empty();
        let obs = Observation {
            t: 42.0,
            spacecraft: &spacecraft,
            epoch: Some(&epoch),
            env: &env,
        };
        let wit_obs = observation_to_wit(&obs);
        assert_eq!(wit_obs.t, 42.0);
        assert_eq!(wit_obs.spacecraft.mass, 50.0);
        assert_eq!(wit_obs.spacecraft.orbit.position.x, 7000.0);
        assert_eq!(wit_obs.spacecraft.attitude.orientation.w, 1.0);
        assert_eq!(wit_obs.spacecraft.attitude.angular_velocity.x, 0.1);
        let wit_epoch = wit_obs.epoch.expect("epoch must be Some");
        assert_eq!(wit_epoch.julian_date, epoch.jd());
    }

    #[test]
    fn command_roundtrip_magnetic_moment() {
        let wit_cmd = wit::Command::MagneticMoment(wit::Vec3 {
            x: 1.0,
            y: -2.0,
            z: 0.5,
        });
        let cmd = command_from_wit(wit_cmd).unwrap();
        assert_eq!(cmd.as_magnetic_moment(), Some(Vector3::new(1.0, -2.0, 0.5)));
    }

    #[test]
    fn command_from_wit_rejects_nan() {
        let wit_cmd = wit::Command::MagneticMoment(wit::Vec3 {
            x: 1.0,
            y: f64::NAN,
            z: 0.0,
        });
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
