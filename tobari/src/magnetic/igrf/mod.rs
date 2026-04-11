//! IGRF-14 spherical harmonic magnetic field model.

use kaname::epoch::Epoch;
use kaname::frame::{self, Rotation};
use kaname::{SimpleEcef, SimpleEci};
use nalgebra::Vector3;

mod coeff;

use super::MagneticFieldModel;
use coeff::*;

/// Gauss coefficient set for a single epoch.
#[derive(Clone)]
pub struct GaussCoefficients {
    /// g coefficients \[nT\], flat-indexed by `coeff_index(n, m)`.
    pub g: Vec<f64>,
    /// h coefficients \[nT\], flat-indexed by `coeff_index(n, m)`.
    pub h: Vec<f64>,
    /// Epoch year for this coefficient set.
    pub year: f64,
}

/// IGRF (International Geomagnetic Reference Field) model.
///
/// Evaluates the geomagnetic field using spherical harmonic expansion.
///
/// By default uses built-in IGRF-14 Gauss coefficients (2020 DGRF + 2025 IGRF + SV).
/// Custom coefficients can be injected at runtime via [`Igrf::from_coefficients`].
pub struct Igrf {
    max_degree: usize,
    /// Custom coefficient overrides. When `None`, uses built-in IGRF-14.
    custom_coeffs: Option<CustomCoeffs>,
}

struct CustomCoeffs {
    epoch_a: GaussCoefficients,
    epoch_b: GaussCoefficients,
    /// Secular variation \[nT/yr\] for extrapolation beyond epoch_b.
    sv_g: Vec<f64>,
    sv_h: Vec<f64>,
}

impl Igrf {
    /// Create an IGRF model with built-in IGRF-14 coefficients (degree 13).
    pub fn earth() -> Self {
        Self {
            max_degree: IGRF_MAX_DEGREE,
            custom_coeffs: None,
        }
    }

    /// Create an IGRF model truncated to the given maximum degree.
    ///
    /// Lower degrees are faster but less accurate. Degree 1 gives a dipole.
    ///
    /// # Panics
    /// Panics if `max_degree` is 0 or exceeds [`IGRF_MAX_DEGREE`].
    pub fn with_max_degree(max_degree: usize) -> Self {
        assert!(
            (1..=IGRF_MAX_DEGREE).contains(&max_degree),
            "max_degree must be in 1..={IGRF_MAX_DEGREE}, got {max_degree}"
        );
        Self {
            max_degree,
            custom_coeffs: None,
        }
    }

    /// Create an IGRF model with custom coefficient data injected at runtime.
    ///
    /// This allows using coefficient sets from different IGRF generations,
    /// or coefficients downloaded/parsed externally.
    ///
    /// `epoch_a` and `epoch_b` define the two bracketing epochs for interpolation.
    /// `sv` contains the secular variation for extrapolation beyond `epoch_b`.
    pub fn from_coefficients(
        epoch_a: GaussCoefficients,
        epoch_b: GaussCoefficients,
        sv: GaussCoefficients,
        max_degree: usize,
    ) -> Self {
        assert!(
            (1..=IGRF_MAX_DEGREE).contains(&max_degree),
            "max_degree must be in 1..={IGRF_MAX_DEGREE}, got {max_degree}"
        );
        Self {
            max_degree,
            custom_coeffs: Some(CustomCoeffs {
                epoch_a,
                epoch_b,
                sv_g: sv.g,
                sv_h: sv.h,
            }),
        }
    }
}

impl MagneticFieldModel for Igrf {
    fn field_eci(&self, position_eci: &SimpleEci, epoch: &Epoch) -> frame::Vec3<frame::SimpleEci> {
        let gmst = epoch.gmst();
        let r_eci_to_ecef = Rotation::<frame::SimpleEci, frame::SimpleEcef>::from_era(gmst);
        let r_ecef_to_eci = Rotation::<frame::SimpleEcef, frame::SimpleEci>::from_era(gmst);

        // ECI → ECEF
        let ecef = r_eci_to_ecef.transform(position_eci);
        let (x, y, z) = (ecef.x(), ecef.y(), ecef.z());

        // Cartesian → geocentric spherical
        let r_km = (x * x + y * y + z * z).sqrt();
        if r_km < 1.0 {
            return frame::Vec3::zeros();
        }
        let p = (x * x + y * y).sqrt(); // distance from z-axis
        let cos_theta = z / r_km;
        let sin_theta = p / r_km;
        let phi = y.atan2(x); // longitude

        // Interpolate Gauss coefficients to epoch
        let year = decimal_year(epoch);
        let (g, h) = match &self.custom_coeffs {
            Some(c) => interpolate_custom(year, c, self.max_degree),
            None => interpolate_builtin(year, self.max_degree),
        };

        // Evaluate spherical harmonic expansion
        let (b_r, b_theta, b_phi) =
            evaluate_sh(&g, &h, r_km, cos_theta, sin_theta, phi, self.max_degree);

        // Convert nT → T
        let b_r_t = b_r * 1e-9;
        let b_theta_t = b_theta * 1e-9;
        let b_phi_t = b_phi * 1e-9;

        // Spherical (B_r, B_theta, B_phi) → ECEF Cartesian
        let cos_phi = phi.cos();
        let sin_phi = phi.sin();

        let b_ecef = Vector3::new(
            sin_theta * cos_phi * b_r_t + cos_theta * cos_phi * b_theta_t - sin_phi * b_phi_t,
            sin_theta * sin_phi * b_r_t + cos_theta * sin_phi * b_theta_t + cos_phi * b_phi_t,
            cos_theta * b_r_t - sin_theta * b_theta_t,
        );

        // ECEF → ECI
        r_ecef_to_eci.transform(&SimpleEcef::from_raw(b_ecef))
    }
}

// ---------------------------------------------------------------------------
// Time utilities
// ---------------------------------------------------------------------------

fn decimal_year(epoch: &Epoch) -> f64 {
    let dt = epoch.to_datetime();
    let jan1 = Epoch::from_gregorian(dt.year, 1, 1, 0, 0, 0.0);
    let jan1_next = Epoch::from_gregorian(dt.year + 1, 1, 1, 0, 0, 0.0);
    let denom = jan1_next.jd() - jan1.jd();
    if denom.abs() < 1e-10 {
        return dt.year as f64;
    }
    dt.year as f64 + (epoch.jd() - jan1.jd()) / denom
}

// ---------------------------------------------------------------------------
// Coefficient interpolation
// ---------------------------------------------------------------------------

fn interpolate_builtin(year: f64, max_degree: usize) -> (Vec<f64>, Vec<f64>) {
    let n = N_COEFFS;
    let mut g = vec![0.0; n];
    let mut h = vec![0.0; n];
    let years = &EPOCH_YEARS;
    let last_year = years[NUM_EPOCHS - 1];

    if year >= last_year {
        // At or beyond the last main-field epoch: extrapolate using SV (dt=0 at exact epoch)
        let dt = year - last_year;
        for i in 0..n {
            g[i] = G_EPOCHS[NUM_EPOCHS - 1][i] + DG_SV[i] * dt;
            h[i] = H_EPOCHS[NUM_EPOCHS - 1][i] + DH_SV[i] * dt;
        }
    } else if year <= years[0] {
        // Before the first epoch: extrapolate backward from first interval
        let span = years[1] - years[0];
        let dt = year - years[0];
        for i in 0..n {
            let sv_g = (G_EPOCHS[1][i] - G_EPOCHS[0][i]) / span;
            let sv_h = (H_EPOCHS[1][i] - H_EPOCHS[0][i]) / span;
            g[i] = G_EPOCHS[0][i] + sv_g * dt;
            h[i] = H_EPOCHS[0][i] + sv_h * dt;
        }
    } else {
        // Find bracketing epochs and interpolate
        let mut idx_a = 0;
        for j in 0..(NUM_EPOCHS - 1) {
            if year >= years[j] && year < years[j + 1] {
                idx_a = j;
                break;
            }
        }
        let idx_b = idx_a + 1;
        let ya = years[idx_a];
        let span = years[idx_b] - ya;
        let dt = year - ya;

        for i in 0..n {
            let sv_g = (G_EPOCHS[idx_b][i] - G_EPOCHS[idx_a][i]) / span;
            let sv_h = (H_EPOCHS[idx_b][i] - H_EPOCHS[idx_a][i]) / span;
            g[i] = G_EPOCHS[idx_a][i] + sv_g * dt;
            h[i] = H_EPOCHS[idx_a][i] + sv_h * dt;
        }
    }

    // Zero out coefficients beyond max_degree
    zero_above_degree(&mut g, &mut h, max_degree);

    (g, h)
}

fn zero_above_degree(g: &mut [f64], h: &mut [f64], max_degree: usize) {
    let n = g.len();
    for nn in (max_degree + 1)..=IGRF_MAX_DEGREE {
        for mm in 0..=nn {
            let idx = coeff_index(nn, mm);
            if idx < n {
                g[idx] = 0.0;
                h[idx] = 0.0;
            }
        }
    }
}

fn interpolate_custom(year: f64, c: &CustomCoeffs, max_degree: usize) -> (Vec<f64>, Vec<f64>) {
    let n = N_COEFFS;
    let mut g = vec![0.0; n];
    let mut h = vec![0.0; n];

    let ya = c.epoch_a.year;
    let yb = c.epoch_b.year;
    let span = yb - ya;

    if year <= yb && span.abs() > 1e-10 {
        // Interpolate between epoch_a and epoch_b
        let dt = year - ya;
        let len = n
            .min(c.epoch_a.g.len())
            .min(c.epoch_a.h.len())
            .min(c.epoch_b.g.len())
            .min(c.epoch_b.h.len());
        for i in 0..len {
            let sv_g = (c.epoch_b.g[i] - c.epoch_a.g[i]) / span;
            let sv_h = (c.epoch_b.h[i] - c.epoch_a.h[i]) / span;
            g[i] = c.epoch_a.g[i] + sv_g * dt;
            h[i] = c.epoch_a.h[i] + sv_h * dt;
        }
    } else {
        // Extrapolate from epoch_b using SV
        let dt = year - yb;
        let len = n
            .min(c.epoch_b.g.len())
            .min(c.epoch_b.h.len())
            .min(c.sv_g.len())
            .min(c.sv_h.len());
        for i in 0..len {
            g[i] = c.epoch_b.g[i] + c.sv_g[i] * dt;
            h[i] = c.epoch_b.h[i] + c.sv_h[i] * dt;
        }
    }

    zero_above_degree(&mut g, &mut h, max_degree);

    (g, h)
}

// ---------------------------------------------------------------------------
// Spherical harmonic evaluation
// ---------------------------------------------------------------------------

/// Evaluate the IGRF spherical harmonic expansion.
///
/// Returns (B_r, B_theta, B_phi) in nT, where:
/// - B_r: radial (outward)
/// - B_theta: southward (colatitude direction)
/// - B_phi: eastward (longitude direction)
fn evaluate_sh(
    g: &[f64],
    h: &[f64],
    r_km: f64,
    cos_theta: f64,
    sin_theta: f64,
    phi: f64,
    max_degree: usize,
) -> (f64, f64, f64) {
    let a = IGRF_REFERENCE_RADIUS;
    let ratio = a / r_km;

    // Schmidt semi-normalized associated Legendre polynomials P[n][m] and dP/dtheta
    // We compute these recursively. Array size: [N_MAX+1][N_MAX+1]
    let nd = max_degree + 1;
    let mut p = vec![vec![0.0; nd]; nd];
    let mut dp = vec![vec![0.0; nd]; nd];

    // Initialize P[0][0] = 1
    p[0][0] = 1.0;
    dp[0][0] = 0.0;

    // Diagonal: P[n][n]
    for n in 1..nd {
        let factor = if n == 1 {
            1.0
        } else {
            ((2 * n - 1) as f64 / (2 * n) as f64).sqrt()
        };
        p[n][n] = factor * sin_theta * p[n - 1][n - 1];
        dp[n][n] = factor * (sin_theta * dp[n - 1][n - 1] + cos_theta * p[n - 1][n - 1]);
    }

    // Sub-diagonal: P[n][n-1]
    for n in 1..nd {
        p[n][n - 1] = cos_theta * (2 * n - 1) as f64 * p[n - 1][n - 1];
        dp[n][n - 1] =
            (2 * n - 1) as f64 * (cos_theta * dp[n - 1][n - 1] - sin_theta * p[n - 1][n - 1]);
    }

    // General recursion: P[n][m] for m < n-1
    for n in 2..nd {
        for m in 0..=(n.saturating_sub(2)) {
            let n_f = n as f64;
            let m_f = m as f64;
            let k = ((n_f - 1.0) * (n_f - 1.0) - m_f * m_f).sqrt();
            let denom = (n_f * n_f - m_f * m_f).sqrt();
            p[n][m] = ((2.0 * n_f - 1.0) * cos_theta * p[n - 1][m] - k * p[n - 2][m]) / denom;
            dp[n][m] = ((2.0 * n_f - 1.0) * (cos_theta * dp[n - 1][m] - sin_theta * p[n - 1][m])
                - k * dp[n - 2][m])
                / denom;
        }
    }

    // Accumulate field components
    let mut b_r = 0.0;
    let mut b_theta = 0.0;
    let mut b_phi = 0.0;

    let mut r_power = ratio * ratio; // (a/r)^2 for n=1

    for n in 1..nd {
        r_power *= ratio; // (a/r)^(n+2)
        let n_plus_1 = (n + 1) as f64;

        for m in 0..=n {
            let idx = coeff_index(n, m);
            let g_nm = g[idx];
            let h_nm = h[idx];
            let m_f = m as f64;

            let cos_m_phi = (m_f * phi).cos();
            let sin_m_phi = (m_f * phi).sin();

            let gh_cos_sin = g_nm * cos_m_phi + h_nm * sin_m_phi;

            // B_r = -dV/dr = sum (n+1)(a/r)^(n+2) * (g cos + h sin) * P
            b_r += n_plus_1 * r_power * gh_cos_sin * p[n][m];

            // B_theta = -(1/r) dV/dtheta = -sum (a/r)^(n+2) * (g cos + h sin) * dP/dtheta
            b_theta -= r_power * gh_cos_sin * dp[n][m];

            // B_phi = -(1/(r sin theta)) dV/dphi
            //       = sum (a/r)^(n+2) * m * (-g sin + h cos) * P / sin(theta)
            if m > 0 {
                let gh_sin_cos = -g_nm * sin_m_phi + h_nm * cos_m_phi;
                if sin_theta.abs() > 1e-10 {
                    b_phi += r_power * m_f * gh_sin_cos * p[n][m] / sin_theta;
                } else if m == 1 {
                    // At poles (sin_theta ≈ 0), only m=1 contributes a finite limit.
                    // For Schmidt semi-normalized:
                    //   lim_{θ→0,π} P_n^1(cos θ) / sin θ = sqrt(n*(n+1)/2)
                    // Sign: at θ=0 (north pole) the limit is positive,
                    //        at θ=π (south pole) it's (-1)^(n+1) * sqrt(n*(n+1)/2).
                    let n_f = n as f64;
                    let limit = (n_f * (n_f + 1.0) / 2.0).sqrt();
                    // At south pole (cos_theta < 0), odd-(n+1) terms flip sign.
                    let sign = if cos_theta < 0.0 && (n + 1) % 2 != 0 {
                        -1.0
                    } else {
                        1.0
                    };
                    b_phi += r_power * gh_sin_cos * sign * limit;
                }
                // For m > 1: P_n^m / sin(theta) → 0 at the poles.
            }
        }
    }

    (b_r, b_theta, b_phi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch_2025() -> Epoch {
        Epoch::from_gregorian(2025, 1, 1, 0, 0, 0.0)
    }

    #[test]
    fn decimal_year_j2000() {
        let dy = decimal_year(&Epoch::j2000());
        assert!(
            (dy - 2000.0).abs() < 0.01,
            "J2000 should be ~2000.0, got {dy}"
        );
    }

    #[test]
    fn decimal_year_2025_jan1() {
        let dy = decimal_year(&epoch_2025());
        assert!(
            (dy - 2025.0).abs() < 0.01,
            "2025 Jan 1 should be ~2025.0, got {dy}"
        );
    }

    #[test]
    fn igrf_field_magnitude_at_equatorial_leo() {
        // At equatorial LEO (7000 km from centre), expect |B| ~ 20-50 uT
        let igrf = Igrf::earth();
        let pos = SimpleEci::new(7000.0, 0.0, 0.0);
        let epoch = epoch_2025();
        let b = igrf.field_eci(&pos, &epoch);
        let b_micro_t = b.magnitude() * 1e6;

        assert!(
            b_micro_t > 15.0 && b_micro_t < 60.0,
            "Equatorial LEO field should be ~20-50 uT, got {b_micro_t:.2} uT"
        );
    }

    #[test]
    fn igrf_field_magnitude_at_north_pole() {
        // Near north pole, expect stronger field ~50-60 uT
        let igrf = Igrf::earth();
        let r = 6771.0; // ~400km altitude at pole
        let pos = SimpleEci::new(0.0, 0.0, r);
        let epoch = epoch_2025();
        let b = igrf.field_eci(&pos, &epoch);
        let b_micro_t = b.magnitude() * 1e6;

        assert!(
            b_micro_t > 40.0 && b_micro_t < 80.0,
            "Polar field should be ~50-65 uT, got {b_micro_t:.2} uT"
        );
    }

    #[test]
    fn igrf_inverse_cube_at_high_altitude() {
        // At large distances, field should approximately follow 1/r^3
        let igrf = Igrf::earth();
        let epoch = epoch_2025();
        let b1 = igrf
            .field_eci(&SimpleEci::new(20000.0, 0.0, 0.0), &epoch)
            .magnitude();
        let b2 = igrf
            .field_eci(&SimpleEci::new(40000.0, 0.0, 0.0), &epoch)
            .magnitude();

        let ratio = b1 / b2;
        // Should be close to 8.0 (exact for pure dipole)
        assert!(
            (ratio - 8.0).abs() < 0.5,
            "Expected ~8.0 ratio at high altitude, got {ratio:.2}"
        );
    }

    #[test]
    fn igrf_differs_from_dipole_at_leo() {
        // At LEO, IGRF should differ meaningfully from a simple dipole
        use super::super::TiltedDipole;

        let igrf = Igrf::earth();
        let dipole = TiltedDipole::earth();
        let epoch = epoch_2025();

        // South Atlantic Anomaly region (~-30° lat, -50° lon in ECEF)
        // Use a position that will be roughly there at some epoch
        let pos = SimpleEci::new(4000.0, -4000.0, -3500.0);

        let b_igrf = igrf.field_eci(&pos, &epoch).magnitude();
        let b_dipole = dipole.field_eci(&pos, &epoch).magnitude();

        let diff_pct = ((b_igrf - b_dipole) / b_dipole).abs() * 100.0;
        assert!(
            diff_pct > 0.5,
            "IGRF and dipole should differ by >0.5% at LEO, got {diff_pct:.1}%"
        );
    }

    #[test]
    fn igrf_converges_to_dipole_at_geo() {
        // At GEO altitude, higher harmonics are negligible
        use super::super::TiltedDipole;

        let igrf = Igrf::earth();
        let dipole = TiltedDipole::earth();
        let epoch = epoch_2025();

        let geo_r = 42164.0; // GEO radius in km
        let pos = SimpleEci::new(geo_r, 0.0, 0.0);

        let b_igrf = igrf.field_eci(&pos, &epoch).magnitude();
        let b_dipole = dipole.field_eci(&pos, &epoch).magnitude();

        // At GEO the dipole dominates; expect <10% difference
        // (TiltedDipole uses approximate parameters, so some difference is expected)
        let diff_pct = ((b_igrf - b_dipole) / b_dipole).abs() * 100.0;
        assert!(
            diff_pct < 15.0,
            "IGRF and dipole should converge at GEO (<15%), got {diff_pct:.1}%"
        );
    }

    #[test]
    fn igrf_zero_inside_earth() {
        let igrf = Igrf::earth();
        let epoch = epoch_2025();
        let b = igrf.field_eci(&SimpleEci::new(0.5, 0.0, 0.0), &epoch);
        assert_eq!(b, frame::Vec3::<frame::SimpleEci>::zeros());
    }

    #[test]
    fn igrf_field_is_finite() {
        let igrf = Igrf::earth();
        let epoch = epoch_2025();
        let b = igrf.field_eci(&SimpleEci::new(6778.0, 0.0, 0.0), &epoch);
        assert!(b.is_finite(), "Field must be finite: {b:?}");
    }

    #[test]
    fn igrf_secular_variation() {
        // Field should change between 2020 and 2025
        let igrf = Igrf::earth();
        let pos = SimpleEci::new(7000.0, 0.0, 0.0);

        let e2020 = Epoch::from_gregorian(2020, 1, 1, 0, 0, 0.0);
        let e2025 = epoch_2025();

        let b2020 = igrf.field_eci(&pos, &e2020);
        let b2025 = igrf.field_eci(&pos, &e2025);

        let diff = (b2020 - b2025).magnitude();
        assert!(
            diff > 1e-10,
            "Field should change between 2020 and 2025, diff={diff:.3e}"
        );
    }

    #[test]
    fn igrf_truncation_degree1_is_dipole_like() {
        let igrf1 = Igrf::with_max_degree(1);
        let igrf13 = Igrf::earth();
        let epoch = epoch_2025();

        let pos = SimpleEci::new(7000.0, 0.0, 0.0);
        let b1 = igrf1.field_eci(&pos, &epoch).magnitude();
        let b13 = igrf13.field_eci(&pos, &epoch).magnitude();

        // Degree-1 should be within ~20% of full model at LEO
        let diff_pct = ((b1 - b13) / b13).abs() * 100.0;
        assert!(
            diff_pct < 25.0,
            "Degree-1 truncation should be within 25% of full model, got {diff_pct:.1}%"
        );
    }
}
