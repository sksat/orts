//! `TickInput` is the plugin-facing snapshot, and the v0 plugin contract is
//! defined in simple-ECI. A sensor bundle evaluated in another inertial frame
//! carries that frame in its type, so it cannot be substituted here.

use arika::frame::Gcrs;
use nalgebra::{Vector3, Vector4};
use orts::attitude::AttitudeState;
use orts::orbital::OrbitalState;
use orts::plugin::tick_input::{ActuatorTelemetry, Sensors, TickInput};
use orts::SpacecraftState;

fn main() {
    let spacecraft = SpacecraftState {
        orbit: OrbitalState::new(Vector3::new(7000.0, 0.0, 0.0), Vector3::new(0.0, 7.5, 0.0)),
        attitude: AttitudeState {
            quaternion: Vector4::new(1.0, 0.0, 0.0, 0.0),
            angular_velocity: Vector3::zeros(),
        },
        mass: 50.0,
    };
    let actuators = ActuatorTelemetry::default();
    let sensors = Sensors::<Gcrs>::empty();
    // This must fail: `TickInput::sensors` is `&Sensors<SimpleEci>`.
    let _input = TickInput {
        t: 0.0,
        epoch: None,
        sensors: &sensors,
        actuators: &actuators,
        spacecraft: &spacecraft,
    };
}
