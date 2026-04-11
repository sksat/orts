use wasm_bindgen::prelude::*;

use crate::epoch::Epoch;
use crate::frame::{self, Rotation};
use crate::sun;
use crate::{SimpleEcef, SimpleEci};
use nalgebra::{UnitQuaternion, Vector3, Vector4};

/// Batch ECI→ECEF transform with per-point time.
///
/// `positions`: flat `[x0,y0,z0, x1,y1,z1, ...]` (length = N×3, km)
/// `times`: `[t0, t1, ...]` (length = N, simulation elapsed seconds)
/// `epoch_jd`: Julian Date of the simulation epoch
///
/// Returns flat ECEF `[ex0,ey0,ez0, ...]` (length = N×3, km).
///
/// For each point, computes ERA from `epoch_jd + t` and applies the
/// Z-axis rotation (SimpleEci → SimpleEcef).
#[wasm_bindgen]
pub fn eci_to_ecef_batch(positions: &[f32], times: &[f32], epoch_jd: f64) -> Vec<f32> {
    let n = times.len();
    debug_assert_eq!(positions.len(), n * 3);

    let mut out = Vec::with_capacity(n * 3);

    for i in 0..n {
        let epoch = Epoch::from_jd(epoch_jd).add_seconds(times[i] as f64);
        let r = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(epoch.gmst());

        let off = i * 3;
        let eci = SimpleEci::new(
            positions[off] as f64,
            positions[off + 1] as f64,
            positions[off + 2] as f64,
        );
        let ecef = r.transform(&eci);

        out.push(ecef.x() as f32);
        out.push(ecef.y() as f32);
        out.push(ecef.z() as f32);
    }

    out
}

/// Single-point ECI→ECEF transform.
///
/// Returns flat ECEF `[ex, ey, ez]` (3 floats, km).
#[wasm_bindgen]
pub fn eci_to_ecef(x: f32, y: f32, z: f32, epoch_jd: f64, t: f32) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t as f64);
    let eci = SimpleEci::new(x as f64, y as f64, z as f64);
    let ecef =
        Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(epoch.gmst()).transform(&eci);
    vec![ecef.x() as f32, ecef.y() as f32, ecef.z() as f32]
}

/// Compute the Earth Rotation Angle (ERA, historically called GMST) in radians.
///
/// `epoch_jd`: Julian Date of the simulation epoch
/// `t`: elapsed simulation time in seconds
#[wasm_bindgen]
pub fn earth_rotation_angle(epoch_jd: f64, t: f64) -> f64 {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    epoch.gmst()
}

/// Approximate sun direction (unit vector) in Gcrs frame.
///
/// Returns `[x, y, z]` (3 floats).
#[wasm_bindgen]
pub fn sun_direction_eci(epoch_jd: f64, t: f64) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    let dir = sun::sun_direction_eci(&epoch);
    vec![dir.x() as f32, dir.y() as f32, dir.z() as f32]
}

/// Sun direction (unit vector) as seen from a given central body, in J2000 equatorial frame.
///
/// Returns `[x, y, z]` (3 floats).
/// `body`: body identifier string (e.g., "earth", "mars")
/// `epoch_jd`: Julian Date of the simulation epoch
/// `t`: elapsed simulation time in seconds
#[wasm_bindgen]
pub fn sun_direction_from_body(body: &str, epoch_jd: f64, t: f64) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    let dir = sun::sun_direction_from_body(body, &epoch);
    vec![dir.x() as f32, dir.y() as f32, dir.z() as f32]
}

/// Sun distance [km] from a given central body.
///
/// `body`: body identifier string (e.g., "earth", "mars")
/// `epoch_jd`: Julian Date of the simulation epoch
/// `t`: elapsed simulation time in seconds
#[wasm_bindgen]
pub fn sun_distance_from_body(body: &str, epoch_jd: f64, t: f64) -> f64 {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    sun::sun_distance_from_body(body, &epoch)
}

/// Convert Julian Date + elapsed sim time to a UTC date/time string.
///
/// Returns ISO 8601 string like "2024-03-20T12:00:00Z".
#[wasm_bindgen]
pub fn jd_to_utc_string(epoch_jd: f64, t: f64) -> String {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    epoch.to_datetime().to_string()
}

/// Geodetic (lat_deg, lon_deg, altitude_km) → SimpleEcef [km].
///
/// Returns `[x, y, z]` (3 floats, km).
#[wasm_bindgen]
pub fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, altitude_km: f64) -> Vec<f64> {
    let geod = crate::earth::Geodetic {
        latitude: lat_deg.to_radians(),
        longitude: lon_deg.to_radians(),
        altitude: altitude_km,
    };
    let ecef = SimpleEcef::from(geod);
    vec![ecef.x(), ecef.y(), ecef.z()]
}

/// Geodetic (lat_deg, lon_deg, altitude_km) → SimpleEci [km] at given epoch.
///
/// Returns `[x, y, z]` (3 floats, km).
#[wasm_bindgen]
pub fn geodetic_to_eci(lat_deg: f64, lon_deg: f64, altitude_km: f64, epoch_jd: f64) -> Vec<f64> {
    let epoch = Epoch::from_jd(epoch_jd);
    let geod = crate::earth::Geodetic {
        latitude: lat_deg.to_radians(),
        longitude: lon_deg.to_radians(),
        altitude: altitude_km,
    };
    let ecef = SimpleEcef::from(geod);
    let eci =
        Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(epoch.gmst()).transform(&ecef);
    vec![eci.x(), eci.y(), eci.z()]
}

/// Body-fixed → ECI orientation quaternion using the IAU rotation model.
///
/// `body`: body identifier string (e.g., "moon", "mars", "sun")
/// `epoch_jd`: Julian Date of the simulation epoch
/// `t`: elapsed simulation time in seconds
///
/// Returns `[w, x, y, z]` quaternion (4 f64 values, Hamilton scalar-first).
/// Returns an empty vec if the body has no IAU rotation model.
#[wasm_bindgen]
pub fn body_orientation(body: &str, epoch_jd: f64, t: f64) -> Vec<f64> {
    // JS-side callers pass a UTC JD + elapsed seconds. IAU WGCCRE 2009 takes
    // TDB, so convert UTC → TDB before calling the rotation API.
    let epoch_utc = Epoch::from_jd(epoch_jd).add_seconds(t);
    let epoch_tdb = epoch_utc.to_tdb();
    match crate::rotation::body_orientation(body, &epoch_tdb) {
        Some(q) => vec![q.w, q.i, q.j, q.k],
        None => vec![],
    }
}

/// Transform a body-to-ECI quaternion into a body-to-RSW quaternion.
///
/// `pos_x/y/z`: satellite position in ECI \[km\]
/// `vel_x/y/z`: satellite velocity in ECI \[km/s\]
/// `qw/qx/qy/qz`: body-to-ECI quaternion (Hamilton scalar-first: w,x,y,z)
///
/// Returns `[w, x, y, z]` body-to-RSW quaternion (4 floats, f64).
/// Returns an empty vec if the RSW frame cannot be computed (degenerate orbit).
#[wasm_bindgen]
pub fn body_quat_to_rsw(
    pos_x: f64,
    pos_y: f64,
    pos_z: f64,
    vel_x: f64,
    vel_y: f64,
    vel_z: f64,
    qw: f64,
    qx: f64,
    qy: f64,
    qz: f64,
) -> Vec<f64> {
    let pos = Vector3::new(pos_x, pos_y, pos_z);
    let vel = Vector3::new(vel_x, vel_y, vel_z);
    let q_body_eci =
        UnitQuaternion::from_quaternion(nalgebra::Quaternion::from(Vector4::new(qx, qy, qz, qw)));

    match crate::body_quat_to_rsw(&pos, &vel, &q_body_eci) {
        Some(q) => vec![q.w, q.i, q.j, q.k],
        None => vec![],
    }
}
