//! FFI declarations for NOS3 generic_adcs C library.
//!
//! These mirror the `_Payload_t` types in generic_adcs_msg.h (standalone shim)
//! and the function signatures in generic_adcs_adac.h.
//!
//! NOS3 quaternion convention: **scalar-last** `[x, y, z, w]`.

use std::ffi::c_double;

// ── DI (Data Ingest) ──

#[repr(C, packed)]
pub struct DiMag {
    pub qbs: [c_double; 4],
    pub bvb: [c_double; 3],
}

#[repr(C, packed)]
pub struct DiFss {
    pub qbs: [c_double; 4],
    pub valid: u8,
    pub svb: [c_double; 3],
}

#[repr(C, packed)]
pub struct DiCssSensor {
    pub axis: [c_double; 3],
    pub scale: c_double,
    pub percenton: c_double,
}

#[repr(C, packed)]
pub struct DiCss {
    pub sensor: [DiCssSensor; 6],
    pub valid: u8,
    pub svb: [c_double; 3],
}

#[repr(C, packed)]
pub struct DiImu {
    pub qbs: [c_double; 4],
    pub pos: [c_double; 3],
    pub valid: u8,
    pub wbn: [c_double; 3],
    pub acc: [c_double; 3],
}

#[repr(C, packed)]
pub struct DiRw {
    pub whl_axis: [[c_double; 3]; 3],
    pub h_max_b: [c_double; 3],
    pub hwhl_b: [c_double; 3],
}

#[repr(C, packed)]
pub struct DiSt {
    pub qbs: [c_double; 4],
    pub q: [c_double; 4],
    pub valid: u8,
}

#[repr(C, packed)]
pub struct Di {
    pub mag: DiMag,
    pub fss: DiFss,
    pub css: DiCss,
    pub imu: DiImu,
    pub rw: DiRw,
    pub st: DiSt,
}

// ── AD (Attitude Determination) ──

#[repr(C, packed)]
pub struct AdMag {
    pub bvb: [c_double; 3],
}

#[repr(C, packed)]
pub struct AdSol {
    pub sun_valid: u8,
    pub fss_valid: u8,
    pub svb: [c_double; 3],
}

#[repr(C, packed)]
pub struct AdImu {
    pub init: u8,
    pub alpha: c_double,
    pub valid: u8,
    pub wbn_prev: [c_double; 3],
    pub wbn: [c_double; 3],
    pub acc: [c_double; 3],
}

#[repr(C, packed)]
pub struct AdSt {
    pub valid: u8,
    pub qbn: [c_double; 4],
}

#[repr(C, packed)]
pub struct Ad {
    pub mag: AdMag,
    pub sol: AdSol,
    pub imu: AdImu,
    pub st: AdSt,
}

// ── GNC ──

#[repr(C, packed)]
pub struct GncHmgmt {
    pub kb: c_double,
    pub b_range: c_double,
    pub lo_frac: c_double,
    pub hi_frac: c_double,
    pub mm_active: [u8; 3],
    pub mcmd: [c_double; 3],
}

#[repr(C, packed)]
pub struct Gnc {
    pub dt: c_double,
    pub max_mcmd: c_double,
    pub mode: u8,
    pub hmgmt_on: u8,
    pub hmgmt: GncHmgmt,
    pub bvb: [c_double; 3],
    pub svb: [c_double; 3],
    pub sun_valid: u8,
    pub wbn: [c_double; 3],
    pub hwhl_max_b: [c_double; 3],
    pub hwhl_b: [c_double; 3],
    pub mcmd: [c_double; 3],
    pub tcmd: [c_double; 3],
    pub q_valid: u8,
    pub qbn: [c_double; 4],
    pub q_err: [c_double; 4],
}

// ── AC (Attitude Control) ──

#[repr(C, packed)]
pub struct AcBdot {
    pub b_range: c_double,
    pub kb: c_double,
    pub bold: [c_double; 3],
    pub bdot: [c_double; 3],
}

#[repr(C, packed)]
pub struct AcSunsafe {
    pub kp: [c_double; 3],
    pub kr: [c_double; 3],
    pub sside: [c_double; 3],
    pub vmax: c_double,
    pub cmd_wbn: [c_double; 3],
    pub h_mgmt: u8,
    pub therr: [c_double; 3],
    pub werr: [c_double; 3],
    pub tcmd: [c_double; 3],
    pub err_t: c_double,
}

#[repr(C, packed)]
pub struct AcInertial {
    pub kp: [c_double; 3],
    pub kr: [c_double; 3],
    pub ki: [c_double; 3],
    pub phi_err_max: c_double,
    pub qbn_cmd: [c_double; 4],
    pub h_mgmt: std::ffi::c_long,
    pub therr: [c_double; 3],
    pub sumtherr: [c_double; 3],
    pub q_err: [c_double; 4],
    pub werr: [c_double; 3],
    pub tcmd: [c_double; 3],
}

#[repr(C, packed)]
pub struct Ac {
    pub bdot: AcBdot,
    pub sunsafe: AcSunsafe,
    pub inertial: AcInertial,
}

// ── Mode constants ──

pub const PASSIVE_MODE: u8 = 0;
pub const BDOT_MODE: u8 = 1;
pub const SUNSAFE_MODE: u8 = 2;
pub const INERTIAL_MODE: u8 = 3;

// ── C function declarations ──

unsafe extern "C" {
    pub fn Generic_ADCS_execute_attitude_determination_and_attitude_control(
        di: *const Di,
        ad: *mut Ad,
        gnc: *mut Gnc,
        acs: *mut Ac,
    );
}
