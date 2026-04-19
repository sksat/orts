//! NOS3 generic_adcs — NASA ADCS flight software as orts WASM plugin.
//!
//! NASA NOS3 (Operational Simulator for Space Systems) の generic_adcs コンポーネントの
//! 制御アルゴリズム (fsw/shared/) を WASM Component としてビルドし、orts の SILS
//! (Software-In-the-Loop Simulation) として実行するデモ。
//!
//! ## ADCS モード
//!
//! - **Passive** (0): 制御なし
//! - **B-dot** (1): 磁気デタンブリング (magnetometer → magnetorquer)
//! - **Sun-Safe** (2): PD 太陽指向 (sun sensor → RW + MTQ momentum dump)
//! - **Inertial** (3): クォータニオン PID 3軸制御 (star tracker + gyro → RW + MTQ)
//!
//! ## NOS3 クォータニオン規約
//!
//! NOS3 は **scalar-last** `[x, y, z, w]` を使用。
//! orts WIT は **scalar-first** `{w, x, y, z}` (Hamilton 規約)。
//! 変換が必要。

mod ffi;

use orts_plugin_sdk::bindings::orts::plugin::types::*;
use orts_plugin_sdk::{Plugin, orts_plugin};

struct Controller {
    sample_period: f64,
    di: ffi::Di,
    ad: ffi::Ad,
    gnc: ffi::Gnc,
    acs: ffi::Ac,
}

/// Packed struct field access via raw pointers.
/// All NOS3 structs use `__attribute__((packed))` / `#[repr(C, packed)]`,
/// so we must use `read_unaligned` / `write_unaligned` for all field access.
macro_rules! pread {
    ($base:expr, $field:ident) => {
        unsafe { std::ptr::addr_of!((*std::ptr::addr_of!($base)).$field).read_unaligned() }
    };
}
macro_rules! pwrite {
    ($base:expr, $field:ident, $val:expr) => {
        unsafe {
            std::ptr::addr_of_mut!((*std::ptr::addr_of_mut!($base)).$field).write_unaligned($val)
        }
    };
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

        // Zero-initialize all C state
        let di: ffi::Di = unsafe { std::mem::zeroed() };
        let ad = init_ad(&cfg);
        let gnc = init_gnc(&cfg);
        let acs = init_acs(&cfg);

        Ok(Self {
            sample_period: cfg.sample_period,
            di,
            ad,
            gnc,
            acs,
        })
    }

    fn update(&mut self, input: &TickInput) -> Result<Option<Command>, String> {
        self.populate_di(input);

        unsafe {
            ffi::Generic_ADCS_execute_attitude_determination_and_attitude_control(
                std::ptr::addr_of!(self.di),
                std::ptr::addr_of_mut!(self.ad),
                std::ptr::addr_of_mut!(self.gnc),
                std::ptr::addr_of_mut!(self.acs),
            );
        }

        let tcmd: [f64; 3] = pread!(self.gnc, tcmd);
        let mcmd: [f64; 3] = pread!(self.gnc, mcmd);

        // Always send explicit commands (including zeros) so the host clears
        // stale ZOH values when the C controller transitions to zero output.
        Ok(Some(Command {
            // NOS3 GNC->Tcmd is the motor command (not body torque).
            // orts RwCommand::Torques is also motor torque, so pass directly.
            rw: Some(RwCommand::Torques(tcmd.to_vec())),
            mtq: Some(MtqCommand::Moments(mcmd.to_vec())),
            thruster: None,
        }))
    }

    fn current_mode(&self) -> Option<&str> {
        let mode: u8 = pread!(self.gnc, mode);
        Some(match mode {
            ffi::PASSIVE_MODE => "passive",
            ffi::BDOT_MODE => "bdot",
            ffi::SUNSAFE_MODE => "sunsafe",
            ffi::INERTIAL_MODE => "inertial",
            _ => "unknown",
        })
    }
}

orts_plugin!(Controller, mode);

// ── Sensor mapping ──

impl Controller {
    fn populate_di(&mut self, input: &TickInput) {
        // Magnetometer
        if let Some(mag) = input.sensors.magnetometers.first() {
            pwrite!(self.di.mag, bvb, [mag.x, mag.y, mag.z]);
        }

        // Gyroscope → IMU angular rate
        if let Some(gyro) = input.sensors.gyroscopes.first() {
            pwrite!(self.di.imu, valid, 1u8);
            pwrite!(self.di.imu, wbn, [gyro.x, gyro.y, gyro.z]);
        } else {
            pwrite!(self.di.imu, valid, 0u8);
        }

        // Star tracker
        // orts WIT: scalar-first {w, x, y, z} → NOS3: scalar-last [x, y, z, w]
        if let Some(st) = input.sensors.star_trackers.first() {
            pwrite!(self.di.st, valid, 1u8);
            pwrite!(self.di.st, q, [st.x, st.y, st.z, st.w]);
            // qbs = identity (sensor frame = body frame)
            pwrite!(self.di.st, qbs, [0.0, 0.0, 0.0, 1.0]);
        } else {
            pwrite!(self.di.st, valid, 0u8);
        }

        // Sun sensor → FSS (extract direction from variant)
        if let Some(sun) = input.sensors.sun_sensors.first() {
            match sun {
                SunSensorOutput::Fine(fine) => {
                    if let Some(dir) = &fine.direction {
                        pwrite!(self.di.fss, valid, 1u8);
                        pwrite!(self.di.fss, svb, [dir.x, dir.y, dir.z]);
                    } else {
                        // Eclipse: no sun direction available
                        pwrite!(self.di.fss, valid, 0u8);
                    }
                }
                SunSensorOutput::Coarse(_cos_angle) => {
                    // CSS scalar can't provide direction vector — mark invalid for FSS
                    pwrite!(self.di.fss, valid, 0u8);
                }
            }
        } else {
            pwrite!(self.di.fss, valid, 0u8);
        }

        // Actuator telemetry: RW momentum + max momentum
        if let Some(rw_tel) = &input.actuators.rw {
            // Per-wheel momentum → body-frame (3-axis orthogonal assumed)
            let mut hwhl_b = [0.0f64; 3];
            for (i, &h) in rw_tel.momentum.iter().enumerate().take(3) {
                hwhl_b[i] = h;
            }
            pwrite!(self.di.rw, hwhl_b, hwhl_b);

            // Max momentum per axis (from config, used by momentum management thresholds)
            let max_h: [f64; 3] = pread!(self.gnc, hwhl_max_b);
            pwrite!(self.di.rw, h_max_b, max_h);

            // Wheel axes: orthogonal
            pwrite!(
                self.di.rw,
                whl_axis,
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            );
        }
    }
}

// ── Initialization (replaces C init with FILE*) ──

fn init_ad(cfg: &Config) -> ffi::Ad {
    let mut ad: ffi::Ad = unsafe { std::mem::zeroed() };
    pwrite!(ad.imu, alpha, cfg.imu_alpha);
    ad
}

fn init_gnc(cfg: &Config) -> ffi::Gnc {
    let mut gnc: ffi::Gnc = unsafe { std::mem::zeroed() };
    pwrite!(gnc, dt, cfg.sample_period);
    pwrite!(gnc, max_mcmd, cfg.max_mcmd);
    pwrite!(gnc, mode, cfg.initial_mode);
    pwrite!(
        gnc,
        hmgmt_on,
        if cfg.momentum_management { 1u8 } else { 0u8 }
    );
    pwrite!(gnc.hmgmt, kb, cfg.hmgmt_kb);
    pwrite!(gnc.hmgmt, b_range, cfg.hmgmt_b_range);
    pwrite!(gnc.hmgmt, lo_frac, cfg.hmgmt_lo_frac);
    pwrite!(gnc.hmgmt, hi_frac, cfg.hmgmt_hi_frac);
    pwrite!(gnc, hwhl_max_b, [cfg.rw_max_momentum; 3]);
    gnc
}

fn init_acs(cfg: &Config) -> ffi::Ac {
    let mut acs: ffi::Ac = unsafe { std::mem::zeroed() };
    pwrite!(acs.bdot, b_range, cfg.bdot_b_range);
    pwrite!(acs.bdot, kb, cfg.bdot_kb);
    pwrite!(acs.sunsafe, kp, cfg.sunsafe_kp);
    pwrite!(acs.sunsafe, kr, cfg.sunsafe_kr);
    pwrite!(acs.sunsafe, sside, cfg.sunsafe_sside);
    pwrite!(acs.sunsafe, vmax, cfg.sunsafe_vmax);
    pwrite!(acs.inertial, kp, cfg.inertial_kp);
    pwrite!(acs.inertial, kr, cfg.inertial_kr);
    pwrite!(acs.inertial, ki, cfg.inertial_ki);
    pwrite!(acs.inertial, phi_err_max, cfg.inertial_phi_err_max);
    pwrite!(acs.inertial, qbn_cmd, cfg.inertial_qbn_cmd);
    pwrite!(
        acs.inertial,
        h_mgmt,
        if cfg.momentum_management { 1 } else { 0 } as std::ffi::c_long
    );
    acs
}

// ── Configuration ──

#[derive(serde::Deserialize)]
#[serde(default)]
struct Config {
    sample_period: f64,
    initial_mode: u8,
    momentum_management: bool,
    max_mcmd: f64,
    rw_max_momentum: f64,
    imu_alpha: f64,
    bdot_b_range: f64,
    bdot_kb: f64,
    sunsafe_kp: [f64; 3],
    sunsafe_kr: [f64; 3],
    sunsafe_sside: [f64; 3],
    sunsafe_vmax: f64,
    inertial_kp: [f64; 3],
    inertial_kr: [f64; 3],
    inertial_ki: [f64; 3],
    inertial_phi_err_max: f64,
    inertial_qbn_cmd: [f64; 4], // scalar-last [x,y,z,w]
    hmgmt_kb: f64,
    hmgmt_b_range: f64,
    hmgmt_lo_frac: f64,
    hmgmt_hi_frac: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_period: 1.0,
            initial_mode: ffi::BDOT_MODE,
            momentum_management: false,
            max_mcmd: 10.0,
            rw_max_momentum: 1.0,
            imu_alpha: 0.1,
            bdot_b_range: 1e-9,
            bdot_kb: 1e4,
            sunsafe_kp: [0.01, 0.01, 0.01],
            sunsafe_kr: [0.1, 0.1, 0.1],
            sunsafe_sside: [1.0, 0.0, 0.0],
            sunsafe_vmax: 0.01,
            inertial_kp: [0.1, 0.1, 0.1],
            inertial_kr: [1.0, 1.0, 1.0],
            inertial_ki: [0.0, 0.0, 0.0],
            inertial_phi_err_max: 1.0,
            inertial_qbn_cmd: [0.0, 0.0, 0.0, 1.0],
            hmgmt_kb: 1e4,
            hmgmt_b_range: 1e-9,
            hmgmt_lo_frac: 0.2,
            hmgmt_hi_frac: 0.8,
        }
    }
}
