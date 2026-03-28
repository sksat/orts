use nalgebra::Vector3;

use crate::record::archetype::Archetype;
use crate::record::component::{Component, ComponentName};
use crate::record::components::*;

/// The primary state of an orbiting body: position + velocity.
#[derive(Debug, Clone)]
pub struct OrbitalState {
    pub position: Position3D,
    pub velocity: Velocity3D,
}

impl OrbitalState {
    pub fn new(position: Vector3<f64>, velocity: Vector3<f64>) -> Self {
        OrbitalState {
            position: Position3D(position),
            velocity: Velocity3D(velocity),
        }
    }
}

impl Archetype for OrbitalState {
    fn archetype_name() -> &'static str {
        "OrbitalState"
    }
    fn required_components() -> Vec<ComponentName> {
        vec![Position3D::component_name(), Velocity3D::component_name()]
    }
    fn optional_components() -> Vec<ComponentName> {
        vec![
            KeplerianState::component_name(),
            Quaternion4D::component_name(),
            AngularVelocity3D::component_name(),
        ]
    }
}

/// Static properties of a celestial body.
#[derive(Debug, Clone)]
pub struct CelestialBody {
    pub mu: GravitationalParameter,
    pub radius: BodyRadius,
    pub name: String,
}

impl Archetype for CelestialBody {
    fn archetype_name() -> &'static str {
        "CelestialBody"
    }
    fn required_components() -> Vec<ComponentName> {
        vec![
            GravitationalParameter::component_name(),
            BodyRadius::component_name(),
        ]
    }
    fn optional_components() -> Vec<ComponentName> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbital_state_from_vectors() {
        let pos = Vector3::new(6778.137, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.669, 0.0);
        let os = OrbitalState::new(pos, vel);
        assert_eq!(os.position.0, pos);
        assert_eq!(os.velocity.0, vel);
    }

    #[test]
    fn orbital_state_archetype_components() {
        let required = OrbitalState::required_components();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&Position3D::component_name()));
        assert!(required.contains(&Velocity3D::component_name()));

        let optional = OrbitalState::optional_components();
        assert!(optional.contains(&KeplerianState::component_name()));
    }

    #[test]
    fn celestial_body_archetype() {
        let earth = CelestialBody {
            mu: GravitationalParameter(398600.4418),
            radius: BodyRadius(6378.137),
            name: "Earth".to_string(),
        };
        assert_eq!(CelestialBody::archetype_name(), "CelestialBody");
        assert_eq!(earth.mu.0, 398600.4418);
        assert_eq!(earth.radius.0, 6378.137);
    }
}
