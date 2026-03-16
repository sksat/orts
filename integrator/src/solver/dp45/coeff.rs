// ---------------------------------------------------------------------------
// Dormand-Prince RK5(4)7M coefficients (Dormand & Prince, 1980)
// ---------------------------------------------------------------------------

// Nodes (c_i)
pub(super) const DP_C2: f64 = 1.0 / 5.0;
pub(super) const DP_C3: f64 = 3.0 / 10.0;
pub(super) const DP_C4: f64 = 4.0 / 5.0;
pub(super) const DP_C5: f64 = 8.0 / 9.0;
// c6 = 1.0, c7 = 1.0 (used inline)

// a-matrix coefficients
pub(super) const DP_A21: f64 = 1.0 / 5.0;

pub(super) const DP_A31: f64 = 3.0 / 40.0;
pub(super) const DP_A32: f64 = 9.0 / 40.0;

pub(super) const DP_A41: f64 = 44.0 / 45.0;
pub(super) const DP_A42: f64 = -56.0 / 15.0;
pub(super) const DP_A43: f64 = 32.0 / 9.0;

pub(super) const DP_A51: f64 = 19372.0 / 6561.0;
pub(super) const DP_A52: f64 = -25360.0 / 2187.0;
pub(super) const DP_A53: f64 = 64448.0 / 6561.0;
pub(super) const DP_A54: f64 = -212.0 / 729.0;

pub(super) const DP_A61: f64 = 9017.0 / 3168.0;
pub(super) const DP_A62: f64 = -355.0 / 33.0;
pub(super) const DP_A63: f64 = 46732.0 / 5247.0;
pub(super) const DP_A64: f64 = 49.0 / 176.0;
pub(super) const DP_A65: f64 = -5103.0 / 18656.0;

// 5th-order weights (b_i) — also row 7 of a-matrix (FSAL property)
pub(super) const DP_B1: f64 = 35.0 / 384.0;
// DP_B2 = 0
pub(super) const DP_B3: f64 = 500.0 / 1113.0;
pub(super) const DP_B4: f64 = 125.0 / 192.0;
pub(super) const DP_B5: f64 = -2187.0 / 6784.0;
pub(super) const DP_B6: f64 = 11.0 / 84.0;
// DP_B7 = 0

// Error coefficients (e_i = b_i - b*_i)
pub(super) const DP_E1: f64 = 71.0 / 57600.0;
// DP_E2 = 0
pub(super) const DP_E3: f64 = -71.0 / 16695.0;
pub(super) const DP_E4: f64 = 71.0 / 1920.0;
pub(super) const DP_E5: f64 = -17253.0 / 339200.0;
pub(super) const DP_E6: f64 = 22.0 / 525.0;
pub(super) const DP_E7: f64 = -1.0 / 40.0;

// Step-size controller constants
pub(super) const DP_SAFETY: f64 = 0.9;
pub(super) const DP_MIN_FACTOR: f64 = 0.2;
pub(super) const DP_MAX_FACTOR: f64 = 5.0;
