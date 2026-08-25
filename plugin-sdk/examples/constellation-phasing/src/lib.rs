//! In-plane phasing demo。parking 軌道で `raise_delay_s`
//! だけ待機してから Hohmann 遷移で operational 軌道へ上昇することで、
//! parking と operational の平均運動差
//! `Δn = √(μ/r_park³) − √(μ/r_op³)` を使って phase offset を作り出す。
//!
//! 同じ .wasm を複数衛星に assign して per-sat config で `raise_delay_s`
//! だけ変える構成を想定している。
//!
//! # State Machine
//!
//! ```text
//! ┌────────┐  t >= raise_delay_s    ┌───────────┐          ┌───────┐          ┌────────────┐          ┌──────┐
//! │ Parked │───────────────────────▶│ FirstBurn │─────────▶│ Coast │─────────▶│ SecondBurn │─────────▶│ Trim │
//! └────────┘                         └───────────┘          └───────┘          └────────────┘          └──────┘
//! ```
//!
//! 姿勢追従 (body-Y を prograde に向ける PD + RW) と thruster throttle の
//! composite controller は transfer-burn-with-tcm と同じロジックを使う。

use nalgebra::{Matrix3, UnitQuaternion, Vector3};
use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, orts_plugin};

const EARTH_RADIUS_KM: f64 = 6378.137;

/// Coast の watchdog 期限 \[s\]: `sma` の軌道 1 周期後（+10% の余裕）。
///
/// apogee はどの点から数えても 1 周期以内に来るので、これを過ぎても `r·v` の
/// 符号反転を観測できないのは state stream 側の異常。余裕は摂動と有限
/// sample_period のぶん。楕円でない状態（`sma <= 0` や非有限）では watchdog を
/// 張らない（時間だけを根拠に噴かせない）。
fn coast_watchdog_t(t: f64, sma: f64, mu: f64) -> Option<f64> {
    if !sma.is_finite() || sma <= 0.0 {
        return None;
    }
    let period = 2.0 * std::f64::consts::PI * (sma.powi(3) / mu).sqrt();
    Some(t + 1.1 * period)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Parked,
    FirstBurn,
    Coast,
    SecondBurn,
    Trim,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Parked => "parked",
            Phase::FirstBurn => "first_burn",
            Phase::Coast => "coast",
            Phase::SecondBurn => "second_burn",
            Phase::Trim => "trim",
        }
    }
}

struct ConstellationPhasing {
    raise_delay_s: f64,
    target_r_km: f64,
    mu_km3_s2: f64,
    deadband_km: f64,
    num_thrusters: usize,
    num_rws: usize,
    kp: f64,
    kd: f64,
    sample_period: f64,
    transfer_sma_km: Option<f64>,
    /// Coast → SecondBurn の watchdog 期限 \[s\]。主判定は `r·v` の符号反転
    /// （= apogee）で、これは「apogee を検出できないまま coast 軌道 1 周期が
    /// 過ぎた」場合に Coast から抜けるための保険。Coast 遷移時の実測 SMA から
    /// 求める（nominal transfer SMA だと finite-burn の overshoot 分だけ実周期を
    /// 下回り、本来の apogee より先に発火しうる）。
    coast_watchdog_t: Option<f64>,
    phase: Phase,
    /// Last SMA [km] seen by `update()`. Used by SecondBurn/Trim to predict
    /// the next step's SMA change and throttle back to avoid overshoot.
    prev_sma_km: Option<f64>,
    /// 前 tick の径方向速度 `r·v` \[km²/s\]。apogee（上昇 → 非上昇）の
    /// 符号反転を見るために保持する。
    prev_radial_rate: Option<f64>,
}

impl Plugin<TickInput, Command> for ConstellationPhasing {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(config: &str) -> Result<Self, String> {
        let cfg: Config = if config.is_empty() {
            Config::default()
        } else {
            serde_json::from_str(config).map_err(|e| format!("config parse error: {e}"))?
        };
        if !cfg.target_altitude_km.is_finite() || cfg.target_altitude_km <= 0.0 {
            return Err("target_altitude_km must be positive and finite".into());
        }
        if !cfg.mu_km3_s2.is_finite() || cfg.mu_km3_s2 <= 0.0 {
            return Err("mu_km3_s2 must be positive and finite".into());
        }
        if !cfg.deadband_km.is_finite() || cfg.deadband_km <= 0.0 {
            return Err("deadband_km must be positive and finite".into());
        }
        if !cfg.raise_delay_s.is_finite() || cfg.raise_delay_s < 0.0 {
            return Err("raise_delay_s must be non-negative and finite".into());
        }
        if cfg.num_thrusters == 0 {
            return Err("num_thrusters must be >= 1".into());
        }
        if cfg.num_rws == 0 {
            return Err("num_rws must be >= 1".into());
        }
        if !cfg.sample_period.is_finite() || cfg.sample_period <= 0.0 {
            return Err("sample_period must be positive and finite".into());
        }
        let initial_phase = if cfg.raise_delay_s > 0.0 {
            Phase::Parked
        } else {
            Phase::FirstBurn
        };
        Ok(Self {
            raise_delay_s: cfg.raise_delay_s,
            target_r_km: EARTH_RADIUS_KM + cfg.target_altitude_km,
            mu_km3_s2: cfg.mu_km3_s2,
            deadband_km: cfg.deadband_km,
            num_thrusters: cfg.num_thrusters,
            num_rws: cfg.num_rws,
            kp: cfg.kp,
            kd: cfg.kd,
            sample_period: cfg.sample_period,
            transfer_sma_km: None,
            coast_watchdog_t: None,
            phase: initial_phase,
            prev_sma_km: None,
            prev_radial_rate: None,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        let p = &input.spacecraft.orbit.position;
        let v = &input.spacecraft.orbit.velocity;
        let r_vec = Vector3::new(p.x, p.y, p.z);
        let v_vec = Vector3::new(v.x, v.y, v.z);
        let r = r_vec.norm();
        let v_sq = v_vec.norm_squared();

        let epsilon = 0.5 * v_sq - self.mu_km3_s2 / r;
        let sma = -self.mu_km3_s2 / (2.0 * epsilon);

        // target 高度が parking 高度より低い場合（prograde-only では deorbit 不可）。
        // Parked 中はまだ burn していないので check しない。FirstBurn 遷移時に検証する。
        if self.phase == Phase::FirstBurn && r > self.target_r_km + self.deadband_km {
            return Err(format!(
                "target altitude ({:.1} km) is lower than parking altitude ({:.1} km); \
                 prograde-only thruster cannot deorbit. Aborting.",
                self.target_r_km - EARTH_RADIUS_KM,
                r - EARTH_RADIUS_KM,
            ));
        }

        let throttle = match self.phase {
            Phase::Parked => {
                if input.t >= self.raise_delay_s {
                    // Check reachability BEFORE transitioning so we don't
                    // accidentally command one prograde step on an impossible
                    // (target below parking) configuration.
                    if r > self.target_r_km + self.deadband_km {
                        return Err(format!(
                            "target altitude ({:.1} km) is lower than parking altitude ({:.1} km); \
                             prograde-only thruster cannot deorbit. Aborting.",
                            self.target_r_km - EARTH_RADIUS_KM,
                            r - EARTH_RADIUS_KM,
                        ));
                    }
                    self.phase = Phase::FirstBurn;
                    // FirstBurn に入ったら transfer orbit のパラメータをキャッシュする
                    // （parking 軌道の現 r を起点に計算）。
                    self.transfer_sma_km = Some((r + self.target_r_km) / 2.0);
                    1.0
                } else {
                    0.0
                }
            }
            Phase::FirstBurn => {
                let transfer_sma = *self
                    .transfer_sma_km
                    .get_or_insert((r + self.target_r_km) / 2.0);
                if sma >= transfer_sma {
                    self.phase = Phase::Coast;
                    self.coast_watchdog_t = coast_watchdog_t(input.t, sma, self.mu_km3_s2);
                    0.0
                } else {
                    1.0
                }
            }
            Phase::Coast => {
                // apogee は状態量で判定する: 径方向速度 r·v が正（上昇）から
                // 非正に変わる点が apogee。降下中に Coast へ入った場合は、
                // まず上昇に転じるのを待つ。
                //
                // 時間ベースの判定を主にしてはいけない: transfer ellipse の
                // 半周期は *perigee* 起点の時間なので、有限時間の FirstBurn の
                // 終了時刻から測ると、burn 中に飛んだ弧の分だけ apogee を過ぎて
                // から噴く。時間はここでは watchdog（apogee を 1 周期検出でき
                // なかった場合の脱出）にだけ使う。
                let radial_rate = r_vec.dot(&v_vec);
                let was_ascending = self.prev_radial_rate.unwrap_or(radial_rate) > 0.0;
                let watchdog_expired = self.coast_watchdog_t.is_some_and(|tw| input.t >= tw);
                if (was_ascending && radial_rate <= 0.0) || watchdog_expired {
                    self.phase = Phase::SecondBurn;
                    1.0
                } else {
                    0.0
                }
            }
            Phase::SecondBurn => {
                // Predict next step's SMA from the previous step's delta and
                // taper the throttle in the final step so we land near the
                // target instead of overshooting by a whole step's worth of
                // burn. Without this, sat-2 ended up at SMA=6957 km (target
                // 6928) which left a visible eccentricity and made Δφ
                // oscillate at the orbital period.
                if sma >= self.target_r_km {
                    self.phase = Phase::Trim;
                    0.0
                } else {
                    let dsma = self
                        .prev_sma_km
                        .map(|prev| (sma - prev).max(0.0))
                        .unwrap_or(0.0);
                    if dsma > 0.0 && sma + dsma >= self.target_r_km {
                        // fractional throttle to land exactly at target
                        ((self.target_r_km - sma) / dsma).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                }
            }
            Phase::Trim => {
                // Same predictive tapering for TCM burns.
                if sma < self.target_r_km - self.deadband_km {
                    let dsma = self
                        .prev_sma_km
                        .map(|prev| (sma - prev).max(0.0))
                        .unwrap_or(0.0);
                    if dsma > 0.0 && sma + dsma >= self.target_r_km {
                        ((self.target_r_km - sma) / dsma).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                } else {
                    0.0
                }
            }
        };
        self.prev_sma_km = Some(sma);
        self.prev_radial_rate = Some(r_vec.dot(&v_vec));

        // 姿勢 target: body-Y を velocity 方向、body-Z を orbit normal に向ける。
        // Parked 中でも attitude tracking は有効にしておく（姿勢が安定していないと
        // FirstBurn 開始時に誤差が大きい）。
        let y_target = v_vec.normalize();
        let z_target = r_vec.cross(&v_vec).normalize();
        let x_target = y_target.cross(&z_target);
        let rot = Matrix3::from_columns(&[x_target, y_target, z_target]);
        let q_target = UnitQuaternion::from_matrix(&rot);

        let att = &input.spacecraft.attitude.orientation;
        let q_current =
            UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(att.w, att.x, att.y, att.z));
        let q_err = q_target.inverse() * q_current;
        let q_err = if q_err.w < 0.0 {
            UnitQuaternion::from_quaternion(-q_err.into_inner())
        } else {
            q_err
        };
        let theta = 2.0 * q_err.vector();
        let omega_body = Vector3::new(
            input.spacecraft.attitude.angular_velocity.x,
            input.spacecraft.attitude.angular_velocity.y,
            input.spacecraft.attitude.angular_velocity.z,
        );
        let tau = -self.kp * theta - self.kd * omega_body;

        let rw_torques = if self.num_rws == 3 {
            vec![-tau.x, -tau.y, -tau.z]
        } else {
            let mut v = vec![0.0; self.num_rws];
            if !v.is_empty() {
                v[0] = -tau.x;
            }
            if v.len() > 1 {
                v[1] = -tau.y;
            }
            if v.len() > 2 {
                v[2] = -tau.z;
            }
            v
        };

        Ok(Some(Command {
            rw: Some(RwCommand::Torques(rw_torques)),
            mtq: None,
            thruster: Some(ThrusterCommand::Throttles(vec![
                throttle;
                self.num_thrusters
            ])),
        }))
    }

    fn current_mode(&self) -> Option<&str> {
        Some(self.phase.as_str())
    }
}

orts_plugin!(ConstellationPhasing, mode);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    target_altitude_km: f64,
    raise_delay_s: f64,
    mu_km3_s2: f64,
    deadband_km: f64,
    num_thrusters: usize,
    num_rws: usize,
    kp: f64,
    kd: f64,
    sample_period: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_altitude_km: 550.0,
            raise_delay_s: 0.0,
            mu_km3_s2: 398_600.441_8,
            deadband_km: 1.0,
            num_thrusters: 1,
            num_rws: 3,
            kp: 10.0,
            kd: 20.0,
            sample_period: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 398_600.441_8;

    /// 近点通過から `dt` 秒後の 2 体問題の状態（軌道面 = x-y）。
    /// Kepler 方程式を Newton 法で解くだけの self-contained な伝播。
    fn kepler_state(a: f64, e: f64, dt: f64) -> (Vector3<f64>, Vector3<f64>) {
        let n = (MU / (a * a * a)).sqrt();
        let m = n * dt;
        let mut ecc = m;
        for _ in 0..60 {
            let step = (ecc - e * ecc.sin() - m) / (1.0 - e * ecc.cos());
            ecc -= step;
            if step.abs() < 1e-14 {
                break;
            }
        }
        let b = a * (1.0 - e * e).sqrt();
        let ecc_dot = n / (1.0 - e * ecc.cos());
        (
            Vector3::new(a * (ecc.cos() - e), b * ecc.sin(), 0.0),
            Vector3::new(-a * ecc.sin() * ecc_dot, b * ecc.cos() * ecc_dot, 0.0),
        )
    }

    fn tick(t: f64, r: Vector3<f64>, v: Vector3<f64>) -> TickInput {
        TickInput {
            t,
            spacecraft: SpacecraftState {
                orbit: OrbitalState {
                    position: PositionEciKm {
                        x: r.x,
                        y: r.y,
                        z: r.z,
                    },
                    velocity: VelocityEciKms {
                        x: v.x,
                        y: v.y,
                        z: v.z,
                    },
                },
                attitude: AttitudeState {
                    orientation: Quat {
                        w: 1.0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    angular_velocity: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                mass: 500.0,
            },
            epoch: None,
            sensors: Sensors {
                magnetometers: vec![],
                gyroscopes: vec![],
                star_trackers: vec![],
                sun_sensors: vec![],
            },
            actuators: ActuatorTelemetry { rw: None },
        }
    }

    const R_PARK: f64 = EARTH_RADIUS_KM + 350.0;
    const R_TARGET: f64 = EARTH_RADIUS_KM + 550.0;
    const CONFIG: &str = r#"{"target_altitude_km":550.0,"raise_delay_s":0.0,"sample_period":1.0}"#;

    fn transfer_half_period() -> f64 {
        let a = (R_PARK + R_TARGET) / 2.0;
        std::f64::consts::PI * (a.powi(3) / MU).sqrt()
    }

    /// FirstBurn 終了を transfer ellipse 上の `burn_end`（perigee 通過からの
    /// 経過時間）に置き、そこから 1 s 刻みで伝播して SecondBurn に入る tick を
    /// 返す。戻り値は `(遷移時刻, r, v, 1 sample 前の r·v)`。
    fn coast_until_second_burn(burn_end: f64) -> (f64, Vector3<f64>, Vector3<f64>, f64) {
        let a = (R_PARK + R_TARGET) / 2.0;
        let e = (R_TARGET - R_PARK) / (R_TARGET + R_PARK);
        let half_period = transfer_half_period();

        let mut ctrl = ConstellationPhasing::init(CONFIG).expect("config");

        // t=0: parking 円軌道。ここで transfer_sma がキャッシュされる。
        let v_circ = (MU / R_PARK).sqrt();
        ctrl.update(&tick(
            0.0,
            Vector3::new(R_PARK, 0.0, 0.0),
            Vector3::new(0.0, v_circ, 0.0),
        ))
        .expect("no error");
        assert_eq!(ctrl.current_mode(), Some("first_burn"));

        // transfer ellipse 上を 1 s 刻みで伝播する。sma を確実に
        // transfer_sma 以上にするため a をごくわずか大きくとる。
        let a_prop = a * (1.0 + 1e-9);
        let mut prev_radial_rate = f64::NAN;
        let mut t = burn_end;
        while t < burn_end + 4.0 * half_period {
            let (r, v) = kepler_state(a_prop, e, t);
            ctrl.update(&tick(t, r, v)).expect("no error");
            if ctrl.current_mode() == Some("second_burn") {
                return (t, r, v, prev_radial_rate);
            }
            prev_radial_rate = r.dot(&v);
            t += 1.0;
        }
        panic!("Coast から SecondBurn へ遷移するはず");
    }

    fn assert_switched_at_apogee(t_switch: f64, r: Vector3<f64>, v: Vector3<f64>, prev: f64) {
        assert!(
            prev > 0.0,
            "apogee の 1 sample 前はまだ上昇中のはず: r·v = {prev}"
        );
        assert!(
            r.dot(&v) <= 0.0,
            "遷移 tick では下降に転じているはず: r·v = {}",
            r.dot(&v)
        );
        let flight_path = r.dot(&v) / (r.norm() * v.norm());
        assert!(
            flight_path.abs() < 1e-3,
            "SecondBurn は apogee で始まるはず: sin(fpa) = {flight_path:.3e} at t = {t_switch}"
        );
    }

    /// SecondBurn は apogee で始まらなければならない。
    ///
    /// FirstBurn 終了時刻から半周期を数えると、burn 中に飛んだ弧の分だけ
    /// apogee を過ぎてから噴く（同梱設定では 2808 s の半周期に対し burn
    /// 5.6 s、ここでは低推力を模して 300 s）。
    #[test]
    fn second_burn_starts_at_apogee() {
        let half_period = transfer_half_period();
        let (t_switch, r, v, prev) = coast_until_second_burn(300.0);

        assert_switched_at_apogee(t_switch, r, v, prev);
        assert!(
            (t_switch - half_period).abs() <= 1.0,
            "apogee は t = {half_period:.1} s、遷移は t = {t_switch:.1} s"
        );
    }

    /// 降下中に Coast へ入った場合（apogee を過ぎてから burn が終わった等）は、
    /// その場で噴かずに次の apogee を待つ。`r·v <= 0` を単発で見ると、遷移直後の
    /// 任意の点で噴いてしまう。
    #[test]
    fn coast_entered_while_descending_waits_for_the_next_apogee() {
        let half_period = transfer_half_period();
        // apogee の 500 s 後（降下中）で FirstBurn を終える。
        let (t_switch, r, v, prev) = coast_until_second_burn(half_period + 500.0);

        assert_switched_at_apogee(t_switch, r, v, prev);
        let next_apogee = 3.0 * half_period;
        assert!(
            (t_switch - next_apogee).abs() <= 1.0,
            "次の apogee は t = {next_apogee:.1} s、遷移は t = {t_switch:.1} s"
        );
    }
}
