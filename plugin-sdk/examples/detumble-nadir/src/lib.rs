//! Detumble → nadir pointing モード遷移デモ — コールバック型。
//!
//! enum でモードを表現し、収束条件で遷移する。
//!
//! - **Detumble**: B-dot 則 `m = +k (ω × B)` で MTQ を駆動し `|ω|` を落とす。
//! - **Nadir**: 軌道状態から LVLH フレームを組んで目標姿勢とし、誤差
//!   クォータニオンと軌道角速度に対する PD を RW トルクとして出す。
//!   `orts::attitude::control::{NadirPointing, TrackingPdController}` と
//!   同じ定義・同じ誤差規約を使う。

use nalgebra::{Matrix3, Rotation3, UnitQuaternion, Vector3};
use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, orts_plugin};

enum Mode {
    Detumble {
        gain: f64,
        max_moment: f64,
        omega_threshold: f64,
    },
    Nadir {
        kp: f64,
        kd: f64,
    },
}

impl From<&Mode> for &'static str {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Detumble { .. } => "detumble",
            Mode::Nadir { .. } => "nadir",
        }
    }
}

struct Controller {
    mode: Mode,
    sample_period: f64,
    // nadir パラメータ（遷移時に使う）
    nadir_kp: f64,
    nadir_kd: f64,
}

impl Plugin<TickInput, Command> for Controller {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn init(config: &str) -> Result<Self, String> {
        let cfg: Config = if config.is_empty() {
            Config::default()
        } else {
            serde_json::from_str(config).map_err(|e| format!("config parse error: {e}"))?
        };
        Ok(Self {
            mode: Mode::Detumble {
                gain: cfg.detumble_gain,
                max_moment: cfg.max_moment,
                omega_threshold: cfg.omega_threshold,
            },
            sample_period: cfg.sample_period,
            nadir_kp: cfg.nadir_kp,
            nadir_kd: cfg.nadir_kd,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        match &self.mode {
            Mode::Detumble {
                gain,
                max_moment,
                omega_threshold,
            } => {
                let omega = match input.sensors.gyroscopes.first() {
                    Some(g) => Vector3::new(g.x, g.y, g.z),
                    None => return Ok(Some(stop_detumble())),
                };

                if omega.norm() < *omega_threshold {
                    self.mode = Mode::Nadir {
                        kp: self.nadir_kp,
                        kd: self.nadir_kd,
                    };
                    // MTQ は明示的にゼロを指令する。`mtq: None` は「コマンド
                    // なし = 前回値を ZOH 保持」なので、detumble 最後の磁気
                    // モーメントが nadir フェーズ中ずっと通電し続けてしまう。
                    return Ok(Some(stop_detumble()));
                }

                let b = match input.sensors.magnetometers.first() {
                    Some(m) => Vector3::new(m.x, m.y, m.z),
                    None => return Ok(Some(stop_detumble())),
                };
                if b.norm_squared() < 1e-60 {
                    return Ok(Some(stop_detumble()));
                }

                // B-dot law: m = -k dB/dt, and in the body frame
                // dB/dt ~ -omega x B, so m = +k (omega x B). The negated form
                // commands the opposite moment and *spins the satellite up*.
                // Matches `orts::attitude::control::BdotCross`.
                let m = *gain * omega.cross(&b);
                let max = *max_moment;

                Ok(Some(Command {
                    mtq: Some(MtqCommand::Moments(vec![
                        m.x.clamp(-max, max),
                        m.y.clamp(-max, max),
                        m.z.clamp(-max, max),
                    ])),
                    rw: None,
                    thruster: None,
                }))
            }

            Mode::Nadir { kp, kd } => {
                let att = match input.sensors.star_trackers.first() {
                    Some(a) => a,
                    None => return Ok(Some(stop_nadir())),
                };
                let omega = match input.sensors.gyroscopes.first() {
                    Some(g) => Vector3::new(g.x, g.y, g.z),
                    None => return Ok(Some(stop_nadir())),
                };

                // star tracker は body→inertial 姿勢を返す。nadir 指向の
                // 誤差はこれを LVLH 目標姿勢と比べて初めて得られる。
                let q_measured = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                    att.w, att.x, att.y, att.z,
                ));

                let Some((q_target, omega_target)) = lvlh_target(&input.spacecraft.orbit) else {
                    // 半径 0 または r ∥ v（純径方向軌道）では LVLH が定義でき
                    // ない。目標が無いままトルク指令を残すと wheel が飽和する。
                    return Ok(Some(stop_nadir()));
                };

                // 左不変誤差 q_err = q_target⁻¹ q_measured（body frame の誤差）。
                let q_err = q_target.inverse() * q_measured;
                // 半球選択（最短経路）。
                let q_err = if q_err.w < 0.0 {
                    UnitQuaternion::from_quaternion(-q_err.into_inner())
                } else {
                    q_err
                };
                let theta = 2.0 * q_err.vector();

                // omega_target は目標(LVLH)フレーム成分なので、現 body frame へ
                // 移してから角速度誤差を取る。q_err は current→target なので逆変換。
                let omega_error = omega - q_err.inverse() * omega_target;
                let tau = -*kp * theta - *kd * omega_error;

                // Per-wheel motor torque (Newton's 3rd law for orthogonal 3-axis)
                Ok(Some(Command {
                    rw: Some(RwCommand::Torques(vec![-tau.x, -tau.y, -tau.z])),
                    mtq: None,
                    thruster: None,
                }))
            }
        }
    }

    fn current_mode(&self) -> Option<&str> {
        Some((&self.mode).into())
    }
}

/// Detumble が駆動するアクチュエータ（MTQ）を止める指令。
///
/// WIT の `command` 契約では、指令を省く（`mtq: None`）のは「前回値を ZOH
/// 保持」であってゼロではない。センサが欠けて B-dot を組めないときに指令を
/// 返さないと、直前の磁気モーメントが通電し続ける。制御できない間は切る。
fn stop_detumble() -> Command {
    Command {
        mtq: Some(MtqCommand::Moments(vec![0.0; 3])),
        rw: None,
        thruster: None,
    }
}

/// Nadir が駆動するアクチュエータ（RW）を止める指令。
///
/// こちらは ZOH 保持の害がさらに大きい: トルク指令が残り続けると wheel は
/// 飽和へ向かって加速し続ける。姿勢・角速度・軌道のいずれかが使えない間は
/// トルク 0 を指令し、wheel を現在の角運動量で惰走させる。
fn stop_nadir() -> Command {
    Command {
        rw: Some(RwCommand::Torques(vec![0.0; 3])),
        mtq: None,
        thruster: None,
    }
}

/// LVLH (nadir 指向) 目標姿勢と目標角速度を軌道状態から求める。
///
/// `orts::attitude::control::NadirPointing` と同じ定義:
///
/// - `z_lvlh = -r̂`（nadir）
/// - `y_lvlh = -ĥ`（軌道法線の逆、`h = r × v`）
/// - `x_lvlh = y_lvlh × z_lvlh`（円軌道なら概ね速度方向）
///
/// 戻り値は `(q_target, omega_target)`。`q_target` は body→inertial、
/// `omega_target` は LVLH フレーム成分の `[0, -n, 0]`（`n = |h|/r²` \[rad/s\]）。
/// `r` または `h` が退化している場合は `None`。
fn lvlh_target(orbit: &OrbitalState) -> Option<(UnitQuaternion<f64>, Vector3<f64>)> {
    let r = Vector3::new(orbit.position.x, orbit.position.y, orbit.position.z);
    let v = Vector3::new(orbit.velocity.x, orbit.velocity.y, orbit.velocity.z);
    let h = r.cross(&v);
    let r_mag = r.norm();
    let h_mag = h.norm();
    // 0 除算を floor で弾く（`arika::rsw_quaternion` と同じ方針）。非有限
    // 入力も同時に落とす: NaN は比較が全て false になるため明示的に検査する。
    const DEGENERATE: f64 = 1e-10;
    let well_posed =
        r_mag.is_finite() && h_mag.is_finite() && r_mag > DEGENERATE && h_mag > DEGENERATE;
    if !well_posed {
        return None;
    }

    let z_lvlh = -r / r_mag;
    let y_lvlh = -h / h_mag;
    let x_lvlh = y_lvlh.cross(&z_lvlh);
    let rot = Matrix3::from_columns(&[x_lvlh, y_lvlh, z_lvlh]);
    let q_target = UnitQuaternion::from_rotation_matrix(&Rotation3::from_matrix_unchecked(rot));

    let n = h_mag / (r_mag * r_mag);
    Some((q_target, Vector3::new(0.0, -n, 0.0)))
}

orts_plugin!(Controller, mode);

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    sample_period: f64,
    detumble_gain: f64,
    max_moment: f64,
    omega_threshold: f64,
    nadir_kp: f64,
    nadir_kd: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_period: 1.0,
            detumble_gain: 1e4,
            max_moment: 10.0,
            omega_threshold: 0.01,
            nadir_kp: 1.0,
            nadir_kd: 2.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 398_600.441_8;
    const R_CIRC: f64 = 6_878.137;

    /// 赤道円軌道の状態（x 軸上、+y 方向へ prograde）。
    fn circular_orbit() -> (Vector3<f64>, Vector3<f64>) {
        (
            Vector3::new(R_CIRC, 0.0, 0.0),
            Vector3::new(0.0, (MU / R_CIRC).sqrt(), 0.0),
        )
    }

    fn tick_input(
        r: Vector3<f64>,
        v: Vector3<f64>,
        q: UnitQuaternion<f64>,
        omega: Vector3<f64>,
    ) -> TickInput {
        TickInput {
            t: 0.0,
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
                        w: q.w,
                        x: q.i,
                        y: q.j,
                        z: q.k,
                    },
                    angular_velocity: Vec3 {
                        x: omega.x,
                        y: omega.y,
                        z: omega.z,
                    },
                },
                mass: 100.0,
            },
            epoch: None,
            sensors: Sensors {
                magnetometers: vec![MagneticFieldBody {
                    x: 0.0,
                    y: 0.0,
                    z: 3e-5,
                }],
                gyroscopes: vec![AngularVelocityBody {
                    x: omega.x,
                    y: omega.y,
                    z: omega.z,
                }],
                star_trackers: vec![AttitudeBodyToInertial {
                    w: q.w,
                    x: q.i,
                    y: q.j,
                    z: q.k,
                }],
                sun_sensors: vec![],
            },
            actuators: ActuatorTelemetry { rw: None },
        }
    }

    fn nadir_controller() -> Controller {
        let mut c = Controller::init("").expect("default config");
        c.mode = Mode::Nadir {
            kp: c.nadir_kp,
            kd: c.nadir_kd,
        };
        c
    }

    fn rw_torques(cmd: &Command) -> Vec<f64> {
        match cmd.rw.as_ref().expect("nadir mode must command the wheels") {
            RwCommand::Torques(t) => t.clone(),
            other => panic!("expected wheel torques, got {other:?}"),
        }
    }

    /// LVLH 目標姿勢の定義そのものを固定する（`orts` の
    /// `nadir_z_axis_points_toward_earth` と同じ不変量）。
    #[test]
    fn lvlh_target_points_body_z_at_nadir() {
        let (r, v) = circular_orbit();
        let input = tick_input(r, v, UnitQuaternion::identity(), Vector3::zeros());
        let (q_target, omega_target) = lvlh_target(&input.spacecraft.orbit).expect("well-posed");

        let z_body = q_target * Vector3::z();
        assert!(
            (z_body - (-r.normalize())).norm() < 1e-14,
            "body +Z must point nadir, got {z_body:?}"
        );
        let h_hat = r.cross(&v).normalize();
        let y_body = q_target * Vector3::y();
        assert!(
            (y_body - (-h_hat)).norm() < 1e-14,
            "body +Y must point along -h, got {y_body:?}"
        );

        // 円軌道の軌道角速度 n = v/r（LVLH 成分で -Y 周り）。
        let n = v.norm() / r.norm();
        assert!((omega_target - Vector3::new(0.0, -n, 0.0)).norm() < 1e-15);
        assert!(n > 1e-3, "LEO の軌道角速度は ~1.1e-3 rad/s のはず: {n}");
    }

    /// 姿勢と角速度が LVLH 目標に一致していればトルクは 0。
    /// 慣性 identity 保持則ならここで大きなトルクを出す。
    #[test]
    fn nadir_commands_no_torque_while_tracking() {
        let (r, v) = circular_orbit();
        let (q_target, omega_target) = lvlh_target(&OrbitalState {
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
        })
        .expect("well-posed");

        let mut ctrl = nadir_controller();
        let cmd = ctrl
            .update(&tick_input(r, v, q_target, omega_target))
            .expect("no error")
            .expect("command");

        let torques = rw_torques(&cmd);
        let mag = torques.iter().fold(0.0_f64, |m, t| m.max(t.abs()));
        assert!(
            mag < 1e-12,
            "tracking the LVLH target must need no torque, got {torques:?}"
        );
    }

    /// nadir 周りの yaw 誤差に対して復元トルクを出す（大きさは kp·θ）。
    #[test]
    fn nadir_torque_restores_a_yaw_offset() {
        let (r, v) = circular_orbit();
        let (q_target, omega_target) = lvlh_target(
            &tick_input(r, v, UnitQuaternion::identity(), Vector3::zeros())
                .spacecraft
                .orbit,
        )
        .expect("well-posed");

        let offset = 5.0_f64.to_radians();
        let q_err = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), offset);
        let q_measured = q_target * q_err;
        // 角速度誤差 0 になるよう、目標角速度を現 body frame に写して与える。
        let omega = q_err.inverse() * omega_target;

        let mut ctrl = nadir_controller();
        let cmd = ctrl
            .update(&tick_input(r, v, q_measured, omega))
            .expect("no error")
            .expect("command");
        let torques = rw_torques(&cmd);

        // wheel torque = -body torque。body への復元トルクは -Z 周り。
        assert!(
            torques[2] > 0.0,
            "wheel torque about +Z must absorb the -Z restoring torque, got {torques:?}"
        );
        let expected = ctrl.nadir_kp * offset;
        let rel_err = ((torques[2] - expected) / expected).abs();
        assert!(
            rel_err < 0.01,
            "|tau_z| ~ kp*theta = {expected:.5}, got {:.5}",
            torques[2]
        );
        for (axis, tau) in [("x", torques[0]), ("y", torques[1])] {
            assert!(
                tau.abs() < 1e-12,
                "pure yaw offset must not torque {axis}: {tau}"
            );
        }
    }

    /// r ∥ v（純径方向）では LVLH が定義できない。目標が無いので PD は組めない
    /// が、指令を省くと直前の RW トルクが ZOH で残り wheel が飽和へ向かうので、
    /// トルク 0 を明示的に指令する。
    #[test]
    fn nadir_stops_the_wheels_on_a_degenerate_orbit() {
        let r = Vector3::new(R_CIRC, 0.0, 0.0);
        let v = Vector3::new(1.0, 0.0, 0.0);
        assert!(
            lvlh_target(
                &tick_input(r, v, UnitQuaternion::identity(), Vector3::zeros())
                    .spacecraft
                    .orbit
            )
            .is_none()
        );

        let mut ctrl = nadir_controller();
        let cmd = ctrl
            .update(&tick_input(
                r,
                v,
                UnitQuaternion::identity(),
                Vector3::zeros(),
            ))
            .expect("no error")
            .expect("a degenerate orbit must still stop the wheels");
        let torques = rw_torques(&cmd);
        assert!(
            torques.iter().all(|t| *t == 0.0),
            "expected zero wheel torque, got {torques:?}"
        );
    }

    /// センサが欠けて B-dot を組めない tick でも、指令を省かずゼロを出す。
    /// 省くと直前の磁気モーメントが ZOH で通電し続ける。
    #[test]
    fn detumble_without_a_magnetometer_zeroes_the_mtq() {
        let (r, v) = circular_orbit();
        let mut ctrl = Controller::init("").expect("default config");

        // |omega| = 0.1 rad/s は遷移閾値 (1e-2) より大きいので detumble のまま。
        let mut input = tick_input(
            r,
            v,
            UnitQuaternion::identity(),
            Vector3::new(0.1, 0.0, 0.0),
        );
        input.sensors.magnetometers.clear();

        let cmd = ctrl
            .update(&input)
            .expect("no error")
            .expect("a missing magnetometer must still stop the MTQ");
        assert_eq!(ctrl.current_mode(), Some("detumble"));
        match cmd.mtq {
            Some(MtqCommand::Moments(ref m)) => assert!(
                m.iter().all(|v| *v == 0.0),
                "expected zero magnetic moment, got {m:?}"
            ),
            other => panic!("expected an explicit zero MTQ command, got {other:?}"),
        }
    }

    /// detumble → nadir の遷移 tick で MTQ を明示的に 0 にする。
    /// `mtq: None` は「前回値を ZOH 保持」なので、最後の磁気モーメントが
    /// nadir フェーズ中ずっと残ってしまう。
    #[test]
    fn transition_to_nadir_zeroes_the_mtq() {
        let (r, v) = circular_orbit();
        let mut ctrl = Controller::init("").expect("default config");
        assert_eq!(ctrl.current_mode(), Some("detumble"));

        // |omega| = 1e-3 < omega_threshold (1e-2) で遷移条件を満たす。
        let input = tick_input(
            r,
            v,
            UnitQuaternion::identity(),
            Vector3::new(1e-3, 0.0, 0.0),
        );
        let cmd = ctrl
            .update(&input)
            .expect("no error")
            .expect("transition tick must command something");

        assert_eq!(ctrl.current_mode(), Some("nadir"));
        match cmd.mtq {
            Some(MtqCommand::Moments(ref m)) => assert!(
                m.iter().all(|v| *v == 0.0),
                "transition must zero the MTQ, got {m:?}"
            ),
            other => panic!("expected an explicit zero MTQ command, got {other:?}"),
        }
    }
    /// 傾斜円軌道上の状態を時刻 `t` で返す（`R_CIRC` 半径、軌道面は X 軸周りに
    /// `inc_rad` 傾ける）。
    fn inclined_circular_orbit(inc_rad: f64, t: f64) -> (Vector3<f64>, Vector3<f64>) {
        let n = (MU / R_CIRC.powi(3)).sqrt();
        let u = Vector3::new(1.0, 0.0, 0.0);
        let w = Vector3::new(0.0, inc_rad.cos(), inc_rad.sin());
        let (s, c) = (n * t).sin_cos();
        (R_CIRC * (c * u + s * w), R_CIRC * n * (-s * u + c * w))
    }

    /// 剛体 + 理想 wheel の閉ループで、body +Z が nadir に乗り、そのまま
    /// **回り続ける**ことを確認する。
    ///
    /// 慣性固定則は「収束後は静止」なので、姿勢誤差の瞬間値ではなく軌道弧に
    /// わたる追従で区別できる。収束後の body 角速度は軌道角速度に一致する
    /// はずで、ゼロに落ちるならそれは慣性固定。
    #[test]
    fn nadir_tracks_the_rotating_lvlh_frame_over_an_orbit_arc() {
        // 等方慣性なのでジャイロ項 ω × (Iω) が消え、プラントは I dω/dt = τ。
        const INERTIA_KGM2: f64 = 10.0;
        const INC_RAD: f64 = 0.5;
        const DT: f64 = 0.1;
        const SAMPLE_PERIOD: f64 = 0.5;
        const DURATION: f64 = 1500.0; // 軌道弧 ~95°
        // これ以降は追従が確立していること。
        const SETTLED_AFTER: f64 = 400.0;

        let mut ctrl = nadir_controller();
        let mut q = UnitQuaternion::identity();
        let mut omega = Vector3::zeros();
        let mut torque = Vector3::zeros();
        let mut next_tick = 0.0;
        let mut worst_settled_cos = f64::INFINITY;

        let mut t = 0.0;
        while t < DURATION {
            let (r, v) = inclined_circular_orbit(INC_RAD, t);

            if t + 1e-9 >= next_tick {
                let cmd = ctrl
                    .update(&tick_input(r, v, q, omega))
                    .expect("no error")
                    .expect("nadir mode always commands the wheels");
                // wheel torque の反作用が body トルク（`RwAssemblyCore::
                // reaction_torque` と同じ符号）。
                let wheel = rw_torques(&cmd);
                torque = -Vector3::new(wheel[0], wheel[1], wheel[2]);
                next_tick += SAMPLE_PERIOD;
            }

            if t >= SETTLED_AFTER {
                let z_body = q * Vector3::z();
                worst_settled_cos = worst_settled_cos.min(z_body.dot(&(-r.normalize())));
            }

            omega += torque / INERTIA_KGM2 * DT;
            q *= UnitQuaternion::from_scaled_axis(omega * DT);
            t += DT;
        }

        assert!(
            worst_settled_cos > 0.999,
            "body +Z must stay on nadir over the whole arc; worst cos = \
             {worst_settled_cos:.6} ({:.2}° off)",
            worst_settled_cos.clamp(-1.0, 1.0).acos().to_degrees()
        );

        let orbital_rate = (MU / R_CIRC.powi(3)).sqrt();
        let rate_error = (omega.norm() - orbital_rate).abs() / orbital_rate;
        assert!(
            rate_error < 0.05,
            "converged body rate {:.4e} rad/s must match the orbital rate \
             {orbital_rate:.4e} rad/s (an inertial hold converges to zero)",
            omega.norm()
        );
    }
}
