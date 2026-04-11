pub mod body;
pub mod constants;
pub mod epoch;
pub mod frame;
pub mod horizons;
pub mod moon;
pub mod planets;
pub mod rotation;
pub mod sun;

#[cfg(feature = "wasm")]
pub mod wasm;

use nalgebra::{Matrix3, Rotation3, UnitQuaternion, Vector3};

/// Approximate Earth-centered inertial frame vector.
///
/// Type alias for `frame::Vec3<frame::SimpleEci>`. This is the "parent frame"
/// of the ERA-only Z rotation used by the simple (visualization-grade) path.
/// Constructed via `SimpleEci::new(x, y, z)` or `SimpleEci::from_raw(vector3)`.
pub type SimpleEci = frame::Vec3<frame::SimpleEci>;

/// Approximate Earth-centered Earth-fixed frame vector.
///
/// Type alias for `frame::Vec3<frame::SimpleEcef>`. Complement of [`SimpleEci`]
/// under the ERA-only rotation. WGS-84 geodetic conversion is defined on this
/// frame.
pub type SimpleEcef = frame::Vec3<frame::SimpleEcef>;

/// Geocentric Celestial Reference System frame vector.
///
/// Type alias for `frame::Vec3<frame::Gcrs>`. Current usage: return type of
/// the Meeus analytic ephemerides (Sun / Moon / planets). In a later phase
/// the IAU 2006 precession/nutation chain will make this a strict GCRS.
pub type Gcrs = frame::Vec3<frame::Gcrs>;

/// Local orbital frame vector (Radial / Along-track / Cross-track).
///
/// Type alias for `frame::Vec3<frame::Rsw>`.
pub type Rsw = frame::Vec3<frame::Rsw>;

/// Geodetic coordinates (WGS84)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geodetic {
    pub latitude: f64,  // rad
    pub longitude: f64, // rad
    pub altitude: f64,  // km
}

// ─── SimpleEcef ↔ Geodetic type-to-type conversions ──────────────
//
// These are the WGS-84 ellipsoid Cartesian ↔ (lat, lon, height)
// conversions. They are parameter-free (the ellipsoid constants are
// hardcoded WGS-84) so `From` / `Into` is the natural shape.
//
// Scale/ERA conversions between `SimpleEci` and `SimpleEcef` are not
// `From` / `Into` (they require an Epoch / ERA parameter) — use
// `Rotation::<SimpleEci, SimpleEcef>::from_ut1(&epoch)` for those.

impl From<SimpleEcef> for Geodetic {
    /// Convert a WGS-84 Cartesian `SimpleEcef` vector to geodetic
    /// (latitude, longitude, height). Uses iterative Bowring method.
    fn from(ecef: SimpleEcef) -> Self {
        let v = ecef.inner();
        let p = (v.x * v.x + v.y * v.y).sqrt();
        let longitude = v.y.atan2(v.x);

        // Near-polar special case
        if p < 1e-10 {
            return Geodetic {
                latitude: v.z.signum() * std::f64::consts::FRAC_PI_2,
                longitude,
                altitude: v.z.abs() - WGS84_B,
            };
        }

        // Bowring iteration with convergence check
        let mut lat = v.z.atan2(p * (1.0 - WGS84_E2));
        let mut alt = 0.0_f64;

        for _ in 0..5 {
            let sin_lat = lat.sin();
            let cos_lat = lat.cos();
            let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
            let new_alt = p / cos_lat - n;
            lat = (v.z / p / (1.0 - WGS84_E2 * n / (n + new_alt))).atan();
            if (new_alt - alt).abs() < 1e-12 {
                alt = new_alt;
                break;
            }
            alt = new_alt;
        }

        Geodetic {
            latitude: lat,
            longitude,
            altitude: alt,
        }
    }
}

impl From<Geodetic> for SimpleEcef {
    /// Convert geodetic (latitude, longitude, height) to a WGS-84 Cartesian
    /// `SimpleEcef` vector.
    fn from(geo: Geodetic) -> Self {
        let sin_lat = geo.latitude.sin();
        let cos_lat = geo.latitude.cos();
        let sin_lon = geo.longitude.sin();
        let cos_lon = geo.longitude.cos();

        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

        SimpleEcef::from_raw(Vector3::new(
            (n + geo.altitude) * cos_lat * cos_lon,
            (n + geo.altitude) * cos_lat * sin_lon,
            (n * (1.0 - WGS84_E2) + geo.altitude) * sin_lat,
        ))
    }
}

/// WGS84 semi-major axis (km)
pub const WGS84_A: f64 = 6378.137;

/// WGS84 flattening
pub const WGS84_F: f64 = 1.0 / 298.257223563;

/// WGS84 semi-minor axis (km)
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);

/// WGS84 first eccentricity squared
pub const WGS84_E2: f64 = 1.0 - (1.0 - WGS84_F) * (1.0 - WGS84_F);

/// Compute WGS-84 geodetic altitude \[km\] directly from a position vector \[km\].
///
/// Works on any Earth-centered frame (SimpleEci or SimpleEcef) — geodetic
/// altitude depends only on `p = sqrt(x² + y²)` and `z`, which are invariant
/// under Z-axis rotation. Uses Bowring iteration (converges in 2-3 iterations
/// to sub-mm accuracy at LEO).
pub fn geodetic_altitude(position: &Vector3<f64>) -> f64 {
    let p = (position.x * position.x + position.y * position.y).sqrt();
    let z = position.z;

    // Near-polar special case: avoid p/cos(lat) singularity
    if p < 1e-10 {
        return z.abs() - WGS84_B;
    }

    // Bowring iteration for geodetic latitude
    let mut lat = z.atan2(p * (1.0 - WGS84_E2));
    let mut alt = 0.0_f64;

    for _ in 0..5 {
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        let new_alt = p / cos_lat - n;
        let new_lat = (z / p / (1.0 - WGS84_E2 * n / (n + new_alt))).atan();
        if (new_alt - alt).abs() < 1e-9 {
            return new_alt;
        }
        alt = new_alt;
        lat = new_lat;
    }

    alt
}

/// Compute the RSW (Radial / Along-track / Cross-track) orbital frame
/// quaternion from satellite position and velocity in a simple Earth-centered
/// inertial frame.
///
/// RSW basis (standard Vallado convention, right-handed):
/// - R̂ = `normalize(r)` — radial (earth-to-satellite)
/// - Ŵ = `normalize(r × v)` — cross-track (orbit normal)
/// - Ŝ = `Ŵ × R̂` — along-track (circular prograde: roughly +v̂ direction)
///
/// Returns the **RSW-to-ECI** rotation as a unit quaternion, or `None` if
/// position/velocity are zero or parallel (degenerate orbit).
///
/// # Axis order
///
/// This differs from the pre-redesign `lvlh_quaternion` which used axis
/// order [InTrack, CrossTrack, Radial]. The new RSW convention is
/// [Radial, Along-track, Cross-track]. The returned quaternion has
/// different numerical values; call sites that pin Lvlh values must be
/// updated to new RSW-basis expectations.
pub fn rsw_quaternion(pos: &Vector3<f64>, vel: &Vector3<f64>) -> Option<UnitQuaternion<f64>> {
    let r_len = pos.norm();
    if r_len < 1e-10 {
        return None;
    }
    let r = pos / r_len;

    let w_raw = pos.cross(vel);
    let w_len = w_raw.norm();
    if w_len < 1e-10 {
        return None;
    }
    let w = w_raw / w_len;

    let s = w.cross(&r);

    // RSW-to-ECI: columns = [Radial, Along-track, Cross-track]
    let mat = Matrix3::from_columns(&[r, s, w]);
    Some(UnitQuaternion::from_rotation_matrix(
        &Rotation3::from_matrix_unchecked(mat),
    ))
}

/// Transform a body-to-ECI quaternion into a body-to-RSW quaternion.
///
/// `pos`: satellite position in a simple Earth-centered inertial frame \[km\]
/// `vel`: satellite velocity in the same frame \[km/s\]
/// `q_body_eci`: body-to-ECI attitude quaternion
///
/// Returns `body_to_rsw = rsw_to_eci⁻¹ * body_to_eci`, or `None` if
/// the RSW frame cannot be computed (degenerate orbit).
pub fn body_quat_to_rsw(
    pos: &Vector3<f64>,
    vel: &Vector3<f64>,
    q_body_eci: &UnitQuaternion<f64>,
) -> Option<UnitQuaternion<f64>> {
    let q_rsw_eci = rsw_quaternion(pos, vel)?;
    Some(q_rsw_eci.inverse() * q_body_eci)
}

#[cfg(test)]
mod tests {
    use super::*;

    // SimpleEci <-> SimpleEcef conversion via Rotation<SimpleEci, SimpleEcef>
    //
    // All ERA-parametric conversions go through `Rotation::from_era(era)`
    // (or `from_ut1(&epoch)`). There are no shortcut methods on SimpleEci /
    // SimpleEcef themselves — the `Rotation` object is the single source of
    // truth for frame rotation, avoiding duplication.

    fn eci_to_ecef(eci: &SimpleEci, era: f64) -> SimpleEcef {
        frame::Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(era).transform(eci)
    }

    fn ecef_to_eci(ecef: &SimpleEcef, era: f64) -> SimpleEci {
        frame::Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(era).transform(ecef)
    }

    #[test]
    fn test_simple_eci_ecef_zero_era() {
        let eci = SimpleEci::new(7000.0, 1000.0, 500.0);
        let ecef = eci_to_ecef(&eci, 0.0);
        let eps = 1e-10;
        assert!((ecef.x() - eci.x()).abs() < eps);
        assert!((ecef.y() - eci.y()).abs() < eps);
        assert!((ecef.z() - eci.z()).abs() < eps);
    }

    #[test]
    fn test_simple_eci_ecef_90deg() {
        let era = std::f64::consts::FRAC_PI_2;
        let eci = SimpleEci::new(1.0, 0.0, 0.0);
        let ecef = eci_to_ecef(&eci, era);
        let eps = 1e-10;
        // ECEF = R_z(-ERA) × ECI: with ERA=90°, +X_ECI → −Y_ECEF
        assert!(ecef.x().abs() < eps);
        assert!((ecef.y() - (-1.0)).abs() < eps);
        assert!(ecef.z().abs() < eps);

        let eci2 = SimpleEci::new(0.0, 1.0, 0.0);
        let ecef2 = eci_to_ecef(&eci2, era);
        assert!((ecef2.x() - 1.0).abs() < eps);
        assert!(ecef2.y().abs() < eps);
        assert!(ecef2.z().abs() < eps);
    }

    #[test]
    fn test_simple_eci_ecef_roundtrip() {
        let original = SimpleEci::new(6700.0, 1500.0, 3200.0);
        let era = 1.234;
        let roundtrip = ecef_to_eci(&eci_to_ecef(&original, era), era);
        let eps = 1e-10;
        assert!((roundtrip.x() - original.x()).abs() < eps);
        assert!((roundtrip.y() - original.y()).abs() < eps);
        assert!((roundtrip.z() - original.z()).abs() < eps);
    }

    #[test]
    fn test_simple_eci_ecef_magnitude_preserved() {
        let eci = SimpleEci::new(6700.0, 1500.0, 3200.0);
        let era = 2.5;
        let ecef = eci_to_ecef(&eci, era);
        let eps = 1e-10;
        assert!((eci.magnitude() - ecef.magnitude()).abs() < eps);
    }

    // Geodetic <-> SimpleEcef conversion via From / Into

    #[test]
    fn test_equator_prime_meridian() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = SimpleEcef::from(geo);
        let eps = 1e-10;
        assert!((ecef.x() - WGS84_A).abs() < eps);
        assert!(ecef.y().abs() < eps);
        assert!(ecef.z().abs() < eps);
    }

    #[test]
    fn test_equator_90east() {
        let geo = Geodetic {
            latitude: 0.0,
            longitude: std::f64::consts::FRAC_PI_2,
            altitude: 0.0,
        };
        let ecef: SimpleEcef = geo.into();
        let eps = 1e-10;
        assert!(ecef.x().abs() < eps);
        assert!((ecef.y() - WGS84_A).abs() < eps);
        assert!(ecef.z().abs() < eps);
    }

    #[test]
    fn test_north_pole() {
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_2,
            longitude: 0.0,
            altitude: 0.0,
        };
        let ecef = SimpleEcef::from(geo);
        let eps = 1e-6;
        assert!(ecef.x().abs() < eps);
        assert!(ecef.y().abs() < eps);
        assert!((ecef.z() - WGS84_B).abs() < eps);
    }

    #[test]
    fn test_roundtrip_geodetic() {
        let original = Geodetic {
            latitude: 0.7,
            longitude: 2.1,
            altitude: 350.0,
        };
        let ecef = SimpleEcef::from(original);
        let roundtrip = Geodetic::from(ecef);
        let eps = 1e-10;
        assert!((roundtrip.latitude - original.latitude).abs() < eps);
        assert!((roundtrip.longitude - original.longitude).abs() < eps);
        assert!((roundtrip.altitude - original.altitude).abs() < eps);
    }

    #[test]
    fn test_with_altitude() {
        let alt = 500.0;
        let geo_surface = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: 0.0,
        };
        let geo_alt = Geodetic {
            latitude: 0.0,
            longitude: 0.0,
            altitude: alt,
        };
        let ecef_surface = SimpleEcef::from(geo_surface);
        let ecef_alt = SimpleEcef::from(geo_alt);
        let eps = 1e-10;
        assert!((ecef_alt.x() - ecef_surface.x() - alt).abs() < eps);
        assert!(ecef_alt.y().abs() < eps);
        assert!(ecef_alt.z().abs() < eps);
    }

    // geodetic_altitude() tests

    #[test]
    fn geodetic_altitude_equator() {
        let pos = Vector3::new(WGS84_A + 400.0, 0.0, 0.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-9);
    }

    #[test]
    fn geodetic_altitude_north_pole() {
        let pos = Vector3::new(0.0, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-6);
    }

    #[test]
    fn geodetic_altitude_south_pole() {
        let pos = Vector3::new(0.0, 0.0, -(WGS84_B + 400.0));
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-6);
    }

    #[test]
    fn geodetic_altitude_matches_to_geodetic() {
        let geo = Geodetic {
            latitude: std::f64::consts::FRAC_PI_4,
            longitude: 0.5,
            altitude: 400.0,
        };
        let ecef = SimpleEcef::from(geo);
        let expected = Geodetic::from(ecef).altitude;
        let actual = geodetic_altitude(ecef.inner());
        assert!((actual - expected).abs() < 1e-9);
    }

    #[test]
    fn geodetic_altitude_spherical_difference_at_iss_inclination() {
        let lat = 51.6_f64.to_radians();
        let geo = Geodetic {
            latitude: lat,
            longitude: 0.0,
            altitude: 400.0,
        };
        let ecef = SimpleEcef::from(geo);
        let r = ecef.magnitude();
        let spherical_alt = r - WGS84_A;
        let geodetic_alt = geodetic_altitude(ecef.inner());

        let diff = spherical_alt - geodetic_alt;
        assert!(
            diff.abs() > 5.0 && diff.abs() < 20.0,
            "spherical-geodetic diff at 51.6° should be ~10-15 km, got {diff:.2} km"
        );
    }

    #[test]
    fn geodetic_altitude_near_polar_edge_case() {
        let pos = Vector3::new(1e-12, 0.0, WGS84_B + 400.0);
        let alt = geodetic_altitude(&pos);
        assert!((alt - 400.0).abs() < 1e-3);
    }

    #[test]
    fn geodetic_altitude_invariant_under_z_rotation() {
        let r = WGS84_A + 400.0;
        let z = 3000.0;
        let p = (r * r - z * z).sqrt();

        let alt1 = geodetic_altitude(&Vector3::new(p, 0.0, z));
        let alt2 = geodetic_altitude(&Vector3::new(p * 0.6, p * 0.8, z));
        let alt3 = geodetic_altitude(&Vector3::new(-p * 0.5, p * (3.0_f64).sqrt() / 2.0, z));

        assert!((alt1 - alt2).abs() < 1e-10);
        assert!((alt1 - alt3).abs() < 1e-10);
    }

    // RSW quaternion tests (new convention: columns [R, S, W])

    #[test]
    fn rsw_quaternion_equatorial_prograde_is_identity() {
        // Equatorial prograde: pos=+X, vel=+Y
        //   R̂ = +X
        //   Ŵ = normalize(X × Y) = +Z
        //   Ŝ = W × R = Z × X = +Y
        // RSW-to-ECI = [R, S, W] = [X, Y, Z] = identity.
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.5, 0.0);
        let q = rsw_quaternion(&pos, &vel).unwrap();

        let identity = UnitQuaternion::identity();
        assert!(
            q.angle_to(&identity) < 1e-10,
            "equatorial prograde RSW-to-ECI should be identity, got angle {}",
            q.angle_to(&identity)
        );
    }

    #[test]
    fn rsw_quaternion_equatorial_prograde_basis_vectors() {
        // Same setup as above: verify each RSW basis vector maps correctly.
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.5, 0.0);
        let q = rsw_quaternion(&pos, &vel).unwrap();

        // q maps RSW basis vectors to ECI directions.
        let r_eci = q * Vector3::new(1.0, 0.0, 0.0); // R (radial) → +X
        let s_eci = q * Vector3::new(0.0, 1.0, 0.0); // S (along-track) → +Y
        let w_eci = q * Vector3::new(0.0, 0.0, 1.0); // W (cross-track) → +Z

        let eps = 1e-10;
        assert!((r_eci - Vector3::new(1.0, 0.0, 0.0)).norm() < eps);
        assert!((s_eci - Vector3::new(0.0, 1.0, 0.0)).norm() < eps);
        assert!((w_eci - Vector3::new(0.0, 0.0, 1.0)).norm() < eps);
    }

    #[test]
    fn rsw_quaternion_inclined_orbit_basis_vectors() {
        // 90° inclined orbit: pos=+X, vel=+Z
        //   R̂ = normalize(pos) = +X
        //   Ŵ = normalize(pos × vel) = normalize((1,0,0) × (0,0,1))
        //     = normalize((0,-1,0)) = -Y
        //   Ŝ = Ŵ × R̂ = (0,-1,0) × (1,0,0) = (0,0,1) = +Z
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 0.0, 7.5);
        let q = rsw_quaternion(&pos, &vel).unwrap();

        let r_eci = q * Vector3::new(1.0, 0.0, 0.0); // R → +X
        let s_eci = q * Vector3::new(0.0, 1.0, 0.0); // S → +Z
        let w_eci = q * Vector3::new(0.0, 0.0, 1.0); // W → -Y

        let eps = 1e-10;
        assert!(
            (r_eci - Vector3::new(1.0, 0.0, 0.0)).norm() < eps,
            "R: {r_eci}"
        );
        assert!(
            (s_eci - Vector3::new(0.0, 0.0, 1.0)).norm() < eps,
            "S: {s_eci}"
        );
        assert!(
            (w_eci - Vector3::new(0.0, -1.0, 0.0)).norm() < eps,
            "W: {w_eci}"
        );
    }

    #[test]
    fn rsw_quaternion_degenerate_zero_pos() {
        let pos = Vector3::new(0.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.5, 0.0);
        assert!(rsw_quaternion(&pos, &vel).is_none());
    }

    #[test]
    fn rsw_quaternion_degenerate_parallel() {
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(1.0, 0.0, 0.0);
        assert!(rsw_quaternion(&pos, &vel).is_none());
    }

    // body_quat_to_rsw tests

    #[test]
    fn body_quat_to_rsw_identity_is_rsw_aligned() {
        // If body-to-ECI == rsw-to-ECI, then body-to-RSW should be identity.
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.5, 0.0);
        let q_rsw = rsw_quaternion(&pos, &vel).unwrap();

        let result = body_quat_to_rsw(&pos, &vel, &q_rsw).unwrap();
        assert!(result.angle_to(&UnitQuaternion::identity()) < 1e-10);
    }

    #[test]
    fn body_quat_to_rsw_90deg_yaw() {
        // Body is rotated 90° around RSW Z axis (= cross-track).
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(0.0, 7.5, 0.0);

        let q_rsw = rsw_quaternion(&pos, &vel).unwrap();
        let yaw_in_rsw =
            UnitQuaternion::from_axis_angle(&Vector3::z_axis(), std::f64::consts::FRAC_PI_2);
        let q_body_eci = q_rsw * yaw_in_rsw;

        let result = body_quat_to_rsw(&pos, &vel, &q_body_eci).unwrap();
        assert!(result.angle_to(&yaw_in_rsw) < 1e-10);
    }

    #[test]
    fn body_quat_to_rsw_degenerate() {
        let pos = Vector3::new(7000.0, 0.0, 0.0);
        let vel = Vector3::new(1.0, 0.0, 0.0);
        let q = UnitQuaternion::identity();
        assert!(body_quat_to_rsw(&pos, &vel, &q).is_none());
    }

    #[test]
    fn body_quat_to_rsw_eci_fixed_body_rotates_with_orbit() {
        // Body fixed in ECI, viewed from RSW should appear to rotate at
        // exactly the orbital angular velocity (circular equatorial orbit).
        let r = 7000.0;
        let v = 7.5;
        let q_body_eci = UnitQuaternion::identity();

        let q0 = body_quat_to_rsw(
            &Vector3::new(r, 0.0, 0.0),
            &Vector3::new(0.0, v, 0.0),
            &q_body_eci,
        )
        .unwrap();
        let q90 = body_quat_to_rsw(
            &Vector3::new(0.0, r, 0.0),
            &Vector3::new(-v, 0.0, 0.0),
            &q_body_eci,
        )
        .unwrap();
        let q180 = body_quat_to_rsw(
            &Vector3::new(-r, 0.0, 0.0),
            &Vector3::new(0.0, -v, 0.0),
            &q_body_eci,
        )
        .unwrap();

        let eps = 1e-10;
        assert!(
            (q0.angle_to(&q90) - std::f64::consts::FRAC_PI_2).abs() < eps,
            "0°→90°: expected π/2, got {}",
            q0.angle_to(&q90)
        );
        assert!(
            (q0.angle_to(&q180) - std::f64::consts::PI).abs() < eps,
            "0°→180°: expected π, got {}",
            q0.angle_to(&q180)
        );
    }

    #[test]
    fn body_quat_to_rsw_nadir_pointing_is_identity() {
        // Nadir-pointing: body = RSW at all orbital positions.
        let r = 7000.0;
        let v = 7.5;

        for theta in [0.0_f64, 0.3, 1.2, 2.5, 4.7] {
            let pos = Vector3::new(r * theta.cos(), r * theta.sin(), 0.0);
            let vel = Vector3::new(-v * theta.sin(), v * theta.cos(), 0.0);
            let q_rsw = rsw_quaternion(&pos, &vel).unwrap();
            let result = body_quat_to_rsw(&pos, &vel, &q_rsw).unwrap();
            assert!(
                result.angle_to(&UnitQuaternion::identity()) < 1e-10,
                "nadir at θ={theta}: expected identity, got angle {}",
                result.angle_to(&UnitQuaternion::identity())
            );
        }
    }

    #[test]
    fn body_quat_to_rsw_roundtrip_with_rsw_quaternion() {
        // q_be = q_re * q_br
        let pos = Vector3::new(3000.0, 5000.0, 2000.0);
        let vel = Vector3::new(-2.0, 1.5, 6.0);

        let q_body_eci = UnitQuaternion::from_axis_angle(
            &nalgebra::Unit::new_normalize(Vector3::new(1.0, 2.0, 3.0)),
            1.234,
        );

        let q_rsw = rsw_quaternion(&pos, &vel).unwrap();
        let q_body_rsw = body_quat_to_rsw(&pos, &vel, &q_body_eci).unwrap();

        let reconstructed = q_rsw * q_body_rsw;
        assert!(reconstructed.angle_to(&q_body_eci) < 1e-10);
    }
}
