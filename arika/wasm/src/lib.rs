//! WebAssembly bindings for the arika coordinate/epoch/ephemeris library.
//!
//! Thin `wasm_bindgen` facade exposing arika's coordinate transforms,
//! epoch conversions, and ephemeris queries to browser-side JavaScript.

use wasm_bindgen::prelude::*;

use arika::epoch::Epoch;
use arika::frame::{self, Rotation};
use arika::sun;
use arika::{SimpleEcef, SimpleEci};
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
///
/// # Errors
///
/// Returns an error (a JS exception) unless `positions.len() == times.len() * 3`.
/// The length agreement is the caller's half of the contract and is checked at
/// run time, in every build: a short `positions` would otherwise index out of
/// bounds and trap the whole wasm instance, and a long one would silently
/// return fewer points than the caller sized its read loop for.
#[wasm_bindgen]
pub fn eci_to_ecef_batch(
    positions: &[f32],
    times: &[f32],
    epoch_jd: f64,
) -> Result<Vec<f32>, JsValue> {
    batch_eci_to_ecef(positions, times, epoch_jd).map_err(|e| JsValue::from_str(&e))
}

/// Length-checked core of [`eci_to_ecef_batch`], split out because `JsValue`
/// exists only on `wasm32` — this half is unit-testable on the host target.
fn batch_eci_to_ecef(positions: &[f32], times: &[f32], epoch_jd: f64) -> Result<Vec<f32>, String> {
    let n = times.len();
    let expected = n
        .checked_mul(3)
        .ok_or("eci_to_ecef_batch: times is too long to address as N×3 positions")?;
    if positions.len() != expected {
        return Err(format!(
            "eci_to_ecef_batch: positions has {} elements, expected {expected} (3 per time, {n} times)",
            positions.len()
        ));
    }

    let mut out = Vec::with_capacity(expected);

    for (chunk, &t) in positions.chunks_exact(3).zip(times) {
        let epoch = Epoch::from_jd(epoch_jd).add_seconds(t as f64);
        let r = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(epoch.gmst());

        let eci = SimpleEci::new(chunk[0] as f64, chunk[1] as f64, chunk[2] as f64);
        let ecef = r.transform(&eci);

        out.push(ecef.x() as f32);
        out.push(ecef.y() as f32);
        out.push(ecef.z() as f32);
    }

    Ok(out)
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

/// How many values [`orbit_derived_batch`] emits per state.
pub const ORBIT_DERIVED_STRIDE: usize = 10;

/// Batch Keplerian elements plus the scalar orbit quantities charts plot.
///
/// `states`: flat `[x,y,z,vx,vy,vz, ...]` (length = N×6; km and km/s)
/// `mu`: gravitational parameter of the central body [km³/s²]
/// `body_radius`: central body radius [km], for the altitude term
///
/// Returns, per state, `[a, e, inc, raan, omega, nu, altitude,
/// specific_energy, angular_momentum, velocity]` — 10 values, angles in
/// radians. A state whose elements are undefined yields ten `NaN`s; see below.
///
/// The decoder that reads a `.rrd` recovers position and velocity only, so the
/// browser has to derive the rest. Doing it here keeps one implementation:
/// `arika::kepler::KeplerianElements::from_state_vector` is what the CLI writes into
/// CSV and sends over the WebSocket.
///
/// # Errors
///
/// Returns an error (a JS exception) unless `states.len()` is a multiple of 6,
/// and unless `mu` and `body_radius` are finite with `mu > 0`. A ragged
/// `states` would otherwise drop a partial state in silence.
///
/// # Undefined elements
///
/// Classical elements need an orbital plane, so a state with `r = 0` or
/// `r × v = 0` (which includes `v = 0`) has none — `from_state_vector` divides
/// by those magnitudes. Such a state, and any state with a non-finite
/// component, yields `NaN` for all ten values rather than zeros: zero is a
/// legitimate reading for a circular equatorial orbit's angles, so it cannot
/// double as "no value".
#[wasm_bindgen]
pub fn orbit_derived_batch(states: &[f64], mu: f64, body_radius: f64) -> Result<Vec<f64>, JsValue> {
    batch_orbit_derived(states, mu, body_radius).map_err(|e| JsValue::from_str(&e))
}

/// Length-checked core of [`orbit_derived_batch`], split out because `JsValue`
/// exists only on `wasm32` — this half is unit-testable on the host target.
fn batch_orbit_derived(states: &[f64], mu: f64, body_radius: f64) -> Result<Vec<f64>, String> {
    if !mu.is_finite() || mu <= 0.0 {
        return Err(format!(
            "orbit_derived_batch: mu must be positive and finite (got {mu})"
        ));
    }
    if !body_radius.is_finite() {
        return Err(format!(
            "orbit_derived_batch: body_radius must be finite (got {body_radius})"
        ));
    }
    if !states.len().is_multiple_of(6) {
        return Err(format!(
            "orbit_derived_batch: states has {} elements, expected a multiple of 6 \
             (x, y, z, vx, vy, vz per state)",
            states.len()
        ));
    }

    let mut out = Vec::with_capacity(states.len() / 6 * ORBIT_DERIVED_STRIDE);
    for s in states.chunks_exact(6) {
        let pos = Vector3::new(s[0], s[1], s[2]);
        let vel = Vector3::new(s[3], s[4], s[5]);
        out.extend_from_slice(&orbit_derived_one(&pos, &vel, mu, body_radius));
    }
    Ok(out)
}

/// One state's ten derived values, or ten `NaN`s where the elements have no
/// definition.
fn orbit_derived_one(
    pos: &Vector3<f64>,
    vel: &Vector3<f64>,
    mu: f64,
    body_radius: f64,
) -> [f64; ORBIT_DERIVED_STRIDE] {
    const UNDEFINED: [f64; ORBIT_DERIVED_STRIDE] = [f64::NAN; ORBIT_DERIVED_STRIDE];

    if !pos.iter().chain(vel.iter()).all(|v| v.is_finite()) {
        return UNDEFINED;
    }
    let r = pos.magnitude();
    let h = pos.cross(vel);
    // The plane and the radial direction are what every angle is measured
    // from. A `.rrd` with no velocity columns decodes as `v = 0`, so `h = 0` is
    // reachable from a real recording rather than only in theory.
    if r == 0.0 || h.magnitude() == 0.0 {
        return UNDEFINED;
    }

    let el = arika::kepler::KeplerianElements::from_state_vector(pos, vel, mu);
    let v = vel.magnitude();
    [
        el.semi_major_axis,
        el.eccentricity,
        el.inclination,
        el.raan,
        el.argument_of_periapsis,
        el.true_anomaly,
        r - body_radius,
        v * v / 2.0 - mu / r,
        h.magnitude(),
        v,
    ]
}

/// Approximate sun direction (unit vector) in Gcrs frame.
///
/// Returns `[x, y, z]` (3 floats).
#[wasm_bindgen]
pub fn sun_direction_eci(epoch_jd: f64, t: f64) -> Vec<f32> {
    let epoch = Epoch::from_jd(epoch_jd).add_seconds(t);
    let dir = sun::sun_direction_eci(&epoch.to_tdb());
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
    let dir = sun::sun_direction_from_body(body, &epoch.to_tdb());
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
    sun::sun_distance_from_body(body, &epoch.to_tdb())
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
    let geod = arika::earth::Geodetic {
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
    let geod = arika::earth::Geodetic {
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
    match arika::rotation::body_orientation(body, &epoch_tdb) {
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
#[allow(clippy::too_many_arguments)]
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

    match arika::body_quat_to_rsw(&pos, &vel, &q_body_eci) {
        Some(q) => vec![q.w, q.i, q.j, q.k],
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JD: f64 = 2460390.0;

    #[test]
    fn batch_requires_three_positions_per_time() {
        // A short `positions` used to index out of bounds and trap the wasm
        // instance (the length contract was a `debug_assert`, absent from the
        // release build the viewer loads).
        assert!(batch_eci_to_ecef(&[], &[0.0], JD).is_err());
        assert!(batch_eci_to_ecef(&[1.0, 2.0], &[0.0], JD).is_err());
        // A long `positions` used to silently return fewer points than the
        // caller asked for, which a JS reader sizing its loop from
        // `positions.length / 3` turns into NaNs in the vertex buffer.
        assert!(batch_eci_to_ecef(&[1.0; 6], &[0.0], JD).is_err());
        // The matched case is unaffected.
        let out = batch_eci_to_ecef(&[7000.0, 0.0, 0.0], &[0.0], JD).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn batch_agrees_with_the_single_point_transform() {
        // Cross-path invariant: the batch loop must be the single-point
        // transform, per point, with each point's own time.
        let positions = [7000.0_f32, 0.0, 0.0, 0.0, 6800.0, 100.0];
        let times = [0.0_f32, 600.0];
        let batch = batch_eci_to_ecef(&positions, &times, JD).unwrap();
        for (i, &t) in times.iter().enumerate() {
            let single = eci_to_ecef(
                positions[i * 3],
                positions[i * 3 + 1],
                positions[i * 3 + 2],
                JD,
                t,
            );
            assert_eq!(&batch[i * 3..i * 3 + 3], &single[..]);
        }
    }

    // orbit_derived_batch

    /// The gravitational parameter and radius of the Earth, as the CLI uses them.
    const MU_EARTH: f64 = 398600.4418;
    const R_EARTH: f64 = 6378.137;

    /// A circular orbit 400 km up comes back with the radius as its semi-major
    /// axis and no eccentricity.
    #[test]
    fn derived_values_of_a_circular_orbit() {
        let r = R_EARTH + 400.0;
        let v = (MU_EARTH / r).sqrt();
        let out = batch_orbit_derived(&[r, 0.0, 0.0, 0.0, v, 0.0], MU_EARTH, R_EARTH)
            .expect("a well-formed state");

        assert_eq!(out.len(), ORBIT_DERIVED_STRIDE);
        assert!((out[0] - r).abs() < 1e-6, "a = {}", out[0]);
        assert!(out[1] < 1e-12, "e = {}", out[1]);
        assert!((out[6] - 400.0).abs() < 1e-6, "altitude = {}", out[6]);
        assert!((out[9] - v).abs() < 1e-12, "velocity = {}", out[9]);
        // Specific energy of a circular orbit is -mu/(2a).
        assert!(
            (out[7] + MU_EARTH / (2.0 * r)).abs() < 1e-9,
            "energy = {}",
            out[7]
        );
        // |h| = r·v for a circular orbit.
        assert!((out[8] - r * v).abs() < 1e-6, "|h| = {}", out[8]);
    }

    /// Each state in a batch is converted on its own, in order.
    #[test]
    fn batch_converts_each_state_independently() {
        let r1 = R_EARTH + 400.0;
        let v1 = (MU_EARTH / r1).sqrt();
        let r2 = R_EARTH + 800.0;
        let v2 = (MU_EARTH / r2).sqrt();
        let out = batch_orbit_derived(
            &[r1, 0.0, 0.0, 0.0, v1, 0.0, r2, 0.0, 0.0, 0.0, v2, 0.0],
            MU_EARTH,
            R_EARTH,
        )
        .expect("two well-formed states");

        assert_eq!(out.len(), 2 * ORBIT_DERIVED_STRIDE);
        assert!((out[0] - r1).abs() < 1e-6);
        assert!((out[ORBIT_DERIVED_STRIDE] - r2).abs() < 1e-6);
    }

    /// The angles match the same call through arika's own API, so the WASM
    /// facade cannot drift from what the CLI writes.
    #[test]
    fn derived_angles_match_the_arika_api() {
        // A 51.6-degree inclined, slightly eccentric orbit.
        let pos = Vector3::new(5000.0, 3000.0, 4000.0);
        let vel = Vector3::new(-3.0, 5.5, 2.0);
        let out = batch_orbit_derived(
            &[pos.x, pos.y, pos.z, vel.x, vel.y, vel.z],
            MU_EARTH,
            R_EARTH,
        )
        .expect("a well-formed state");

        let el = arika::kepler::KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);
        assert_eq!(out[0], el.semi_major_axis);
        assert_eq!(out[1], el.eccentricity);
        assert_eq!(out[2], el.inclination);
        assert_eq!(out[3], el.raan);
        assert_eq!(out[4], el.argument_of_periapsis);
        assert_eq!(out[5], el.true_anomaly);
    }

    /// A state with no orbital plane yields `NaN`, not zeros.
    ///
    /// Zero is a real reading for a circular equatorial orbit's angles, so it
    /// cannot also mean "no value". A `.rrd` without velocity columns decodes
    /// as `v = 0`, which is this case.
    #[test]
    fn states_without_an_orbital_plane_are_nan() {
        let r = R_EARTH + 400.0;
        for (label, state) in [
            ("v = 0", [r, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ("r = 0", [0.0, 0.0, 0.0, 0.0, 7.6, 0.0]),
            ("r parallel to v", [r, 0.0, 0.0, 7.6, 0.0, 0.0]),
            ("NaN component", [r, f64::NAN, 0.0, 0.0, 7.6, 0.0]),
            ("infinite component", [r, f64::INFINITY, 0.0, 0.0, 7.6, 0.0]),
        ] {
            let out = batch_orbit_derived(&state, MU_EARTH, R_EARTH).expect("length is fine");
            assert!(
                out.iter().all(|v| v.is_nan()),
                "{label} should be all NaN, got {out:?}"
            );
        }
    }

    /// A ragged `states` is refused rather than silently dropping the tail.
    #[test]
    fn batch_requires_six_values_per_state() {
        let err = batch_orbit_derived(&[1.0, 2.0, 3.0, 4.0, 5.0], MU_EARTH, R_EARTH)
            .expect_err("5 values is not a whole state");
        assert!(err.contains("multiple of 6"), "{err}");
    }

    /// A `mu` that cannot scale an orbit is refused at the boundary.
    #[test]
    fn batch_requires_a_usable_mu_and_radius() {
        let state = [7000.0, 0.0, 0.0, 0.0, 7.5, 0.0];
        for mu in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                batch_orbit_derived(&state, mu, R_EARTH).is_err(),
                "mu = {mu} should be refused"
            );
        }
        assert!(batch_orbit_derived(&state, MU_EARTH, f64::NAN).is_err());
        // Zero radius is a legitimate request for the radial distance itself.
        assert!(batch_orbit_derived(&state, MU_EARTH, 0.0).is_ok());
    }

    /// An empty batch is an empty result.
    #[test]
    fn an_empty_batch_returns_nothing() {
        assert_eq!(
            batch_orbit_derived(&[], MU_EARTH, R_EARTH).expect("empty is fine"),
            Vec::<f64>::new()
        );
    }
}
