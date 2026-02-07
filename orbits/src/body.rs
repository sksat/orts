use serde::Serialize;

use crate::constants;

/// Known celestial bodies with pre-defined physical properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownBody {
    Sun,
    Mercury,
    Venus,
    Earth,
    Moon,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

/// Physical properties of a celestial body.
pub struct BodyProperties {
    /// Standard gravitational parameter (km^3/s^2)
    pub mu: f64,
    /// Mean equatorial radius (km)
    pub radius: f64,
    /// Display name
    pub name: &'static str,
}

impl KnownBody {
    /// Return the physical properties for a known body.
    pub fn properties(&self) -> BodyProperties {
        match self {
            KnownBody::Sun => BodyProperties {
                mu: constants::MU_SUN,
                radius: 695700.0,
                name: "Sun",
            },
            KnownBody::Mercury => BodyProperties {
                mu: 22031.868551,
                radius: 2439.7,
                name: "Mercury",
            },
            KnownBody::Venus => BodyProperties {
                mu: 324858.592000,
                radius: 6051.8,
                name: "Venus",
            },
            KnownBody::Earth => BodyProperties {
                mu: constants::MU_EARTH,
                radius: constants::R_EARTH,
                name: "Earth",
            },
            KnownBody::Moon => BodyProperties {
                mu: 4902.800066,
                radius: 1737.4,
                name: "Moon",
            },
            KnownBody::Mars => BodyProperties {
                mu: 42828.375214,
                radius: 3396.2,
                name: "Mars",
            },
            KnownBody::Jupiter => BodyProperties {
                mu: 126686534.921800,
                radius: 71492.0,
                name: "Jupiter",
            },
            KnownBody::Saturn => BodyProperties {
                mu: 37931206.159000,
                radius: 60268.0,
                name: "Saturn",
            },
            KnownBody::Uranus => BodyProperties {
                mu: 5793951.256000,
                radius: 25559.0,
                name: "Uranus",
            },
            KnownBody::Neptune => BodyProperties {
                mu: 6835099.975400,
                radius: 24764.0,
                name: "Neptune",
            },
        }
    }
}

/// Category of a simulation object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectCategory {
    CelestialBody,
    Satellite,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_BODIES: [KnownBody; 10] = [
        KnownBody::Sun,
        KnownBody::Mercury,
        KnownBody::Venus,
        KnownBody::Earth,
        KnownBody::Moon,
        KnownBody::Mars,
        KnownBody::Jupiter,
        KnownBody::Saturn,
        KnownBody::Uranus,
        KnownBody::Neptune,
    ];

    #[test]
    fn all_bodies_have_positive_mu() {
        for body in &ALL_BODIES {
            let props = body.properties();
            assert!(props.mu > 0.0, "{:?} has non-positive mu: {}", body, props.mu);
        }
    }

    #[test]
    fn all_bodies_have_positive_radius() {
        for body in &ALL_BODIES {
            let props = body.properties();
            assert!(
                props.radius > 0.0,
                "{:?} has non-positive radius: {}",
                body,
                props.radius
            );
        }
    }

    #[test]
    fn earth_mu_matches_constant() {
        assert_eq!(KnownBody::Earth.properties().mu, constants::MU_EARTH);
    }

    #[test]
    fn earth_radius_matches_constant() {
        assert_eq!(KnownBody::Earth.properties().radius, constants::R_EARTH);
    }

    #[test]
    fn sun_mu_matches_constant() {
        assert_eq!(KnownBody::Sun.properties().mu, constants::MU_SUN);
    }

    #[test]
    fn serde_serialization_earth() {
        let json = serde_json::to_string(&KnownBody::Earth).unwrap();
        assert_eq!(json, "\"earth\"");
    }

    #[test]
    fn serde_serialization_sun() {
        let json = serde_json::to_string(&KnownBody::Sun).unwrap();
        assert_eq!(json, "\"sun\"");
    }

    #[test]
    fn serde_serialization_object_category() {
        let json = serde_json::to_string(&ObjectCategory::CelestialBody).unwrap();
        assert_eq!(json, "\"celestial_body\"");
        let json = serde_json::to_string(&ObjectCategory::Satellite).unwrap();
        assert_eq!(json, "\"satellite\"");
    }
}
