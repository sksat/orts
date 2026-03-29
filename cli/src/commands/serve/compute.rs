use std::collections::HashMap;

use orts::OrbitalState;
use orts::orbital::kepler::KeplerianElements;

use crate::commands::serve::protocol::WsMessage;
use crate::sim::core::AttitudePayload;

/// Pre-computed derived values for chart display.
pub struct DerivedValues {
    pub altitude: f64,
    pub specific_energy: f64,
    pub angular_momentum: f64,
    pub velocity_mag: f64,
}

/// Compute chart-display derived values from position/velocity.
pub fn compute_derived(
    pos: &nalgebra::Vector3<f64>,
    vel: &nalgebra::Vector3<f64>,
    mu: f64,
    body_radius: f64,
) -> DerivedValues {
    let r_mag = pos.magnitude();
    let v_mag = vel.magnitude();
    let h = pos.cross(vel);
    DerivedValues {
        altitude: r_mag - body_radius,
        specific_energy: v_mag * v_mag / 2.0 - mu / r_mag,
        angular_momentum: h.magnitude(),
        velocity_mag: v_mag,
    }
}

pub fn state_message(
    satellite_id: &str,
    t: f64,
    state: &OrbitalState,
    mu: f64,
    body_radius: f64,
    accelerations: HashMap<String, f64>,
    attitude: Option<AttitudePayload>,
) -> String {
    let elements = KeplerianElements::from_state_vector(state.position(), state.velocity(), mu);
    let derived = compute_derived(state.position(), state.velocity(), mu, body_radius);
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
        altitude: derived.altitude,
        specific_energy: derived.specific_energy,
        angular_momentum: derived.angular_momentum,
        velocity_mag: derived.velocity_mag,
        accelerations,
        attitude,
    };
    serde_json::to_string(&msg).expect("failed to serialize state message")
}
