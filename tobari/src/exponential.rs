//! Piecewise exponential atmosphere density model.
//!
//! Based on US Standard Atmosphere 1976 reference values.
//! Returns atmospheric density [kg/m³] at a given altitude [km].

use arika::epoch::Epoch;

use crate::AtmosphereModel;

/// Atmosphere layer: base altitude, base density, and scale height.
struct Layer {
    h_base: f64,       // km
    rho_base: f64,     // kg/m³
    scale_height: f64, // km
}

const LAYERS: &[Layer] = &[
    Layer {
        h_base: 0.0,
        rho_base: 1.225,
        scale_height: 7.249,
    },
    Layer {
        h_base: 100.0,
        rho_base: 5.297e-7,
        scale_height: 5.877,
    },
    Layer {
        h_base: 150.0,
        rho_base: 1.454e-9,
        scale_height: 8.382,
    },
    Layer {
        h_base: 200.0,
        rho_base: 2.789e-10,
        scale_height: 37.105,
    },
    Layer {
        h_base: 300.0,
        rho_base: 1.916e-11,
        scale_height: 40.590,
    },
    Layer {
        h_base: 400.0,
        rho_base: 3.725e-12,
        scale_height: 58.515,
    },
    Layer {
        h_base: 500.0,
        rho_base: 6.967e-13,
        scale_height: 73.700,
    },
    Layer {
        h_base: 600.0,
        rho_base: 1.454e-13,
        scale_height: 88.667,
    },
    Layer {
        h_base: 700.0,
        rho_base: 3.614e-14,
        scale_height: 124.64,
    },
    Layer {
        h_base: 800.0,
        rho_base: 1.170e-14,
        scale_height: 181.05,
    },
    Layer {
        h_base: 900.0,
        rho_base: 5.245e-15,
        scale_height: 268.00,
    },
    Layer {
        h_base: 1000.0,
        rho_base: 3.019e-15,
        scale_height: 408.88,
    },
];

/// Compute atmospheric density [kg/m³] at the given altitude [km].
///
/// Uses a piecewise exponential model: ρ(h) = ρ_base * exp(-(h - h_base) / H).
/// Returns 0.0 for altitudes below 0 km or above ~2500 km (where density is negligible).
pub fn density(altitude_km: f64) -> f64 {
    if altitude_km < 0.0 {
        return 0.0;
    }

    // Find the appropriate layer (last layer whose h_base <= altitude)
    let layer = LAYERS.iter().rev().find(|l| altitude_km >= l.h_base);

    match layer {
        Some(l) => {
            let rho = l.rho_base * (-(altitude_km - l.h_base) / l.scale_height).exp();
            // Cut off at negligible density (below ~1e-16 kg/m³, drag is insignificant)
            if rho < 1e-16 { 0.0 } else { rho }
        }
        None => 0.0,
    }
}

/// Piecewise exponential atmosphere model based on US Standard Atmosphere 1976.
///
/// This is an altitude-only model with no time or position dependence.
pub struct Exponential;

impl AtmosphereModel for Exponential {
    fn density(
        &self,
        altitude_km: f64,
        _position_eci: &arika::SimpleEci,
        _epoch: Option<&Epoch<arika::epoch::Utc>>,
    ) -> f64 {
        density(altitude_km)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_level_density() {
        let rho = density(0.0);
        assert!(
            (rho - 1.225).abs() < 0.001,
            "Sea level density should be ~1.225 kg/m³, got {rho}"
        );
    }

    #[test]
    fn density_at_layer_boundaries() {
        // At each layer boundary, density should match the reference value
        for layer in LAYERS {
            let rho = density(layer.h_base);
            let rel_err = (rho - layer.rho_base).abs() / layer.rho_base;
            assert!(
                rel_err < 1e-10,
                "Density at {} km: expected {:.6e}, got {:.6e}",
                layer.h_base,
                layer.rho_base,
                rho
            );
        }
    }

    #[test]
    fn density_decreases_with_altitude() {
        let altitudes = [0.0, 100.0, 200.0, 400.0, 600.0, 800.0, 1000.0];
        for i in 0..altitudes.len() - 1 {
            let rho_low = density(altitudes[i]);
            let rho_high = density(altitudes[i + 1]);
            assert!(
                rho_high < rho_low,
                "Density should decrease: ρ({})={:.6e} > ρ({})={:.6e}",
                altitudes[i],
                rho_low,
                altitudes[i + 1],
                rho_high
            );
        }
    }

    #[test]
    fn iss_altitude_density() {
        // ISS at ~400 km: density should be ~3.7e-12 kg/m³
        let rho = density(400.0);
        assert!(
            (rho - 3.725e-12).abs() / 3.725e-12 < 1e-10,
            "ISS altitude density: expected ~3.725e-12, got {rho:.6e}"
        );
    }

    #[test]
    fn density_at_midlayer() {
        // At 450 km (between 400 and 500 layer), should be between the two
        let rho = density(450.0);
        assert!(rho < density(400.0), "450 km density should be < 400 km");
        assert!(rho > density(500.0), "450 km density should be > 500 km");
    }

    #[test]
    fn very_high_altitude_zero() {
        // Above ~2500 km, density should be effectively zero
        let rho = density(3000.0);
        assert_eq!(rho, 0.0, "Density at 3000 km should be zero, got {rho:.6e}");
    }

    #[test]
    fn negative_altitude_zero() {
        assert_eq!(density(-1.0), 0.0);
    }

    #[test]
    fn density_orders_of_magnitude() {
        // Rough sanity checks on density ranges
        assert!(density(200.0) > 1e-11 && density(200.0) < 1e-9);
        assert!(density(400.0) > 1e-13 && density(400.0) < 1e-11);
        assert!(density(600.0) > 1e-14 && density(600.0) < 1e-12);
        assert!(density(800.0) > 1e-16 && density(800.0) < 1e-13);
    }

    #[test]
    fn trait_ignores_position_and_epoch() {
        let model = Exponential;
        let pos = arika::SimpleEci::new(6778.0, 0.0, 0.0);
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

        let rho_trait = model.density(400.0, &pos, Some(&epoch));
        let rho_free = density(400.0);
        assert_eq!(
            rho_trait, rho_free,
            "Trait should delegate to free function"
        );
    }
}
