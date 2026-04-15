use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cli::{AtmosphereChoice, IntegratorChoice};
use crate::satellite::{OrbitSpec, SatelliteSpec};
use crate::tle::fetch_tle_by_norad_id;
use arika::body::KnownBody;
use orts::tle::Tle;

/// JSON/TOML/YAML simulation configuration.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SimConfig {
    #[serde(default = "default_body")]
    pub body: String,
    #[serde(default = "default_dt")]
    pub dt: f64,
    pub output_interval: Option<f64>,
    pub stream_interval: Option<f64>,
    pub epoch: Option<String>,
    #[serde(default)]
    pub integrator: IntegratorConfig,
    #[serde(default = "default_atmosphere")]
    pub atmosphere: String,
    #[serde(default = "default_f107")]
    pub f107: f64,
    #[serde(default = "default_ap")]
    pub ap: f64,
    pub space_weather: Option<String>,
    pub duration: Option<f64>,
    #[serde(default)]
    pub satellites: Vec<SatelliteConfig>,
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
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct IntegratorConfig {
    #[serde(rename = "type", default = "default_integrator")]
    pub kind: String,
    #[serde(default = "default_atol")]
    pub atol: f64,
    #[serde(default = "default_rtol")]
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
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AttitudeConfig {
    /// Diagonal inertia tensor [Ixx, Iyy, Izz] kg·m².
    pub inertia_diag: [f64; 3],
    /// Off-diagonal inertia elements [Ixy, Ixz, Iyz] (default: all zero).
    #[serde(default)]
    pub inertia_off_diag: [f64; 3],
    /// Spacecraft mass [kg].
    pub mass: f64,
    /// Initial quaternion [w, x, y, z] body-to-inertial (default: identity).
    #[serde(default = "default_identity_quat")]
    pub initial_quaternion: [f64; 4],
    /// Initial angular velocity [wx, wy, wz] rad/s body frame (default: zero).
    #[serde(default)]
    pub initial_angular_velocity: [f64; 3],
}

fn default_identity_quat() -> [f64; 4] {
    [1.0, 0.0, 0.0, 0.0]
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
}

/// コントローラ設定。
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum ControllerConfig {
    /// WASM Component ゲストプラグイン。
    #[serde(rename = "wasm")]
    Wasm {
        /// `.wasm` ファイルのパス。
        path: String,
        /// ゲストの `init` に渡す設定 (JSON value)。
        #[serde(default)]
        config: serde_json::Value,
    },
}

/// センサ選択。
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SensorChoice {
    Magnetometer,
    Gyroscope,
    StarTracker,
}

/// リアクションホイール設定。
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
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
        speed_control_gain: Option<f64>,
    },
}

/// MTQ (磁気トルカ) 設定。
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum MtqConfig {
    /// 直交 3 軸配置。
    #[serde(rename = "three_axis")]
    ThreeAxis {
        /// 最大ダイポールモーメント [A·m²]。
        max_moment: f64,
    },
}

/// Per-satellite configuration.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SatelliteConfig {
    pub id: Option<String>,
    pub name: Option<String>,
    pub orbit: OrbitConfig,
    pub ballistic_coeff: Option<f64>,
    pub srp_area_to_mass: Option<f64>,
    pub srp_cr: Option<f64>,
    /// Attitude dynamics configuration. When present, SpacecraftDynamics is used.
    pub attitude: Option<AttitudeConfig>,
    /// プラグインコントローラ設定。
    pub controller: Option<ControllerConfig>,
    /// 有効にするセンサ一覧。
    pub sensors: Option<Vec<SensorChoice>>,
    /// リアクションホイール設定。
    pub reaction_wheels: Option<ReactionWheelConfig>,
    /// MTQ 設定。
    pub mtq: Option<MtqConfig>,
}

/// Orbit specification in config files.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum OrbitConfig {
    /// Circular orbit at given altitude.
    #[serde(rename = "circular")]
    Circular {
        altitude: f64,
        /// Inclination in degrees (default: 0).
        #[serde(default)]
        inclination: f64,
        /// RAAN in degrees (default: 0).
        #[serde(default)]
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

        match ext.as_str() {
            "json" => serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse JSON config: {e}")),
            "toml" => {
                toml::from_str(&content).map_err(|e| format!("Failed to parse TOML config: {e}"))
            }
            "yaml" | "yml" => serde_yaml::from_str(&content)
                .map_err(|e| format!("Failed to parse YAML config: {e}")),
            _ => Err(format!(
                "Unknown config file extension '.{ext}'. Supported: .json, .toml, .yaml, .yml"
            )),
        }
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
                let tle = Tle::parse(&text)
                    .unwrap_or_else(|e| panic!("Failed to parse TLE in config: {e}"));
                let elements = tle.to_keplerian_elements(mu);
                let period = elements.period(mu);
                let tle_name = tle.name.clone();
                (
                    OrbitSpec::Tle {
                        tle_data: tle,
                        elements,
                    },
                    period,
                    tle_name,
                )
            }
            OrbitConfig::Norad { norad_id } => {
                let tle = fetch_tle_by_norad_id(*norad_id);
                let elements = tle.to_keplerian_elements(mu);
                let period = elements.period(mu);
                let tle_name = tle.name.clone();
                (
                    OrbitSpec::Tle {
                        tle_data: tle,
                        elements,
                    },
                    period,
                    tle_name,
                )
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
            controller_config: self.controller.clone(),
            sensor_choices: self.sensors.clone(),
            rw_config: self.reaction_wheels.clone(),
            mtq_config: self.mtq.clone(),
        }
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
                    "line1": "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993",
                    "line2": "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000"
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
            attitude: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
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
            attitude: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
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
                line1: "1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993"
                    .into(),
                line2: "2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000"
                    .into(),
            },
            ballistic_coeff: None,
            srp_area_to_mass: None,
            srp_cr: None,
            attitude: None,
            controller: None,
            sensors: None,
            reaction_wheels: None,
            mtq: None,
        };
        let body = KnownBody::Earth;
        let mu = body.properties().mu;
        let spec = sat_cfg.to_satellite_spec(0, body, mu);

        assert_eq!(spec.id, "iss");
        assert!(matches!(spec.orbit, OrbitSpec::Tle { .. }));
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
                attitude: None,
                controller: None,
                sensors: None,
                reaction_wheels: None,
                mtq: None,
            }],
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
      path: plugins/pd-rw-control/target/plugin.wasm
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
}
