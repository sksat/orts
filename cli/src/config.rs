use std::collections::HashMap;
use std::path::Path;

use clap::ValueEnum;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::cli::{AtmosphereChoice, IntegratorChoice};
use crate::satellite::{OrbitSpec, SatelliteSpec};
use crate::tle::fetch_tle_by_norad_id;
use arika::body::KnownBody;
use orts::plugin::{Message, NamedValue, NodeId, Payload, Value};
use orts::setup::DisturbanceTorques;
use orts::spacecraft::{PanelOptics, SpacecraftShape, SurfacePanel};

/// Resolve a config string through the [`ValueEnum`] impl the matching CLI
/// flag uses, so `--atmosphere x` and `atmosphere = "x"` accept exactly the
/// same set by construction: there is one list of spellings, clap's.
///
/// The config transport keeps these fields as `String` (the TypeScript clients
/// of `start_simulation` send strings), so this is where the string becomes a
/// model choice — and where an unknown spelling has to be rejected rather than
/// silently resolved to some default model.
fn parse_choice<T: ValueEnum>(field: &str, value: &str) -> Result<T, String> {
    <T as ValueEnum>::from_str(value, false).map_err(|_| {
        let allowed = T::value_variants()
            .iter()
            .filter_map(|v| v.to_possible_value())
            .map(|p| p.get_name().to_string())
            .collect::<Vec<_>>()
            .join(" | ");
        format!("unknown {field} '{value}' (expected one of: {allowed})")
    })
}

/// Reject an unknown `[integrator] type` at deserialization time, so every
/// entry point (file, `orts config validate`, the `start_simulation`
/// WebSocket payload) fails on a typo instead of running a different method.
fn de_integrator_kind<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    let s = String::deserialize(de)?;
    parse_choice::<IntegratorChoice>("integrator.type", &s).map_err(serde::de::Error::custom)?;
    Ok(s)
}

/// Reject an unknown `atmosphere` at deserialization time. A typo used to fall
/// back to the exponential model, i.e. quietly integrate different physics.
fn de_atmosphere<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    let s = String::deserialize(de)?;
    parse_choice::<AtmosphereChoice>("atmosphere", &s).map_err(serde::de::Error::custom)?;
    Ok(s)
}

/// JSON/TOML/YAML simulation configuration.
///
/// Also the payload of the `start_simulation` WebSocket message, so the
/// whole tree derives [`TS`]. Fields the server defaults when absent are
/// `#[ts(optional)]` so TypeScript clients may omit them too.
// A key nothing reads is reported by `load_with_warnings`, not rejected: a
// dropped key is indistinguishable from one never written, so `duraton = 100`
// used to run with the default duration and report success in silence. Refusing
// the file instead would stop an older `orts` reading a config written for a
// newer one.
//
// The `#[serde(tag = "type")]` enums below keep `deny_unknown_fields` and refuse
// instead. `serde_ignored`, which collects the paths, cannot see into an
// internally tagged enum — serde buffers the variant's content and replays it,
// so an unknown key there is dropped with nothing to report (measured: a typo
// inside `[satellites.orbit]` yields no path, while one at the top level and one
// in `[integrator]` both do). A silent `inclinaton = 51.6` leaves the orbit
// equatorial, so those four reject rather than say nothing. The alternative,
// external tagging, spells the orbit `[satellites.orbit.circular]` and gives the
// same error for the same typo, changing the config format and the WebSocket
// payload to buy nothing.
//
// Kept out of the doc comment because doc comments are copied into the generated
// TypeScript.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct SimConfig {
    #[serde(default = "default_body")]
    #[ts(as = "Option<_>", optional)]
    pub body: String,
    #[serde(default = "default_dt")]
    #[ts(as = "Option<_>", optional)]
    pub dt: f64,
    #[ts(optional)]
    pub output_interval: Option<f64>,
    #[ts(optional)]
    pub stream_interval: Option<f64>,
    #[ts(optional)]
    pub epoch: Option<String>,
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub integrator: IntegratorConfig,
    #[serde(default = "default_atmosphere", deserialize_with = "de_atmosphere")]
    #[ts(as = "Option<_>", optional)]
    pub atmosphere: String,
    #[serde(default = "default_f107")]
    #[ts(as = "Option<_>", optional)]
    pub f107: f64,
    #[serde(default = "default_ap")]
    #[ts(as = "Option<_>", optional)]
    pub ap: f64,
    #[ts(optional)]
    pub space_weather: Option<String>,
    #[ts(optional)]
    pub duration: Option<f64>,
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub satellites: Vec<SatelliteConfig>,
    /// 時刻指定コマンドシーケンス（FSW への C&T アップリンク）。
    /// 各エントリは指定 sim 時刻に対象衛星のコントローラへ配送される。
    /// TOML では array-of-tables の慣習に従い `[[command]]`（単数）で宣言する。
    #[serde(default, rename = "command")]
    #[ts(as = "Option<_>", optional)]
    pub commands: Vec<CommandConfig>,
    /// 地上局定義（contact window 検出用）。Earth 中心のシミュレーション
    /// でのみ有効。TOML では `[[ground_station]]`（単数）で宣言する。
    #[serde(default, rename = "ground_station")]
    #[ts(as = "Option<_>", optional)]
    pub ground_stations: Vec<GroundStationConfig>,
}

/// Ground station definition for visibility / contact window detection.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct GroundStationConfig {
    pub name: String,
    /// Geodetic latitude [deg] (WGS-84).
    pub latitude_deg: f64,
    /// Longitude [deg].
    pub longitude_deg: f64,
    /// Height above the WGS-84 ellipsoid [km].
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub altitude_km: f64,
    /// Minimum elevation mask [deg] (default: 5°).
    #[serde(default = "default_min_elevation_deg")]
    #[ts(as = "Option<_>", optional)]
    pub min_elevation_deg: f64,
}

fn default_min_elevation_deg() -> f64 {
    5.0
}

impl GroundStationConfig {
    /// Convert to the orts domain type (degrees → radians at the boundary).
    pub fn to_ground_station(&self) -> orts::visibility::GroundStation {
        orts::visibility::GroundStation {
            name: self.name.clone(),
            geodetic: arika::earth::Geodetic {
                latitude: self.latitude_deg.to_radians(),
                longitude: self.longitude_deg.to_radians(),
                altitude: self.altitude_km,
            },
            min_elevation: self.min_elevation_deg.to_radians(),
        }
    }
}

/// 時刻指定コマンド（config transport）。
///
/// `orts.toml` の `[[command]]` として宣言し、`t` 秒の時点で `sat` の
/// コントローラ(FSW)へ `kind` + `args`(key-value payload) を配送する。
/// host が配送 tick を確定するので決定論的。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct CommandConfig {
    /// 配送するシミュレーション時刻 \[s\]。
    pub t: f64,
    /// 配送先衛星 id（`[[satellites]]` の id と一致する必要がある）。
    pub sat: String,
    /// メッセージの論理型（content-type）。例 `"orts.cmd.set-mode.v1"`。
    pub kind: String,
    /// key-value payload 引数。TOML/JSON のスカラ値が型付き値に対応する
    /// （string→text, integer→integer, float→number, bool→boolean）。
    /// 省略時は空の key-value。
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub args: serde_json::Value,
}

impl CommandConfig {
    /// 配送先衛星インデックスを指定して host-native [`Message`] を組み立てる。
    /// `src` は配送時に host(controller) が確定する。
    pub fn to_message(&self, sat_index: usize) -> Result<Message, String> {
        let payload = args_to_payload(&self.args).map_err(|e| {
            format!(
                "command t={} sat='{}' kind='{}': {e}",
                self.t, self.sat, self.kind
            )
        })?;
        Ok(Message {
            src: NodeId::Ground,
            dst: NodeId::Satellite(sat_index as u32),
            kind: self.kind.clone(),
            payload,
        })
    }
}

/// `args`（JSON/TOML object）を key-value [`Payload`] に変換する。
/// スカラ値のみ対応。null（省略）は空の key-value。
fn args_to_payload(args: &serde_json::Value) -> Result<Payload, String> {
    use serde_json::Value as J;
    let map = match args {
        J::Null => return Ok(Payload::KeyValue(Vec::new())),
        J::Object(m) => m,
        other => return Err(format!("args must be a table/object, got {other}")),
    };
    let mut kvs = Vec::with_capacity(map.len());
    for (name, v) in map {
        let value = match v {
            J::Bool(b) => Value::Boolean(*b),
            J::String(s) => Value::Text(s.clone()),
            J::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Integer(i)
                } else if n.is_u64() {
                    // Reached only for u64 > i64::MAX (`as_i64` handled the
                    // rest), which always overflows s64. Reject rather than
                    // wrap silently into a negative integer.
                    return Err(format!("arg '{name}': integer {n} exceeds s64 (i64) range"));
                } else if let Some(f) = n.as_f64() {
                    Value::Number(f)
                } else {
                    return Err(format!("arg '{name}': unsupported number {n}"));
                }
            }
            other => {
                return Err(format!(
                    "arg '{name}': unsupported value type {other} (only scalars allowed in args)"
                ));
            }
        };
        kvs.push(NamedValue {
            name: name.clone(),
            value,
        });
    }
    Ok(Payload::KeyValue(kvs))
}

fn default_body() -> String {
    "earth".to_string()
}
fn default_dt() -> f64 {
    10.0
}
fn default_atmosphere() -> String {
    "exponential".to_string()
}
fn default_f107() -> f64 {
    150.0
}
fn default_ap() -> f64 {
    15.0
}

/// Integrator configuration within a config file.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct IntegratorConfig {
    #[serde(
        rename = "type",
        default = "default_integrator",
        deserialize_with = "de_integrator_kind"
    )]
    #[ts(as = "Option<_>", optional)]
    pub kind: String,
    #[serde(default = "default_atol")]
    #[ts(as = "Option<_>", optional)]
    pub atol: f64,
    #[serde(default = "default_rtol")]
    #[ts(as = "Option<_>", optional)]
    pub rtol: f64,
}

fn default_integrator() -> String {
    "dp45".to_string()
}
fn default_atol() -> f64 {
    1e-10
}
fn default_rtol() -> f64 {
    1e-8
}

impl Default for IntegratorConfig {
    fn default() -> Self {
        Self {
            kind: default_integrator(),
            atol: default_atol(),
            rtol: default_rtol(),
        }
    }
}

/// How far a normalized `initial_quaternion` may sit from unit norm.
///
/// An empirical bound on the normalization residual, not a bound on attitude
/// error: a quaternion's norm scales it radially in 4D and says nothing on its
/// own about the rotation it names. It exists to separate a residual that is
/// floating-point rounding from one that says the input lost its mantissa
/// before the square root — see [`AttitudeConfig::validate`] for the measured
/// cases it sits between, 1.8e-11 on the last accepted and 5.6e-6 on the first
/// refused.
const QUATERNION_UNIT_TOLERANCE: f64 = 1e-9;

/// Attitude dynamics configuration for a satellite.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct AttitudeConfig {
    /// Diagonal inertia tensor [Ixx, Iyy, Izz] kg·m².
    pub inertia_diag: [f64; 3],
    /// Off-diagonal inertia elements [Ixy, Ixz, Iyz] (default: all zero).
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub inertia_off_diag: [f64; 3],
    /// Spacecraft mass [kg].
    pub mass: f64,
    /// Initial quaternion [w, x, y, z] body-to-inertial (default: identity).
    #[serde(default = "default_identity_quat")]
    #[ts(as = "Option<_>", optional)]
    pub initial_quaternion: [f64; 4],
    /// Initial angular velocity [wx, wy, wz] rad/s body frame (default: zero).
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub initial_angular_velocity: [f64; 3],
}

fn default_identity_quat() -> [f64; 4] {
    [1.0, 0.0, 0.0, 0.0]
}

/// Which environmental disturbance torques to model for a satellite.
///
/// A sibling of `[satellites.attitude]` rather than a field inside it: that
/// table states the attitude state and the body's properties, while this one
/// selects which environment models get solved. Requires attitude dynamics —
/// a torque needs an orientation to act on.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct DisturbancesConfig {
    /// Gravity-gradient torque from the central body (default: true).
    #[serde(default = "default_true")]
    #[ts(as = "Option<_>", optional)]
    pub gravity_gradient: bool,
}

fn default_true() -> bool {
    true
}

/// One flat panel of the spacecraft's outer surface.
///
/// Panels are the geometry both SRP and atmospheric drag read, so writing them
/// replaces the isotropic parameters for both forces at once. See
/// [`crate::config::SatelliteConfig::panels`].
///
/// A panel is one face: both force models drop a panel whose normal points away
/// from the Sun or the flow. A thin structure exposed on both sides — a solar
/// array — therefore needs both faces, or it produces nothing for half of the
/// attitudes it sees, and the torque about an off-centre `cp_offset` goes with
/// it. Ask for the far face with `two_sided` when the two sides look the same,
/// or with `back` when they differ optically, which is the usual case for an
/// array: cells one side, substrate the other.
///
/// Either way, only for a plate. A face of a closed body already has its far
/// side in the panel list — the opposite face of the box — so a second face
/// here sits at the same place pointing the same way, and both are lit
/// together: twice the force, and a torque pair that partly cancels.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct PanelConfig {
    /// Panel area [m²]. Write this or `half_extent`, not both.
    #[ts(optional)]
    pub area: Option<f64>,
    /// In-plane half-extents [m]: along `in_plane_x`, then along
    /// `normal × in_plane_x`. The area follows from these.
    ///
    /// Writing them gives the panel a boundary, which is what lets another
    /// panel be found standing in front of it. Without them the panel still
    /// produces the same force when lit, but takes no part in shadowing.
    #[ts(optional)]
    pub half_extent: Option<[f64; 2]>,
    /// In-plane reference axis for `half_extent`, perpendicular to `normal`.
    #[ts(optional)]
    pub in_plane_x: Option<[f64; 3]>,
    /// Outward-pointing normal in the body frame. Normalised internally.
    pub normal: [f64; 3],
    /// Drag coefficient (typically 2.0–2.2 for LEO free-molecular flow).
    pub cd: f64,
    /// Specular reflectivity ρ_s (default: 0).
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub specular: f64,
    /// Diffuse reflectivity ρ_d (default: 0). The rest is absorbed.
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub diffuse: f64,
    /// Centre-of-pressure offset from the CoM [m, body frame] (default: zero).
    ///
    /// This is what makes a panel force an attitude disturbance.
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub cp_offset: [f64; 3],
    /// Give the plate its far face, sharing everything with this one.
    ///
    /// The short spelling for a thin plate whose two sides look the same. Use
    /// `back` when they differ optically — that also asks for the far face, so
    /// `two_sided = true` adds nothing beside it, and `two_sided = false`
    /// beside it is a contradiction and rejected.
    #[serde(default)]
    #[ts(optional)]
    pub two_sided: Option<bool>,
    /// The other side of the plate, when it differs from this one.
    ///
    /// Asks for the far face as `two_sided` does, and carries the
    /// reflectivities that differ; one left out comes from this side. An empty
    /// table says the same as `two_sided = true`. Area, `cd` and `cp_offset`
    /// are shared either way, since it is one plate — a different `cd` per side
    /// needs the two faces written out as separate panels.
    #[ts(optional)]
    pub back: Option<PanelBackConfig>,
}

/// The back face of a thin plate, as far as it differs from the front.
///
/// Either field may be left out and comes from the front, so an empty table
/// says the same as `two_sided = true`.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct PanelBackConfig {
    /// Specular reflectivity ρ_s of the back face (default: the front's).
    #[ts(optional)]
    pub specular: Option<f64>,
    /// Diffuse reflectivity ρ_d of the back face (default: the front's).
    #[ts(optional)]
    pub diffuse: Option<f64>,
}

/// Reject reflectivities `PanelOptics::new` would assert on, naming `where`
/// so a multi-panel config says which face to go and fix.
///
/// `inherited` marks values that came from the front face rather than from what
/// the reader wrote, so the sum message cannot quote a number they cannot find.
fn validate_optics(
    where_: &str,
    specular: f64,
    diffuse: f64,
    inherited: Inherited,
) -> Result<(), String> {
    // Per field, so the message names the key rather than the pair.
    for (key, value) in [("specular", specular), ("diffuse", diffuse)] {
        if !value.is_finite() {
            return Err(format!("{where_}.{key} must be finite"));
        }
        if value < 0.0 {
            return Err(format!("{where_}.{key} must be non-negative"));
        }
    }
    if specular + diffuse > 1.0 {
        return Err(format!(
            "{where_}.specular + {where_}.diffuse must be at most 1, got {specular}{} + {diffuse}{}",
            if inherited.specular {
                " (from the front)"
            } else {
                ""
            },
            if inherited.diffuse {
                " (from the front)"
            } else {
                ""
            },
        ));
    }
    Ok(())
}

/// Which of a back face's reflectivities were taken from the front.
#[derive(Debug, Clone, Copy, Default)]
struct Inherited {
    specular: bool,
    diffuse: bool,
}

impl PanelConfig {
    /// Validate the fields `SurfacePanel::at_com` and `PanelOptics::new` would
    /// otherwise assert on, so malformed config reports an error instead of
    /// panicking.
    fn validate(&self, index: usize) -> Result<(), String> {
        match (self.half_extent, self.in_plane_x) {
            (Some(_), None) => {
                return Err(format!(
                    "panels[{index}].half_extent needs in_plane_x to say which way it runs"
                ));
            }
            (None, Some(_)) => {
                return Err(format!(
                    "panels[{index}].in_plane_x describes no extent without half_extent"
                ));
            }
            (Some(half_extent), Some(_)) => {
                if self.area.is_some() {
                    return Err(format!(
                        "panels[{index}]: area and half_extent both give the size; keep one"
                    ));
                }
                if !half_extent.iter().all(|h| h.is_finite() && *h > 0.0) {
                    return Err(format!(
                        "panels[{index}].half_extent components must be positive and finite"
                    ));
                }
            }
            (None, None) => {
                let Some(area) = self.area else {
                    return Err(format!(
                        "panels[{index}] needs area, or half_extent with in_plane_x"
                    ));
                };
                if !area.is_finite() || area <= 0.0 {
                    return Err(format!("panels[{index}].area must be positive and finite"));
                }
            }
        }
        if !self.cd.is_finite() || self.cd < 0.0 {
            return Err(format!(
                "panels[{index}].cd must be non-negative and finite"
            ));
        }
        let n = self.normal;
        let norm_sq = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
        if !norm_sq.is_finite() || norm_sq <= 0.0 {
            return Err(format!(
                "panels[{index}].normal must be a non-zero finite vector"
            ));
        }
        validate_optics(
            &format!("panels[{index}]"),
            self.specular,
            self.diffuse,
            Inherited::default(),
        )?;
        if self.two_sided == Some(false) && self.back.is_some() {
            return Err(format!(
                "panels[{index}]: back asks for a far face and two_sided = false \
                 refuses one; keep one"
            ));
        }
        if let Some(back) = &self.back {
            // The resolved values, since an omitted one comes from the front:
            // `back = { specular = 0.9 }` on a front with `diffuse = 0.2` is
            // over 1 without either field being wrong on its own.
            let (specular, diffuse) = self.back_optics();
            validate_optics(
                &format!("panels[{index}].back"),
                specular,
                diffuse,
                Inherited {
                    specular: back.specular.is_none(),
                    diffuse: back.diffuse.is_none(),
                },
            )?;
        }
        if !self.cp_offset.iter().all(|x| x.is_finite()) {
            return Err(format!(
                "panels[{index}].cp_offset components must be finite"
            ));
        }
        Ok(())
    }

    /// Whether this panel describes a plate with a far face.
    ///
    /// `two_sided = false` beside `back` does not reach here: `validate`
    /// rejects the pair. Picking a winner would be worse than the rejection —
    /// letting the flag win discards `back`, letting `back` win discards the
    /// flag, and neither is what the reader asked for.
    fn has_back_face(&self) -> bool {
        self.two_sided == Some(true) || self.back.is_some()
    }

    /// The far face's reflectivities, each falling back to this face's.
    ///
    /// Both come from the front when the far face was asked for with
    /// `two_sided`, which is what that spelling means.
    fn back_optics(&self) -> (f64, f64) {
        let back = self.back.as_ref();
        (
            back.and_then(|b| b.specular).unwrap_or(self.specular),
            back.and_then(|b| b.diffuse).unwrap_or(self.diffuse),
        )
    }

    /// The one or two faces this panel describes.
    ///
    /// Two when `two_sided` or `back` asks for it: a thin plate exposed on both
    /// sides produces no force at all for the attitudes where the flow comes
    /// from behind, so the far face has to be there to be seen.
    ///
    /// Both constructors normalise the normal, which is what keeps the
    /// unit-length assert in the model constructors from firing.
    ///
    /// `validate` has already established that exactly one of `area` and
    /// `half_extent` is there, so the `expect` below cannot fire from config.
    fn to_surface_panels(&self) -> Vec<SurfacePanel> {
        let normal = nalgebra::Vector3::from_row_slice(&self.normal);
        let optics = PanelOptics::new(self.specular, self.diffuse);
        let front = match (self.half_extent, self.in_plane_x) {
            (Some(half_extent), Some(in_plane_x)) => SurfacePanel::rectangle(
                half_extent,
                nalgebra::Vector3::from_row_slice(&in_plane_x),
                normal,
                self.cd,
                optics,
            ),
            _ => SurfacePanel::at_com(
                self.area
                    .expect("validate requires area without half_extent"),
                normal,
                self.cd,
                optics,
            ),
        }
        .with_cp_offset(nalgebra::Vector3::from_row_slice(&self.cp_offset));

        if !self.has_back_face() {
            return vec![front];
        }
        let (specular, diffuse) = self.back_optics();
        let back = front.back_face(PanelOptics::new(specular, diffuse));
        vec![front, back]
    }
}

impl DisturbancesConfig {
    fn to_disturbance_torques(&self) -> DisturbanceTorques {
        DisturbanceTorques {
            gravity_gradient: self.gravity_gradient,
        }
    }
}

impl AttitudeConfig {
    /// Build the full 3×3 inertia tensor from diagonal and off-diagonal elements.
    pub fn inertia_matrix(&self) -> nalgebra::Matrix3<f64> {
        let [ixx, iyy, izz] = self.inertia_diag;
        let [ixy, ixz, iyz] = self.inertia_off_diag;
        nalgebra::Matrix3::new(
            ixx, ixy, ixz, //
            ixy, iyy, iyz, //
            ixz, iyz, izz,
        )
    }

    /// The configured initial quaternion, normalized.
    ///
    /// The config accepts any non-zero quaternion, but the simulation should
    /// start from the unit quaternion it denotes rather than from its scale.
    /// Integrating the raw one lets a large quaternion grow until its sum of
    /// squares overflows, and the post-step projection then divides every
    /// component by infinity and hands back all zeros — a state that passes
    /// `is_finite`, so nothing stops the run, and that normalizes to `NaN` when
    /// a sensor reads it.
    ///
    /// Assumes [`validate`](Self::validate) has passed: the norm is positive
    /// and finite there. Falls back to identity rather than producing `NaN` if
    /// it has not.
    pub fn normalized_initial_quaternion(&self) -> nalgebra::Vector4<f64> {
        let q = nalgebra::Vector4::from_row_slice(&self.initial_quaternion);
        let norm = q.norm();
        if norm > 0.0 && norm.is_finite() {
            q / norm
        } else {
            nalgebra::Vector4::new(1.0, 0.0, 0.0, 0.0)
        }
    }

    /// Reject the attitude configs that this config alone shows the dynamics
    /// cannot be built from or started.
    ///
    /// Lives on the config type so every entry point that loads a config gets
    /// it — `orts config validate` included, which would otherwise report a
    /// config as valid that `run` and `serve` then refuse.
    ///
    /// The scope is what the config alone determines: that the inertia tensor
    /// inverts to something usable, and that the *torque-free* derivative at
    /// `t = 0` is finite. It is not the derivative the simulation actually
    /// starts from — that includes the gravity-gradient torque, which needs the
    /// orbit. An inertia scaled so far from the orbit that the gravity gradient
    /// alone overflows therefore passes here and stops at the first step with
    /// [`utsuroi::IntegrationError::NonFiniteState`].
    /// Equality `I1 + I2 == I3` is accepted: that is the flat-plate (lamina)
    /// limit, physically attainable and numerically well-posed. The relative
    /// slack keeps a config that states the lamina case exactly from being
    /// rejected by the eigenvalue solver's last bits.
    ///
    /// The WebSocket `add_satellite` path reaches the same rules:
    /// [`crate::sim::mode::validate_satellite_spec`] takes a built
    /// `SatelliteSpec` and delegates to this, so neither surface can accept a
    /// spacecraft the other refuses.
    pub fn validate(&self) -> Result<(), String> {
        // Non-finite first: every comparison below is false for `NaN`, so a
        // `NaN` mass would pass `mass <= 0.0` and a `NaN` determinant would
        // pass the singularity check. The state then integrates to `NaN` and
        // the run fails partway through instead of at the config.
        for (name, values) in [
            ("inertia_diag", &self.inertia_diag[..]),
            ("inertia_off_diag", &self.inertia_off_diag[..]),
            ("mass", std::slice::from_ref(&self.mass)),
            ("initial_quaternion", &self.initial_quaternion[..]),
            (
                "initial_angular_velocity",
                &self.initial_angular_velocity[..],
            ),
        ] {
            if let Some(bad) = values.iter().find(|v| !v.is_finite()) {
                return Err(format!("non-finite `{name}` component: {bad}"));
            }
        }
        if self.mass <= 0.0 {
            return Err(format!("non-positive mass: {}", self.mass));
        }
        // The quaternion need not be normalized — `AttitudeState::orientation`
        // normalizes on use and `OdeState::project` renormalizes after every
        // step — but the normalization has to land on a unit quaternion. Ask
        // that the way the dynamics do, by running the same call and looking at
        // what comes out, rather than by putting a floor on the squared norm:
        // a floor on the smallest normal rejects `[1e-154, 0, 0, 0]`, whose
        // squared norm is subnormal yet normalizes exactly.
        //
        // Measured across the range: `[1e200, 0, 0, 0]` squares to infinity and
        // normalizes to all zeros; `[1e-164, 0, 0, 0]` squares to zero and
        // normalizes to infinity; a squared norm deep in the subnormal range
        // has lost mantissa bits before the square root, so `[1e-160, 0, 0, 0]`
        // normalizes to 1.0000056. The last accepted case, `[1e-157, 0, 0, 0]`,
        // is off by 1.8e-11.
        //
        // `QUATERNION_UNIT_TOLERANCE` sits between those two.
        let normalized = nalgebra::UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
            self.initial_quaternion[0],
            self.initial_quaternion[1],
            self.initial_quaternion[2],
            self.initial_quaternion[3],
        ))
        .into_inner()
        .norm();
        if !normalized.is_finite() || (normalized - 1.0).abs() > QUATERNION_UNIT_TOLERANCE {
            let quat_norm_sq: f64 = self.initial_quaternion.iter().map(|q| q * q).sum();
            return Err(format!(
                "`initial_quaternion` does not normalize to a unit quaternion (its \
                 components square to {quat_norm_sq}, and normalizing gives a norm of \
                 {normalized}); it names no attitude"
            ));
        }
        // Ask the inertia question the way the dynamics do: `SpacecraftDynamics::new`
        // takes `try_inverse().expect(…)`, so the test is whether that inverse
        // exists and is usable. A magnitude threshold on the determinant cannot
        // answer it — the determinant carries the cube of the units, so
        // `[1e-11; 3]` is perfectly conditioned yet its determinant is 1e-33,
        // while `[5e-324, 1e154, 1e154]` has a determinant near 5e-16 and an
        // inverse whose first component is infinite.
        //
        // Checking the product rather than the inverse's components catches the
        // quieter failure too: nalgebra's 3x3 inverse divides cofactors by the
        // determinant, so `[1e154; 3]` — condition number 1 — overflows both to
        // give `Some(zero matrix)`. Every component is finite, and a spacecraft
        // built on it would answer every torque with no angular acceleration.
        // `I · I⁻¹` is dimensionless, so one tolerance holds at every scale.
        let inertia = self.inertia_matrix();
        let inverse = inertia.try_inverse().filter(|inv| {
            let residual = inertia * inv - nalgebra::Matrix3::identity();
            residual.iter().all(|v| v.abs() < 1e-6)
        });
        let Some(inverse) = inverse else {
            return Err(format!(
                "inertia tensor cannot be inverted: {:?} with off-diagonal {:?}",
                self.inertia_diag, self.inertia_off_diag
            ));
        };
        // Numeric usability is not physical possibility, and these come before
        // the derivative below: a tensor no mass distribution can produce should
        // be reported as such rather than as whatever its derivative happens to
        // do. State both constraints in terms of the principal moments
        // `I1 <= I2 <= I3` — the eigenvalues, which equal `inertia_diag` only
        // when `inertia_off_diag` is zero, so read them off the tensor.
        let mut moments: Vec<f64> = inertia.symmetric_eigenvalues().iter().copied().collect();
        moments.sort_by(|a, b| a.partial_cmp(b).expect("eigenvalues of a finite tensor"));
        let (i1, i2, i3) = (moments[0], moments[1], moments[2]);
        // A non-positive principal moment means zero or negative mass off that
        // axis. `I1 == 0` is the singular case the inverse already refuses; a
        // negative one can invert cleanly and still describe nothing.
        if i1 <= 0.0 {
            return Err(format!(
                "inertia tensor is not positive definite: smallest principal moment is \
                 {i1} (diag {:?}, off-diag {:?})",
                self.inertia_diag, self.inertia_off_diag
            ));
        }
        // No mass distribution can violate `I1 + I2 >= I3`: in principal axes
        // `∫z² dm = (I1 + I2 − I3)/2`, so a tensor that violates it needs
        // negative mass. Equality is the flat-plate (lamina) limit, attainable
        // and well-posed, and a config on the physical side of the boundary is
        // accepted however close it sits.
        //
        // The slack is relative because the moments carry kg·m² and an absolute
        // tolerance would mean different things at different scales. `1e-9` is
        // far looser than the eigenvalue solver needs — a few hundred
        // `f64::EPSILON` would cover its rounding — and is set for hand-entered
        // and rounded engineering figures, which can land just outside the
        // boundary they were meant to state.
        const TRIANGLE_SLACK: f64 = 1e-9;
        if i1 + i2 < i3 * (1.0 - TRIANGLE_SLACK) {
            return Err(format!(
                "inertia tensor violates the triangle inequality: principal moments \
                 [{i1}, {i2}, {i3}] have I1 + I2 < I3, which no mass distribution can \
                 produce (diag {:?}, off-diag {:?})",
                self.inertia_diag, self.inertia_off_diag
            ));
        }
        // A usable, physically possible tensor is still not an integrable state:
        // the torque-free Euler term `I⁻¹ (−ω × Iω)` can overflow on the rate
        // alone. `diag(1, 1, 2)` with `ω = 1e200` squares out of range in the
        // cross product.
        //
        // The tensor cannot do it by itself once the inequality holds: in
        // principal axes `α1 = ((I2 − I3)/I1) ω2 ω3`, and `|I2 − I3| <= I1`
        // there, so the gain never exceeds 1.
        let omega = nalgebra::Vector3::from_row_slice(&self.initial_angular_velocity);
        let alpha = inverse * -omega.cross(&(inertia * omega));
        if !alpha.iter().all(|v| v.is_finite()) {
            return Err(format!(
                "the initial angular acceleration is not finite ({:?}): the inertia tensor \
                 {:?} and `initial_angular_velocity` {:?} are too far apart in scale to \
                 integrate",
                alpha.as_slice(),
                self.inertia_diag,
                self.initial_angular_velocity
            ));
        }
        // The other half of the derivative, `q̇ = ½ q ⊗ (0, ω)`, needs no check
        // of its own. Each component is a three-term inner product `q · ½ω`, so
        // for the unit quaternion the state is built from
        // (`normalized_initial_quaternion`) Cauchy-Schwarz bounds it by
        // `½ ‖q‖ √3 max|ω| = ½ √3 f64::MAX ≈ 1.56e308`, inside the range for any
        // finite `ω` — and `ω` is finite by the check above.
        //
        // The bound holds of the partial sums too only because `q_dot` halves
        // `ω` before forming the products. Halving the sum afterwards instead
        // lets it overflow at twice the answer's magnitude.
        Ok(())
    }
}

/// コントローラ設定。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(tag = "type", deny_unknown_fields)]
#[ts(export)]
pub enum ControllerConfig {
    /// WASM Component ゲストプラグイン。
    #[serde(rename = "wasm")]
    Wasm {
        /// `.wasm` ファイルのパス。
        path: String,
        /// ゲストの `init` に渡す設定 (JSON value)。
        #[serde(default)]
        #[ts(as = "Option<_>", optional)]
        config: serde_json::Value,
    },
}

/// センサ選択。
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SensorChoice {
    Magnetometer,
    Gyroscope,
    StarTracker,
    SunSensor,
}

/// リアクションホイール設定。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(tag = "type", deny_unknown_fields)]
#[ts(export)]
pub enum ReactionWheelConfig {
    /// 直交 3 軸配置。
    #[serde(rename = "three_axis")]
    ThreeAxis {
        /// ホイール慣性モーメント [kg·m²]。
        inertia: f64,
        /// 最大角運動量 [N·m·s]。
        max_momentum: f64,
        /// 最大トルク [N·m]。
        max_torque: f64,
        /// 速度制御ゲイン [N·m / (rad/s)]。省略時はデフォルト (I_wheel * 10)。
        #[serde(default)]
        #[ts(optional)]
        speed_control_gain: Option<f64>,
    },
}

/// MTQ (磁気トルカ) 設定。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(tag = "type", deny_unknown_fields)]
#[ts(export)]
pub enum MtqConfig {
    /// 直交 3 軸配置。
    #[serde(rename = "three_axis")]
    ThreeAxis {
        /// 最大ダイポールモーメント [A·m²]。
        max_moment: f64,
    },
}

/// 推進器一機分の静的パラメータ。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct ThrusterSpecConfig {
    /// 最大推力 [N]。
    pub thrust_n: f64,
    /// 比推力 [s]。
    pub isp_s: f64,
    /// 機体系推力方向（単位ベクトル化される）。
    pub direction_body: [f64; 3],
    /// CoM からの取り付けオフセット [m, body frame]。省略時は 0。
    #[serde(default)]
    #[ts(optional)]
    pub offset_body: Option<[f64; 3]>,
}

/// 推進器群 (ThrusterAssembly) 設定。
///
/// `thrusters` に各推進器の静的パラメータを並べ、`dry_mass` で
/// 推進剤枯渇時の停止閾値 (spacecraft total mass [kg]) を指定する。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct ThrusterConfig {
    /// 推進器一覧（空リストは reject）。
    pub thrusters: Vec<ThrusterSpecConfig>,
    /// Assembly-level propellant floor [kg]。
    /// spacecraft total mass がこの値以下になったら全推進器を停止。
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub dry_mass: f64,
}

/// Per-satellite configuration.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[ts(export)]
pub struct SatelliteConfig {
    #[ts(optional)]
    pub id: Option<String>,
    #[ts(optional)]
    pub name: Option<String>,
    /// Viewer marker shape when this satellite has no 3D model (sphere / axes-cube).
    /// Display hint only; the viewer can override it. Defaults to automatic.
    #[ts(optional)]
    pub shape: Option<crate::sim::core::MarkerShape>,
    pub orbit: OrbitConfig,
    #[ts(optional)]
    pub ballistic_coeff: Option<f64>,
    #[ts(optional)]
    pub srp_area_to_mass: Option<f64>,
    #[ts(optional)]
    pub srp_cr: Option<f64>,
    /// Attitude dynamics configuration. When present, SpacecraftDynamics is used.
    #[ts(optional)]
    pub attitude: Option<AttitudeConfig>,
    /// Environmental disturbance torques. Requires `attitude`.
    #[ts(optional)]
    pub disturbances: Option<DisturbancesConfig>,
    /// Flat-panel outer surface. Drives both SRP and drag, and requires
    /// `attitude`; conflicts with the isotropic `srp_area_to_mass` / `srp_cr` /
    /// `ballistic_coeff`. A list of zero panels is rejected — omit the key to
    /// model an isotropic cross-section.
    #[ts(optional)]
    pub panels: Option<Vec<PanelConfig>>,
    /// プラグインコントローラ設定。
    #[ts(optional)]
    pub controller: Option<ControllerConfig>,
    /// 有効にするセンサ一覧。
    #[ts(optional)]
    pub sensors: Option<Vec<SensorChoice>>,
    /// リアクションホイール設定。
    #[ts(optional)]
    pub reaction_wheels: Option<ReactionWheelConfig>,
    /// MTQ 設定。TOML キーは隣の `reaction_wheels` に揃えてフルワード `magnetorquers`。
    #[serde(rename = "magnetorquers")]
    #[ts(optional)]
    pub mtq: Option<MtqConfig>,
    /// 推進器 (thruster) 設定。
    #[ts(optional)]
    pub thruster: Option<ThrusterConfig>,
    /// stream-io の名前付きバイトストリーム宣言（kble 統合）。
    /// 宣言した stream は serve の `ws://…/stream/{sat}/{name}` に公開される。
    #[serde(default)]
    #[ts(as = "Option<_>", optional)]
    pub streams: Vec<String>,
}

/// Orbit specification in config files.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(tag = "type", deny_unknown_fields)]
#[ts(export)]
pub enum OrbitConfig {
    /// Circular orbit at given altitude.
    #[serde(rename = "circular")]
    Circular {
        altitude: f64,
        /// Inclination in degrees (default: 0).
        #[serde(default)]
        #[ts(as = "Option<_>", optional)]
        inclination: f64,
        /// RAAN in degrees (default: 0).
        #[serde(default)]
        #[ts(as = "Option<_>", optional)]
        raan: f64,
    },
    /// Two-line element set.
    #[serde(rename = "tle")]
    Tle { line1: String, line2: String },
    /// Fetch TLE by NORAD catalog number.
    #[serde(rename = "norad")]
    Norad { norad_id: u32 },
}

/// How many of a client message's unread keys are named before the rest are
/// only counted.
///
/// `/ws` takes messages from whoever reaches the port, and each named key costs
/// one synchronous `eprintln!` on the connection task — stderr is unbuffered, so
/// that is a write syscall each. One frame of `{"a":1,"b":2,…}` is cheap to send
/// and would otherwise become as many writes as it has keys. Twenty is well past
/// what anyone reads to find a typo.
const CLIENT_MESSAGE_KEY_LIMIT: usize = 20;

/// How many characters of one key reach the log.
///
/// The key-count limit says nothing about how long each is, and a key over `/ws`
/// is as long as its sender cares to make it. A config key that anyone typed
/// fits in far less than this.
const PRINTED_KEY_LIMIT: usize = 128;

/// The unread keys of a client message, and how many did not fit.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct UnreadClientKeys {
    /// At most [`CLIENT_MESSAGE_KEY_LIMIT`] paths.
    pub named: Vec<String>,
    /// How many more keys nothing read, past the ones named.
    pub unnamed: usize,
}

impl UnreadClientKeys {
    /// Name a key while there is room for one, and count it after that.
    fn push(&mut self, key: String) {
        if self.named.len() < CLIENT_MESSAGE_KEY_LIMIT {
            self.named.push(key);
        } else {
            self.unnamed += 1;
        }
    }
}

/// The keys of a client message that no field of it reads.
///
/// `ClientMessage` is `#[serde(tag = "type")]`, and `serde_ignored` cannot see
/// inside an internally tagged enum: serde buffers the variant's content and
/// replays it, so deserializing the message itself collects nothing. Reaching
/// into the JSON for the part that is a config and targeting that directly gets
/// past the tag — the same trick reaches `add_satellite`'s flattened
/// `SatelliteConfig`, where `flatten` removes the unknown-field check outright.
///
/// Two things are inspected: the message's own keys, against what the variant
/// reads (`ClientMessage` ignores an unknown one), and the payload the variant
/// carries. Paths are relative to whichever they came from, so an envelope
/// `dtt` and a config `dtt` are both named `dtt` — the name is what finds the
/// typo either way.
///
/// Takes the message already parsed, so the caller can hand the same `Value` to
/// `ClientMessage` rather than paying for a second parse of every frame.
pub fn unread_client_message_keys(value: &serde_json::Value) -> UnreadClientKeys {
    let Some(kind) = value.get("type").and_then(|v| v.as_str()) else {
        return UnreadClientKeys::default();
    };

    let mut keys = UnreadClientKeys::default();

    // The envelope, against the keys the variant reads. `ClientMessage` ignores
    // an unknown one, so `{"type":"start_simulation","config":{…},"dtt":10}`
    // used to start the simulation with `dtt` dropped in silence.
    if let (Some(read), Some(object)) = (
        crate::commands::serve::protocol::variant_envelope_keys(kind),
        value.as_object(),
    ) {
        for key in object.keys() {
            if key != "type" && !read.contains(&key.as_str()) {
                keys.push(key.clone());
            }
        }
    }

    let mut note = |path: serde_ignored::Path| keys.push(path.to_string());
    match kind {
        "start_simulation" => {
            if let Some(config) = value.get("config") {
                // Borrowed, not cloned: the config subtree of a fleet-sized
                // message is the largest thing here.
                let _ = serde_ignored::deserialize::<_, _, SimConfig>(config, &mut note);
            }
        }
        "add_satellite" => {
            // The satellite is flattened next to the tag, so the tag itself is
            // the one key that belongs to the message rather than the satellite.
            // Only the map is rebuilt, without the tag; the values it points at
            // are borrowed.
            if let Some(object) = value.as_object() {
                let satellite: Vec<(&str, &serde_json::Value)> = object
                    .iter()
                    .filter(|(k, _)| k.as_str() != "type")
                    .map(|(k, v)| (k.as_str(), v))
                    .collect();
                let _ = serde_ignored::deserialize::<_, _, SatelliteConfig>(
                    serde::de::value::MapDeserializer::new(satellite.into_iter()),
                    &mut note,
                );
            }
        }
        // The rest carry scalars the message's own fields all read.
        _ => {}
    }
    keys
}

/// Load a config and report, on stderr, the keys nothing read.
///
/// For the paths that run a simulation. `orts config validate` renders them
/// itself, since it also has a `--json` form to put them in.
pub fn load_config_reporting_unread_keys(path: &Path) -> Result<SimConfig, String> {
    let loaded = SimConfig::load_with_warnings(path)?;
    for key in &loaded.unread_keys {
        log::warn!(
            "{}: nothing reads `{}`; its value is ignored",
            path.display(),
            printable_key(key)
        );
    }
    Ok(loaded.config)
}

/// A key as it can be written to a terminal.
///
/// A key name is arbitrary text. Over the WebSocket it is whatever a client
/// sent, and a `\n` in one would put a second `Warning:` line in the log while a
/// terminal escape would move the cursor or repaint what is already there.
/// Measured: `{"a\nWarning: forged line": 1}` in a `start_simulation` config
/// comes back as a key holding that newline.
///
/// `escape_debug` leaves an ordinary key alone — what it rewrites is the
/// characters no key needs. The JSON form of `orts config validate` needs none
/// of this; a JSON string encodes them itself.
///
/// The length is bounded too: a key is as long as its sender cares to make it,
/// and escaping can turn one character into six (`\u{1b}`). Counting the escaped
/// characters bounds the line whatever the input expands to. What is left out is
/// the middle of a name nobody was going to read to the end; the byte count says
/// how much.
pub fn printable_key(key: &str) -> String {
    let mut escaped = key.escape_debug();
    let mut out: String = escaped.by_ref().take(PRINTED_KEY_LIMIT).collect();
    if escaped.next().is_some() {
        out.push_str(&format!("… ({} bytes in all)", key.len()));
    }
    out
}

/// A loaded config and the keys in its file that nothing read.
///
/// The keys are paths as `serde_ignored` spells them — `satellites.0.disturbanses`
/// rather than `satellites[0].disturbanses`.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: SimConfig,
    /// Paths of keys the file carried and no field claimed, in the order the
    /// deserialize met them. Empty for a config whose every key was read.
    pub unread_keys: Vec<String>,
}

impl SimConfig {
    /// Load a config file, auto-detecting format by extension, and discard the
    /// keys nothing read.
    ///
    /// For a caller with nowhere to report them. The paths that run a
    /// simulation go through [`load_config_reporting_unread_keys`], and
    /// `orts config validate` renders them itself.
    #[cfg(test)]
    pub fn load(path: &Path) -> Result<Self, String> {
        Ok(Self::load_with_warnings(path)?.config)
    }

    /// Load a config, and say which of its keys nothing read.
    ///
    /// An unknown key is a warning rather than an error so that a config
    /// written for a newer `orts` still runs on an older one: rejecting the file
    /// would make one added option enough to stop it being read at all. A known
    /// key holding an unknown value stays an error — there the file names
    /// something the simulation cannot do.
    ///
    /// The paths come back as a value so that each caller decides where they
    /// go: `orts config validate --json` puts them in `warnings`, `run` and
    /// `serve` report them through `log::warn!`, and a test reads them without
    /// a logger installed.
    pub fn load_with_warnings(path: &Path) -> Result<LoadedConfig, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {e}", path.display()))?;

        // One funnel for all three formats, so the paths are collected the same
        // way whichever one the file is in.
        let mut unread_keys = Vec::new();
        let mut note = |path: serde_ignored::Path| unread_keys.push(path.to_string());

        let config: SimConfig = match ext.as_str() {
            "json" => {
                let mut de = serde_json::Deserializer::from_str(&content);
                let config = serde_ignored::deserialize(&mut de, &mut note)
                    .map_err(|e| format!("Failed to parse JSON config: {e}"))?;
                // `serde_json::from_str` ends by checking that the input is
                // spent; driving the `Deserializer` directly does not, so
                // anything after the config would be dropped without a word.
                de.end()
                    .map_err(|e| format!("Failed to parse JSON config: {e}"))?;
                config
            }
            "toml" => {
                let de = toml::Deserializer::parse(&content)
                    .map_err(|e| format!("Failed to parse TOML config: {e}"))?;
                serde_ignored::deserialize(de, &mut note)
                    .map_err(|e| format!("Failed to parse TOML config: {e}"))?
            }
            "yaml" | "yml" => {
                let de = serde_yaml::Deserializer::from_str(&content);
                serde_ignored::deserialize(de, &mut note)
                    .map_err(|e| format!("Failed to parse YAML config: {e}"))?
            }
            _ => {
                return Err(format!(
                    "Unknown config file extension '.{ext}'. Supported: .json, .toml, .yaml, .yml"
                ));
            }
        };

        config.validate()?;

        Ok(LoadedConfig {
            config,
            unread_keys,
        })
    }

    /// The integrator selected by `[integrator] type`.
    ///
    /// # Panics
    /// If the string is not one of the [`IntegratorChoice`] spellings. Both
    /// `Deserialize` and [`validate`](Self::validate) reject those, so this is
    /// only reachable for a hand-built `SimConfig`; the previous fallback to
    /// `dp45` meant a typo integrated with a different method and exited 0.
    pub fn integrator_choice(&self) -> IntegratorChoice {
        self.try_integrator_choice()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// The atmosphere model selected by `atmosphere`.
    ///
    /// # Panics
    /// As [`integrator_choice`](Self::integrator_choice), for the same reason:
    /// falling back to the exponential model would silently substitute the
    /// physics the user asked for.
    pub fn atmosphere_choice(&self) -> AtmosphereChoice {
        self.try_atmosphere_choice()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn try_integrator_choice(&self) -> Result<IntegratorChoice, String> {
        parse_choice("integrator.type", &self.integrator.kind)
    }

    fn try_atmosphere_choice(&self) -> Result<AtmosphereChoice, String> {
        parse_choice("atmosphere", &self.atmosphere)
    }

    /// Parse the central body from the config string.
    pub fn known_body(&self) -> KnownBody {
        crate::satellite::parse_body(&self.body)
    }
}

impl SatelliteConfig {
    /// The id this satellite is known by downstream: the recording entity
    /// path, the CSV section, the `[[command]]` target and the WebSocket
    /// messages. An omitted `id` becomes `sat-{index}`.
    ///
    /// Single source of the default so validation and
    /// [`to_satellite_spec`](Self::to_satellite_spec) cannot disagree about
    /// which ids a fleet actually resolves to.
    pub fn resolved_id(&self, index: usize) -> String {
        self.id.clone().unwrap_or_else(|| format!("sat-{index}"))
    }

    /// Convert a SatelliteConfig to a SatelliteSpec.
    pub fn to_satellite_spec(&self, index: usize, body: KnownBody, mu: f64) -> SatelliteSpec {
        let id = self.resolved_id(index);

        let (orbit, period, derived_name) = match &self.orbit {
            OrbitConfig::Circular {
                altitude,
                inclination,
                raan,
            } => {
                let r0 = body.properties().radius + altitude;
                let period = 2.0 * std::f64::consts::PI * (r0.powi(3) / mu).sqrt();
                let inc = inclination.to_radians();
                let ra = raan.to_radians();
                (
                    OrbitSpec::Circular {
                        altitude: *altitude,
                        r0,
                        inclination: inc,
                        raan: ra,
                    },
                    period,
                    None,
                )
            }
            OrbitConfig::Tle { line1, line2 } => {
                let text = format!("{line1}\n{line2}");
                let parsed = arika::tle::parse(&text)
                    .unwrap_or_else(|e| panic!("Failed to parse TLE in config: {e}"));
                let tle = parsed.elements;
                let period = tle.period();
                let tle_name = parsed.object_name.clone();
                (OrbitSpec::ElementSet { elements: tle }, period, tle_name)
            }
            OrbitConfig::Norad { norad_id } => {
                let parsed = fetch_tle_by_norad_id(*norad_id);
                let tle = parsed.elements;
                let period = tle.period();
                let tle_name = parsed.object_name.clone();
                (OrbitSpec::ElementSet { elements: tle }, period, tle_name)
            }
        };

        SatelliteSpec {
            id,
            name: self.name.clone().or(derived_name),
            orbit,
            period,
            ballistic_coeff: self.ballistic_coeff,
            srp_area_to_mass: self.srp_area_to_mass,
            srp_cr: self.srp_cr,
            attitude_config: self.attitude.clone(),
            disturbances: self
                .disturbances
                .as_ref()
                .map(DisturbancesConfig::to_disturbance_torques)
                .unwrap_or_default(),
            panels: self.panels.as_ref().map(|panels| {
                SpacecraftShape::panels(
                    panels
                        .iter()
                        .flat_map(PanelConfig::to_surface_panels)
                        .collect(),
                )
            }),
            shape: self.shape,
            controller_config: self.controller.clone(),
            sensor_choices: self.sensors.clone(),
            rw_config: self.reaction_wheels.clone(),
            mtq_config: self.mtq.clone(),
            thruster_config: self.thruster.clone(),
            streams: self.streams.clone(),
        }
    }
}

impl ThrusterConfig {
    /// Validate the config: reject empty list, zero-length directions, and
    /// non-finite numeric fields.
    ///
    /// We validate explicitly (rather than panicking inside `ThrusterSpec::new()`)
    /// so malformed user config produces a proper error message.
    pub fn validate(&self) -> Result<(), String> {
        if self.thrusters.is_empty() {
            return Err("thruster.thrusters must not be empty".into());
        }
        for (i, t) in self.thrusters.iter().enumerate() {
            let d = t.direction_body;
            let norm_sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if !norm_sq.is_finite() || norm_sq <= 0.0 {
                return Err(format!(
                    "thruster[{i}].direction_body must be a non-zero finite vector"
                ));
            }
            if let Some(off) = t.offset_body
                && !off.iter().all(|x| x.is_finite())
            {
                return Err(format!(
                    "thruster[{i}].offset_body components must be finite"
                ));
            }
            if !t.thrust_n.is_finite() || t.thrust_n <= 0.0 {
                return Err(format!(
                    "thruster[{i}].thrust_n must be positive and finite"
                ));
            }
            if !t.isp_s.is_finite() || t.isp_s <= 0.0 {
                return Err(format!("thruster[{i}].isp_s must be positive and finite"));
            }
        }
        if !self.dry_mass.is_finite() || self.dry_mass < 0.0 {
            return Err("thruster.dry_mass must be non-negative and finite".into());
        }
        Ok(())
    }
}

impl SatelliteConfig {
    /// Validate per-satellite config. Currently only covers thruster config;
    /// extend as other composite fields grow validation needs.
    ///
    /// Called from both [`SimConfig::load`] (file input) and the serve
    /// command's WebSocket entry points so that malformed JSON from a
    /// dynamic `add_satellite` is rejected before it can reach
    /// `ThrusterSpec::new()` and panic on e.g. zero-length directions.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(thruster) = &self.thruster {
            thruster.validate().map_err(|e| format!("thruster: {e}"))?;
        }
        if let Some(attitude) = &self.attitude {
            attitude.validate().map_err(|e| format!("attitude: {e}"))?;
        }
        // A disturbance torque acts on an orientation, and an orbit-only
        // satellite has none, so the selection would be read and then dropped.
        if self.disturbances.is_some() && self.attitude.is_none() {
            return Err(
                "disturbances requires attitude: a torque needs an orientation to act on".into(),
            );
        }
        if let Some(panels) = &self.panels {
            // A list of zero panels describes no surface at all, so it is a
            // mistake rather than a way to say "isotropic" — leave the key out
            // for that, or write `null`, both of which land here as `None`.
            if panels.is_empty() {
                return Err("panels must not be empty; omit the key instead".into());
            }
            // Panel forces are attitude-dependent by construction: `PanelSrp`
            // and `PanelDrag` need `HasAttitude`, so they only ever reach a
            // `SpacecraftDynamics`.
            if self.attitude.is_none() {
                return Err(
                    "panels requires attitude: a panel force depends on which way the panel faces"
                        .into(),
                );
            }
            // The isotropic parameters describe the same two forces. Honouring
            // both would mean choosing one silently.
            for (key, present) in [
                ("srp_area_to_mass", self.srp_area_to_mass.is_some()),
                ("srp_cr", self.srp_cr.is_some()),
                ("ballistic_coeff", self.ballistic_coeff.is_some()),
            ] {
                if present {
                    return Err(format!(
                        "panels and {key} both describe the same force; keep one"
                    ));
                }
            }
            for (i, panel) in panels.iter().enumerate() {
                panel.validate(i)?;
            }
        }
        // A non-finite orbit number propagates into the derived orbital
        // period. `run` then loops on `while !group.all_finished()` with a NaN
        // end time that no comparison can ever satisfy, so the simulation
        // never terminates. Reject the input instead.
        if let OrbitConfig::Circular {
            altitude,
            inclination,
            raan,
        } = &self.orbit
        {
            for (name, value) in [
                ("altitude", *altitude),
                ("inclination", *inclination),
                ("raan", *raan),
            ] {
                if !value.is_finite() {
                    return Err(format!("orbit.{name} must be finite (got {value})"));
                }
            }
        }
        Ok(())
    }

    /// Reject a circular orbit whose semi-major axis is not positive.
    ///
    /// Split from [`validate`](Self::validate) because `a = R_body + altitude`
    /// needs the central body, which lives on [`SimConfig`].
    fn validate_against_body(&self, body: KnownBody) -> Result<(), String> {
        if let OrbitConfig::Circular { altitude, .. } = &self.orbit {
            let r0 = body.properties().radius + altitude;
            if r0 <= 0.0 {
                return Err(format!(
                    "orbit.altitude ({altitude}) puts the semi-major axis at or below zero \
                     for body '{}' (radius {} km)",
                    body.properties().name,
                    body.properties().radius
                ));
            }
        }
        Ok(())
    }
}

/// Reject time knobs the integrator cannot make progress with.
///
/// Shared by the config-file path ([`SimConfig::validate`]) and the direct-CLI
/// path (`orts run --sat ...`, `orts serve --dt ...`), which reach
/// `SimParams` through different constructors and would otherwise disagree
/// about what is accepted.
///
/// Each value is load-bearing:
/// - `dt` drives `while t < t_end`, so zero never advances.
/// - `output_interval` drives `t += output_interval` in the orbit-only and
///   spacecraft `run` loops, so zero never reaches the time they stop at
///   (`duration` when given, otherwise the longest satellite period). The
///   controlled loop advances on controller ticks instead and only compares
///   `output_interval` to decide when to log, so zero there means logging every
///   tick rather than a clock that cannot move.
/// - `stream_interval` is a divisor in the serve loop's pacing, where zero
///   yields `0 * inf = NaN` and panics `Duration::from_secs_f64`.
/// - `duration` is how far `run` propagates the fleet.
///
/// `output_interval < dt` is rejected too: `SimParams` clamps
/// `stream_interval` into `[dt, output_interval]`, which panics outright on
/// inverted bounds.
pub fn validate_time_params(
    dt: f64,
    output_interval: Option<f64>,
    stream_interval: Option<f64>,
    duration: Option<f64>,
) -> Result<(), String> {
    if !dt.is_finite() || dt <= 0.0 {
        return Err(format!("dt must be positive and finite (got {dt})"));
    }
    for (name, value) in [
        ("output_interval", output_interval),
        ("stream_interval", stream_interval),
        ("duration", duration),
    ] {
        if let Some(v) = value
            && (!v.is_finite() || v <= 0.0)
        {
            return Err(format!("{name} must be positive and finite (got {v})"));
        }
    }
    if let Some(output_interval) = output_interval
        && output_interval < dt
    {
        return Err(format!(
            "output_interval ({output_interval}) must be >= dt ({dt}): the simulation \
             cannot emit output more often than it steps"
        ));
    }
    Ok(())
}

/// Reject a controller tick that cannot advance the control loop.
///
/// Both controlled-run loops schedule ticks at `start_t + n · sample_period`,
/// so a zero period leaves every tick on the start time and a negative one
/// walks backwards. The period comes from the plugin/controller rather than
/// from user config, so it has to be checked where it is first used. A period
/// that is positive but below the sim clock's resolution at the satellite's
/// start time is caught separately, where that start time is known.
pub fn validate_sample_period(dt_ctrl: f64) -> Result<(), String> {
    if dt_ctrl.is_finite() && dt_ctrl > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "controller sample period must be positive and finite (got {dt_ctrl})"
        ))
    }
}

/// Reject tolerances that cannot drive adaptive error control.
///
/// `sc = atol + rtol * |y|` is the adaptive error scale. With both zero, an
/// exactly-zero state component gives `0 / 0 = NaN`, which the stepper can
/// neither accept nor shrink away from.
///
/// Skipped for RK4, which ignores the tolerances entirely — rejecting them
/// there would fail configs that carry unused `atol`/`rtol`.
pub fn validate_tolerances(
    integrator: IntegratorChoice,
    atol: f64,
    rtol: f64,
) -> Result<(), String> {
    if !matches!(
        integrator,
        IntegratorChoice::Dp45 | IntegratorChoice::Dop853
    ) {
        return Ok(());
    }
    if !atol.is_finite() || atol < 0.0 {
        return Err(format!(
            "integrator.atol must be non-negative and finite (got {atol})"
        ));
    }
    if !rtol.is_finite() || rtol < 0.0 {
        return Err(format!(
            "integrator.rtol must be non-negative and finite (got {rtol})"
        ));
    }
    if atol == 0.0 && rtol == 0.0 {
        return Err(
            "integrator.atol and integrator.rtol must not both be zero: adaptive \
             error control needs at least one positive tolerance"
                .to_string(),
        );
    }
    Ok(())
}

impl SimConfig {
    /// Validate the config. Idempotent and side-effect-free (no network /
    /// filesystem access), so it is safe for both `config validate` and the
    /// `SimConfig::load` path shared by `orts run` / `orts serve`.
    ///
    /// Covers the semantic fields that `SimParams::from_config` would otherwise
    /// panic on (unknown body, malformed epoch, malformed inline TLE). It does
    /// not resolve `norad` orbits — that requires a network fetch and is left
    /// to run time.
    pub fn validate(&self) -> Result<(), String> {
        // Resolve the model choices first: a deserialized config cannot carry
        // an unknown spelling, but a hand-built one can, and every later step
        // (tolerances, drag) depends on which model was actually selected.
        let integrator = self.try_integrator_choice()?;
        self.try_atmosphere_choice()?;
        validate_time_params(
            self.dt,
            self.output_interval,
            self.stream_interval,
            self.duration,
        )?;
        validate_tolerances(integrator, self.integrator.atol, self.integrator.rtol)?;
        if crate::satellite::try_parse_body(&self.body).is_none() {
            return Err(format!(
                "unknown body '{}' (expected one of: sun, mercury, venus, earth, \
                 moon, mars, jupiter, saturn, uranus, neptune)",
                self.body
            ));
        }
        if let Some(epoch) = &self.epoch
            && arika::epoch::Epoch::from_iso8601(epoch).is_none()
        {
            return Err(format!(
                "invalid epoch '{epoch}': expected ISO 8601 (e.g. 2026-01-01T00:00:00Z)"
            ));
        }
        let body = crate::satellite::try_parse_body(&self.body)
            .expect("body was validated as parseable above");
        // Resolved ids must be unique: they are the recording entity path, the
        // CSV section header and the `[[command]]` target, and every consumer
        // resolves an id by first match or by `HashMap` insert. Duplicates
        // therefore merge two satellites' rows under one path and route all
        // commands to whichever one won the map — silently. Note the collision
        // an explicit `id` can have with the `sat-{index}` default of an
        // id-less entry, which is why this resolves the id first.
        //
        // Compared on the entity the id names, not on the id text: `EntityPath`
        // drops empty segments, so `a` and `/a` (or `a/b` and `a//b`) are two id
        // strings naming one entity. `ensure_unique_ids` compares the same way
        // for fleets built from repeated `--sat`.
        let mut seen: HashMap<String, usize> = HashMap::with_capacity(self.satellites.len());
        for (i, sat) in self.satellites.iter().enumerate() {
            let id = sat.resolved_id(i);
            // An id has to name an entity before two of them can be compared:
            // one made only of separators contributes no segment and collapses
            // to the `/world/sat` root the whole fleet shares, which a fleet of
            // one reaches as readily as a fleet of many.
            crate::satellite::validate_id(&id).map_err(|e| format!("satellites[{i}]: {e}"))?;
            let entity = crate::satellite::entity_path_for_id(&id).to_string();
            if let Some(first) = seen.insert(entity, i) {
                return Err(format!(
                    "satellites[{i}]: duplicate satellite id '{id}' (already used by \
                     satellites[{first}]); ids must be unique{}",
                    if sat.id.is_none() {
                        format!(
                            " — this entry has no `id`, so it defaults to '{id}'; \
                             give it an explicit id"
                        )
                    } else {
                        String::new()
                    }
                ));
            }
        }
        for (i, sat) in self.satellites.iter().enumerate() {
            sat.validate()
                .map_err(|e| format!("satellites[{i}]: {e}"))?;
            sat.validate_against_body(body)
                .map_err(|e| format!("satellites[{i}]: {e}"))?;
            // Parse inline TLE lines with the same parser `from_config` uses, so
            // a malformed element set is rejected here rather than panicking.
            if let OrbitConfig::Tle { line1, line2 } = &sat.orbit {
                arika::tle::parse(&format!("{line1}\n{line2}"))
                    .map_err(|e| format!("satellites[{i}]: invalid TLE: {e}"))?;
            }
            // SGP4 is Earth's, and `SimParams::from_config` reaches this rule
            // through an `unwrap_or_else(panic)`: a non-Earth TLE config
            // validated clean and then took down `orts run --config`.
            if matches!(
                sat.orbit,
                OrbitConfig::Tle { .. } | OrbitConfig::Norad { .. }
            ) {
                crate::sim::params::ensure_body_carries_an_element_set(body)
                    .map_err(|e| format!("satellites[{i}]: {e}"))?;
            }
        }
        // Which mode the fleet runs in is settled by the config, so a fleet no
        // mode can serve is settled here too. `SatelliteSpec` carries these two
        // as clones of the fields counted below, so the count the engine makes
        // is this count.
        let fleet_size = self.satellites.len();
        let with_attitude = self
            .satellites
            .iter()
            .filter(|s| s.attitude.is_some())
            .count();
        let with_controller = self
            .satellites
            .iter()
            .filter(|s| s.controller.is_some())
            .count();
        crate::sim::mode::ensure_fleet_declares_uniformly(
            fleet_size,
            with_attitude,
            with_controller,
        )?;
        for (i, cmd) in self.commands.iter().enumerate() {
            // A non-finite or negative `t` would never satisfy the
            // schedule's `t <= t_due`, silently dropping the command.
            if !cmd.t.is_finite() || cmd.t < 0.0 {
                // Use the TOML key (`[[command]]`) in the message, not the
                // Rust field name, so the index matches what the user wrote.
                return Err(format!(
                    "command[{i}]: t must be finite and >= 0 (got {})",
                    cmd.t
                ));
            }
        }
        Ok(())
    }

    /// Reject configs that `orts serve` cannot honor.
    ///
    /// `[[command]]` timelines are an `orts run` (deterministic batch)
    /// transport; the serve loop does not build/drain a `CommandSchedule`,
    /// so a timeline would be silently dropped. Interactive commanding in
    /// serve is the (future) WebSocket console, not a config timeline.
    pub fn ensure_serve_supported(&self) -> Result<(), String> {
        if !self.commands.is_empty() {
            return Err(format!(
                "config has {} `[[command]]` timeline entr{}: command timelines run under \
                 `orts run`, not `orts serve` (they would be silently dropped). \
                 Use `orts run` for scheduled commands.",
                self.commands.len(),
                if self.commands.len() == 1 { "y" } else { "ies" }
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// The unread keys of a message, parsed the way `connection.rs` parses it.
    ///
    /// The collector takes the tree, since the server hands the same one to
    /// `ClientMessage` rather than parsing each frame twice.
    fn unread_keys_of(text: &str) -> Vec<String> {
        let value: serde_json::Value =
            serde_json::from_str(text).expect("the message is valid JSON");
        let unread = unread_client_message_keys(&value);
        assert_eq!(
            unread.unnamed, 0,
            "these messages hold fewer keys than the limit"
        );
        unread.named
    }

    use super::*;

    #[test]
    fn deserialize_json_minimal() {
        let json = r#"{
            "satellites": [
                { "orbit": { "type": "circular", "altitude": 400 } }
            ]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.body, "earth");
        assert!((config.dt - 10.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);
        assert!(matches!(
            config.satellites[0].orbit,
            OrbitConfig::Circular { altitude, .. } if (altitude - 400.0).abs() < 1e-9
        ));
    }

    #[test]
    fn deserialize_json_full() {
        let json = r#"{
            "body": "mars",
            "dt": 5.0,
            "output_interval": 20.0,
            "stream_interval": 10.0,
            "epoch": "2024-03-20T12:00:00Z",
            "integrator": { "type": "rk4", "atol": 1e-12, "rtol": 1e-10 },
            "atmosphere": "nrlmsise00",
            "f107": 200.0,
            "ap": 30.0,
            "space_weather": "auto",
            "duration": 86400.0,
            "satellites": [
                {
                    "id": "sat1",
                    "name": "My Satellite",
                    "orbit": { "type": "circular", "altitude": 800, "inclination": 98.6, "raan": 45.0 },
                    "ballistic_coeff": 0.005,
                    "srp_area_to_mass": 0.01,
                    "srp_cr": 1.8
                }
            ]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.body, "mars");
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.output_interval, Some(20.0));
        assert_eq!(config.stream_interval, Some(10.0));
        assert_eq!(config.epoch.as_deref(), Some("2024-03-20T12:00:00Z"));
        assert_eq!(config.integrator.kind, "rk4");
        assert!((config.integrator.atol - 1e-12).abs() < 1e-20);
        assert_eq!(config.atmosphere, "nrlmsise00");
        assert!((config.f107 - 200.0).abs() < 1e-9);
        assert!((config.ap - 30.0).abs() < 1e-9);
        assert_eq!(config.space_weather.as_deref(), Some("auto"));
        assert_eq!(config.duration, Some(86400.0));

        let sat = &config.satellites[0];
        assert_eq!(sat.id.as_deref(), Some("sat1"));
        assert_eq!(sat.name.as_deref(), Some("My Satellite"));
        assert_eq!(sat.ballistic_coeff, Some(0.005));
        assert_eq!(sat.srp_area_to_mass, Some(0.01));
        assert_eq!(sat.srp_cr, Some(1.8));
        assert!(matches!(
            sat.orbit,
            OrbitConfig::Circular { altitude, inclination, raan }
            if (altitude - 800.0).abs() < 1e-9
            && (inclination - 98.6).abs() < 1e-9
            && (raan - 45.0).abs() < 1e-9
        ));
    }

    #[test]
    fn deserialize_tle_orbit() {
        let json = r#"{
            "satellites": [{
                "id": "iss",
                "orbit": {
                    "type": "tle",
                    "line1": "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996",
                    "line2": "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008"
                }
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(
            config.satellites[0].orbit,
            OrbitConfig::Tle { .. }
        ));
    }

    #[test]
    fn deserialize_norad_orbit() {
        let json = r#"{
            "satellites": [{
                "orbit": { "type": "norad", "norad_id": 25544 }
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(
            config.satellites[0].orbit,
            OrbitConfig::Norad { norad_id: 25544 }
        ));
    }

    #[test]
    fn defaults_applied() {
        let json = r#"{ "satellites": [] }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.body, "earth");
        assert!((config.dt - 10.0).abs() < 1e-9);
        assert_eq!(config.atmosphere, "exponential");
        assert!((config.f107 - 150.0).abs() < 1e-9);
        assert!((config.ap - 15.0).abs() < 1e-9);
        assert_eq!(config.integrator.kind, "dp45");
        assert!((config.integrator.atol - 1e-10).abs() < 1e-20);
        assert!((config.integrator.rtol - 1e-8).abs() < 1e-16);
    }

    #[test]
    fn integrator_choice_parsing() {
        let config = SimConfig {
            body: "earth".into(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            integrator: IntegratorConfig {
                kind: "rk4".into(),
                atol: 1e-10,
                rtol: 1e-8,
            },
            atmosphere: "exponential".into(),
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            satellites: vec![],
            commands: vec![],
            ground_stations: vec![],
        };
        assert!(matches!(config.integrator_choice(), IntegratorChoice::Rk4));
    }

    #[test]
    fn atmosphere_choice_parsing() {
        let mut config = SimConfig {
            body: "earth".into(),
            dt: 10.0,
            output_interval: None,
            stream_interval: None,
            epoch: None,
            integrator: IntegratorConfig::default(),
            atmosphere: "harris-priester".into(),
            f107: 150.0,
            ap: 15.0,
            space_weather: None,
            duration: None,
            satellites: vec![],
            commands: vec![],
            ground_stations: vec![],
        };
        assert!(matches!(
            config.atmosphere_choice(),
            AtmosphereChoice::HarrisPriester
        ));
        config.atmosphere = "nrlmsise00".into();
        assert!(matches!(
            config.atmosphere_choice(),
            AtmosphereChoice::Nrlmsise00
        ));
        config.atmosphere = "exponential".into();
        assert!(matches!(
            config.atmosphere_choice(),
            AtmosphereChoice::Exponential
        ));
    }

    #[test]
    fn satellite_config_to_spec_circular() {
        let sat_cfg = SatelliteConfig {
            id: Some("sso".into()),
            name: Some("SSO 800km".into()),
            orbit: OrbitConfig::Circular {
                altitude: 800.0,
                inclination: 98.6,
                raan: 0.0,
            },
            ballistic_coeff: Some(0.005),
            srp_area_to_mass: None,
            srp_cr: None,
            shape: None,
            attitude: None,
            disturbances: None,
            panels: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
            thruster: None,
            streams: Vec::new(),
        };
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let spec = sat_cfg.to_satellite_spec(0, body, mu);

        assert_eq!(spec.id, "sso");
        assert_eq!(spec.name.as_deref(), Some("SSO 800km"));
        assert_eq!(spec.ballistic_coeff, Some(0.005));
        assert!(matches!(
            spec.orbit,
            OrbitSpec::Circular { altitude, inclination, .. }
            if (altitude - 800.0).abs() < 1e-9
            && (inclination - 98.6_f64.to_radians()).abs() < 1e-9
        ));
        assert!(spec.period > 0.0);
    }

    #[test]
    fn satellite_config_auto_id() {
        let sat_cfg = SatelliteConfig {
            id: None,
            name: None,
            orbit: OrbitConfig::Circular {
                altitude: 400.0,
                inclination: 0.0,
                raan: 0.0,
            },
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            shape: None,
            attitude: None,
            disturbances: None,
            panels: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
            thruster: None,
            streams: Vec::new(),
        };
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let spec = sat_cfg.to_satellite_spec(3, body, mu);
        assert_eq!(spec.id, "sat-3");
    }

    #[test]
    fn satellite_config_tle_to_spec() {
        let sat_cfg = SatelliteConfig {
            id: Some("iss".into()),
            name: None,
            orbit: OrbitConfig::Tle {
                line1: "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9996"
                    .into(),
                line2: "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480008"
                    .into(),
            },
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            shape: None,
            attitude: None,
            disturbances: None,
            panels: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
            thruster: None,
            streams: Vec::new(),
        };
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let spec = sat_cfg.to_satellite_spec(0, body, mu);

        assert_eq!(spec.id, "iss");
        assert!(matches!(spec.orbit, OrbitSpec::ElementSet { .. }));
        assert!(spec.period > 0.0);
    }

    #[test]
    fn deserialize_toml() {
        let toml_str = r#"
body = "earth"
dt = 5.0

[integrator]
type = "dp45"

[[satellites]]
id = "sso"
ballistic_coeff = 0.005

[satellites.orbit]
type = "circular"
altitude = 800.0
inclination = 98.6
"#;
        let config: SimConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.body, "earth");
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);
        assert_eq!(config.satellites[0].id.as_deref(), Some("sso"));
    }

    #[test]
    fn deserialize_yaml() {
        let yaml_str = r#"
body: earth
dt: 5.0
satellites:
  - id: sso
    orbit:
      type: circular
      altitude: 800.0
      inclination: 98.6
"#;
        let config: SimConfig = serde_yaml::from_str(yaml_str).unwrap();
        assert_eq!(config.body, "earth");
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);
        assert_eq!(config.satellites[0].id.as_deref(), Some("sso"));
    }

    #[test]
    fn load_unknown_extension() {
        let dir = std::env::temp_dir().join(format!("orts-config-test-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.xml");
        std::fs::write(&path, "{}").unwrap();
        let result = SimConfig::load(&path);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Unknown config file extension"),
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn json_roundtrip() {
        let config = SimConfig {
            body: "earth".into(),
            dt: 5.0,
            output_interval: Some(10.0),
            stream_interval: None,
            epoch: Some("2024-03-20T12:00:00Z".into()),
            integrator: IntegratorConfig {
                kind: "dp45".into(),
                atol: 1e-12,
                rtol: 1e-10,
            },
            atmosphere: "nrlmsise00".into(),
            f107: 200.0,
            ap: 30.0,
            space_weather: Some("auto".into()),
            duration: Some(86400.0),
            satellites: vec![SatelliteConfig {
                id: Some("test".into()),
                name: Some("Test Sat".into()),
                orbit: OrbitConfig::Circular {
                    altitude: 400.0,
                    inclination: 51.6,
                    raan: 90.0,
                },
                ballistic_coeff: Some(0.01),
                srp_area_to_mass: Some(0.02),
                srp_cr: Some(1.5),
                shape: None,
                attitude: None,
                disturbances: None,
                panels: None,
                controller: None,
                sensors: None,
                reaction_wheels: None,
                mtq: None,
                thruster: None,
                streams: Vec::new(),
            }],
            commands: vec![],
            ground_stations: vec![],
        };
        let json = serde_json::to_string(&config).unwrap();
        let roundtrip: SimConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.body, config.body);
        assert!((roundtrip.dt - config.dt).abs() < 1e-9);
        assert_eq!(roundtrip.satellites.len(), 1);
        assert_eq!(roundtrip.satellites[0].id, config.satellites[0].id);
    }

    #[test]
    fn load_json_file() {
        let dir = std::env::temp_dir().join(format!("orts-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.json");
        std::fs::write(
            &path,
            r#"{ "dt": 5.0, "satellites": [{ "orbit": { "type": "circular", "altitude": 400 } }] }"#,
        )
        .unwrap();

        let config = SimConfig::load(&path).unwrap();
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_toml_file() {
        let dir =
            std::env::temp_dir().join(format!("orts-config-test-toml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.toml");
        std::fs::write(
            &path,
            r#"
dt = 5.0

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 400.0
"#,
        )
        .unwrap();

        let config = SimConfig::load(&path).unwrap();
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn satellite_shape_parses_and_propagates() {
        use crate::sim::core::MarkerShape;
        let orbit =
            r#""orbit": { "type": "circular", "altitude": 500, "inclination": 0, "raan": 0 }"#;

        // kebab-case rename: "axes-cube" → AxesCube.
        let cfg: SatelliteConfig =
            serde_json::from_str(&format!(r#"{{ {orbit}, "shape": "axes-cube" }}"#)).unwrap();
        assert_eq!(cfg.shape, Some(MarkerShape::AxesCube));
        // Carried into the runtime spec.
        let spec = cfg.to_satellite_spec(0, KnownBody::Earth, 398_600.4418);
        assert_eq!(spec.shape, Some(MarkerShape::AxesCube));

        // Absent → None (the viewer decides).
        let none: SatelliteConfig = serde_json::from_str(&format!(r#"{{ {orbit} }}"#)).unwrap();
        assert_eq!(none.shape, None);

        // Serializes back to the kebab-case wire form sent in SatelliteInfo.
        assert_eq!(
            serde_json::to_string(&MarkerShape::AxesCube).unwrap(),
            "\"axes-cube\""
        );
    }

    #[test]
    fn attitude_config_defaults() {
        let json = r#"{ "inertia_diag": [100, 100, 50], "mass": 500 }"#;
        let att: AttitudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(att.inertia_diag, [100.0, 100.0, 50.0]);
        assert_eq!(att.inertia_off_diag, [0.0, 0.0, 0.0]);
        assert_eq!(att.mass, 500.0);
        assert_eq!(att.initial_quaternion, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(att.initial_angular_velocity, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn attitude_config_full() {
        let json = r#"{
            "inertia_diag": [100, 200, 300],
            "inertia_off_diag": [1.5, 0.5, -0.3],
            "mass": 1000,
            "initial_quaternion": [0.707, 0, 0.707, 0],
            "initial_angular_velocity": [0.01, -0.02, 0.03]
        }"#;
        let att: AttitudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(att.inertia_off_diag, [1.5, 0.5, -0.3]);
        assert!((att.initial_quaternion[0] - 0.707).abs() < 1e-9);
        assert!((att.initial_angular_velocity[2] - 0.03).abs() < 1e-9);
    }

    #[test]
    fn attitude_config_inertia_matrix() {
        let att = AttitudeConfig {
            inertia_diag: [10.0, 20.0, 30.0],
            inertia_off_diag: [1.0, 2.0, 3.0],
            mass: 100.0,
            initial_quaternion: [1.0, 0.0, 0.0, 0.0],
            initial_angular_velocity: [0.0, 0.0, 0.0],
        };
        let m = att.inertia_matrix();
        // Diagonal
        assert_eq!(m[(0, 0)], 10.0);
        assert_eq!(m[(1, 1)], 20.0);
        assert_eq!(m[(2, 2)], 30.0);
        // Symmetric off-diagonal
        assert_eq!(m[(0, 1)], 1.0);
        assert_eq!(m[(1, 0)], 1.0);
        assert_eq!(m[(0, 2)], 2.0);
        assert_eq!(m[(2, 0)], 2.0);
        assert_eq!(m[(1, 2)], 3.0);
        assert_eq!(m[(2, 1)], 3.0);
    }

    #[test]
    fn satellite_config_with_attitude() {
        let json = r#"{
            "satellites": [{
                "orbit": { "type": "circular", "altitude": 400 },
                "attitude": {
                    "inertia_diag": [100, 100, 50],
                    "mass": 500
                }
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        let att = config.satellites[0].attitude.as_ref().unwrap();
        assert_eq!(att.mass, 500.0);
        assert_eq!(att.initial_quaternion, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn satellite_config_without_attitude() {
        let json = r#"{
            "satellites": [{
                "orbit": { "type": "circular", "altitude": 400 }
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert!(config.satellites[0].attitude.is_none());
    }

    #[test]
    fn deserialize_controller_config() {
        let yaml = r#"
satellites:
  - orbit: { type: circular, altitude: 400 }
    attitude: { inertia_diag: [10, 10, 10], mass: 500 }
    controller:
      type: wasm
      path: plugin-sdk/examples/pd-rw-control/target/plugin.wasm
      config:
        kp: 1.0
        kd: 2.0
    sensors: [gyroscope, star_tracker]
    reaction_wheels:
      type: three_axis
      inertia: 0.01
      max_momentum: 1.0
      max_torque: 0.5
"#;
        let config: SimConfig = serde_yaml::from_str(yaml).unwrap();
        let sat = &config.satellites[0];

        // Controller
        let ctrl = sat.controller.as_ref().unwrap();
        assert!(
            matches!(ctrl, ControllerConfig::Wasm { path, .. } if path.contains("plugin.wasm"))
        );

        // Sensors
        let sensors = sat.sensors.as_ref().unwrap();
        assert_eq!(sensors.len(), 2);
        assert!(sensors.contains(&SensorChoice::Gyroscope));
        assert!(sensors.contains(&SensorChoice::StarTracker));

        // Reaction wheels
        let rw = sat.reaction_wheels.as_ref().unwrap();
        assert!(matches!(
            rw,
            ReactionWheelConfig::ThreeAxis { inertia, max_momentum, max_torque, .. }
            if (*inertia - 0.01).abs() < 1e-9
            && (*max_momentum - 1.0).abs() < 1e-9
            && (*max_torque - 0.5).abs() < 1e-9
        ));
    }

    #[test]
    fn deserialize_satellite_streams() {
        let toml = r#"
[[satellites]]
orbit = { type = "circular", altitude = 400 }
streams = ["comlink", "uart0"]
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        let body = KnownBody::Earth;
        let spec = config.satellites[0].to_satellite_spec(0, body, body.properties().mu);
        assert_eq!(
            spec.streams,
            vec!["comlink".to_string(), "uart0".to_string()]
        );
    }

    #[test]
    fn satellite_streams_default_empty() {
        let toml = r#"
[[satellites]]
orbit = { type = "circular", altitude = 400 }
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        let body = KnownBody::Earth;
        let spec = config.satellites[0].to_satellite_spec(0, body, body.properties().mu);
        assert!(spec.streams.is_empty());
    }

    #[test]
    fn deserialize_commands() {
        let toml = r#"
[[satellites]]
id = "sat-a"
[satellites.orbit]
type = "circular"
altitude = 500

[[command]]
t = 300.0
sat = "sat-a"
kind = "orts.cmd.set-mode.v1"
args = { mode = "nadir" }

[[command]]
t = 10.0
sat = "sat-a"
kind = "orts.cmd.ping.v1"
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.commands.len(), 2);
        // Raw declaration order preserved (scheduling sorts separately).
        assert!((config.commands[0].t - 300.0).abs() < 1e-9);
        assert_eq!(config.commands[0].sat, "sat-a");
        assert_eq!(config.commands[0].kind, "orts.cmd.set-mode.v1");
        // No args → null.
        assert!(config.commands[1].args.is_null());
    }

    #[test]
    fn commands_absent_by_default() {
        let json = r#"{ "satellites": [] }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert!(config.commands.is_empty());
    }

    #[test]
    fn deserialize_ground_station() {
        // Canonical TOML key is the singular `[[ground_station]]` (the
        // `ground_stations` Rust field is renamed); pin that it parses.
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[ground_station]]
name = "tokyo"
latitude_deg = 35.68
longitude_deg = 139.69
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.ground_stations.len(), 1);
        assert_eq!(config.ground_stations[0].name, "tokyo");
    }

    #[test]
    fn deserialize_magnetorquers() {
        // The MTQ block's canonical TOML key is `magnetorquers` (the `mtq`
        // Rust field is renamed to match the sibling `reaction_wheels`).
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[satellites.magnetorquers]
type = "three_axis"
max_moment = 10.0
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert!(config.satellites[0].mtq.is_some());
    }

    #[test]
    fn command_to_message_builds_keyvalue_payload() {
        let toml = r#"
t = 1.0
sat = "x"
kind = "orts.cmd.set-mode.v1"
args = { mode = "nadir", req-id = 7 }
"#;
        let cmd: CommandConfig = toml::from_str(toml).unwrap();
        let msg = cmd.to_message(2).unwrap();
        assert_eq!(msg.dst, NodeId::Satellite(2));
        assert_eq!(msg.src, NodeId::Ground);
        assert_eq!(msg.kind, "orts.cmd.set-mode.v1");
        assert_eq!(
            msg.payload.get("mode").and_then(Value::as_text),
            Some("nadir")
        );
        assert_eq!(
            msg.payload.get("req-id").and_then(Value::as_integer),
            Some(7)
        );
    }

    #[test]
    fn command_empty_args_is_empty_keyvalue() {
        let cmd = CommandConfig {
            t: 0.0,
            sat: "x".into(),
            kind: "k".into(),
            args: serde_json::Value::Null,
        };
        let msg = cmd.to_message(0).unwrap();
        assert!(matches!(msg.payload, Payload::KeyValue(ref v) if v.is_empty()));
    }

    #[test]
    fn command_rejects_non_scalar_arg() {
        let cmd = CommandConfig {
            t: 0.0,
            sat: "x".into(),
            kind: "k".into(),
            args: serde_json::json!({ "nested": { "a": 1 } }),
        };
        assert!(cmd.to_message(0).is_err());
    }

    #[test]
    fn serve_rejects_command_timeline() {
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[command]]
t = 1.0
sat = "sat-0"
kind = "orts.cmd.set-mode.v1"
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        // serve does not deliver config command timelines (run-only feature)
        // → reject loudly instead of silently dropping them.
        assert!(config.ensure_serve_supported().is_err());
    }

    #[test]
    fn serve_accepts_config_without_commands() {
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert!(config.ensure_serve_supported().is_ok());
    }

    #[test]
    fn command_rejects_out_of_range_integer() {
        // u64 > i64::MAX must be rejected, not silently wrapped to a
        // negative s64 (WIT `value.integer` is s64).
        let cmd = CommandConfig {
            t: 0.0,
            sat: "x".into(),
            kind: "k".into(),
            args: serde_json::json!({ "big": 9_223_372_036_854_775_808u64 }),
        };
        assert!(cmd.to_message(0).is_err());
    }

    fn config_with(extra: &str) -> SimConfig {
        let toml = format!(
            r#"
{extra}

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500
"#
        );
        toml::from_str(&toml).expect("test config should deserialize")
    }

    /// `dt <= 0` (and NaN) used to reach the integrator, where
    /// `while t < t_end` never advances, so `orts run` hung instead of
    /// reporting the bad config.
    #[test]
    fn validate_rejects_non_positive_dt() {
        for dt in ["0.0", "-1.0", "nan", "inf"] {
            let config = config_with(&format!("dt = {dt}"));
            let err = config
                .validate()
                .expect_err(&format!("dt = {dt} should be rejected"));
            assert!(err.contains("dt"), "dt = {dt} gave {err:?}");
        }
    }

    #[test]
    fn validate_accepts_positive_dt() {
        assert!(config_with("dt = 5.0").validate().is_ok());
    }

    /// A non-finite orbit number reaches the derived orbital period, and a NaN
    /// end time makes `while !group.all_finished()` loop forever.
    #[test]
    fn validate_rejects_non_finite_orbit_numbers() {
        for field in ["altitude", "inclination", "raan"] {
            // `altitude` is required, so only add the 500 km default when the
            // field under test is one of the optional ones.
            let base = if field == "altitude" {
                String::new()
            } else {
                "altitude = 500\n".to_string()
            };
            let toml = format!(
                r#"
dt = 10.0

[[satellites]]
[satellites.orbit]
type = "circular"
{base}{field} = nan
"#
            );
            let config: SimConfig = toml::from_str(&toml).expect("deserializes");
            let err = config
                .validate()
                .expect_err(&format!("{field} = nan should be rejected"));
            assert!(err.contains(field), "{field} gave {err:?}");
        }
    }

    /// `a = R_body + altitude`, so a large negative altitude has no orbit.
    #[test]
    fn validate_rejects_altitude_below_body_centre() {
        let toml = r#"
dt = 10.0
body = "earth"

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = -7000.0
"#;
        let config: SimConfig = toml::from_str(toml).expect("deserializes");
        let err = config.validate().expect_err("altitude below body centre");
        assert!(err.contains("semi-major axis"), "{err:?}");
    }

    #[test]
    fn validate_accepts_negative_altitude_above_body_centre() {
        // Physically underground but numerically a valid orbit; not this
        // check's job to reject.
        let toml = r#"
dt = 10.0
body = "earth"

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = -100.0
"#;
        let config: SimConfig = toml::from_str(toml).expect("deserializes");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn sample_period_validator() {
        assert!(validate_sample_period(0.1).is_ok());
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(validate_sample_period(dt).is_err(), "dt_ctrl = {dt}");
        }
    }

    /// Why the controlled-run loops validate each satellite's period rather
    /// than only the folded minimum: `f64::min` returns the *other* argument
    /// when one side is NaN, so a NaN period vanishes in the fold.
    #[test]
    fn fold_min_hides_a_nan_sample_period() {
        let periods = [f64::NAN, 0.1];
        let folded = periods.iter().copied().fold(f64::INFINITY, f64::min);
        assert_eq!(
            folded, 0.1,
            "f64::min drops the NaN instead of propagating it"
        );
        assert!(
            validate_sample_period(folded).is_ok(),
            "so validating only the fold result accepts a fleet containing a NaN period"
        );
        assert!(
            periods
                .iter()
                .copied()
                .any(|p| validate_sample_period(p).is_err()),
            "validating per satellite catches it"
        );
    }

    #[test]
    fn tolerance_validator_skips_rk4() {
        // RK4 never reads the tolerances, so unused zeros must stay valid.
        assert!(validate_tolerances(IntegratorChoice::Rk4, 0.0, 0.0).is_ok());
        assert!(validate_tolerances(IntegratorChoice::Dp45, 0.0, 0.0).is_err());
        assert!(validate_tolerances(IntegratorChoice::Dop853, 0.0, 0.0).is_err());
    }

    #[test]
    fn validate_accepts_rk4_with_zero_tolerances() {
        let config =
            config_with("dt = 10.0\n\n[integrator]\ntype = \"rk4\"\natol = 0.0\nrtol = 0.0");
        assert!(config.validate().is_ok(), "RK4 ignores tolerances");
    }

    /// `output_interval = 0` never reaches the fleet's end time in `run`;
    /// `stream_interval = 0` is a divisor in the serve loop's pacing, where it
    /// produces `NaN` and panics `Duration::from_secs_f64`.
    #[test]
    fn validate_rejects_non_positive_intervals() {
        for key in ["output_interval", "stream_interval", "duration"] {
            for value in ["0.0", "-1.0", "nan", "inf"] {
                let config = config_with(&format!("dt = 1.0\n{key} = {value}"));
                let err = config
                    .validate()
                    .expect_err(&format!("{key} = {value} should be rejected"));
                assert!(err.contains(key), "{key} = {value} gave {err:?}");
            }
        }
    }

    /// `SimParams::from_config` clamps `stream_interval` into
    /// `[dt, output_interval]`; inverted bounds used to panic inside `clamp`.
    #[test]
    fn validate_rejects_output_interval_below_dt() {
        let config = config_with("dt = 10.0\noutput_interval = 1.0");
        let err = config.validate().expect_err("output_interval < dt");
        assert!(
            err.contains("output_interval") && err.contains("dt"),
            "{err:?}"
        );
    }

    #[test]
    fn validate_accepts_output_interval_at_or_above_dt() {
        assert!(
            config_with("dt = 10.0\noutput_interval = 10.0")
                .validate()
                .is_ok()
        );
        assert!(
            config_with("dt = 10.0\noutput_interval = 60.0")
                .validate()
                .is_ok()
        );
    }

    /// With `atol == rtol == 0` the adaptive error scale is zero, so an
    /// exactly-zero state component makes the error norm `0 / 0`.
    #[test]
    fn validate_rejects_both_zero_tolerances() {
        let config = config_with("[integrator]\ntype = \"dp45\"\natol = 0.0\nrtol = 0.0");
        let err = config.validate().expect_err("both-zero tolerances");
        assert!(err.contains("atol") && err.contains("rtol"), "{err:?}");
    }

    #[test]
    fn validate_rejects_non_finite_tolerances() {
        for (atol, rtol) in [("nan", "1e-8"), ("1e-10", "-1.0"), ("1e-10", "inf")] {
            let config = config_with(&format!(
                "[integrator]\ntype = \"dp45\"\natol = {atol}\nrtol = {rtol}"
            ));
            assert!(
                config.validate().is_err(),
                "atol = {atol}, rtol = {rtol} should be rejected"
            );
        }
    }

    #[test]
    fn validate_accepts_one_zero_tolerance() {
        // Only one of the two needs to be positive.
        let config = config_with("[integrator]\ntype = \"dp45\"\natol = 0.0\nrtol = 1e-8");
        assert!(
            config.validate().is_ok(),
            "atol = 0 with rtol > 0 is usable"
        );
    }

    #[test]
    fn validate_rejects_non_finite_command_t() {
        // A non-finite `t` (TOML allows `nan`/`inf`) sorts but never becomes
        // `<= t_due`, so the command would be silently undelivered. Reject it.
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[command]]
t = nan
sat = "sat-0"
kind = "orts.cmd.set-mode.v1"
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_command_t() {
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[command]]
t = -1.0
sat = "sat-0"
kind = "orts.cmd.set-mode.v1"
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn controller_config_absent_by_default() {
        let json = r#"{
            "satellites": [{ "orbit": { "type": "circular", "altitude": 400 } }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).unwrap();
        assert!(config.satellites[0].controller.is_none());
        assert!(config.satellites[0].sensors.is_none());
        assert!(config.satellites[0].reaction_wheels.is_none());
    }

    #[test]
    fn load_yaml_file() {
        let dir =
            std::env::temp_dir().join(format!("orts-config-test-yaml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.yml");
        std::fs::write(
            &path,
            r#"
dt: 5.0
satellites:
  - orbit:
      type: circular
      altitude: 400.0
"#,
        )
        .unwrap();

        let config = SimConfig::load(&path).unwrap();
        assert!((config.dt - 5.0).abs() < 1e-9);
        assert_eq!(config.satellites.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn thruster_config_parses() {
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[satellites.thruster]
dry_mass = 400.0

[[satellites.thruster.thrusters]]
thrust_n = 10.0
isp_s = 230.0
direction_body = [1.0, 0.0, 0.0]

[[satellites.thruster.thrusters]]
thrust_n = 5.0
isp_s = 200.0
direction_body = [0.0, 1.0, 0.0]
offset_body = [0.1, 0.0, 0.0]
"#;
        let config: SimConfig = toml::from_str(toml).expect("parse");
        let t = config.satellites[0].thruster.as_ref().expect("thruster");
        assert_eq!(t.thrusters.len(), 2);
        assert!((t.dry_mass - 400.0).abs() < 1e-9);
        assert!((t.thrusters[0].thrust_n - 10.0).abs() < 1e-9);
        assert!(t.thrusters[0].offset_body.is_none());
        assert_eq!(t.thrusters[1].offset_body.unwrap(), [0.1, 0.0, 0.0]);
        t.validate().expect("valid");
    }

    #[test]
    fn thruster_config_rejects_empty_list() {
        let cfg = ThrusterConfig {
            thrusters: vec![],
            dry_mass: 0.0,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("must not be empty"), "msg: {err}");
    }

    #[test]
    fn thruster_config_rejects_zero_direction() {
        let cfg = ThrusterConfig {
            thrusters: vec![ThrusterSpecConfig {
                thrust_n: 10.0,
                isp_s: 230.0,
                direction_body: [0.0, 0.0, 0.0],
                offset_body: None,
            }],
            dry_mass: 0.0,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("direction_body"), "msg: {err}");
    }

    #[test]
    fn thruster_config_rejects_non_positive_thrust() {
        let cfg = ThrusterConfig {
            thrusters: vec![ThrusterSpecConfig {
                thrust_n: -1.0,
                isp_s: 230.0,
                direction_body: [1.0, 0.0, 0.0],
                offset_body: None,
            }],
            dry_mass: 0.0,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("thrust_n"), "msg: {err}");
    }

    #[test]
    fn thruster_config_rejects_nonfinite_offset() {
        let cfg = ThrusterConfig {
            thrusters: vec![ThrusterSpecConfig {
                thrust_n: 10.0,
                isp_s: 230.0,
                direction_body: [1.0, 0.0, 0.0],
                offset_body: Some([f64::NAN, 0.0, 0.0]),
            }],
            dry_mass: 0.0,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("offset_body"), "msg: {err}");
    }

    const PANEL_SAT: &str = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 20, 30], mass = 50 }

[[satellites.panels]]
area = 2.0
normal = [1.0, 0.0, 1.0]
cd = 2.2
specular = 0.2
diffuse = 0.1
cp_offset = [0.0, 1.5, 0.0]
"#;

    #[test]
    fn panels_reach_the_spec_as_a_shape() {
        let config: SimConfig = toml::from_str(PANEL_SAT).expect("parses");
        config.satellites[0].validate().expect("valid");
        let spec = config.satellites[0].to_satellite_spec(0, KnownBody::Earth, 398600.4418);
        let shape = spec.panels.expect("panels reach the spec");
        let orts::spacecraft::SpacecraftShape::Panels(panels) = shape else {
            panic!("expected a panel shape");
        };
        assert_eq!(panels.len(), 1);
        // `at_com` normalises, so a non-unit normal in config is fine and the
        // model constructors' unit-length assert cannot fire.
        assert!((panels[0].normal.magnitude() - 1.0).abs() < 1e-15);
        assert_eq!(panels[0].cp_offset, nalgebra::Vector3::new(0.0, 1.5, 0.0));
    }

    /// An explicit `panels = []` is a mistake, and the type has to keep it
    /// apart from an absent key: with `Vec` + `serde(default)` the two are the
    /// same value, so the empty list would read as "no panels" and the
    /// satellite would silently fall back to an isotropic cross-section.
    ///
    /// `null` is a separate case, pinned by
    /// [`a_null_panels_key_means_no_panels`].
    #[test]
    fn an_empty_panels_list_is_rejected() {
        let json = r#"{
            "satellites": [{
                "id": "a",
                "orbit": { "type": "circular", "altitude": 500 },
                "attitude": { "inertia_diag": [10, 10, 10], "mass": 50 },
                "panels": []
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("panels must not be empty"), "got: {err}");

        // Leaving the key out is the way to say "no panels".
        let without = json.replace(",\n                \"panels\": []", "");
        let config: SimConfig = serde_json::from_str(&without).expect("parses");
        config.satellites[0]
            .validate()
            .expect("valid without panels");
        assert!(config.satellites[0].panels.is_none());
    }

    /// `"panels": null` reads as no panels, the same as leaving the key out.
    ///
    /// Worth pinning because it changed: with `Vec` + `serde(default)` a JSON
    /// `null` was a parse error. It has to be accepted now, because `None`
    /// serialises back as `null` and `orts config example --format json` has to
    /// re-read its own output — the eleven sibling `Option` fields print `null`
    /// there too.
    ///
    /// The field's own doc says to omit the key rather than write `null`, since
    /// that doc ships into the generated TypeScript, where `#[ts(optional)]`
    /// renders `panels?: Array<PanelConfig>` with no `| null` — as it does for
    /// those eleven siblings.
    #[test]
    fn a_null_panels_key_means_no_panels() {
        let json = r#"{
            "satellites": [{
                "id": "a",
                "orbit": { "type": "circular", "altitude": 500 },
                "srp_area_to_mass": 0.02,
                "panels": null
            }]
        }"#;
        let config: SimConfig = serde_json::from_str(json).expect("parses");
        assert!(config.satellites[0].panels.is_none());
        config.satellites[0]
            .validate()
            .expect("no panels, so neither the attitude nor the conflict rule applies");
        let spec = config.satellites[0].to_satellite_spec(0, KnownBody::Earth, 398600.4418);
        assert!(
            spec.panels.is_none(),
            "a null key must not reach the dynamics as a panelled shape"
        );
    }

    fn panel_sat_toml(panel_body: &str) -> String {
        format!(
            r#"
[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 500 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 50 }}

[[satellites.panels]]
{panel_body}
"#
        )
    }

    /// Both faces present with the front's optics on each, for a config whose
    /// single panel carries an empty `back`.
    ///
    /// Asserting on the parsed `PanelBackConfig` alone would pass even if the
    /// fallback resolved to zero, so this goes through to the panels.
    fn assert_two_faces_copying_the_front(config: &SimConfig) {
        let spec = config.satellites[0].to_satellite_spec(0, KnownBody::Earth, 398600.4418);
        let orts::spacecraft::SpacecraftShape::Panels(panels) =
            spec.panels.expect("panels reach the spec")
        else {
            panic!("expected a panelled shape");
        };
        assert_eq!(panels.len(), 2, "an empty back table still adds a face");
        assert_eq!(panels[1].normal, -panels[0].normal);
        assert_eq!(
            panels[1].optics, panels[0].optics,
            "an empty back table copies the front's optics"
        );
        assert!((panels[1].optics.specular() - 0.1).abs() < 1e-15);
    }

    fn panels_of(toml_src: &str) -> Vec<orts::spacecraft::SurfacePanel> {
        let config: SimConfig = toml::from_str(toml_src).expect("parses");
        let spec = config.satellites[0].to_satellite_spec(0, KnownBody::Earth, 398600.4418);
        let orts::spacecraft::SpacecraftShape::Panels(panels) =
            spec.panels.expect("panels reach the spec")
        else {
            panic!("expected a panelled shape");
        };
        panels
    }

    /// A `back` table turns one written panel into the plate's two faces.
    #[test]
    fn a_back_table_produces_the_opposite_face() {
        let panels = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\ncp_offset = [0.0, 1.5, 0.0]\nback = { specular = 0.05, diffuse = 0.4 }",
        ));
        assert_eq!(panels.len(), 2);
        let (front, back) = (&panels[0], &panels[1]);

        assert_eq!(back.normal, -front.normal);
        assert_eq!(back.area, front.area);
        assert_eq!(back.cd, front.cd);
        assert_eq!(
            back.cp_offset, front.cp_offset,
            "one plate, so one centre of pressure"
        );
        assert!((front.optics.specular() - 0.1).abs() < 1e-15);
        assert!((back.optics.specular() - 0.05).abs() < 1e-15);
        assert!((back.optics.diffuse() - 0.4).abs() < 1e-15);
    }

    /// `half_extent` gives the panel a boundary, and the area follows from it.
    #[test]
    fn half_extent_gives_the_panel_an_outline() {
        let panels = panels_of(&panel_sat_toml(
            "normal = [1, 0, 0]\ncd = 2.2\nhalf_extent = [1.0, 2.0]\n\
             in_plane_x = [0, 1, 0]\ncp_offset = [0.0, 0.0, 1.5]",
        ));
        assert_eq!(panels.len(), 1);
        assert!(
            panels[0].outline.is_some(),
            "the outline has to reach the model"
        );
        assert!(
            (panels[0].area - 8.0).abs() < 1e-15,
            "area follows from the half-extents: got {}",
            panels[0].area
        );
        assert_eq!(
            panels[0].cp_offset,
            nalgebra::Vector3::new(0.0, 0.0, 1.5),
            "the outline is centred on the centre of pressure"
        );
    }

    /// Without it the panel is as before: an area, and no part in shadowing.
    #[test]
    fn area_alone_leaves_the_panel_without_an_outline() {
        let panels = panels_of(&panel_sat_toml("area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2"));
        assert!(panels[0].outline.is_none());
    }

    /// The two ways to give the size are exclusive: which one drives the force
    /// would otherwise be unreadable.
    #[test]
    fn area_beside_half_extent_is_rejected() {
        let toml_src = panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nhalf_extent = [1.0, 2.0]\n\
             in_plane_x = [0, 1, 0]",
        );
        let config: SimConfig = toml::from_str(&toml_src).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("both give the size"), "got: {err}");
    }

    /// The outline keys come as a pair, and neither works alone.
    #[test]
    fn a_lone_outline_key_is_rejected() {
        for (body, expected) in [
            (
                "normal = [1, 0, 0]\ncd = 2.2\nhalf_extent = [1.0, 2.0]",
                "needs in_plane_x",
            ),
            (
                "normal = [1, 0, 0]\ncd = 2.2\nin_plane_x = [0, 1, 0]",
                "describes no extent",
            ),
        ] {
            let config: SimConfig = toml::from_str(&panel_sat_toml(body)).expect("parses");
            let err = config.satellites[0].validate().unwrap_err();
            assert!(err.contains(expected), "{body}: got {err}");
        }
    }

    /// Neither way to give the size is an error too, rather than a zero-area
    /// panel that produces no force and says nothing.
    #[test]
    fn a_panel_with_no_size_at_all_is_rejected() {
        let config: SimConfig =
            toml::from_str(&panel_sat_toml("normal = [1, 0, 0]\ncd = 2.2")).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("needs area, or half_extent"), "got: {err}");
    }

    /// A non-positive half-extent is caught before `rectangle` asserts on it.
    #[test]
    fn a_non_positive_half_extent_is_rejected() {
        let config: SimConfig = toml::from_str(&panel_sat_toml(
            "normal = [1, 0, 0]\ncd = 2.2\nhalf_extent = [0.0, 2.0]\nin_plane_x = [0, 1, 0]",
        ))
        .expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("half_extent components"), "got: {err}");
    }

    /// `two_sided = true` is the short way to say what an empty `back` says.
    ///
    /// Both spellings stay: `back` is the override, and reaching for it to say
    /// "no override" reads oddly, which is what the flag is for.
    #[test]
    fn two_sided_gives_the_plate_an_identical_far_face() {
        let by_flag = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\n\
             cp_offset = [0.0, 1.5, 0.0]\ntwo_sided = true",
        ));
        let by_empty_back = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\n\
             cp_offset = [0.0, 1.5, 0.0]\nback = {}",
        ));

        assert_eq!(by_flag.len(), 2);
        assert_eq!(by_flag, by_empty_back, "the two spellings have to agree");

        let (front, back) = (&by_flag[0], &by_flag[1]);
        assert_eq!(back.normal, -front.normal);
        assert_eq!(back.optics, front.optics);
        assert_eq!(back.cp_offset, front.cp_offset);
    }

    /// `two_sided = false` is the same as leaving it out.
    #[test]
    fn two_sided_false_leaves_one_face() {
        let panels = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\ntwo_sided = false",
        ));
        assert_eq!(panels.len(), 1);
    }

    /// `two_sided = false` beside `back` is the one combination that disagrees
    /// with itself, so it is rejected rather than resolved.
    ///
    /// The alternative was to let `back` win, which discards a value the reader
    /// wrote — the failure mode `panels = []` and the isotropic-parameter
    /// conflict are both rejected for. Distinguishing it from an absent key is
    /// why the field is `Option<bool>` rather than `bool`.
    #[test]
    fn two_sided_false_beside_back_is_rejected() {
        let toml_src = panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\n\
             two_sided = false\nback = { specular = 0.05 }",
        );
        let config: SimConfig = toml::from_str(&toml_src).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("two_sided = false"), "got: {err}");
        assert!(err.contains("keep one"), "got: {err}");

        // Absent is not `false`: it is how every single-sided panel is written,
        // and `back` alone has to keep working.
        let ok =
            panel_sat_toml("area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nback = { specular = 0.05 }");
        let config: SimConfig = toml::from_str(&ok).expect("parses");
        config.satellites[0]
            .validate()
            .expect("back alone is valid");
        assert_eq!(panels_of(&ok).len(), 2);
    }

    /// The flag beside `back` is redundant, not contradictory: both ask for a
    /// far face, and `back` says how it differs.
    #[test]
    fn two_sided_beside_back_keeps_the_override() {
        let with_flag = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\n\
             two_sided = true\nback = { specular = 0.05 }",
        ));
        let without = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\n\
             back = { specular = 0.05 }",
        ));
        assert_eq!(with_flag, without);
        assert_eq!(with_flag.len(), 2);
        assert!((with_flag[1].optics.specular() - 0.05).abs() < 1e-15);
    }

    /// An empty `back` gives the plate two identical sides.
    ///
    /// The optics fields are `Option` for this: as bare `f64` with
    /// `serde(default)`, `back = {}` would have produced a black surface
    /// instead of a copy, which is not what writing an empty table asks for.
    #[test]
    fn an_empty_back_table_copies_the_front_optics() {
        let panels = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\nback = {}",
        ));
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[1].optics, panels[0].optics);
        assert!((panels[1].optics.specular() - 0.1).abs() < 1e-15);
        assert!((panels[1].optics.diffuse() - 0.2).abs() < 1e-15);
    }

    /// One field in `back` leaves the other inherited, rather than zeroed.
    #[test]
    fn a_partial_back_table_inherits_the_other_field() {
        let panels = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\nback = { specular = 0.5 }",
        ));
        assert!((panels[1].optics.specular() - 0.5).abs() < 1e-15);
        assert!(
            (panels[1].optics.diffuse() - 0.2).abs() < 1e-15,
            "diffuse was not written for the back, so it comes from the front"
        );
    }

    /// Without `back` a panel stays one face, as before.
    #[test]
    fn a_panel_without_a_back_table_stays_one_face() {
        let panels = panels_of(&panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2",
        ));
        assert_eq!(panels.len(), 1);
    }

    /// The sum is checked after inheritance, so a `back` that is fine on its own
    /// but not once combined with the front is still rejected — and the message
    /// says which face.
    #[test]
    fn a_back_table_over_one_once_inherited_is_rejected() {
        let toml_src = panel_sat_toml(
            "area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.1\ndiffuse = 0.2\nback = { specular = 0.9 }",
        );
        let config: SimConfig = toml::from_str(&toml_src).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("panels[0].back.specular"), "got: {err}");
        assert!(err.contains("at most 1"), "got: {err}");

        // The same value on the front alone is fine, so the error is about the
        // pair rather than about `0.9`.
        let ok = panel_sat_toml("area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.9");
        let config: SimConfig = toml::from_str(&ok).expect("parses");
        config.satellites[0].validate().expect("valid");
    }

    /// A negative back reflectivity names the back, not the front.
    #[test]
    fn a_negative_back_reflectivity_names_the_back() {
        let toml_src =
            panel_sat_toml("area = 4.0\nnormal = [1, 0, 0]\ncd = 2.2\nback = { diffuse = -0.1 }");
        let config: SimConfig = toml::from_str(&toml_src).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(
            err.contains("panels[0].back.diffuse must be non-negative"),
            "got: {err}"
        );
    }

    /// JSON and YAML express an empty `back` too, so the meaning cannot depend
    /// on the file format.
    #[test]
    fn an_empty_back_table_works_in_json_and_yaml() {
        let json = r#"{"satellites":[{"id":"a","orbit":{"type":"circular","altitude":500},
            "attitude":{"inertia_diag":[10,10,10],"mass":50},
            "panels":[{"area":4.0,"normal":[1,0,0],"cd":2.2,"specular":0.1,"back":{}}]}]}"#;
        let config: SimConfig = serde_json::from_str(json).expect("json parses");
        assert_two_faces_copying_the_front(&config);

        let yaml = r#"
satellites:
  - id: a
    orbit:
      type: circular
      altitude: 500
    attitude:
      inertia_diag: [10, 10, 10]
      mass: 50
    panels:
      - area: 4.0
        normal: [1, 0, 0]
        cd: 2.2
        specular: 0.1
        back: {}
"#;
        let config: SimConfig = serde_yaml::from_str(yaml).expect("yaml parses");
        assert_two_faces_copying_the_front(&config);
    }

    #[test]
    fn panels_without_attitude_is_rejected() {
        let toml_src = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }

[[satellites.panels]]
area = 2.0
normal = [1.0, 0.0, 0.0]
cd = 2.2
"#;
        let config: SimConfig = toml::from_str(toml_src).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("panels requires attitude"), "got: {err}");
    }

    #[test]
    fn panels_alongside_an_isotropic_parameter_is_rejected() {
        for (key, value) in [
            ("srp_area_to_mass", "0.02"),
            ("srp_cr", "1.5"),
            ("ballistic_coeff", "0.01"),
        ] {
            let toml_src = format!(
                r#"
[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 500 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 50 }}
{key} = {value}

[[satellites.panels]]
area = 2.0
normal = [1.0, 0.0, 0.0]
cd = 2.2
"#
            );
            let config: SimConfig = toml::from_str(&toml_src).expect("parses");
            let err = config.satellites[0].validate().unwrap_err();
            assert!(
                err.contains(key) && err.contains("same force"),
                "{key}: got {err}"
            );
        }
    }

    #[test]
    fn a_malformed_panel_is_rejected_rather_than_panicking() {
        // `SurfacePanel::at_com` and `PanelOptics::new` both assert, so config
        // has to catch these first and say what is wrong.
        let cases = [
            (
                "area = 0.0\nnormal = [1, 0, 0]\ncd = 2.2",
                "area must be positive",
            ),
            (
                "area = 2.0\nnormal = [0, 0, 0]\ncd = 2.2",
                "non-zero finite vector",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = -1.0",
                "panels[0].cd must be non-negative",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = 0.8\ndiffuse = 0.5",
                "panels[0].specular + panels[0].diffuse must be at most 1",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = -0.1",
                "panels[0].specular must be non-negative",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\ndiffuse = -0.1",
                "panels[0].diffuse must be non-negative",
            ),
            // TOML has `nan` and `inf` literals, so the finiteness checks are
            // reachable from a config file, not only from a programmatic
            // construction. (JSON has no literal: `1e400` there is a parse
            // error, "number out of range", before validation sees it.)
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\ncp_offset = [nan, 0.0, 0.0]",
                "panels[0].cp_offset components must be finite",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\nspecular = nan",
                "panels[0].specular must be finite",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = 2.2\ndiffuse = inf",
                "panels[0].diffuse must be finite",
            ),
            (
                "area = inf\nnormal = [1, 0, 0]\ncd = 2.2",
                "panels[0].area must be positive and finite",
            ),
            (
                "area = 2.0\nnormal = [1, 0, 0]\ncd = nan",
                "panels[0].cd must be non-negative and finite",
            ),
            (
                "area = 2.0\nnormal = [nan, 0, 0]\ncd = 2.2",
                "panels[0].normal must be a non-zero finite vector",
            ),
        ];
        // Each expectation names its field: "must be non-negative" alone is
        // satisfied by the `cd` message too, so it would not tell the optics
        // checks apart from it or from each other.
        for (panel_body, expected) in cases {
            let toml_src = format!(
                r#"
[[satellites]]
id = "a"
orbit = {{ type = "circular", altitude = 500 }}
attitude = {{ inertia_diag = [10, 10, 10], mass = 50 }}

[[satellites.panels]]
{panel_body}
"#
            );
            let config: SimConfig =
                toml::from_str(&toml_src).unwrap_or_else(|e| panic!("{panel_body}: {e}"));
            let err = config.satellites[0].validate().unwrap_err();
            assert!(err.contains(expected), "{panel_body}: got {err}");
        }
    }

    #[test]
    fn disturbances_without_attitude_is_rejected() {
        let toml = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
disturbances = { gravity_gradient = true }
"#;
        let config: SimConfig = toml::from_str(toml).expect("parses");
        let err = config.satellites[0].validate().unwrap_err();
        assert!(err.contains("disturbances requires attitude"), "got: {err}");
    }

    #[test]
    fn disturbances_with_attitude_is_accepted_and_defaults_on() {
        let toml = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
disturbances = {}
"#;
        let config: SimConfig = toml::from_str(toml).expect("parses");
        config.satellites[0].validate().expect("valid");
        let d = config.satellites[0].disturbances.as_ref().expect("present");
        assert!(
            d.gravity_gradient,
            "an empty disturbances table should keep the historical default on"
        );
    }

    /// No `disturbances` table at all must reach the same selection as an empty
    /// one, so adding the table is not what turns the torque on.
    #[test]
    fn omitting_disturbances_matches_the_default() {
        let toml = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
"#;
        let config: SimConfig = toml::from_str(toml).expect("parses");
        let spec = config.satellites[0].to_satellite_spec(0, KnownBody::Earth, 398600.4418);
        assert_eq!(spec.disturbances, DisturbanceTorques::default());
        assert!(spec.disturbances.gravity_gradient);
    }

    #[test]
    fn sim_config_validate_surfaces_satellite_index() {
        // Two satellites, the second with invalid thruster config. Ensure
        // the error message points at the right index so serve-path WebSocket
        // users can find the offending entry.
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 600

[satellites.thruster]

[[satellites.thruster.thrusters]]
thrust_n = 10.0
isp_s = 230.0
direction_body = [0.0, 0.0, 0.0]
"#;
        let config: SimConfig = toml::from_str(toml).expect("parse");
        let err = config.validate().unwrap_err();
        assert!(
            err.contains("satellites[1]"),
            "expected index in error: {err}"
        );
        assert!(err.contains("direction_body"), "msg: {err}");
    }

    #[test]
    fn thruster_config_load_surfaces_validation_error() {
        let toml = r#"
[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[satellites.thruster]

[[satellites.thruster.thrusters]]
thrust_n = 10.0
isp_s = 230.0
direction_body = [0.0, 0.0, 0.0]
"#;
        let dir = std::env::temp_dir().join(format!("orts_config_thr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SimConfig::load(&path).unwrap_err();
        assert!(err.contains("direction_body"), "msg: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The name clap shows for a `ValueEnum` variant — the exact spelling a
    /// `--integrator` / `--atmosphere` flag accepts.
    fn choice_name<T: ValueEnum>(v: &T) -> String {
        v.to_possible_value()
            .expect("no choice variant is skipped")
            .get_name()
            .to_string()
    }

    /// A config file must accept exactly the spellings the equivalent CLI flag
    /// accepts, and resolve each to the same model. The reverse inclusion holds
    /// by construction (both sides go through the one `ValueEnum` impl); this
    /// pins it, so adding a variant to only one side fails here.
    #[test]
    fn config_accepts_exactly_the_cli_choice_sets() {
        for variant in IntegratorChoice::value_variants() {
            let name = choice_name(variant);
            let config = config_with(&format!("[integrator]\ntype = \"{name}\""));
            assert_eq!(
                choice_name(&config.integrator_choice()),
                name,
                "config resolved integrator '{name}' to a different method"
            );
            config.validate().unwrap_or_else(|e| {
                panic!("integrator '{name}' is a CLI value but config rejects it: {e}")
            });
        }
        for variant in AtmosphereChoice::value_variants() {
            let name = choice_name(variant);
            let config = config_with(&format!("atmosphere = \"{name}\""));
            assert_eq!(
                choice_name(&config.atmosphere_choice()),
                name,
                "config resolved atmosphere '{name}' to a different model"
            );
            config.validate().unwrap_or_else(|e| {
                panic!("atmosphere '{name}' is a CLI value but config rejects it: {e}")
            });
        }
    }

    /// `--integrator dop835` is rejected by clap; the config spelling used to
    /// fall back to dp45 and exit 0, integrating with a different method than
    /// the one written down.
    #[test]
    fn unknown_integrator_type_is_rejected() {
        let err = toml::from_str::<SimConfig>("[integrator]\ntype = \"dop835\"\n")
            .expect_err("an unknown integrator must not deserialize")
            .to_string();
        assert!(err.contains("dop835"), "error should name the typo: {err}");
        assert!(
            err.contains("dop853"),
            "error should list the legal spellings: {err}"
        );
    }

    /// The same for `atmosphere`, where the old fallback to the exponential
    /// model substituted the drag physics silently — nothing in the run
    /// summary echoes the atmosphere model.
    #[test]
    fn unknown_atmosphere_is_rejected() {
        for typo in ["nrlmsise0", "harris_priester", "NRLMSISE00", ""] {
            let err = toml::from_str::<SimConfig>(&format!("atmosphere = \"{typo}\"\n"))
                .expect_err("an unknown atmosphere must not deserialize")
                .to_string();
            assert!(
                err.contains("harris-priester"),
                "error should list the legal spellings: {err}"
            );
        }
    }

    /// A hand-built config (no deserialization) is caught by `validate`
    /// instead, so `run`/`serve` cannot reach the model resolution with an
    /// unknown spelling.
    #[test]
    fn validate_rejects_an_unknown_model_on_a_hand_built_config() {
        let mut config = config_with("");
        config.atmosphere = "nrlmsise0".into();
        let err = config.validate().expect_err("unknown atmosphere");
        assert!(err.contains("nrlmsise0"), "msg: {err}");

        let mut config = config_with("");
        config.integrator.kind = "dop835".into();
        let err = config.validate().expect_err("unknown integrator");
        assert!(err.contains("dop835"), "msg: {err}");
    }

    /// The resolution itself has no fallback arm any more: a model that was
    /// never validated aborts loudly rather than quietly becoming another one.
    #[test]
    #[should_panic(expected = "unknown atmosphere 'nrlmsise0'")]
    fn atmosphere_choice_has_no_silent_fallback() {
        let mut config = config_with("");
        config.atmosphere = "nrlmsise0".into();
        let _ = config.atmosphere_choice();
    }

    /// A key nothing reads is named, at whatever depth it sits.
    ///
    /// Dropping it in silence is indistinguishable from never writing it:
    /// `duraton = 100` ran for one orbital period and reported success. The file
    /// still loads, so a config written for a newer `orts` runs here; what
    /// changes is that the key is reported.
    #[test]
    fn unread_keys_are_named() {
        let cases = [
            ("top level", "duraton = 100.0\n"),
            (
                "satellite",
                "[[satellites]]\naltitide = 400\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\n",
            ),
            ("integrator", "[integrator]\natoll = 1.0e-9\n"),
            (
                "attitude",
                "[[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\n\
                 \n[satellites.attitude]\ninertia_diag = [10, 10, 10]\nmass = 100\nmas = 50\n",
            ),
            (
                "ground station",
                "[[ground_station]]\nname = \"gs\"\nlatitude_deg = 35.0\nlongitude_deg = 139.0\nelevation_deg = 5.0\n",
            ),
            (
                "command",
                "[[command]]\nt = 1.0\nsat = \"a\"\nkind = \"x\"\nargs = {}\nkid = \"y\"\n",
            ),
        ];
        for (label, toml) in cases {
            let dir = std::env::temp_dir().join(format!(
                "orts-unread-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            let path = dir.join("sim.toml");
            std::fs::write(&path, toml).expect("write config");

            let loaded = SimConfig::load_with_warnings(&path)
                .unwrap_or_else(|e| panic!("{label}: the file should still load: {e}"));
            assert!(
                !loaded.unread_keys.is_empty(),
                "{label}: the unread key must be named"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// A `type`-tagged block refuses an unknown key rather than reporting it.
    ///
    /// `serde_ignored` cannot see inside an internally tagged enum, so there is
    /// nothing to report there: serde buffers the variant's content and replays
    /// it. A dropped `inclinaton = 51.6` leaves the orbit equatorial, so those
    /// blocks reject instead of saying nothing.
    #[test]
    fn a_type_tagged_block_refuses_an_unknown_key() {
        let cases = [
            (
                "orbit",
                "[[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\ninclinaton = 51.6\n",
                "inclinaton",
            ),
            (
                "controller",
                "[[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\n\
                 \n[satellites.controller]\ntype = \"wasm\"\npath = \"p.wasm\"\npth = \"q.wasm\"\n",
                "pth",
            ),
            (
                "reaction_wheels",
                "[[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\n\
                 \n[satellites.reaction_wheels]\ntype = \"three_axis\"\ninertia = 1e-4\n\
                 max_momentum = 0.03\nmax_torqe = 0.001\n",
                "max_torqe",
            ),
            (
                "magnetorquers",
                "[[satellites]]\n[satellites.orbit]\ntype = \"circular\"\naltitude = 500\n\
                 \n[satellites.magnetorquers]\ntype = \"three_axis\"\nmax_momnet = 0.2\n",
                "max_momnet",
            ),
        ];
        for (label, toml_src, key) in cases {
            let err = toml::from_str::<SimConfig>(toml_src)
                .map(|c| format!("{c:?}"))
                .expect_err(&format!("{label}: an unknown key here must be refused"))
                .to_string();
            assert!(
                err.contains("unknown field") && err.contains(key),
                "{label}: expected an unknown-field error naming `{key}`, got {err}"
            );
        }
    }

    /// The named path is the key's, so a typo can be found from it.
    #[test]
    fn an_unread_key_is_named_by_its_path() {
        let dir = std::env::temp_dir().join(format!(
            "orts-unread-path-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sim.toml");
        std::fs::write(
            &path,
            "duraton = 100.0

[[satellites]]
altitide = 400
             [satellites.orbit]
type = \"circular\"
altitude = 500
",
        )
        .expect("write config");

        let loaded = SimConfig::load_with_warnings(&path).expect("the file loads");
        assert_eq!(loaded.unread_keys, vec!["duraton", "satellites.0.altitide"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A config whose every key is read reports nothing.
    #[test]
    fn a_config_read_whole_names_no_key() {
        let dir = std::env::temp_dir().join(format!(
            "orts-unread-none-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sim.toml");
        std::fs::write(
            &path,
            "dt = 1.0
duration = 100.0

[[satellites]]
id = \"a\"
             [satellites.orbit]
type = \"circular\"
altitude = 500
",
        )
        .expect("write config");

        let loaded = SimConfig::load_with_warnings(&path).expect("the file loads");
        assert_eq!(loaded.unread_keys, Vec::<String>::new());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A JSON file with anything after the config is refused.
    ///
    /// Reading the keys nothing reads means driving `serde_json`'s
    /// `Deserializer` instead of calling `from_str`, and only `from_str` ends by
    /// checking that the input is spent. A second object, or a truncated edit
    /// that left the old text behind, would otherwise load as though the file
    /// stopped where the config did.
    #[test]
    fn a_json_config_with_content_after_it_is_refused() {
        let dir = std::env::temp_dir().join(format!(
            "orts-json-trailing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let config =
            r#"{"dt":1.0,"satellites":[{"id":"a","orbit":{"type":"circular","altitude":400}}]}"#;
        let path = dir.join("sim.json");

        std::fs::write(&path, config).expect("write config");
        SimConfig::load_with_warnings(&path).expect("the config alone loads");

        for trailing in ["{\"dt\":99.0}", "this is not json"] {
            std::fs::write(&path, format!("{config}{trailing}")).expect("write config");
            let err = SimConfig::load_with_warnings(&path)
                .expect_err("content after the config must be refused");
            assert!(err.contains("JSON"), "msg: {err}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two satellites resolving to one id share the recording entity path and
    /// the CSV section, and `[[command]]` reaches only whichever one won the
    /// id → index map. Reject the fleet instead.
    #[test]
    fn validate_rejects_duplicate_satellite_ids() {
        let toml = r#"
[[satellites]]
id = "a"
[satellites.orbit]
type = "circular"
altitude = 500

[[satellites]]
id = "a"
[satellites.orbit]
type = "circular"
altitude = 800
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        let err = config
            .validate()
            .expect_err("a duplicate satellite id must be rejected");
        assert!(err.contains("duplicate satellite id 'a'"), "msg: {err}");
        assert!(
            err.contains("satellites[1]") && err.contains("satellites[0]"),
            "error should name both entries: {err}"
        );
    }

    /// The collision an explicit id can have with the `sat-{index}` default of
    /// an id-less entry: neither id is written twice, so it is invisible in the
    /// config file.
    #[test]
    fn validate_rejects_an_id_colliding_with_the_auto_default() {
        let toml = r#"
[[satellites]]
id = "sat-1"
[satellites.orbit]
type = "circular"
altitude = 500

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 800
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        let err = config
            .validate()
            .expect_err("an id colliding with the auto default must be rejected");
        assert!(err.contains("duplicate satellite id 'sat-1'"), "msg: {err}");
        assert!(
            err.contains("no `id`"),
            "error should point at the defaulted entry: {err}"
        );
    }

    /// The ordinary multi-satellite fleet stays valid, including the all-auto
    /// case (`sat-0`, `sat-1`, …).
    #[test]
    fn validate_accepts_distinct_satellite_ids() {
        let toml = r#"
[[satellites]]
id = "iss"
[satellites.orbit]
type = "circular"
altitude = 400

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 500

[[satellites]]
[satellites.orbit]
type = "circular"
altitude = 800
"#;
        let config: SimConfig = toml::from_str(toml).unwrap();
        config.validate().expect("distinct ids must be accepted");
        let ids: Vec<String> = config
            .satellites
            .iter()
            .enumerate()
            .map(|(i, s)| s.resolved_id(i))
            .collect();
        assert_eq!(ids, ["iss", "sat-1", "sat-2"]);
    }

    fn attitude(inertia_diag: [f64; 3], inertia_off_diag: [f64; 3], mass: f64) -> AttitudeConfig {
        AttitudeConfig {
            inertia_diag,
            inertia_off_diag,
            mass,
            initial_quaternion: default_identity_quat(),
            initial_angular_velocity: [0.0; 3],
        }
    }

    /// `SpacecraftDynamics::new` inverts the inertia tensor with
    /// `try_inverse().expect(...)`, so a singular tensor used to abort the
    /// process (exit 101) instead of being reported as bad input.
    #[test]
    fn attitude_validate_rejects_singular_inertia() {
        let err = attitude([0.0, 0.0, 0.0], [0.0; 3], 100.0)
            .validate()
            .expect_err("a zero inertia tensor must be rejected");
        assert!(err.contains("inertia tensor"), "msg: {err}");

        // Singular through the off-diagonals only: [[1,1,0],[1,1,0],[0,0,1]]
        // has principal moments (0, 1, 2), which `inertia_diag` alone does not
        // show.
        let err = attitude([1.0, 1.0, 1.0], [1.0, 0.0, 0.0], 100.0)
            .validate()
            .expect_err("an off-diagonal singular tensor must be rejected");
        assert!(err.contains("inertia tensor"), "msg: {err}");
    }

    /// An indefinite tensor is invertible — a determinant test passes it — but
    /// a negative principal moment is negative mass off that axis.
    #[test]
    fn attitude_validate_rejects_indefinite_inertia() {
        let indefinite = attitude([1.0, 1.0, 1.0], [2.0, 0.0, 0.0], 100.0);
        assert!(
            indefinite.inertia_matrix().try_inverse().is_some(),
            "this tensor is invertible: only the eigenvalues expose it"
        );
        let err = indefinite
            .validate()
            .expect_err("an indefinite tensor must be rejected");
        assert!(err.contains("positive definite"), "msg: {err}");
    }

    /// `I1 + I2 >= I3` holds for every mass distribution, so a config that
    /// breaks it describes no spacecraft. Checked on the principal moments,
    /// not on `inertia_diag`: here the diagonal alone satisfies the inequality
    /// while the tensor's eigenvalues (1, 1, 5) do not.
    #[test]
    fn attitude_validate_rejects_triangle_inequality_violation() {
        let err = attitude([1.0, 1.0, 5.0], [0.0; 3], 100.0)
            .validate()
            .expect_err("I1 + I2 < I3 must be rejected");
        assert!(err.contains("triangle inequality"), "msg: {err}");

        let off_diag = attitude([3.0, 3.0, 1.0], [2.0, 0.0, 0.0], 100.0);
        let mut moments: Vec<f64> = off_diag
            .inertia_matrix()
            .symmetric_eigenvalues()
            .iter()
            .copied()
            .collect();
        moments.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            (moments[2] - 5.0).abs() < 1e-9 && moments[0] + moments[1] < moments[2],
            "test fixture should violate the inequality only through the off-diagonals: {moments:?}"
        );
        let err = off_diag
            .validate()
            .expect_err("an off-diagonal triangle violation must be rejected");
        assert!(err.contains("triangle inequality"), "msg: {err}");
    }

    /// Equality is the flat-plate (lamina) limit: physically attainable, and
    /// the tensor is still invertible, so it stays accepted.
    #[test]
    fn attitude_validate_accepts_the_lamina_boundary() {
        attitude([1.0, 2.0, 3.0], [0.0; 3], 100.0)
            .validate()
            .expect("I1 + I2 == I3 is a flat plate, not an impossible body");
        // Scale-invariant: the slack is relative, so a large plate is accepted
        // too.
        attitude([1e8, 2e8, 3e8], [0.0; 3], 100.0)
            .validate()
            .expect("the triangle slack must be relative, not absolute");
    }

    #[test]
    fn attitude_validate_rejects_non_positive_mass() {
        for mass in [0.0, -1.0, f64::NAN] {
            let err = attitude([10.0, 10.0, 10.0], [0.0; 3], mass)
                .validate()
                .expect_err("non-positive mass must be rejected");
            assert!(
                err.contains("mass") || err.contains("`mass`"),
                "msg for {mass}: {err}"
            );
        }
    }

    /// A zero quaternion normalizes to NaN in `AttitudeState::orientation()`,
    /// so the attitude is NaN from the first step with no error anywhere.
    #[test]
    fn attitude_validate_rejects_a_zero_or_non_finite_quaternion() {
        let mut att = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
        att.initial_quaternion = [0.0; 4];
        let err = att
            .validate()
            .expect_err("a zero quaternion must be rejected");
        assert!(err.contains("initial_quaternion"), "msg: {err}");

        att.initial_quaternion = [1.0, f64::NAN, 0.0, 0.0];
        let err = att
            .validate()
            .expect_err("a non-finite quaternion must be rejected");
        assert!(err.contains("initial_quaternion"), "msg: {err}");
    }

    /// Finite but absurd magnitudes must come back as an error, not as a
    /// panic from the eigenvalue solver or a NaN slipping through.
    #[test]
    fn attitude_validate_rejects_unusable_magnitudes() {
        let huge = attitude([f64::MAX, f64::MAX, f64::MAX], [f64::MAX, 0.0, 0.0], 100.0);
        let err = huge
            .validate()
            .expect_err("an overflowing tensor must be rejected");
        // The inverse check reaches it first: cofactors of `f64::MAX` overflow,
        // so `I · I⁻¹` is nowhere near the identity. Either guard is a correct
        // rejection; what matters is that it is an error and not a panic from
        // the eigenvalue solver.
        assert!(err.contains("inertia tensor"), "got: {err}");

        // Finite principal moments whose determinant still overflows:
        // `try_inverse` returns `Some` for a non-zero (infinite) determinant,
        // so the inverse comes back unusable rather than absent.
        let overflowing_det = attitude([1e308, 1e308, 1e308], [0.0; 3], 100.0);
        let moments = overflowing_det.inertia_matrix().symmetric_eigenvalues();
        assert!(
            moments.iter().all(|m| m.is_finite()),
            "fixture must pass the diagonalization guard: {moments:?}"
        );
        let err = overflowing_det
            .validate()
            .expect_err("an inverse full of non-finite entries must be rejected");
        assert!(err.contains("cannot be inverted"), "msg: {err}");

        // A quaternion whose squared norm overflows: normalization divides
        // every component by infinity.
        let mut huge_quat = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
        huge_quat.initial_quaternion = [1e200, 1e200, 0.0, 0.0];
        let err = huge_quat
            .validate()
            .expect_err("a quaternion whose squared norm overflows must be rejected");
        assert!(err.contains("initial_quaternion"), "msg: {err}");

        // Components small enough that the squared norm underflows to zero:
        // `orientation()` would divide by it.
        let mut denormal = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
        denormal.initial_quaternion = [1e-200, 0.0, 0.0, 0.0];
        let err = denormal
            .validate()
            .expect_err("a quaternion whose squared norm underflows must be rejected");
        assert!(err.contains("initial_quaternion"), "msg: {err}");
    }

    /// The quaternion bound must be exactly what the dynamics break on. `run`
    /// and `serve` divide by the norm once ([`normalized_initial_quaternion`],
    /// which is the same normalization) before storing, and every consumer then
    /// reads the state through `orientation()`, which normalizes again. So the
    /// config check is correct when it accepts a quaternion exactly when that
    /// normalization yields a finite unit quaternion — checked here by feeding
    /// the raw components to `orientation()`, the one call whose result the
    /// config cannot influence. Cross-checked
    /// against the real call rather than against a threshold on the input,
    /// because the interesting boundary is not where the arithmetic fails
    /// outright: `[1e-160, 0, 0, 0]` looks harmless and does produce a finite
    /// result, but its squared norm is subnormal deeply enough that the result
    /// has a norm of 1.0000056. A squared norm that is merely subnormal is fine,
    /// which is why the bound is on the normalized result and not on that sum.
    #[test]
    fn accepted_quaternions_are_exactly_the_normalizable_ones() {
        for q in [
            [1.0, 0.0, 0.0, 0.0],
            [0.5, 0.5, 0.5, 0.5],
            // Not unit norm, but normalization handles it — the config is
            // allowed to state a rotation without pre-normalizing it.
            [2.0, 0.0, 0.0, 0.0],
            // Squared norm is subnormal and normalizes exactly.
            [1e-154, 0.0, 0.0, 0.0],
            // Subnormal enough to lose bits, still inside the tolerance
            // (1.8e-11).
            [1e-157, 0.0, 0.0, 0.0],
            // Subnormal enough to leave it (5.6e-6).
            [1e-160, 0.0, 0.0, 0.0],
            // Squared norm underflows to zero, normalizing to infinity.
            [1e-164, 0.0, 0.0, 0.0],
            // Squared norm underflows to zero.
            [1e-200, 0.0, 0.0, 0.0],
            // Squared norm overflows to infinity.
            [1e200, 1e200, 0.0, 0.0],
            [1e300, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
        ] {
            let mut att = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
            att.initial_quaternion = q;
            // The construction `cli::sim::controlled` and `serve::engine` do.
            let state = orts::attitude::AttitudeState {
                quaternion: nalgebra::Vector4::from_row_slice(&q),
                angular_velocity: nalgebra::Vector3::zeros(),
            };
            let orientation = state.orientation();
            let usable = orientation.coords.iter().all(|c| c.is_finite())
                && (orientation.coords.norm() - 1.0).abs() <= QUATERNION_UNIT_TOLERANCE;
            assert_eq!(
                att.validate().is_ok(),
                usable,
                "validate() and orientation() disagree on {q:?}: normalized to {:?}",
                orientation.coords
            );
        }

        // Where the bound itself sits, written out so that loosening
        // `QUATERNION_UNIT_TOLERANCE` cannot take the oracle with it. Measured:
        // these two normalize 1.8e-11 and 5.6e-6 off unit norm.
        let mut accepted = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
        accepted.initial_quaternion = [1e-157, 0.0, 0.0, 0.0];
        accepted
            .validate()
            .expect("1e-157 normalizes close enough to unit norm");
        let mut refused = attitude([10.0, 10.0, 10.0], [0.0; 3], 100.0);
        refused.initial_quaternion = [1e-160, 0.0, 0.0, 0.0];
        refused
            .validate()
            .expect_err("1e-160 normalizes to 1.0000056, too far to accept");
    }

    #[test]
    fn attitude_validate_rejects_non_finite_inertia() {
        let mut att = attitude([10.0, 10.0, f64::INFINITY], [0.0; 3], 100.0);
        let err = att.validate().expect_err("infinite inertia");
        assert!(err.contains("inertia_diag"), "msg: {err}");

        att = attitude([10.0, 10.0, 10.0], [f64::NAN, 0.0, 0.0], 100.0);
        let err = att.validate().expect_err("NaN off-diagonal");
        assert!(err.contains("inertia_off_diag"), "msg: {err}");
    }

    /// The realistic tensors the repo ships in examples and presets stay valid.
    #[test]
    fn attitude_validate_accepts_realistic_tensors() {
        for diag in [
            [10.0, 10.0, 10.0],
            [100.0, 100.0, 50.0],
            // ISS, approximately [kg·m²]
            [128_913_000.0, 107_321_000.0, 201_433_000.0],
        ] {
            attitude(diag, [0.0; 3], 420_000.0)
                .validate()
                .unwrap_or_else(|e| panic!("{diag:?} should be valid: {e}"));
        }
    }

    /// End to end through the loader: the error names the satellite and the
    /// field, where `orts run` used to reach the panic in
    /// `SpacecraftDynamics::new`.
    #[test]
    fn config_load_rejects_singular_inertia() {
        let toml = r#"
[[satellites]]
id = "a"
[satellites.orbit]
type = "circular"
altitude = 500

[satellites.attitude]
inertia_diag = [0.0, 0.0, 0.0]
mass = 100.0
"#;
        let dir = std::env::temp_dir().join(format!("orts_config_att_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("singular.toml");
        std::fs::write(&path, toml).unwrap();
        let err = SimConfig::load(&path).expect_err("a singular inertia tensor must be rejected");
        assert!(err.contains("satellites[0]"), "msg: {err}");
        assert!(err.contains("attitude:"), "msg: {err}");
        assert!(err.contains("inertia tensor"), "msg: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The WebSocket surface follows the config policy, and names what it drops.
    ///
    /// A message carrying a key no field reads still starts a simulation, so a
    /// client built against a newer `orts` keeps working here — the same reason a
    /// config file warns rather than refuses. Deserializing `ClientMessage`
    /// collects nothing, since `#[serde(tag = "type")]` buffers the variant's
    /// content; reaching into the JSON for the part that is a config gets past
    /// the tag.
    ///
    /// A `type`-tagged block nested inside still refuses: nothing can report it,
    /// and a dropped key there changes the orbit.
    #[test]
    fn websocket_messages_agree_with_the_config_policy() {
        let start = r#"{
            "type": "start_simulation",
            "config": {
                "dt": 10.0,
                "satellites": [{ "orbit": { "type": "circular", "altitude": 500.0 } }]
            }
        }"#;
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(start)
            .expect("start_simulation must parse");
        assert_eq!(unread_keys_of(start), Vec::<String>::new());

        // A struct-level key nothing reads, at two depths: the message still
        // starts a simulation, and both paths are named.
        let typo = start.replace("\"dt\"", "\"dtt\"").replace(
            "\"satellites\": [{",
            "\"satellites\": [{ \"altitide\": 400,",
        );
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(&typo)
            .expect("an unknown key must not stop the message parsing");
        assert_eq!(unread_keys_of(&typo), vec!["dtt", "satellites.0.altitide"]);

        // A typo inside the `type`-tagged orbit: refused, as in a file.
        let orbit_typo = start.replace(
            "\"altitude\": 500.0",
            "\"altitude\": 500.0, \"inclinaton\": 51.6",
        );
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(&orbit_typo)
            .expect_err("a `type`-tagged block cannot report, so it refuses");

        let add = r#"{
            "type": "add_satellite",
            "id": "dynamic-sat",
            "name": "Dyn",
            "orbit": { "type": "circular", "altitude": 500.0 },
            "attitude": { "inertia_diag": [10.0, 10.0, 10.0], "mass": 500.0 }
        }"#;
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(add)
            .expect("add_satellite must still parse");
        assert_eq!(unread_keys_of(add), Vec::<String>::new());

        // `add_satellite` flattens its satellite next to the tag, where
        // `flatten` removes the unknown-field check outright. Targeting the
        // satellite directly names the key anyway: `ballistic_coef`, one `f`
        // short of `ballistic_coeff`, used to add the satellite without drag and
        // without a word.
        let flat_typo = add.replace("\"name\": \"Dyn\",", "\"ballistic_coef\": 100.0,");
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(&flat_typo)
            .expect("the message still adds the satellite");
        assert_eq!(unread_keys_of(&flat_typo), vec!["ballistic_coef"]);

        // A key under a nested struct is named at its path too. An `Option`
        // field puts a `?` in that path, which is how the crate spells the
        // step through it.
        let nested_typo = add.replace("\"mass\": 500.0", "\"mass\": 500.0, \"masss\": 1.0");
        serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(&nested_typo)
            .expect("the message still adds the satellite");
        assert_eq!(unread_keys_of(&nested_typo), vec!["attitude.?.masss"]);
    }

    /// Two ids naming one entity are a duplicate, whatever their text.
    ///
    /// `EntityPath` drops empty segments, so `a` and `/a` both name
    /// `/world/sat/a`. Compared as text they looked distinct and the fleet was
    /// accepted: measured, both entries resolved to `/world/sat/a` and
    /// `validate` returned `Ok`. The `--sat` path compares on the entity
    /// already (`ensure_unique_ids`).
    #[test]
    fn ids_naming_one_entity_are_a_duplicate() {
        for (a, b) in [("a", "/a"), ("a/b", "a//b")] {
            let toml = format!(
                r#"
[[satellites]]
id = "{a}"
[satellites.orbit]
type = "circular"
altitude = 400.0

[[satellites]]
id = "{b}"
[satellites.orbit]
type = "circular"
altitude = 500.0
"#
            );
            let config: SimConfig = toml::from_str(&toml).expect("parse");
            let err = config
                .validate()
                .expect_err(&format!("ids {a:?} and {b:?} name one entity"));
            assert!(
                err.contains("duplicate satellite id"),
                "ids {a:?} and {b:?}: {err}"
            );
        }
    }

    /// An id naming no entity of its own is rejected, fleet of one included.
    ///
    /// A separator-only id contributes no path segment, so the recording entity
    /// collapses to the `/world/sat` root the whole fleet shares. The `--sat`
    /// path rejects it (`validate_id`); a config file accepted it, and a fleet
    /// of one got there without any duplicate to notice.
    #[test]
    fn an_id_naming_no_entity_is_rejected() {
        for id in ["/", "//"] {
            let toml = format!(
                r#"
[[satellites]]
id = "{id}"
[satellites.orbit]
type = "circular"
altitude = 400.0
"#
            );
            let config: SimConfig = toml::from_str(&toml).expect("parse");
            let err = config
                .validate()
                .expect_err(&format!("id {id:?} names no entity"));
            assert!(err.contains("no path segment"), "id {id:?}: {err}");
        }
    }

    /// Ids that name entities of their own are accepted.
    ///
    /// The check above compares on the entity, so it must not read two distinct
    /// ones as a collision.
    #[test]
    fn ids_naming_entities_of_their_own_are_accepted() {
        let toml = r#"
[[satellites]]
id = "a"
[satellites.orbit]
type = "circular"
altitude = 400.0

[[satellites]]
id = "a/b"
[satellites.orbit]
type = "circular"
altitude = 500.0
"#;
        let config: SimConfig = toml::from_str(toml).expect("parse");
        config.validate().expect("two entities, two satellites");
    }

    /// A fleet no mode can serve is refused by the config, not by the engine.
    ///
    /// `orts serve --config` builds its engine inside a spawned manager, so a
    /// mixed fleet used to validate clean, print the startup banner, and leave
    /// the server idle with the caller's config never running.
    #[test]
    fn a_fleet_no_mode_can_serve_is_rejected() {
        let mixed_attitude = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10.0, 10.0, 10.0], mass = 100.0 }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
"#;
        let config: SimConfig = toml::from_str(mixed_attitude).expect("valid toml");
        let err = config
            .validate()
            .expect_err("half a fleet with attitude runs in no mode");
        assert!(err.contains("Mixed attitude config"), "msg: {err}");

        let mixed_controller = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
"#;
        let config: SimConfig = toml::from_str(mixed_controller).expect("valid toml");
        let err = config
            .validate()
            .expect_err("half a fleet with a controller runs in no mode");
        assert!(err.contains("Mixed controller config"), "msg: {err}");

        // A uniform controlled fleet with no attitude anywhere. The mode comes
        // out `Controlled`, and `build_controlled_satellite` then refuses every
        // satellite for want of an attitude state — under `orts serve --config`
        // that refusal arrives after the startup banner.
        let controller_without_attitude = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
controller = { type = "wasm", path = "ctrl.wasm" }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
controller = { type = "wasm", path = "ctrl.wasm" }
"#;
        let config: SimConfig = toml::from_str(controller_without_attitude).expect("valid toml");
        let err = config
            .validate()
            .expect_err("a controller has no attitude state to command");
        assert!(
            err.contains("without `[satellites.attitude]`"),
            "msg: {err}"
        );

        // Both declared on every satellite, and on none, are the fleets that do
        // pick a mode.
        for toml in [
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10.0, 10.0, 10.0], mass = 100.0 }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
attitude = { inertia_diag = [10.0, 10.0, 10.0], mass = 100.0 }
"#,
            r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
"#,
        ] {
            let config: SimConfig = toml::from_str(toml).expect("valid toml");
            config.validate().expect("a uniform fleet picks a mode");
        }
    }

    /// The config's count and the engine's count are the same count.
    ///
    /// `validate` counts `SatelliteConfig::attitude` / `controller` because
    /// building specs would reach the network for a `norad_id` orbit, while
    /// `select_sim_mode` counts the spec fields. They agree only as long as
    /// `to_satellite_spec` keeps cloning them across unchanged.
    #[test]
    fn the_config_counts_what_the_engine_counts() {
        let toml = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10.0, 10.0, 10.0], mass = 100.0 }
controller = { type = "wasm", path = "ctrl.wasm" }

[[satellites]]
id = "b"
orbit = { type = "circular", altitude = 600 }
"#;
        let config: SimConfig = toml::from_str(toml).expect("valid toml");
        let body = crate::satellite::try_parse_body(&config.body).expect("earth");
        for (i, sat) in config.satellites.iter().enumerate() {
            let spec = sat.to_satellite_spec(i, body, 398600.4418);
            assert_eq!(
                sat.attitude.is_some(),
                spec.attitude_config.is_some(),
                "satellites[{i}]: attitude"
            );
            assert_eq!(
                sat.controller.is_some(),
                spec.controller_config.is_some(),
                "satellites[{i}]: controller"
            );
        }
    }

    /// Every format the loader accepts names its unread keys the same way.
    ///
    /// The three go through different `serde_ignored` adapters — a `&mut`
    /// `serde_json::Deserializer`, a `toml::Deserializer` by value, a
    /// `serde_yaml::Deserializer` by value — so a change to one says nothing
    /// about the others. The same two typos, at the top level and inside a
    /// satellite, must come back as the same two paths from all three.
    #[test]
    fn every_format_names_the_same_unread_keys() {
        let dir = std::env::temp_dir().join(format!(
            "orts-unread-formats-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let files = [
            (
                "sim.toml",
                "dt = 1.0\nduraton = 100.0\n\n[[satellites]]\nid = \"a\"\naltitide = 400\n\
                 [satellites.orbit]\ntype = \"circular\"\naltitude = 400\n"
                    .to_string(),
            ),
            (
                "sim.json",
                r#"{"dt":1.0,"duraton":100.0,"satellites":[
                    {"id":"a","altitide":400,
                     "orbit":{"type":"circular","altitude":400}}]}"#
                    .to_string(),
            ),
            (
                "sim.yaml",
                "dt: 1.0\nduraton: 100.0\nsatellites:\n  - id: a\n    altitide: 400\n    \
                 orbit:\n      type: circular\n      altitude: 400\n"
                    .to_string(),
            ),
        ];

        for (name, body) in files {
            let path = dir.join(name);
            std::fs::write(&path, &body).expect("write config");
            let loaded = SimConfig::load_with_warnings(&path)
                .unwrap_or_else(|e| panic!("{name}: the file loads: {e}"));
            assert_eq!(
                loaded.unread_keys,
                vec!["duraton", "satellites.0.altitide"],
                "{name}: both paths, in order"
            );
            assert_eq!(loaded.config.dt, 1.0, "{name}: the read keys still land");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A YAML file holding a second document is refused.
    ///
    /// `serde_yaml::from_str` refuses a multi-document stream; driving the
    /// `Deserializer` to collect the unread keys has to keep doing so, or the
    /// documents after the first would be dropped without a word — the JSON
    /// trailing-input hole in a different format.
    #[test]
    fn a_yaml_config_with_a_second_document_is_refused() {
        let dir = std::env::temp_dir().join(format!(
            "orts-yaml-multidoc-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sim.yaml");
        let one = "dt: 1.0\nsatellites:\n  - id: a\n    orbit:\n      type: circular\n      \
                   altitude: 400\n";

        std::fs::write(&path, one).expect("write config");
        SimConfig::load_with_warnings(&path).expect("one document loads");

        std::fs::write(&path, format!("{one}---\ndt: 99.0\n")).expect("write config");
        let err =
            SimConfig::load_with_warnings(&path).expect_err("a second document must be refused");
        assert!(err.contains("YAML"), "msg: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A key a client chose cannot forge a line or move the cursor.
    ///
    /// The names come from the client's own JSON, and the server writes them to
    /// its log. Measured before this was escaped: a `start_simulation` carrying
    /// `"a\nWarning: forged line"` gave back a key with that newline in it, and
    /// the warning printed as two lines, the second looking like the server's.
    #[test]
    fn a_key_reaches_the_log_on_one_line() {
        let msg = "{\"type\":\"start_simulation\",\"config\":{\
                   \"dt\":1.0,\
                   \"a\\nWarning: forged line\":1,\
                   \"b\\u001b[31m\":2,\
                   \"satellites\":[{\"id\":\"a\",\"orbit\":\
                   {\"type\":\"circular\",\"altitude\":400}}]}}";
        let keys = unread_keys_of(msg);
        assert_eq!(keys.len(), 2, "both keys are collected: {keys:?}");
        assert!(
            keys.iter().any(|k| k.contains('\n')),
            "the raw key holds the newline, so the escaping is what removes it: {keys:?}"
        );

        for key in &keys {
            let line = format!("{}", printable_key(key));
            assert!(
                !line.contains('\n') && !line.contains('\r'),
                "no line break survives: {line:?}"
            );
            assert!(
                !line.contains('\u{1b}'),
                "no escape character survives: {line:?}"
            );
        }
        // An ordinary key is untouched, so the escaping costs nothing to read.
        assert_eq!(
            printable_key("satellites.0.altitide"),
            "satellites.0.altitide"
        );

        // A key is as long as its sender chose, and escaping expands each
        // character up to six, so the rendered line is bounded and says what it
        // left out.
        let long = "\u{1b}".repeat(10_000);
        let rendered = printable_key(&long);
        assert!(
            rendered.chars().count() < PRINTED_KEY_LIMIT + 40,
            "bounded whatever the escaping does to it: {} chars",
            rendered.chars().count()
        );
        assert!(
            rendered.contains("10000 bytes in all"),
            "and says how long the key was: {rendered}"
        );
        assert!(!rendered.contains('\u{1b}'), "still escaped: {rendered:?}");
    }

    /// A client message names at most `CLIENT_MESSAGE_KEY_LIMIT` keys.
    ///
    /// Each named key is one unbuffered `eprintln!` on the connection task, and
    /// `/ws` takes messages from whoever reaches the port, so one frame must not
    /// decide how many writes the server makes. The rest are counted.
    #[test]
    fn a_client_message_names_at_most_the_limit() {
        let extra = 7;
        let typos: String = (0..CLIENT_MESSAGE_KEY_LIMIT + extra)
            .map(|i| format!("\"typo{i}\":{i},"))
            .collect();
        let msg = format!(
            "{{\"type\":\"start_simulation\",\"config\":{{{typos}\
             \"satellites\":[{{\"id\":\"a\",\"orbit\":\
             {{\"type\":\"circular\",\"altitude\":400}}}}]}}}}"
        );
        let value: serde_json::Value =
            serde_json::from_str(&msg).expect("the message is valid JSON");

        let unread = unread_client_message_keys(&value);
        assert_eq!(unread.named.len(), CLIENT_MESSAGE_KEY_LIMIT);
        assert_eq!(unread.unnamed, extra, "the rest are counted, not dropped");
        // The message still parses into something the server runs: the limit is
        // on what gets said about it, not on what it may contain.
        serde_json::from_value::<crate::commands::serve::protocol::ClientMessage>(value)
            .expect("a message full of unknown keys still starts a simulation");
    }

    /// A key beside the variant's own is named too, not only one in the payload.
    ///
    /// `ClientMessage` ignores an envelope key it does not read, so
    /// `{"type":"start_simulation","config":{…},"dtt":10}` started the
    /// simulation with `dtt` dropped in silence. The same held for a typo in an
    /// optional field of `query_range`, and for any key on a variant that reads
    /// none — a typo in a required field fails to deserialize instead, which the
    /// server answers with an error.
    #[test]
    fn a_key_beside_the_variants_own_is_named() {
        let cases = [
            (
                "start_simulation",
                "{\"type\":\"start_simulation\",\"dtt\":10,\"config\":{\"dt\":1.0,\
                 \"satellites\":[{\"id\":\"a\",\"orbit\":\
                 {\"type\":\"circular\",\"altitude\":400}}]}}",
                "dtt",
            ),
            (
                "query_range",
                "{\"type\":\"query_range\",\"t_min\":0.0,\"t_max\":10.0,\"max_pointz\":100}",
                "max_pointz",
            ),
            (
                "pause_simulation",
                "{\"type\":\"pause_simulation\",\"untl\":10.0}",
                "untl",
            ),
        ];
        for (label, msg, key) in cases {
            let value: serde_json::Value =
                serde_json::from_str(msg).unwrap_or_else(|e| panic!("{label}: valid JSON: {e}"));
            // The message is still one the server runs; the key is a warning.
            serde_json::from_value::<crate::commands::serve::protocol::ClientMessage>(
                value.clone(),
            )
            .unwrap_or_else(|e| panic!("{label}: an unknown key must not stop the message: {e}"));
            let unread = unread_client_message_keys(&value);
            assert_eq!(
                unread.named,
                vec![key.to_string()],
                "{label}: the key beside the variant's own"
            );
        }
    }

    /// The keys a variant does read are not reported.
    ///
    /// The list in `protocol::variant_envelope_keys` is written by hand, so a
    /// wrong entry would name a field the message uses.
    #[test]
    fn the_keys_a_variant_reads_are_not_named() {
        for msg in [
            "{\"type\":\"query_range\",\"t_min\":0.0,\"t_max\":10.0,\"max_points\":100,\
             \"entity_path\":\"/world/sat/a\"}",
            "{\"type\":\"pause_simulation\"}",
            "{\"type\":\"resume_simulation\"}",
            "{\"type\":\"terminate_simulation\"}",
        ] {
            let value: serde_json::Value = serde_json::from_str(msg).expect("valid JSON");
            serde_json::from_value::<crate::commands::serve::protocol::ClientMessage>(
                value.clone(),
            )
            .unwrap_or_else(|e| panic!("{msg} must deserialize: {e}"));
            assert_eq!(
                unread_client_message_keys(&value),
                UnreadClientKeys::default(),
                "nothing to report for {msg}"
            );
        }
    }

    /// A frame naming one field twice is refused, not resolved to the last one.
    ///
    /// `serde_json::Value` keeps the last of two members with one name, so
    /// reading the message from a tree would run
    /// `{…,"config":{"dt":1},"config":{"dt":99}}` with 99 and say nothing. The
    /// server reads the text, where serde refuses a duplicate field.
    #[test]
    fn a_duplicate_field_is_refused() {
        let one = "{\"dt\":1.0,\"satellites\":[{\"id\":\"a\",\"orbit\":\
                   {\"type\":\"circular\",\"altitude\":400}}]}";
        let msg = format!("{{\"type\":\"start_simulation\",\"config\":{one},\"config\":{one}}}");

        // Through a tree: the duplicate is gone before anything can object.
        let value: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(
            value.as_object().expect("an object").len(),
            2,
            "the tree holds `type` and one `config`"
        );
        serde_json::from_value::<crate::commands::serve::protocol::ClientMessage>(value)
            .expect("a tree cannot see the duplicate");

        // From the text, which is what the server reads.
        let err = serde_json::from_str::<crate::commands::serve::protocol::ClientMessage>(&msg)
            .err()
            .expect("a duplicate field must be refused");
        assert!(
            err.to_string().contains("duplicate field"),
            "the error says which: {err}"
        );
    }

    /// A TLE or NORAD orbit about anything but Earth is refused by the config.
    ///
    /// SGP4 is Earth's, and `SimParams::from_config` reaches that rule through
    /// an `unwrap_or_else(panic)`. Measured before this: `orts config validate`
    /// called a Moon + TLE config valid and `orts run --config` panicked on it.
    ///
    /// The `norad` case is checked here rather than through a spec, because
    /// building one fetches the element set over the network.
    #[test]
    fn a_tle_about_another_body_is_rejected() {
        let tle = "[[satellites]]\nid = \"a\"\n[satellites.orbit]\ntype = \"tle\"\n\
                   line1 = \"1 25544U 98067A   24079.50000000  .00016717  00000-0  \
                   30000-4 0  9996\"\n\
                   line2 = \"2 25544  51.6400 208.6520 0007417  35.3910 324.7580 \
                   15.49561654480008\"\n";
        let norad = "[[satellites]]\nid = \"a\"\n[satellites.orbit]\n\
                     type = \"norad\"\nnorad_id = 25544\n";

        for (label, orbit) in [("tle", tle), ("norad", norad)] {
            let config: SimConfig = toml::from_str(&format!("body = \"moon\"\n{orbit}"))
                .unwrap_or_else(|e| panic!("{label}: valid toml: {e}"));
            let err = config
                .validate()
                .expect_err(&format!("{label}: SGP4 cannot propagate about the Moon"));
            assert!(err.contains("Earth-centered"), "{label}: {err}");

            // The same orbit about Earth is what the check must not refuse.
            let config: SimConfig = toml::from_str(&format!("body = \"earth\"\n{orbit}"))
                .unwrap_or_else(|e| panic!("{label}: valid toml: {e}"));
            config
                .validate()
                .unwrap_or_else(|e| panic!("{label}: Earth is where SGP4 belongs: {e}"));
        }
    }
}
