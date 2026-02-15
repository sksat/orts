use wasm_bindgen::prelude::*;

use crate::epoch::Epoch;
use crate::Eci;
use nalgebra::Vector3;

/// Batch ECI→ECEF transform with per-point time.
///
/// `positions`: flat `[x0,y0,z0, x1,y1,z1, ...]` (length = N×3, km)
/// `times`: `[t0, t1, ...]` (length = N, simulation elapsed seconds)
/// `epoch_jd`: Julian Date of the simulation epoch
///
/// Returns flat ECEF `[ex0,ey0,ez0, ...]` (length = N×3, km).
///
/// For each point, computes ERA from `epoch_jd + t` and applies the
/// Z-axis rotation (ECI→ECEF).
#[wasm_bindgen]
pub fn eci_to_ecef_batch(positions: &[f32], times: &[f32], epoch_jd: f64) -> Vec<f32> {
    let n = times.len();
    debug_assert_eq!(positions.len(), n * 3);

    let mut out = Vec::with_capacity(n * 3);

    for i in 0..n {
        let epoch = Epoch::from_jd(epoch_jd).add_seconds(times[i] as f64);
        let gmst = epoch.gmst();

        let off = i * 3;
        let eci = Eci(Vector3::new(
            positions[off] as f64,
            positions[off + 1] as f64,
            positions[off + 2] as f64,
        ));
        let ecef = eci.to_ecef(gmst);

        out.push(ecef.0.x as f32);
        out.push(ecef.0.y as f32);
        out.push(ecef.0.z as f32);
    }

    out
}
