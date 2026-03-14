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
    /// J2 zonal harmonic coefficient (None for bodies without known oblateness)
    pub j2: Option<f64>,
    /// J3 zonal harmonic coefficient (None if unknown)
    pub j3: Option<f64>,
    /// J4 zonal harmonic coefficient (None if unknown)
    pub j4: Option<f64>,
    /// Effective atmosphere altitude for reentry detection [km].
    /// When a satellite descends below this altitude, it is considered
    /// to have entered the atmosphere and the simulation terminates.
    /// None for bodies without a significant atmosphere.
    pub atmosphere_altitude: Option<f64>,
}

impl KnownBody {
    /// Return the physical properties for a known body.
    pub fn properties(&self) -> BodyProperties {
        match self {
            KnownBody::Sun => BodyProperties {
                mu: constants::MU_SUN,
                radius: 695700.0,
                name: "Sun",
                j2: None,
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Mercury => BodyProperties {
                mu: 22031.868551,
                radius: 2439.7,
                name: "Mercury",
                j2: None,
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Venus => BodyProperties {
                mu: 324858.592000,
                radius: 6051.8,
                name: "Venus",
                j2: None,
                j3: None,
                j4: None,
                atmosphere_altitude: Some(250.0),
            },
            KnownBody::Earth => BodyProperties {
                mu: constants::MU_EARTH,
                radius: constants::R_EARTH,
                name: "Earth",
                j2: Some(constants::J2_EARTH),
                j3: Some(constants::J3_EARTH),
                j4: Some(constants::J4_EARTH),
                atmosphere_altitude: Some(100.0), // Kármán line
            },
            KnownBody::Moon => BodyProperties {
                mu: constants::MU_MOON,
                radius: 1737.4,
                name: "Moon",
                j2: Some(2.033e-4),
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Mars => BodyProperties {
                mu: 42828.375214,
                radius: 3396.2,
                name: "Mars",
                j2: Some(1.9555e-3),
                j3: None,
                j4: None,
                atmosphere_altitude: Some(125.0),
            },
            KnownBody::Jupiter => BodyProperties {
                mu: 126686534.921800,
                radius: 71492.0,
                name: "Jupiter",
                j2: Some(1.4736e-2),
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Saturn => BodyProperties {
                mu: 37931206.159000,
                radius: 60268.0,
                name: "Saturn",
                j2: Some(1.6298e-2),
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Uranus => BodyProperties {
                mu: 5793951.256000,
                radius: 25559.0,
                name: "Uranus",
                j2: None,
                j3: None,
                j4: None,
                atmosphere_altitude: None,
            },
            KnownBody::Neptune => BodyProperties {
                mu: 6835099.975400,
                radius: 24764.0,
                name: "Neptune",
                j2: None,
                j3: None,
                j4: None,
                atmosphere_altitude: None,
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

    #[test]
    fn earth_j2_matches_constant() {
        assert_eq!(KnownBody::Earth.properties().j2, Some(constants::J2_EARTH));
    }

    #[test]
    fn oblate_bodies_have_j2() {
        let oblate = [
            KnownBody::Earth,
            KnownBody::Moon,
            KnownBody::Mars,
            KnownBody::Jupiter,
            KnownBody::Saturn,
        ];
        for body in &oblate {
            assert!(
                body.properties().j2.is_some(),
                "{:?} should have J2 value",
                body
            );
        }
    }

    #[test]
    fn spherical_bodies_have_no_j2() {
        let spherical = [
            KnownBody::Sun,
            KnownBody::Mercury,
            KnownBody::Venus,
            KnownBody::Uranus,
            KnownBody::Neptune,
        ];
        for body in &spherical {
            assert!(
                body.properties().j2.is_none(),
                "{:?} should not have J2 value",
                body
            );
        }
    }

    #[test]
    fn jupiter_j2_largest_among_planets() {
        let j2_earth = KnownBody::Earth.properties().j2.unwrap();
        let j2_jupiter = KnownBody::Jupiter.properties().j2.unwrap();
        assert!(
            j2_jupiter > j2_earth,
            "Jupiter J2 ({j2_jupiter}) should be larger than Earth J2 ({j2_earth})"
        );
    }

    #[test]
    fn earth_j3_matches_constant() {
        assert_eq!(KnownBody::Earth.properties().j3, Some(constants::J3_EARTH));
    }

    #[test]
    fn earth_j4_matches_constant() {
        assert_eq!(KnownBody::Earth.properties().j4, Some(constants::J4_EARTH));
    }

    #[test]
    fn only_earth_has_j3_j4() {
        assert!(KnownBody::Earth.properties().j3.is_some());
        assert!(KnownBody::Earth.properties().j4.is_some());
        for body in &[KnownBody::Moon, KnownBody::Mars, KnownBody::Jupiter, KnownBody::Saturn] {
            assert!(
                body.properties().j3.is_none(),
                "{:?} should not have J3 value",
                body
            );
        }
    }
}
