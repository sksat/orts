use std::collections::HashMap;

use orts::OrbitalState;
use orts::orbital::kepler::KeplerianElements;

use crate::commands::serve::protocol::WsMessage;

pub fn state_message(
    satellite_id: &str,
    t: f64,
    state: &OrbitalState,
    mu: f64,
    accelerations: HashMap<String, f64>,
) -> String {
    let elements = KeplerianElements::from_state_vector(state.position(), state.velocity(), mu);
    let msg = WsMessage::State {
        satellite_id: satellite_id.to_string(),
        t,
        position: [state.position().x, state.position().y, state.position().z],
        velocity: [state.velocity().x, state.velocity().y, state.velocity().z],
        semi_major_axis: elements.semi_major_axis,
        eccentricity: elements.eccentricity,
        inclination: elements.inclination,
        raan: elements.raan,
        argument_of_periapsis: elements.argument_of_periapsis,
        true_anomaly: elements.true_anomaly,
        accelerations,
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}
