use nalgebra::Vector3;

use crate::record::component::{Component, ComponentName};

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

/// Body-to-inertial quaternion [w, x, y, z] (Hamilton scalar-first).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quaternion4D(pub nalgebra::Vector4<f64>);

impl Component for Quaternion4D {
    fn component_name() -> ComponentName {
        "orts.Quaternion4D".into()
    }
    fn num_scalars() -> usize {
        4
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0[0], self.0[1], self.0[2], self.0[3]]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 4 {
            Some(Quaternion4D(nalgebra::Vector4::new(
                data[0], data[1], data[2], data[3],
            )))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["qw", "qx", "qy", "qz"]
    }
}

/// Angular velocity in body frame [rad/s].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngularVelocity3D(pub Vector3<f64>);

impl Component for AngularVelocity3D {
    fn component_name() -> ComponentName {
        "orts.AngularVelocity3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(AngularVelocity3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["wx", "wy", "wz"]
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

/// MTQ command (magnetic dipole moment) in body frame [A·m²], 3-axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MtqCommand3D(pub Vector3<f64>);

impl Component for MtqCommand3D {
    fn component_name() -> ComponentName {
        "orts.MtqCommand3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(MtqCommand3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["mtq_mx", "mtq_my", "mtq_mz"]
    }
}

/// The disturbance torque one model produces, in the body frame [N·m].
///
/// One of these per model, each logged under a name carrying the model it came
/// from, so a reader can tell SRP from aerodynamics rather than seeing only
/// their sum. [`torque_columns`] builds those names; the values come from
/// `SpacecraftDynamics::torque_breakdown`.
///
/// The vector rather than a magnitude: a magnitude carries neither the sign nor
/// the axis, and a disturbance turning the spacecraft the wrong way reads the
/// same as one turning it the right way.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelTorqueBody3D(pub Vector3<f64>);

impl Component for ModelTorqueBody3D {
    fn component_name() -> ComponentName {
        "orts.ModelTorqueBody3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() == 3 {
            Some(ModelTorqueBody3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["torque_body_x_Nm", "torque_body_y_Nm", "torque_body_z_Nm"]
    }
}

/// Encode a model name into the part of a column name that carries it, using
/// only `[A-Za-z0-9_]`.
///
/// The restriction has two reasons. A model chooses its own `name()`, and a
/// comma or a newline in one would break the CSV header the column name ends up
/// in. A column name also travels as part of an entity path when the recording
/// is written to an `.rrd`, and that path does not carry every character back:
/// measured on a round trip, `plain_x` and `dot.x` return while `brack_x[Nm]`,
/// `star_N*m_x` and `mark_x#2` are gone. That is also why the unit in
/// [`ModelTorqueBody3D`]'s field names is written `_Nm`.
///
/// The encoding is reversible, which is what keeps two models apart: every
/// byte outside the alphabet becomes `_xHH`, so `a-b` and `a_b` — one
/// substitution away from each other — arrive as `a_x2Db` and `a_b` rather
/// than as one column presenting two models as one. An underscore stays as it
/// is, so the names models actually use read as themselves
/// (`panel_srp`, `gravity_gradient`). Two of them are written `_x5F` instead,
/// each so that decoding has a single answer: the one that would read as an
/// escape (`_` before `x` and two hex digits), and the one that starts the
/// name — which leaves a lone `_` for the empty name, a value no other name
/// can encode to.
///
/// No encoded name contains `.`, which is what lets [`torque_columns`] mark a
/// repeat with one.
fn encoded(model: &str) -> String {
    let bytes = model.as_bytes();
    let mut out = String::with_capacity(model.len());
    for (i, byte) in bytes.iter().enumerate() {
        match byte {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => out.push(*byte as char),
            b'_' => {
                let reads_as_escape = bytes.get(i + 1) == Some(&b'x')
                    && bytes.get(i + 2).is_some_and(|c| c.is_ascii_hexdigit())
                    && bytes.get(i + 3).is_some_and(|c| c.is_ascii_hexdigit());
                if reads_as_escape || i == 0 {
                    out.push_str("_x5F");
                } else {
                    out.push('_');
                }
            }
            other => out.push_str(&format!("_x{other:02X}")),
        }
    }
    if out.is_empty() {
        // Unreachable from a non-empty name: the first byte of one is either
        // alphanumeric or an escape.
        "_".to_string()
    } else {
        out
    }
}

/// The component name and column names to log each model's torque under, in
/// the order the names arrive.
///
/// [`Model::name`](crate::model::Model::name) is not unique — nothing stops two
/// models of a spacecraft from answering the same name — and two values logged
/// under one name at one time point are two samples of it, which would open a
/// second row rather than sit beside each other. Repeats therefore get `.2`,
/// `.3`, counted per name, so every column is distinct and the numbering does
/// not depend on how many other models there are. The mark is a `.` because
/// [`encoded`] never produces one and a file returns it, which `#` does not.
pub fn torque_columns<'a>(
    models: impl IntoIterator<Item = &'a str>,
) -> Vec<(ComponentName, Vec<String>)> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    models
        .into_iter()
        .map(|model| {
            let base = encoded(model);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            let key = if *count == 1 {
                base
            } else {
                format!("{base}.{count}")
            };
            let name: ComponentName =
                format!("{}:{key}", ModelTorqueBody3D::component_name()).into();
            let fields = ModelTorqueBody3D::field_names()
                .into_iter()
                .map(|field| format!("{key}.{field}"))
                .collect();
            (name, fields)
        })
        .collect()
}

/// RW command (motor torque) per wheel [N·m], 3-axis orthogonal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RwTorqueCommand3D(pub Vector3<f64>);

impl Component for RwTorqueCommand3D {
    fn component_name() -> ComponentName {
        "orts.RwTorqueCommand3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(RwTorqueCommand3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["rw_tx", "rw_ty", "rw_tz"]
    }
}

/// Thruster throttle per thruster [0, 1], up to 3 thrusters (extras truncated).
///
/// Recorded so that downstream tools can see exactly when each thruster
/// fired without having to infer from orbit-element deltas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThrusterThrottle3D(pub Vector3<f64>);

impl Component for ThrusterThrottle3D {
    fn component_name() -> ComponentName {
        "orts.ThrusterThrottle3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(ThrusterThrottle3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["throttle_0", "throttle_1", "throttle_2"]
    }
}

/// RW momentum per wheel [N·m·s], 3-axis orthogonal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RwMomentum3D(pub Vector3<f64>);

impl Component for RwMomentum3D {
    fn component_name() -> ComponentName {
        "orts.RwMomentum3D".into()
    }
    fn num_scalars() -> usize {
        3
    }
    fn to_scalars(&self) -> Vec<f64> {
        vec![self.0.x, self.0.y, self.0.z]
    }
    fn from_scalars(data: &[f64]) -> Option<Self> {
        if data.len() >= 3 {
            Some(RwMomentum3D(Vector3::new(data[0], data[1], data[2])))
        } else {
            None
        }
    }
    fn field_names() -> Vec<&'static str> {
        vec!["rw_hx", "rw_hy", "rw_hz"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::component::Component;

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

    #[test]
    fn model_torque_round_trips_through_scalars() {
        let t = ModelTorqueBody3D(Vector3::new(1.5e-6, -2.0e-7, 3.25e-8));
        let scalars = t.to_scalars();
        assert_eq!(scalars.len(), ModelTorqueBody3D::num_scalars());
        assert_eq!(ModelTorqueBody3D::from_scalars(&scalars), Some(t));
        assert_eq!(ModelTorqueBody3D::from_scalars(&[1.0, 2.0]), None);
    }

    /// The column name says which model the torque came from, and the field
    /// names carry the frame and the unit: a reader meeting `panel_srp` beside
    /// inertial kilometres and an actuator command has nothing else to go on.
    #[test]
    fn torque_columns_name_the_model_the_frame_and_the_unit() {
        let columns = torque_columns(["panel_srp", "panel_drag"]);
        assert_eq!(
            columns[0].0.as_ref(),
            "orts.ModelTorqueBody3D:panel_srp",
            "the component name carries the model"
        );
        assert_eq!(
            columns[0].1,
            vec![
                "panel_srp.torque_body_x_Nm",
                "panel_srp.torque_body_y_Nm",
                "panel_srp.torque_body_z_Nm",
            ]
        );
        assert_eq!(columns[1].0.as_ref(), "orts.ModelTorqueBody3D:panel_drag");
        assert!(columns[1].1[0].starts_with("panel_drag."));
    }

    /// Two models answering one name would otherwise be logged under one
    /// column, which reads as two samples of it and opens a second row.
    #[test]
    fn torque_columns_separate_models_that_share_a_name() {
        let columns = torque_columns(["thruster", "thruster", "other", "thruster"]);
        let names: Vec<&str> = columns.iter().map(|(name, _)| name.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                "orts.ModelTorqueBody3D:thruster",
                "orts.ModelTorqueBody3D:thruster.2",
                "orts.ModelTorqueBody3D:other",
                "orts.ModelTorqueBody3D:thruster.3",
            ],
            "the count is per name, so `other` does not shift the numbering"
        );
        assert!(columns[1].1[0].starts_with("thruster.2."));
    }

    /// A column name travels as part of an entity path in an `.rrd`, and that
    /// path does not carry every character back: `[`, `]`, `*` and `#` are
    /// lost, which a round trip measures
    /// (`a_per_model_torque_column_survives_the_file`). Keeping the names
    /// inside `[A-Za-z0-9_.]` is what makes them survive.
    #[test]
    fn torque_column_names_use_only_characters_a_file_returns() {
        for (name, fields) in torque_columns(["panel_srp", "panel_srp", "a b*c[1]#2"]) {
            let tail = name
                .split(':')
                .next_back()
                .expect("a model part")
                .to_string();
            for text in std::iter::once(tail).chain(fields) {
                assert!(
                    text.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
                    "column name would not survive a file: {text:?}"
                );
            }
        }
    }

    /// Two models one substitution apart must not land in one column: their
    /// torques would be presented as a single model's, and the second write at
    /// a time point would open a row of its own.
    ///
    /// The repeat counter cannot cover this. It counts within one call, and the
    /// columns are built per satellite, so `a-b` on one satellite and `a_b` on
    /// another meet only in the CSV header.
    #[test]
    fn torque_columns_keep_names_apart_that_a_lossy_substitution_would_merge() {
        let distinct = [
            "a-b", "a_b", "a b", "a.b", "a/b", "AB", "ab", "_x2Db", "-b", "", "_", "unnamed",
        ];
        let mut keys: Vec<String> = distinct
            .iter()
            .map(|model| {
                torque_columns([*model])[0]
                    .0
                    .split(':')
                    .next_back()
                    .expect("a model part")
                    .to_string()
            })
            .collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            before,
            "distinct model names collapsed into one column: {keys:?}"
        );
    }

    /// A model name is whatever the model says, and the column name ends up in
    /// a CSV header: a comma or a newline in one would split a row.
    #[test]
    fn torque_columns_keep_a_model_name_out_of_the_csv_syntax() {
        let columns = torque_columns(["a,b", "line\nbreak", "", "ok_1"]);
        let names: Vec<&str> = columns.iter().map(|(name, _)| name.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                "orts.ModelTorqueBody3D:a_x2Cb",
                "orts.ModelTorqueBody3D:line_x0Abreak",
                "orts.ModelTorqueBody3D:_",
                "orts.ModelTorqueBody3D:ok_1",
            ]
        );
        for (_, fields) in &columns {
            for field in fields {
                assert!(
                    !field.contains(',') && !field.contains('\n'),
                    "field name would break a CSV row: {field:?}"
                );
            }
        }
    }
}
