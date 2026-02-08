use nalgebra::Vector3;

use crate::component::{Component, ComponentName};

/// 3D position in km.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position3D(pub Vector3<f64>);

impl Component for Position3D {
    fn component_name() -> ComponentName {
        "orts.Position3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(Position3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["x", "y", "z"]
    }
}

/// 3D velocity in km/s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity3D(pub Vector3<f64>);

impl Component for Velocity3D {
    fn component_name() -> ComponentName {
        "orts.Velocity3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(Velocity3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["vx", "vy", "vz"]
    }
}

/// Gravitational parameter mu in km^3/s^2. Typically static.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GravitationalParameter(pub f64);

impl Component for GravitationalParameter {
    fn component_name() -> ComponentName {
        "orts.GravitationalParameter".into()
    }
    fn num_scalars() -> usize {
        1
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        data.first().map(|&v| GravitationalParameter(v))
    }
    fn field_names() -> Vec<&'static str> {
        vec!["mu"]
    }
}

/// Mean equatorial radius in km. Typically static.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyRadius(pub f64);

impl Component for BodyRadius {
    fn component_name() -> ComponentName {
        "orts.BodyRadius".into()
    }
    fn num_scalars() -> usize {
        1
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        data.first().map(|&v| BodyRadius(v))
    }
    fn field_names() -> Vec<&'static str> {
        vec!["radius"]
    }
}

/// Classical Keplerian orbital elements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeplerianState {
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub raan: f64,
    pub argument_of_periapsis: f64,
    pub true_anomaly: f64,
}

impl Component for KeplerianState {
    fn component_name() -> ComponentName {
        "orts.KeplerianState".into()
    }
    fn num_scalars() -> usize {
        6
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![
            self.semi_major_axis,
            self.eccentricity,
            self.inclination,
            self.raan,
            self.argument_of_periapsis,
            self.true_anomaly,
        ]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 6 {
            Some(KeplerianState {
                semi_major_axis: data[0],
                eccentricity: data[1],
                inclination: data[2],
                raan: data[3],
                argument_of_periapsis: data[4],
                true_anomaly: data[5],
            })
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["sma", "ecc", "inc", "raan", "aop", "ta"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;

    fn assert_roundtrip<C: Component + PartialEq>(original: &C) {
        let scalars = original.to_scalars();
        assert_eq!(scalars.len(), C::num_scalars());
        let recovered = C::from_scalars(&scalars).expect("from_scalars should succeed");
        assert_eq!(original, &recovered);
    }

    #[test]
    fn position3d_roundtrip() {
        let p = Position3D(Vector3::new(6778.137, 0.0, -42.5));
        assert_roundtrip(&p);
        assert_eq!(Position3D::field_names(), vec!["x", "y", "z"]);
        assert_eq!(Position3D::component_name(), "orts.Position3D");
    }

    #[test]
    fn velocity3d_roundtrip() {
        let v = Velocity3D(Vector3::new(0.0, 7.669, -0.5));
        assert_roundtrip(&v);
        assert_eq!(Velocity3D::field_names(), vec!["vx", "vy", "vz"]);
    }

    #[test]
    fn gravitational_parameter_roundtrip() {
        let mu = GravitationalParameter(398600.4418);
        assert_roundtrip(&mu);
        assert_eq!(GravitationalParameter::num_scalars(), 1);
    }

    #[test]
    fn body_radius_roundtrip() {
        let r = BodyRadius(6378.137);
        assert_roundtrip(&r);
        assert_eq!(BodyRadius::field_names(), vec!["radius"]);
    }

    #[test]
    fn keplerian_state_roundtrip() {
        let k = KeplerianState {
            semi_major_axis: 6778.137,
            eccentricity: 0.001,
            inclination: 0.9,
            raan: 1.5,
            argument_of_periapsis: 0.3,
            true_anomaly: 2.1,
        };
        assert_roundtrip(&k);
        assert_eq!(KeplerianState::num_scalars(), 6);
        assert_eq!(KeplerianState::field_names().len(), 6);
    }

    #[test]
    fn from_scalars_too_short() {
        assert!(Position3D::from_scalars(&[1.0, 2.0]).is_none());
        assert!(Velocity3D::from_scalars(&[]).is_none());
        assert!(KeplerianState::from_scalars(&[1.0, 2.0, 3.0]).is_none());
    }

    #[test]
    fn from_scalars_empty_for_scalar_types() {
        assert!(GravitationalParameter::from_scalars(&[]).is_none());
        assert!(BodyRadius::from_scalars(&[]).is_none());
    }
}
