//! Harris-Priester atmospheric density model.
//!
//! Includes diurnal density variation based on the Sun's position.
//! The density depends on altitude and the angle between the satellite
//! and the atmospheric density bulge apex (lagging ~30° behind the sub-solar point).
//!
//! Reference: Montenbruck & Gill, "Satellite Orbits" (2000), Section 3.5.2.

use kaname::epoch::Epoch;
use kaname::sun;
use nalgebra::Vector3;

use crate::AtmosphereModel;

/// Harris-Priester density table entry.
struct HpEntry {
    h: f64,       // altitude [km]
    rho_min: f64, // minimum density [kg/m³] (night side / solar minimum)
    rho_max: f64, // maximum density [kg/m³] (day side / solar maximum)
}

/// Harris-Priester density table from Montenbruck & Gill "Satellite Orbits", Table 3.1.
/// Covers 100–2000 km altitude.
const HP_TABLE: &[HpEntry] = &[
    HpEntry { h: 100.0, rho_min: 4.974e-7, rho_max: 4.974e-7 },
    HpEntry { h: 120.0, rho_min: 2.490e-8, rho_max: 2.490e-8 },
    HpEntry { h: 130.0, rho_min: 8.377e-9, rho_max: 8.710e-9 },
    HpEntry { h: 140.0, rho_min: 3.899e-9, rho_max: 4.059e-9 },
    HpEntry { h: 150.0, rho_min: 2.122e-9, rho_max: 2.215e-9 },
    HpEntry { h: 160.0, rho_min: 1.263e-9, rho_max: 1.344e-9 },
    HpEntry { h: 170.0, rho_min: 8.008e-10, rho_max: 8.758e-10 },
    HpEntry { h: 180.0, rho_min: 5.283e-10, rho_max: 6.010e-10 },
    HpEntry { h: 190.0, rho_min: 3.617e-10, rho_max: 4.297e-10 },
    HpEntry { h: 200.0, rho_min: 2.557e-10, rho_max: 3.162e-10 },
    HpEntry { h: 210.0, rho_min: 1.839e-10, rho_max: 2.396e-10 },
    HpEntry { h: 220.0, rho_min: 1.341e-10, rho_max: 1.853e-10 },
    HpEntry { h: 230.0, rho_min: 9.949e-11, rho_max: 1.455e-10 },
    HpEntry { h: 240.0, rho_min: 7.488e-11, rho_max: 1.157e-10 },
    HpEntry { h: 250.0, rho_min: 5.709e-11, rho_max: 9.308e-11 },
    HpEntry { h: 260.0, rho_min: 4.403e-11, rho_max: 7.555e-11 },
    HpEntry { h: 270.0, rho_min: 3.430e-11, rho_max: 6.182e-11 },
    HpEntry { h: 280.0, rho_min: 2.697e-11, rho_max: 5.095e-11 },
    HpEntry { h: 290.0, rho_min: 2.139e-11, rho_max: 4.226e-11 },
    HpEntry { h: 300.0, rho_min: 1.708e-11, rho_max: 3.526e-11 },
    HpEntry { h: 320.0, rho_min: 1.099e-11, rho_max: 2.511e-11 },
    HpEntry { h: 340.0, rho_min: 7.214e-12, rho_max: 1.819e-11 },
    HpEntry { h: 360.0, rho_min: 4.824e-12, rho_max: 1.337e-11 },
    HpEntry { h: 380.0, rho_min: 3.274e-12, rho_max: 9.955e-12 },
    HpEntry { h: 400.0, rho_min: 2.249e-12, rho_max: 7.492e-12 },
    HpEntry { h: 420.0, rho_min: 1.558e-12, rho_max: 5.684e-12 },
    HpEntry { h: 440.0, rho_min: 1.091e-12, rho_max: 4.355e-12 },
    HpEntry { h: 460.0, rho_min: 7.701e-13, rho_max: 3.362e-12 },
    HpEntry { h: 480.0, rho_min: 5.474e-13, rho_max: 2.612e-12 },
    HpEntry { h: 500.0, rho_min: 3.916e-13, rho_max: 2.042e-12 },
    HpEntry { h: 520.0, rho_min: 2.819e-13, rho_max: 1.605e-12 },
    HpEntry { h: 540.0, rho_min: 2.042e-13, rho_max: 1.267e-12 },
    HpEntry { h: 560.0, rho_min: 1.488e-13, rho_max: 1.005e-12 },
    HpEntry { h: 580.0, rho_min: 1.092e-13, rho_max: 7.997e-13 },
    HpEntry { h: 600.0, rho_min: 8.070e-14, rho_max: 6.390e-13 },
    HpEntry { h: 620.0, rho_min: 6.012e-14, rho_max: 5.123e-13 },
    HpEntry { h: 640.0, rho_min: 4.519e-14, rho_max: 4.121e-13 },
    HpEntry { h: 660.0, rho_min: 3.430e-14, rho_max: 3.325e-13 },
    HpEntry { h: 680.0, rho_min: 2.632e-14, rho_max: 2.691e-13 },
    HpEntry { h: 700.0, rho_min: 2.043e-14, rho_max: 2.185e-13 },
    HpEntry { h: 720.0, rho_min: 1.607e-14, rho_max: 1.779e-13 },
    HpEntry { h: 740.0, rho_min: 1.281e-14, rho_max: 1.452e-13 },
    HpEntry { h: 760.0, rho_min: 1.036e-14, rho_max: 1.190e-13 },
    HpEntry { h: 780.0, rho_min: 8.496e-15, rho_max: 9.776e-14 },
    HpEntry { h: 800.0, rho_min: 7.069e-15, rho_max: 8.059e-14 },
    HpEntry { h: 840.0, rho_min: 4.680e-15, rho_max: 5.741e-14 },
    HpEntry { h: 880.0, rho_min: 3.200e-15, rho_max: 4.210e-14 },
    HpEntry { h: 920.0, rho_min: 2.210e-15, rho_max: 3.130e-14 },
    HpEntry { h: 960.0, rho_min: 1.560e-15, rho_max: 2.360e-14 },
    HpEntry { h: 1000.0, rho_min: 1.150e-15, rho_max: 1.810e-14 },
];

/// Harris-Priester atmospheric density model.
///
/// Computes density as a function of altitude and the angle between the satellite
/// position and the atmospheric density bulge apex. The bulge lags behind the
/// sub-solar point due to thermal inertia.
///
/// # Formula
///
/// ρ(h, ψ) = ρ_min(h) + \[ρ_max(h) - ρ_min(h)\] × cos^n(ψ/2)
///
/// where ψ is the angle between the satellite and the bulge apex,
/// and n controls the sharpness of the diurnal variation.
pub struct HarrisPriester {
    /// Cosine power exponent. Higher values produce a sharper density bulge.
    ///
    /// Typical values: n=2 (low inclination), n=6 (polar orbit).
    pub n: u32,
    /// Lag angle of the density bulge behind the sub-solar point \[radians\].
    ///
    /// Default: π/6 ≈ 30° (~2 hours in local solar time).
    pub lag_angle: f64,
    /// Function returning the Sun direction (unit vector) in ECI at a given epoch.
    sun_direction_fn: fn(&Epoch) -> Vector3<f64>,
}

impl HarrisPriester {
    /// Create a Harris-Priester model with default parameters.
    ///
    /// Uses n=2 and 30° lag angle.
    pub fn new() -> Self {
        Self {
            n: 2,
            lag_angle: std::f64::consts::FRAC_PI_6, // 30°
            sun_direction_fn: sun::sun_direction_eci,
        }
    }

    /// Set the cosine exponent (builder pattern).
    pub fn with_exponent(mut self, n: u32) -> Self {
        self.n = n;
        self
    }

    /// Set the lag angle in radians (builder pattern).
    pub fn with_lag_angle(mut self, lag_radians: f64) -> Self {
        self.lag_angle = lag_radians;
        self
    }

    /// Override the Sun direction function (for testing).
    pub fn with_sun_direction_fn(mut self, f: fn(&Epoch) -> Vector3<f64>) -> Self {
        self.sun_direction_fn = f;
        self
    }

    /// Compute the density bulge apex direction in ECI.
    ///
    /// The apex is the Sun direction rotated by `+lag_angle` about the Z-axis
    /// (Earth spin axis) in the direction of Earth's rotation (eastward).
    /// This places the bulge at ~14h local solar time (2 hours after local noon),
    /// representing the thermal inertia delay.
    fn bulge_apex(&self, epoch: &Epoch) -> Vector3<f64> {
        let sun_dir = (self.sun_direction_fn)(epoch);
        let cos_lag = self.lag_angle.cos();
        let sin_lag = self.lag_angle.sin();
        // Counter-clockwise rotation about Z-axis by +lag_angle
        Vector3::new(
            cos_lag * sun_dir.x - sin_lag * sun_dir.y,
            sin_lag * sun_dir.x + cos_lag * sun_dir.y,
            sun_dir.z,
        )
        .normalize()
    }

    /// Interpolate ρ_min and ρ_max from the HP table at a given altitude.
    ///
    /// Uses scale-height interpolation (log-linear) between table entries.
    fn interpolate_table(&self, altitude_km: f64) -> (f64, f64) {
        // Find the bracket
        let idx = HP_TABLE
            .iter()
            .position(|e| e.h > altitude_km)
            .unwrap_or(HP_TABLE.len());

        if idx == 0 {
            return (HP_TABLE[0].rho_min, HP_TABLE[0].rho_max);
        }
        if idx >= HP_TABLE.len() {
            let last = &HP_TABLE[HP_TABLE.len() - 1];
            return (last.rho_min, last.rho_max);
        }

        let lo = &HP_TABLE[idx - 1];
        let hi = &HP_TABLE[idx];

        let rho_min = scale_height_interp(altitude_km, lo.h, hi.h, lo.rho_min, hi.rho_min);
        let rho_max = scale_height_interp(altitude_km, lo.h, hi.h, lo.rho_max, hi.rho_max);

        (rho_min, rho_max)
    }
}

impl Default for HarrisPriester {
    fn default() -> Self {
        Self::new()
    }
}

impl AtmosphereModel for HarrisPriester {
    fn density(&self, altitude_km: f64, position: &Vector3<f64>, epoch: Option<&Epoch>) -> f64 {
        // Below HP table range: fall back to exponential model
        if altitude_km < HP_TABLE[0].h {
            return crate::exponential::density(altitude_km);
        }

        // Above HP table range: negligible density
        if altitude_km > HP_TABLE[HP_TABLE.len() - 1].h {
            return 0.0;
        }

        let (rho_min, rho_max) = self.interpolate_table(altitude_km);

        // Without epoch, return average (no diurnal info available)
        let epoch = match epoch {
            Some(e) => e,
            None => return (rho_min + rho_max) / 2.0,
        };

        // Compute angle between satellite and bulge apex
        let apex = self.bulge_apex(epoch);
        let sat_dir = position.normalize();
        let cos_psi = sat_dir.dot(&apex).clamp(-1.0, 1.0);

        // cos(ψ/2) = sqrt((1 + cos ψ) / 2)
        let cos_half_psi = ((1.0 + cos_psi) / 2.0).sqrt();

        rho_min + (rho_max - rho_min) * cos_half_psi.powi(self.n as i32)
    }
}

/// Scale-height interpolation between two density values.
///
/// Computes ρ(h) by treating the density as exponential between two reference points.
fn scale_height_interp(h: f64, h_lo: f64, h_hi: f64, rho_lo: f64, rho_hi: f64) -> f64 {
    if rho_lo <= 0.0 || rho_hi <= 0.0 {
        return 0.0;
    }
    let scale_h = (h_hi - h_lo) / (rho_lo / rho_hi).ln();
    rho_lo * (-(h - h_lo) / scale_h).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Helper: create a Harris-Priester model with a fixed sun direction (+X).
    fn hp_fixed_sun() -> HarrisPriester {
        HarrisPriester::new().with_sun_direction_fn(|_| Vector3::new(1.0, 0.0, 0.0))
    }

    fn dummy_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    #[test]
    fn density_at_table_boundary_apex() {
        // At the bulge apex (same direction as sun+lag), should get rho_max
        let hp = hp_fixed_sun().with_lag_angle(0.0); // no lag, apex = +X
        let epoch = dummy_epoch();

        // Satellite at +X direction → at the apex
        let pos = Vector3::new(6778.0, 0.0, 0.0);

        // At 400 km: rho_max = 7.492e-12
        let rho = hp.density(400.0, &pos, Some(&epoch));
        let expected = 7.492e-12;
        let rel_err = (rho - expected).abs() / expected;
        assert!(rel_err < 1e-6, "At apex, 400 km: expected {expected:.3e}, got {rho:.3e}");
    }

    #[test]
    fn density_at_table_boundary_antapex() {
        // Opposite the bulge apex → should get rho_min
        let hp = hp_fixed_sun().with_lag_angle(0.0);
        let epoch = dummy_epoch();

        // Satellite at -X → anti-apex
        let pos = Vector3::new(-6778.0, 0.0, 0.0);

        // At 400 km: rho_min = 2.249e-12
        let rho = hp.density(400.0, &pos, Some(&epoch));
        let expected = 2.249e-12;
        let rel_err = (rho - expected).abs() / expected;
        assert!(rel_err < 1e-6, "At anti-apex, 400 km: expected {expected:.3e}, got {rho:.3e}");
    }

    #[test]
    fn density_decreases_with_altitude() {
        let hp = hp_fixed_sun();
        let epoch = dummy_epoch();
        let pos_dir = Vector3::new(1.0, 0.0, 0.0); // direction doesn't matter, just altitude

        let altitudes = [100.0, 200.0, 300.0, 400.0, 500.0, 700.0, 1000.0];
        for i in 0..altitudes.len() - 1 {
            let pos_lo = pos_dir * (6371.0 + altitudes[i]);
            let pos_hi = pos_dir * (6371.0 + altitudes[i + 1]);
            let rho_lo = hp.density(altitudes[i], &pos_lo, Some(&epoch));
            let rho_hi = hp.density(altitudes[i + 1], &pos_hi, Some(&epoch));
            assert!(
                rho_hi < rho_lo,
                "Density should decrease: ρ({})={:.3e} > ρ({})={:.3e}",
                altitudes[i],
                rho_lo,
                altitudes[i + 1],
                rho_hi
            );
        }
    }

    #[test]
    fn diurnal_variation_apex_greater_than_antapex() {
        let hp = hp_fixed_sun().with_lag_angle(0.0);
        let epoch = dummy_epoch();

        for alt in [200.0, 400.0, 600.0, 800.0] {
            let r = 6371.0 + alt;
            let pos_apex = Vector3::new(r, 0.0, 0.0);
            let pos_anti = Vector3::new(-r, 0.0, 0.0);

            let rho_apex = hp.density(alt, &pos_apex, Some(&epoch));
            let rho_anti = hp.density(alt, &pos_anti, Some(&epoch));

            assert!(
                rho_apex > rho_anti,
                "At {alt} km: apex ({rho_apex:.3e}) should be > anti-apex ({rho_anti:.3e})"
            );
        }
    }

    #[test]
    fn no_epoch_returns_average() {
        let hp = hp_fixed_sun();
        let pos = Vector3::new(6778.0, 0.0, 0.0);

        let rho = hp.density(400.0, &pos, None);

        // Should be (rho_min + rho_max) / 2 at 400 km
        let expected = (2.249e-12 + 7.492e-12) / 2.0;
        let rel_err = (rho - expected).abs() / expected;
        assert!(
            rel_err < 1e-6,
            "Without epoch: expected {expected:.3e}, got {rho:.3e}"
        );
    }

    #[test]
    fn below_100km_falls_back_to_exponential() {
        let hp = hp_fixed_sun();
        let pos = Vector3::new(6371.0 + 50.0, 0.0, 0.0);
        let epoch = dummy_epoch();

        let rho_hp = hp.density(50.0, &pos, Some(&epoch));
        let rho_exp = crate::exponential::density(50.0);

        assert_eq!(rho_hp, rho_exp, "Below 100 km should fall back to exponential");
    }

    #[test]
    fn above_table_returns_zero() {
        let hp = hp_fixed_sun();
        let pos = Vector3::new(6371.0 + 1500.0, 0.0, 0.0);
        let epoch = dummy_epoch();

        let rho = hp.density(1500.0, &pos, Some(&epoch));
        assert_eq!(rho, 0.0, "Above table range should return 0, got {rho:.3e}");
    }

    #[test]
    fn higher_exponent_sharper_bulge() {
        let epoch = dummy_epoch();
        // Satellite at 90° from apex (equator of the bulge)
        let pos_90 = Vector3::new(0.0, 6778.0, 0.0);

        let hp_n2 = hp_fixed_sun().with_lag_angle(0.0).with_exponent(2);
        let hp_n6 = hp_fixed_sun().with_lag_angle(0.0).with_exponent(6);

        let rho_n2 = hp_n2.density(400.0, &pos_90, Some(&epoch));
        let rho_n6 = hp_n6.density(400.0, &pos_90, Some(&epoch));

        // Higher n → density drops faster away from apex → lower density at 90°
        assert!(
            rho_n6 < rho_n2,
            "Higher n should give lower density at 90°: n=2 ({rho_n2:.3e}) > n=6 ({rho_n6:.3e})"
        );
    }

    #[test]
    fn apex_rotated_from_sun() {
        let hp = hp_fixed_sun(); // sun at +X, lag = π/6

        let epoch = dummy_epoch();
        let apex = hp.bulge_apex(&epoch);

        // Sun at +X, rotated +30° about Z (eastward) → apex.x = cos(30°), apex.y = +sin(30°)
        // The density bulge leads the sub-solar point in the direction of Earth's rotation
        let expected_x = (PI / 6.0).cos();
        let expected_y = (PI / 6.0).sin();

        assert!(
            (apex.x - expected_x).abs() < 1e-10,
            "Apex x: expected {expected_x:.4}, got {:.4}",
            apex.x
        );
        assert!(
            (apex.y - expected_y).abs() < 1e-10,
            "Apex y: expected {expected_y:.4}, got {:.4}",
            apex.y
        );
        assert!(apex.z.abs() < 1e-10, "Apex z should be ~0, got {:.4}", apex.z);
    }

    #[test]
    fn interpolation_monotonic() {
        let hp = hp_fixed_sun();

        // Check that interpolated values are between boundary values
        for i in 0..HP_TABLE.len() - 1 {
            let lo = &HP_TABLE[i];
            let hi = &HP_TABLE[i + 1];
            let mid_h = (lo.h + hi.h) / 2.0;

            let (rho_min, rho_max) = hp.interpolate_table(mid_h);

            assert!(
                rho_min >= hi.rho_min && rho_min <= lo.rho_min,
                "rho_min at {mid_h} km should be between {:.3e} and {:.3e}, got {rho_min:.3e}",
                hi.rho_min,
                lo.rho_min
            );
            assert!(
                rho_max >= hi.rho_max && rho_max <= lo.rho_max,
                "rho_max at {mid_h} km should be between {:.3e} and {:.3e}, got {rho_max:.3e}",
                hi.rho_max,
                lo.rho_max
            );
        }
    }

    #[test]
    fn iss_altitude_order_of_magnitude() {
        let hp = hp_fixed_sun();
        let epoch = dummy_epoch();
        let pos = Vector3::new(6778.0, 0.0, 0.0);

        let rho_hp = hp.density(400.0, &pos, Some(&epoch));
        let rho_exp = crate::exponential::density(400.0);

        // HP and Exponential should agree within ~1 order of magnitude at ISS altitude
        let ratio = rho_hp / rho_exp;
        assert!(
            ratio > 0.1 && ratio < 10.0,
            "HP/Exponential ratio at 400 km: {ratio:.2} (HP={rho_hp:.3e}, Exp={rho_exp:.3e})"
        );
    }
}
