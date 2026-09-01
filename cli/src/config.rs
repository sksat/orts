use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::cli::{AtmosphereChoice, IntegratorChoice};
use crate::satellite::{OrbitSpec, SatelliteSpec};
use crate::tle::fetch_tle_by_norad_id;
use arika::body::KnownBody;
use orts::plugin::{Message, NamedValue, NodeId, Payload, Value};
use orts::setup::DisturbanceTorques;

/// JSON/TOML/YAML simulation configuration.
///
/// Also the payload of the `start_simulation` WebSocket message, so the
/// whole tree derives [`TS`]. Fields the server defaults when absent are
/// `#[ts(optional)]` so TypeScript clients may omit them too.
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
    #[serde(default = "default_atmosphere")]
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
    #[serde(rename = "type", default = "default_integrator")]
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
/// selects which environmental models get solved. Requires attitude dynamics —
/// a torque needs an orientation to act on.
///
/// Unknown keys are rejected. Every selector here defaults to on, so the only
/// reason to write this table is to turn something off — a misspelled key would
/// otherwise be dropped and leave the torque enabled, which is the opposite of
/// what was asked for.
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(deny_unknown_fields)]
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
        // Ask the question the way the dynamics do: `SpacecraftDynamics::new`
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
        // built on it would answer every torque with zero angular acceleration.
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
        // The quaternion need not be normalized — `AttitudeState::orientation`
        // normalizes on use and `OdeState::project` renormalizes after every
        // step — but it has to be something those can normalize. Both divide by
        // the sum of squares, so the test is that sum, not the components: an
        // all-zero quaternion names no attitude, `[1e-200, 0, 0, 0]` squares to
        // zero, and `[1e200, 0, 0, 0]` squares to infinity. Each leaves the
        // orientation undefined even though the components themselves are
        // finite and non-zero.
        let quat_norm_sq: f64 = self.initial_quaternion.iter().map(|q| q * q).sum();
        if !(quat_norm_sq > 0.0 && quat_norm_sq.is_finite()) {
            return Err(format!(
                "`initial_quaternion` cannot be normalized \
                 (its components square to {quat_norm_sq}); it names no attitude"
            ));
        }
        // A usable inverse is not yet an integrable state: the torque-free Euler
        // term `I⁻¹ (−ω × Iω)` can overflow on its own. `[1e-308, 1, 2]` with
        // `ω = [1, 2, 2]` inverts cleanly and still starts at an infinite
        // angular acceleration.
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
        if self.mass <= 0.0 {
            return Err(format!("non-positive mass: {}", self.mass));
        }
        Ok(())
    }
}

/// コントローラ設定。
#[derive(Deserialize, Serialize, Clone, Debug, TS)]
#[serde(tag = "type")]
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
#[serde(tag = "type")]
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
#[serde(tag = "type")]
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
#[serde(tag = "type")]
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

impl SimConfig {
    /// Load a config file, auto-detecting format by extension.
    pub fn load(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {e}", path.display()))?;

        let config: SimConfig = match ext.as_str() {
            "json" => serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse JSON config: {e}"))?,
            "toml" => {
                toml::from_str(&content).map_err(|e| format!("Failed to parse TOML config: {e}"))?
            }
            "yaml" | "yml" => serde_yaml::from_str(&content)
                .map_err(|e| format!("Failed to parse YAML config: {e}"))?,
            _ => {
                return Err(format!(
                    "Unknown config file extension '.{ext}'. Supported: .json, .toml, .yaml, .yml"
                ));
            }
        };

        config.validate()?;

        Ok(config)
    }

    /// Parse the integrator choice from the config string.
    pub fn integrator_choice(&self) -> IntegratorChoice {
        match self.integrator.kind.as_str() {
            "rk4" => IntegratorChoice::Rk4,
            "dop853" => IntegratorChoice::Dop853,
            _ => IntegratorChoice::Dp45,
        }
    }

    /// Parse the atmosphere choice from the config string.
    pub fn atmosphere_choice(&self) -> AtmosphereChoice {
        match self.atmosphere.as_str() {
            "harris-priester" => AtmosphereChoice::HarrisPriester,
            "nrlmsise00" => AtmosphereChoice::Nrlmsise00,
            _ => AtmosphereChoice::Exponential,
        }
    }

    /// Parse the central body from the config string.
    pub fn known_body(&self) -> KnownBody {
        crate::satellite::parse_body(&self.body)
    }
}

impl SatelliteConfig {
    /// Convert a SatelliteConfig to a SatelliteSpec.
    pub fn to_satellite_spec(&self, index: usize, body: KnownBody, mu: f64) -> SatelliteSpec {
        let id = self.id.clone().unwrap_or_else(|| format!("sat-{index}"));

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
                (OrbitSpec::Omm { omm: tle }, period, tle_name)
            }
            OrbitConfig::Norad { norad_id } => {
                let parsed = fetch_tle_by_norad_id(*norad_id);
                let tle = parsed.elements;
                let period = tle.period();
                let tle_name = parsed.object_name.clone();
                (OrbitSpec::Omm { omm: tle }, period, tle_name)
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
/// - `output_interval` drives `t += output_interval` in `run`, so zero never
///   reaches `max_period`.
/// - `stream_interval` is a divisor in the serve loop's pacing, where zero
///   yields `0 * inf = NaN` and panics `Duration::from_secs_f64`.
/// - `duration` becomes each satellite's propagation period.
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
/// Both controlled-run loops step with `dt = sample_period.min(remaining)`, so
/// a zero period leaves `t += 0` spinning and a negative one walks backwards.
/// The period comes from the plugin/controller rather than from user config,
/// so it has to be checked where it is first used.
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
        validate_time_params(
            self.dt,
            self.output_interval,
            self.stream_interval,
            self.duration,
        )?;
        validate_tolerances(
            self.integrator_choice(),
            self.integrator.atol,
            self.integrator.rtol,
        )?;
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
        }
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
        assert!(matches!(spec.orbit, OrbitSpec::Omm { .. }));
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

    /// `output_interval = 0` never reaches `max_period` in `run`;
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

    /// A misspelled selector must fail loudly. Serde would otherwise drop it and
    /// fall back to the default, so `gravity_gradent = false` would leave the
    /// torque on — the reader asked for the opposite and gets no warning.
    #[test]
    fn misspelled_disturbance_selector_is_rejected() {
        let toml = r#"
[[satellites]]
id = "a"
orbit = { type = "circular", altitude = 500 }
attitude = { inertia_diag = [10, 10, 10], mass = 50 }
disturbances = { gravity_gradent = false }
"#;
        let err = toml::from_str::<SimConfig>(toml).expect_err("the typo must not parse");
        let msg = err.to_string();
        // toml echoes the offending line, so the typo appears in the error even
        // when serde diagnosed something else. Pin the diagnosis itself.
        assert!(msg.contains("unknown field"), "got: {msg}");
        assert!(msg.contains("gravity_gradent"), "got: {msg}");
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
}
