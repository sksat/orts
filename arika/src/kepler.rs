//! Classical Keplerian orbital elements and anomaly conversions.
//!
//! Provides the [`KeplerianElements`] type and the Kepler-equation /
//! anomaly conversion helpers used to translate between mean, eccentric,
//! and true anomalies and to convert elements ↔ Cartesian state vectors.

use core::f64::consts::PI;

use nalgebra::Vector3;

// In `no_std` builds, `f64` inherent methods (`sin`, `sqrt`, …) are unavailable;
// this trait delegates to `libm`. Under `std` the inherent methods shadow it.
#[allow(unused_imports)]
use crate::math::F64Ext;

/// Iterations [`solve_kepler_equation`] may take.
///
/// Newton reaches the 1e-14 step threshold in a handful of steps wherever it
/// converges on its own. The bisection fallback sets the floor: halving a
/// bracket of width `2e ≤ 2` down to that threshold takes
/// `log2(2 / 1e-14) ≈ 48` steps, and the rest of the budget covers the Newton
/// steps interleaved with them. As `e` approaches 1 the budget can be spent in
/// full; measured across the eccentricities nearest 1 that `f64` holds, the
/// residual `E - e·sin(E) - M` stays under 2.1e-14 either way.
const KEPLER_MAX_ITERATIONS: usize = 100;

/// Solve Kepler's equation `M = E - e·sin(E)` for eccentric anomaly E.
///
/// Newton-Raphson, kept inside a bracket that contains the root: near periapsis
/// at high eccentricity `f' = 1 - e·cos(E)` approaches `1 - e`, and an
/// unguarded Newton step from `E₀ = M` there is long enough to leave the
/// interval and not come back. Bisecting instead of taking such a step
/// converges for every eccentricity the signature accepts.
///
/// # Arguments
/// * `mean_anomaly` - Mean anomaly M [rad]
/// * `eccentricity` - Orbital eccentricity (0 ≤ e < 1)
///
/// # Returns
/// Eccentric anomaly E [rad], within `e` of the reduced `M`
pub fn solve_kepler_equation(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let m = mean_anomaly % (2.0 * PI);
    // `E = M + e·sin(E)` puts the root within `e` of `M`, and `f` increases
    // monotonically (`f' = 1 - e·cos(E) > 0` for `e < 1`), so `f(m - e) ≤ 0`
    // and `f(m + e) ≥ 0` bracket it for any `M`. For `e = 0` the bracket is the
    // single point `m`, which is the root.
    let mut lo = m - eccentricity;
    let mut hi = m + eccentricity;
    let mut e_anom = m;
    for _ in 0..KEPLER_MAX_ITERATIONS {
        let f = e_anom - eccentricity * e_anom.sin() - m;
        if f > 0.0 {
            hi = e_anom;
        } else {
            lo = e_anom;
        }

        let f_prime = 1.0 - eccentricity * e_anom.cos();
        let newton = e_anom - f / f_prime;
        // A Newton step that leaves the bracket says nothing about where the
        // root is; the bracket does. Its midpoint also makes progress when
        // `f_prime` underflows to zero and the step is not finite at all.
        let next = if newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };

        let delta = next - e_anom;
        e_anom = next;
        if delta.abs() < 1e-14 {
            break;
        }
    }
    e_anom
}

/// Convert eccentric anomaly to true anomaly.
///
/// Uses the relation: `tan(ν/2) = √((1+e)/(1-e)) · tan(E/2)`
pub fn eccentric_to_true_anomaly(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    let half_e = eccentric_anomaly / 2.0;
    let factor = ((1.0 + eccentricity) / (1.0 - eccentricity)).sqrt();
    2.0 * (factor * half_e.tan()).atan()
}

/// Convert mean anomaly to true anomaly (convenience wrapper).
///
/// Solves Kepler's equation for E, then converts E → ν.
pub fn mean_to_true_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let e_anom = solve_kepler_equation(mean_anomaly, eccentricity);
    let nu = eccentric_to_true_anomaly(e_anom, eccentricity);
    // Normalize to [0, 2π)
    let nu = nu % (2.0 * PI);
    if nu < 0.0 { nu + 2.0 * PI } else { nu }
}

/// Convert true anomaly to eccentric anomaly.
///
/// Uses the relation: `tan(E/2) = √((1-e)/(1+e)) · tan(ν/2)`
pub fn true_to_eccentric_anomaly(true_anomaly: f64, eccentricity: f64) -> f64 {
    let half_nu = true_anomaly / 2.0;
    let factor = ((1.0 - eccentricity) / (1.0 + eccentricity)).sqrt();
    2.0 * (factor * half_nu.tan()).atan()
}

/// Convert eccentric anomaly to mean anomaly using Kepler's equation: `M = E - e·sin(E)`
pub fn eccentric_to_mean_anomaly(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    eccentric_anomaly - eccentricity * eccentric_anomaly.sin()
}

/// Convert true anomaly to mean anomaly (convenience wrapper).
pub fn true_to_mean_anomaly(true_anomaly: f64, eccentricity: f64) -> f64 {
    let e_anom = true_to_eccentric_anomaly(true_anomaly, eccentricity);
    eccentric_to_mean_anomaly(e_anom, eccentricity)
}

/// Normalize an angle to `[0, 2π)`.
fn normalize_angle(x: f64) -> f64 {
    let w = x % (2.0 * PI);
    if w < 0.0 { w + 2.0 * PI } else { w }
}

/// Classical Keplerian orbital elements.
///
/// # Degenerate geometries
///
/// Two of the six angles are undefined when the orbit is circular and/or
/// equatorial. [`KeplerianElements::from_state_vector`] resolves this with the
/// classical singular conventions (Vallado, *Fundamentals of Astrodynamics and
/// Applications*, §2.4), which fold the undefined angle into the next
/// well-defined one so that the state is still fully recoverable by
/// [`KeplerianElements::to_state_vector`]:
///
/// | geometry | `raan` | `argument_of_periapsis` | `true_anomaly` |
/// |---|---|---|---|
/// | general | Ω | ω | ν |
/// | circular inclined (`e ≈ 0`) | Ω | 0 | argument of latitude `u = ω + ν` |
/// | eccentric equatorial (`i ≈ 0` or `π`) | 0 | true longitude of periapsis `ϖ = Ω + ω` | ν |
/// | circular equatorial | 0 | 0 | true longitude `λ = Ω + ω + ν` |
///
/// For the *retrograde* equatorial cases (`i ≈ π`) the stored angle is the
/// negated in-plane longitude, because `to_state_vector` maps `i = π, Ω = 0`
/// onto a clockwise sweep of the x-y plane. That keeps the round trip exact for
/// both `i ≈ 0` and `i ≈ π`.
#[derive(Debug, Clone, PartialEq)]
pub struct KeplerianElements {
    /// Semi-major axis [km]
    pub semi_major_axis: f64,
    /// Eccentricity (dimensionless)
    pub eccentricity: f64,
    /// Inclination [rad]
    pub inclination: f64,
    /// Right ascension of ascending node (RAAN) [rad]
    pub raan: f64,
    /// Argument of periapsis [rad]
    pub argument_of_periapsis: f64,
    /// True anomaly [rad]
    pub true_anomaly: f64,
}

impl KeplerianElements {
    /// Convert a Cartesian state vector (position, velocity) to Keplerian elements.
    ///
    /// Degenerate geometries (circular and/or equatorial) follow the singular
    /// conventions documented on [`KeplerianElements`].
    ///
    /// # Arguments
    /// * `pos` - Position vector [km]
    /// * `vel` - Velocity vector [km/s]
    /// * `mu` - Gravitational parameter [km^3/s^2]
    pub fn from_state_vector(pos: &Vector3<f64>, vel: &Vector3<f64>, mu: f64) -> Self {
        let r = pos.magnitude();
        let v = vel.magnitude();

        // Specific angular momentum vector h = r x v
        let h = pos.cross(vel);
        let h_mag = h.magnitude();

        // Node vector n = k x h (k = unit Z)
        let k = Vector3::new(0.0, 0.0, 1.0);
        let n = k.cross(&h);
        let n_mag = n.magnitude();

        // Eccentricity vector e = (1/μ)((v²-μ/r)r - (r·v)v)
        let e_vec = (1.0 / mu) * ((v * v - mu / r) * pos - pos.dot(vel) * vel);
        let e = e_vec.magnitude();

        // Semi-major axis: a = -μ/(2ε) where ε = v²/2 - μ/r
        let energy = v * v / 2.0 - mu / r;
        let a = -mu / (2.0 * energy);

        // Inclination: tan(i) = |k x h| / h_z. `atan2` rather than
        // `acos(h_z/|h|)`, which loses half the mantissa near i = 0 and i = π.
        let i = f64::atan2(n_mag, h[2]);

        // An equatorial orbit has no node line, so Ω is undefined and the
        // in-plane angles are referred to the x-axis instead of the node.
        let equatorial = n_mag <= 1e-15;
        let reference = if equatorial {
            Vector3::new(1.0, 0.0, 0.0)
        } else {
            n
        };

        // Signed angle from `from` to `to` in the orbit plane, measured about
        // the orbit normal and normalized to [0, 2π). Measuring about the
        // normal is what makes the retrograde-equatorial case come out
        // clockwise in the x-y plane, matching `to_state_vector` at i = π.
        let h_hat = h / h_mag;
        let plane_angle = |from: &Vector3<f64>, to: &Vector3<f64>| {
            normalize_angle(f64::atan2(from.cross(to).dot(&h_hat), from.dot(to)))
        };

        // Right ascension of ascending node
        let raan = if equatorial {
            0.0
        } else {
            normalize_angle(f64::atan2(n[1], n[0]))
        };

        // Argument of periapsis. Circular: undefined, folded into the true
        // anomaly. Equatorial: this is the true longitude of periapsis
        // ϖ = Ω + ω, since raan is 0 — the true anomaly is measured from the
        // eccentricity vector, so storing 0 here would lose the periapsis
        // direction entirely and rotate the orbit by ϖ on the way back.
        let omega = if e <= 1e-15 {
            0.0
        } else {
            plane_angle(&reference, &e_vec)
        };

        // True anomaly, from periapsis when it is defined and from the
        // reference direction (argument of latitude / true longitude) when the
        // orbit is circular.
        let nu = if e > 1e-15 {
            plane_angle(&e_vec, pos)
        } else {
            plane_angle(&reference, pos)
        };

        KeplerianElements {
            semi_major_axis: a,
            eccentricity: e,
            inclination: i,
            raan,
            argument_of_periapsis: omega,
            true_anomaly: nu,
        }
    }

    /// Convert Keplerian elements to a Cartesian state vector (position, velocity).
    ///
    /// # Returns
    /// A tuple of (position [km], velocity [km/s])
    pub fn to_state_vector(&self, mu: f64) -> (Vector3<f64>, Vector3<f64>) {
        let a = self.semi_major_axis;
        let e = self.eccentricity;
        let i = self.inclination;
        let raan = self.raan;
        let omega = self.argument_of_periapsis;
        let nu = self.true_anomaly;

        // Semi-latus rectum
        let p = a * (1.0 - e * e);

        // Distance
        let r = p / (1.0 + e * nu.cos());

        // Position and velocity in perifocal frame (PQW)
        let r_pqw = Vector3::new(r * nu.cos(), r * nu.sin(), 0.0);

        let v_factor = (mu / p).sqrt();
        let v_pqw = Vector3::new(-v_factor * nu.sin(), v_factor * (e + nu.cos()), 0.0);

        // Rotation matrix from perifocal to ECI
        // R = R3(-Ω) R1(-i) R3(-ω)
        let cos_raan = raan.cos();
        let sin_raan = raan.sin();
        let cos_i = i.cos();
        let sin_i = i.sin();
        let cos_omega = omega.cos();
        let sin_omega = omega.sin();

        let l1 = cos_raan * cos_omega - sin_raan * sin_omega * cos_i;
        let l2 = -cos_raan * sin_omega - sin_raan * cos_omega * cos_i;

        let m1 = sin_raan * cos_omega + cos_raan * sin_omega * cos_i;
        let m2 = -sin_raan * sin_omega + cos_raan * cos_omega * cos_i;

        let n1 = sin_omega * sin_i;
        let n2 = cos_omega * sin_i;

        let pos = Vector3::new(
            l1 * r_pqw[0] + l2 * r_pqw[1],
            m1 * r_pqw[0] + m2 * r_pqw[1],
            n1 * r_pqw[0] + n2 * r_pqw[1],
        );

        let vel = Vector3::new(
            l1 * v_pqw[0] + l2 * v_pqw[1],
            m1 * v_pqw[0] + m2 * v_pqw[1],
            n1 * v_pqw[0] + n2 * v_pqw[1],
        );

        (pos, vel)
    }

    /// Create Keplerian elements from mean anomaly (converting to true anomaly internally).
    ///
    /// This is useful when working with TLE data which provides mean anomaly.
    pub fn from_mean_anomaly(
        semi_major_axis: f64,
        eccentricity: f64,
        inclination: f64,
        raan: f64,
        argument_of_periapsis: f64,
        mean_anomaly: f64,
    ) -> Self {
        let true_anomaly = mean_to_true_anomaly(mean_anomaly, eccentricity);
        Self {
            semi_major_axis,
            eccentricity,
            inclination,
            raan,
            argument_of_periapsis,
            true_anomaly,
        }
    }

    /// Orbital period [s]: T = 2π√(a³/μ)
    pub fn period(&self, mu: f64) -> f64 {
        2.0 * PI * (self.semi_major_axis.powi(3) / mu).sqrt()
    }

    /// Specific orbital energy [km²/s²]: ε = -μ/(2a)
    pub fn energy(&self, mu: f64) -> f64 {
        -mu / (2.0 * self.semi_major_axis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;
    use nalgebra::Vector3;

    #[test]
    fn test_circular_orbit_elements() {
        // Circular orbit at ISS altitude: r = R_earth + 400 = 6778.137 km
        let r = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        let pos = Vector3::new(r, 0.0, 0.0);
        let vel = Vector3::new(0.0, v, 0.0);

        let elements = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);

        assert!(
            (elements.semi_major_axis - r).abs() < 1e-6,
            "semi_major_axis: expected={r}, got={}",
            elements.semi_major_axis
        );
        assert!(
            elements.eccentricity < 1e-10,
            "eccentricity should be ≈0, got={}",
            elements.eccentricity
        );
        assert!(
            elements.inclination.abs() < 1e-10,
            "inclination should be ≈0, got={}",
            elements.inclination
        );
    }

    #[test]
    fn test_roundtrip_circular() {
        // Circular equatorial orbit
        let r = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        let pos = Vector3::new(r, 0.0, 0.0);
        let vel = Vector3::new(0.0, v, 0.0);

        let elements = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);
        let (pos2, vel2) = elements.to_state_vector(MU_EARTH);

        let pos_err = (pos - pos2).magnitude();
        let vel_err = (vel - vel2).magnitude();
        assert!(pos_err < 1e-6, "position roundtrip error: {pos_err} km");
        assert!(vel_err < 1e-9, "velocity roundtrip error: {vel_err} km/s");
    }

    /// Build an exactly planar state: `z = v_z = 0`, so `k × h` vanishes bit-exactly
    /// and `from_state_vector` is forced down the equatorial singular branch.
    ///
    /// `periapsis_lon` is the azimuth of the periapsis direction in the x-y plane
    /// and `sense` is +1 for prograde (counterclockwise) or -1 for retrograde.
    fn planar_state(
        a: f64,
        e: f64,
        periapsis_lon: f64,
        nu: f64,
        sense: f64,
        mu: f64,
    ) -> (Vector3<f64>, Vector3<f64>) {
        let p = a * (1.0 - e * e);
        let r = p / (1.0 + e * nu.cos());
        let theta = periapsis_lon + sense * nu;
        let (st, ct) = (theta.sin(), theta.cos());
        let r_hat = Vector3::new(ct, st, 0.0);
        let t_hat = Vector3::new(-st, ct, 0.0);
        let vf = (mu / p).sqrt();
        let v_radial = vf * e * nu.sin();
        let v_transverse = vf * (1.0 + e * nu.cos());
        (r * r_hat, v_radial * r_hat + sense * v_transverse * t_hat)
    }

    #[test]
    fn eccentric_equatorial_keeps_the_periapsis_longitude() {
        // Regression: the equatorial branch used to zero BOTH raan and the
        // argument of periapsis while still measuring the true anomaly from the
        // eccentricity vector, so the periapsis longitude was stored nowhere.
        // a=10000 km, e=0.2, ϖ=π/2, ν=0 came back rotated by 90°, an 8000·√2 =
        // 11313.7 km round-trip error.
        for &sense in &[1.0_f64, -1.0] {
            for &periapsis_lon in &[0.0, 0.5, PI / 2.0, 2.0, 4.0, 6.0] {
                for &e in &[0.01, 0.1, 0.2, 0.4] {
                    for &nu in &[0.0, 1.0, 3.0, 5.0] {
                        let (pos, vel) =
                            planar_state(10000.0, e, periapsis_lon, nu, sense, MU_EARTH);
                        let el = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);

                        // The singular branch really is the one under test.
                        assert_eq!(
                            el.raan, 0.0,
                            "equatorial orbits carry raan = 0 by convention"
                        );
                        let expected_i = if sense > 0.0 { 0.0 } else { PI };
                        assert!(
                            (el.inclination - expected_i).abs() < 1e-12,
                            "i: expected {expected_i}, got {}",
                            el.inclination
                        );
                        // The stored angle is the (sense-signed) true longitude
                        // of periapsis, so the direction survives.
                        let expected_argp = normalize_angle(sense * periapsis_lon);
                        let d_argp =
                            normalize_angle(el.argument_of_periapsis - expected_argp + PI) - PI;
                        assert!(
                            d_argp.abs() < 1e-9,
                            "ϖ: expected {expected_argp}, got {} (sense={sense}, e={e}, ν={nu})",
                            el.argument_of_periapsis
                        );

                        let (pos2, vel2) = el.to_state_vector(MU_EARTH);
                        let pos_err = (pos - pos2).magnitude();
                        let vel_err = (vel - vel2).magnitude();
                        assert!(
                            pos_err < 1e-6,
                            "position roundtrip {pos_err} km (sense={sense}, ϖ={periapsis_lon}, e={e}, ν={nu})"
                        );
                        assert!(
                            vel_err < 1e-9,
                            "velocity roundtrip {vel_err} km/s (sense={sense}, ϖ={periapsis_lon}, e={e}, ν={nu})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn circular_equatorial_true_longitude_follows_the_sense_of_motion() {
        // The circular equatorial branch measured the true longitude as the raw
        // x-axis azimuth, which runs backwards for retrograde motion.
        for &sense in &[1.0_f64, -1.0] {
            for &nu in &[0.0, 1.0, 3.0, 5.0] {
                let (pos, vel) = planar_state(10000.0, 0.0, 0.0, nu, sense, MU_EARTH);
                let el = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);
                assert!(el.eccentricity < 1e-15);
                let d = normalize_angle(el.true_anomaly - nu + PI) - PI;
                assert!(
                    d.abs() < 1e-9,
                    "λ: expected {nu}, got {} (sense={sense})",
                    el.true_anomaly
                );
                let (pos2, vel2) = el.to_state_vector(MU_EARTH);
                assert!((pos - pos2).magnitude() < 1e-6, "sense={sense}, ν={nu}");
                assert!((vel - vel2).magnitude() < 1e-9, "sense={sense}, ν={nu}");
            }
        }
    }

    #[test]
    fn equatorial_singularity_is_continuous_in_the_periapsis_longitude() {
        // Ω and ω are individually undefined at i = 0, but Ω + ω (the true
        // longitude of periapsis) is not — so the value recovered just off the
        // singularity must match the value recovered exactly on it.
        let periapsis_lon = 2.0;
        let nu = 1.0;
        let (pos0, vel0) = planar_state(10000.0, 0.2, periapsis_lon, nu, 1.0, MU_EARTH);
        let el0 = KeplerianElements::from_state_vector(&pos0, &vel0, MU_EARTH);

        // Same orbit tilted by 1e-9 rad about the periapsis direction, which
        // leaves the periapsis longitude unchanged.
        let tilt = 1e-9_f64;
        let axis = Vector3::new(periapsis_lon.cos(), periapsis_lon.sin(), 0.0);
        let rot = nalgebra::Rotation3::from_axis_angle(&nalgebra::Unit::new_normalize(axis), tilt);
        let el1 = KeplerianElements::from_state_vector(&(rot * pos0), &(rot * vel0), MU_EARTH);

        let lon0 = normalize_angle(el0.raan + el0.argument_of_periapsis);
        let lon1 = normalize_angle(el1.raan + el1.argument_of_periapsis);
        let d = normalize_angle(lon0 - lon1 + PI) - PI;
        assert!(
            d.abs() < 1e-6,
            "Ω+ω discontinuous across i=0: {lon0} (i=0) vs {lon1} (i={tilt})"
        );
    }

    #[test]
    fn test_roundtrip_elliptical() {
        // Elliptical inclined orbit: a=10000km, e=0.2, i=30°, Ω=45°, ω=60°, ν=90°
        let elements = KeplerianElements {
            semi_major_axis: 10000.0,
            eccentricity: 0.2,
            inclination: 30.0_f64.to_radians(),
            raan: 45.0_f64.to_radians(),
            argument_of_periapsis: 60.0_f64.to_radians(),
            true_anomaly: 90.0_f64.to_radians(),
        };

        let (pos, vel) = elements.to_state_vector(MU_EARTH);
        let elements2 = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);

        assert!(
            (elements.semi_major_axis - elements2.semi_major_axis).abs() < 1e-6,
            "a: {} vs {}",
            elements.semi_major_axis,
            elements2.semi_major_axis
        );
        assert!(
            (elements.eccentricity - elements2.eccentricity).abs() < 1e-10,
            "e: {} vs {}",
            elements.eccentricity,
            elements2.eccentricity
        );
        assert!(
            (elements.inclination - elements2.inclination).abs() < 1e-10,
            "i: {} vs {}",
            elements.inclination,
            elements2.inclination
        );
        assert!(
            (elements.raan - elements2.raan).abs() < 1e-10,
            "Ω: {} vs {}",
            elements.raan,
            elements2.raan
        );
        assert!(
            (elements.argument_of_periapsis - elements2.argument_of_periapsis).abs() < 1e-10,
            "ω: {} vs {}",
            elements.argument_of_periapsis,
            elements2.argument_of_periapsis
        );
        assert!(
            (elements.true_anomaly - elements2.true_anomaly).abs() < 1e-10,
            "ν: {} vs {}",
            elements.true_anomaly,
            elements2.true_anomaly
        );
    }

    #[test]
    fn test_period_iss() {
        // ISS orbit: h=400km, r=6778.137km
        // T = 2π√(r³/μ) ≈ 5553.6s
        let r = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        let pos = Vector3::new(r, 0.0, 0.0);
        let vel = Vector3::new(0.0, v, 0.0);

        let elements = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);
        let period = elements.period(MU_EARTH);

        let expected_period = 2.0 * PI * (r.powi(3) / MU_EARTH).sqrt();
        assert!(
            (period - expected_period).abs() < 0.1,
            "period: expected≈{expected_period}s, got={period}s"
        );
        // Verify approximate value
        assert!(
            (period - 5553.6).abs() < 1.0,
            "period should be ≈5553.6s, got={period}s"
        );
    }

    #[test]
    fn test_keplers_third_law() {
        // T²/a³ = 4π²/μ = const for different orbits
        let constant = 4.0 * PI * PI / MU_EARTH;

        let radii = [7000.0, 10000.0, 20000.0, 42164.0]; // LEO, MEO, HEO, GEO
        for &a in &radii {
            let elements = KeplerianElements {
                semi_major_axis: a,
                eccentricity: 0.0,
                inclination: 0.0,
                raan: 0.0,
                argument_of_periapsis: 0.0,
                true_anomaly: 0.0,
            };
            let t = elements.period(MU_EARTH);
            let ratio = t * t / (a * a * a);
            assert!(
                (ratio - constant).abs() / constant < 1e-12,
                "Kepler's third law violated for a={a}: ratio={ratio}, expected={constant}"
            );
        }
    }

    #[test]
    fn test_energy() {
        let elements = KeplerianElements {
            semi_major_axis: 10000.0,
            eccentricity: 0.3,
            inclination: 0.0,
            raan: 0.0,
            argument_of_periapsis: 0.0,
            true_anomaly: 0.0,
        };
        let energy = elements.energy(MU_EARTH);
        let expected = -MU_EARTH / (2.0 * 10000.0);
        assert!(
            (energy - expected).abs() < 1e-10,
            "energy: expected={expected}, got={energy}"
        );
        // Energy should be negative for bound orbit
        assert!(energy < 0.0, "bound orbit energy should be negative");
    }

    // Kepler equation solver tests

    #[test]
    fn test_kepler_equation_circular() {
        // For e=0, M = E = ν
        let m = 1.0_f64; // 1 radian
        let e_anom = solve_kepler_equation(m, 0.0);
        assert!(
            (e_anom - m).abs() < 1e-14,
            "For e=0, E should equal M: E={e_anom}, M={m}"
        );
    }

    #[test]
    fn test_kepler_equation_known_values() {
        // For M=π, any eccentricity: E=π (by symmetry of Kepler's equation)
        let e_anom = solve_kepler_equation(PI, 0.5);
        assert!(
            (e_anom - PI).abs() < 1e-12,
            "For M=π, E should be π: E={e_anom}"
        );
    }

    #[test]
    fn test_kepler_equation_roundtrip() {
        // Verify: given E, compute M = E - e·sin(E), then solve back to get E
        let eccentricities = [0.0, 0.1, 0.5, 0.9];
        let eccentric_anomalies = [0.0, 0.5, 1.0, PI / 2.0, PI, 2.0 * PI - 0.5];
        for &e in &eccentricities {
            for &e_orig in &eccentric_anomalies {
                let m = eccentric_to_mean_anomaly(e_orig, e);
                let e_solved = solve_kepler_equation(m, e);
                let m_check = eccentric_to_mean_anomaly(e_solved, e);
                assert!(
                    (m - m_check).abs() < 1e-12,
                    "Roundtrip failed: e={e}, E_orig={e_orig}, M={m}, E_solved={e_solved}, M_check={m_check}"
                );
            }
        }
    }

    #[test]
    fn test_kepler_equation_high_eccentricity() {
        // High eccentricity convergence test (e=0.99)
        let e = 0.99;
        let m = 1.0;
        let e_anom = solve_kepler_equation(m, e);
        let m_check = e_anom - e * e_anom.sin();
        assert!(
            (m - m_check).abs() < 1e-12,
            "High-e convergence: M={m}, E={e_anom}, M_check={m_check}"
        );
    }

    /// The returned E solves the equation it is named for, over the whole
    /// eccentricity range the signature accepts.
    ///
    /// The oracle is Kepler's equation itself — `E - e·sin(E) - M`, which the
    /// solver does not compute a residual of — so this does not compare the
    /// solver against itself the way a `ν → M → ν` round-trip does.
    ///
    /// `test_kepler_equation_high_eccentricity` samples one point, `e = 0.99`
    /// with `M = 1.0`, which Newton reaches from `E₀ = M` in a few steps. The
    /// grid here includes the starting points from which it does not.
    #[test]
    fn kepler_solution_satisfies_the_equation_at_every_eccentricity() {
        let mut worst = (0.0f64, 0.0f64, 0.0f64);
        for &e in &[0.0, 0.1, 0.5, 0.7, 0.9, 0.95, 0.99, 0.995, 0.999] {
            // 0.05 rad steps across two full revolutions, so the reduction of
            // `M` and both signs of the residual are covered.
            for i in -252..=252 {
                let m = i as f64 * 0.05;
                let e_anom = solve_kepler_equation(m, e);
                // `solve_kepler_equation` reduces M the same way; the residual
                // is against the reduced value it actually solved for.
                let residual = e_anom - e * e_anom.sin() - m % (2.0 * PI);
                if residual.abs() > worst.0.abs() {
                    worst = (residual, m, e);
                }
            }
        }
        // 1e-13 rather than the solver's 1e-14 step threshold: the step is on
        // E, and near periapsis at e = 0.999 the residual is that step times
        // `f' = 1 - e·cos(E)`, which is 1.999 there.
        assert!(
            worst.0.abs() < 1e-13,
            "E must solve M = E - e·sin(E): residual {:.3e} at M={}, e={} (E={})",
            worst.0,
            worst.1,
            worst.2,
            solve_kepler_equation(worst.1, worst.2)
        );
    }

    /// Newton from `E₀ = M` diverges here, and the solver used to return the
    /// diverged iterate as the answer.
    ///
    /// Measured before the bracket went in: `e = 0.995, M = 0.4` returned
    /// `E = 2.7e6` rad, and the true anomaly that follows from it was 352.9°
    /// away from the one the root gives. `f' = 1 - e·cos(E)` is 5e-3 at
    /// `E₀ = 0.4`, so the first step is three orders of magnitude too long.
    #[test]
    fn a_diverging_newton_start_still_gives_the_root() {
        for &(m, e) in &[
            (0.4, 0.995),
            (0.45, 0.995),
            (5.9, 0.995),
            (6.2, 0.995),
            (0.25, 0.99),
            (5.85, 0.99),
        ] {
            let e_anom = solve_kepler_equation(m, e);
            let residual = e_anom - e * e_anom.sin() - m;
            assert!(
                residual.abs() < 1e-13,
                "M={m}, e={e}: E={e_anom} leaves a residual of {residual:.3e}"
            );
            // `E = M + e·sin(E)` bounds the root to within e of M. The
            // diverged values were 6 orders of magnitude outside it.
            assert!(
                (e_anom - m).abs() <= e + 1e-12,
                "M={m}, e={e}: E={e_anom} is further than e from M"
            );
        }
    }

    #[test]
    fn test_eccentric_to_true_anomaly_circular() {
        // For e=0, E = ν
        let e_anom = 1.5;
        let nu = eccentric_to_true_anomaly(e_anom, 0.0);
        assert!(
            (nu - e_anom).abs() < 1e-14,
            "For e=0, ν should equal E: ν={nu}, E={e_anom}"
        );
    }

    #[test]
    fn test_eccentric_to_true_anomaly_at_periapsis() {
        // At periapsis: E=0 → ν=0
        let nu = eccentric_to_true_anomaly(0.0, 0.5);
        assert!(nu.abs() < 1e-14, "At periapsis, ν should be 0: ν={nu}");
    }

    #[test]
    fn test_eccentric_to_true_anomaly_at_apoapsis() {
        // At apoapsis: E=π → ν=π
        let nu = eccentric_to_true_anomaly(PI, 0.5);
        assert!(
            (nu - PI).abs() < 1e-12,
            "At apoapsis, ν should be π: ν={nu}"
        );
    }

    #[test]
    fn test_mean_to_true_anomaly_roundtrip() {
        // ν → M → ν roundtrip for various eccentricities
        let eccentricities = [0.0, 0.1, 0.3, 0.7];
        let true_anomalies = [0.0, 0.5, 1.0, PI / 2.0, PI, 3.0 * PI / 2.0, 5.5];
        for &e in &eccentricities {
            for &nu_orig in &true_anomalies {
                let m = true_to_mean_anomaly(nu_orig, e);
                let nu_solved = mean_to_true_anomaly(m, e);
                // Compare via state vector to avoid angle wrapping issues
                assert!(
                    (nu_orig.cos() - nu_solved.cos()).abs() < 1e-10
                        && (nu_orig.sin() - nu_solved.sin()).abs() < 1e-10,
                    "Roundtrip failed: e={e}, ν_orig={nu_orig}, M={m}, ν_solved={nu_solved}"
                );
            }
        }
    }

    #[test]
    fn test_from_mean_anomaly_circular() {
        // For circular orbit, mean anomaly = true anomaly
        let m = 1.0;
        let elements = KeplerianElements::from_mean_anomaly(7000.0, 0.0, 0.0, 0.0, 0.0, m);
        assert!(
            (elements.true_anomaly - m).abs() < 1e-12,
            "For e=0, true_anomaly should equal mean_anomaly: ν={}, M={m}",
            elements.true_anomaly
        );
    }

    #[test]
    fn test_from_mean_anomaly_elliptical() {
        // Verify that from_mean_anomaly produces a state vector
        // whose energy matches the expected value for the given semi-major axis
        let a = 10000.0;
        let e = 0.2;
        let m = 1.5; // radians
        let elements = KeplerianElements::from_mean_anomaly(
            a,
            e,
            30.0_f64.to_radians(),
            45.0_f64.to_radians(),
            60.0_f64.to_radians(),
            m,
        );
        let (pos, vel) = elements.to_state_vector(MU_EARTH);

        // Check energy: ε = v²/2 - μ/r = -μ/(2a)
        let r = pos.magnitude();
        let v = vel.magnitude();
        let energy = v * v / 2.0 - MU_EARTH / r;
        let expected_energy = -MU_EARTH / (2.0 * a);
        assert!(
            (energy - expected_energy).abs() / expected_energy.abs() < 1e-10,
            "Energy mismatch: got={energy}, expected={expected_energy}"
        );
    }
}
