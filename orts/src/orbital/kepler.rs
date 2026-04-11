use nalgebra::Vector3;
use std::f64::consts::PI;

/// Solve Kepler's equation `M = E - e·sin(E)` for eccentric anomaly E
/// using Newton-Raphson iteration.
///
/// # Arguments
/// * `mean_anomaly` - Mean anomaly M [rad]
/// * `eccentricity` - Orbital eccentricity (0 ≤ e < 1)
///
/// # Returns
/// Eccentric anomaly E [rad]
pub fn solve_kepler_equation(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let m = mean_anomaly % (2.0 * PI);
    let mut e_anom = m; // initial guess
    for _ in 0..50 {
        let f = e_anom - eccentricity * e_anom.sin() - m;
        let f_prime = 1.0 - eccentricity * e_anom.cos();
        let delta = f / f_prime;
        e_anom -= delta;
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

/// Classical Keplerian orbital elements.
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

        // Inclination: cos(i) = h_z / |h|
        let i = (h[2] / h_mag).acos();

        // Right ascension of ascending node
        let raan = if n_mag > 1e-15 {
            let omega = (n[0] / n_mag).acos();
            if n[1] >= 0.0 { omega } else { 2.0 * PI - omega }
        } else {
            0.0
        };

        // Argument of periapsis
        let omega = if n_mag > 1e-15 && e > 1e-15 {
            let cos_omega = n.dot(&e_vec) / (n_mag * e);
            let w = cos_omega.clamp(-1.0, 1.0).acos();
            if e_vec[2] >= 0.0 { w } else { 2.0 * PI - w }
        } else {
            0.0
        };

        // True anomaly
        let nu = if e > 1e-15 {
            let cos_nu = e_vec.dot(pos) / (e * r);
            let nu_val = cos_nu.clamp(-1.0, 1.0).acos();
            if pos.dot(vel) >= 0.0 {
                nu_val
            } else {
                2.0 * PI - nu_val
            }
        } else {
            // Circular orbit: measure from ascending node or x-axis
            if n_mag > 1e-15 {
                let cos_nu = n.dot(pos) / (n_mag * r);
                let nu_val = cos_nu.clamp(-1.0, 1.0).acos();
                if pos[2] >= 0.0 {
                    nu_val
                } else {
                    2.0 * PI - nu_val
                }
            } else {
                // Circular equatorial: measure from x-axis
                let nu_val = (pos[0] / r).clamp(-1.0, 1.0).acos();
                if pos[1] >= 0.0 {
                    nu_val
                } else {
                    2.0 * PI - nu_val
                }
            }
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
    use arika::earth::MU as MU_EARTH;
    use nalgebra::vector;

    #[test]
    fn test_circular_orbit_elements() {
        // Circular orbit at ISS altitude: r = R_earth + 400 = 6778.137 km
        let r = 6778.137;
        let v = (MU_EARTH / r).sqrt();
        let pos = vector![r, 0.0, 0.0];
        let vel = vector![0.0, v, 0.0];

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
        let pos = vector![r, 0.0, 0.0];
        let vel = vector![0.0, v, 0.0];

        let elements = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);
        let (pos2, vel2) = elements.to_state_vector(MU_EARTH);

        let pos_err = (pos - pos2).magnitude();
        let vel_err = (vel - vel2).magnitude();
        assert!(pos_err < 1e-6, "position roundtrip error: {pos_err} km");
        assert!(vel_err < 1e-9, "velocity roundtrip error: {vel_err} km/s");
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
        let pos = vector![r, 0.0, 0.0];
        let vel = vector![0.0, v, 0.0];

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

    // --- Kepler equation solver tests ---

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
