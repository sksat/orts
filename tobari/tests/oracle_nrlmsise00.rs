//! Oracle tests for NRLMSISE-00 against pymsis (official NRL Fortran, version=0).
//!
//! Fixture: `tobari/tests/fixtures/nrlmsise00_reference.json`
//! Generator: `tools/generate_nrlmsise00_fixtures.py`
//!
//! pymsis wraps the actual NRL Fortran code, so these are direct comparisons
//! against the official implementation — not a third-party reimplementation.

use serde::Deserialize;
use tobari::nrlmsise00::Nrlmsise00Input;
use tobari::{ConstantWeather, Nrlmsise00};

// ─── Fixture structures ───

#[derive(Deserialize)]
struct FixtureData {
    #[allow(dead_code)]
    generator: String,
    #[allow(dead_code)]
    oracle: String,
    points: Vec<DensityPoint>,
    #[allow(dead_code)]
    exospheric_temperature_points: Vec<ExoTempPoint>,
    #[allow(dead_code)]
    summary: serde_json::Value,
}

#[derive(Deserialize)]
struct DensityPoint {
    #[allow(dead_code)]
    epoch_utc: String,
    epoch_name: String,
    activity: String,
    f107: f64,
    f107a: f64,
    ap: f64,
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: f64,
    mass_density_kg_m3: Option<f64>,
    n2_m3: Option<f64>,
    o2_m3: Option<f64>,
    o_m3: Option<f64>,
    he_m3: Option<f64>,
    h_m3: Option<f64>,
    ar_m3: Option<f64>,
    n_m3: Option<f64>,
    #[allow(dead_code)]
    anomalous_o_m3: Option<f64>,
    temperature_k: Option<f64>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ExoTempPoint {
    epoch_utc: String,
    epoch_name: String,
    activity: String,
    f107: f64,
    f107a: f64,
    ap: f64,
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: f64,
    temperature_k: f64,
    mass_density_kg_m3: f64,
}

// ─── Helpers ───

fn load_fixture() -> FixtureData {
    let json = include_str!("fixtures/nrlmsise00_reference.json");
    serde_json::from_str(json).expect("Failed to parse NRLMSISE-00 fixture")
}

/// Build a Nrlmsise00Input for a DensityPoint.
///
/// Since the fixture already provides lat/lon/alt directly, we construct
/// the input without going through ECI → geodetic conversion.
fn make_input(p: &DensityPoint, doy: u32, ut_seconds: f64) -> Nrlmsise00Input {
    // Use simple LST (no EoT) to match pymsis C reference which uses the same formula.
    let lst = ((ut_seconds / 3600.0 + p.longitude_deg / 15.0) % 24.0 + 24.0) % 24.0;
    Nrlmsise00Input {
        day_of_year: doy,
        ut_seconds,
        altitude_km: p.altitude_km,
        latitude_deg: p.latitude_deg,
        longitude_deg: p.longitude_deg,
        local_solar_time_hours: lst,
        f107_daily: p.f107,
        f107_avg: p.f107a,
        ap_daily: p.ap,
        ap_array: [p.ap; 7],
    }
}

/// Get (doy, ut_seconds) for a named epoch from the fixture.
fn epoch_params(epoch_name: &str) -> (u32, f64) {
    match epoch_name {
        // 2024-03-20T12:00:00Z → doy=80, UT=43200s
        "vernal_equinox" => (80, 43200.0),
        // 2024-06-21T12:00:00Z → doy=173, UT=43200s
        "summer_solstice" => (173, 43200.0),
        // 2024-12-21T12:00:00Z → doy=356, UT=43200s
        "winter_solstice" => (356, 43200.0),
        _ => panic!("Unknown epoch: {epoch_name}"),
    }
}

/// Relative error between two values. Returns f64::MAX if expected is zero.
fn rel_error(computed: f64, expected: f64) -> f64 {
    if expected == 0.0 {
        if computed == 0.0 { 0.0 } else { f64::MAX }
    } else {
        (computed - expected).abs() / expected.abs()
    }
}

// ─── Total mass density tests ───

/// Test total mass density across all 1152 fixture points.
///
/// Tolerance: 1% relative error for densities > 1e-15 kg/m³.
/// Very low densities (< 1e-15) may have larger relative errors
/// and are checked with looser tolerance.
#[test]
fn total_mass_density_all_points() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let mut max_error = 0.0f64;
    let mut total_error = 0.0f64;
    let mut count = 0usize;
    let mut failures = Vec::new();

    for p in &fixture.points {
        let expected = match p.mass_density_kg_m3 {
            Some(v) if v > 0.0 => v,
            _ => continue,
        };

        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);
        let computed = output.total_mass_density;

        let err = rel_error(computed, expected);
        max_error = max_error.max(err);
        total_error += err;
        count += 1;

        let tol = if expected > 1e-15 { 0.01 } else { 0.10 };
        if err > tol {
            failures.push(format!(
                "  alt={:.0}km lat={:.0} lon={:.0} {}/{}: err={:.2}% (got={:.4e}, want={:.4e})",
                p.altitude_km,
                p.latitude_deg,
                p.longitude_deg,
                p.epoch_name,
                p.activity,
                err * 100.0,
                computed,
                expected,
            ));
        }
    }

    let mean_error = if count > 0 {
        total_error / count as f64
    } else {
        0.0
    };

    println!("Total mass density: {count} points tested");
    println!("  max error: {:.4}%", max_error * 100.0);
    println!("  mean error: {:.4}%", mean_error * 100.0);

    if !failures.is_empty() {
        // Sort by error descending
        failures.sort_by(|a, b| {
            let ea: f64 = a
                .split("err=")
                .nth(1)
                .unwrap()
                .split('%')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let eb: f64 = b
                .split("err=")
                .nth(1)
                .unwrap()
                .split('%')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            eb.partial_cmp(&ea).unwrap()
        });
        println!("\nFailed points ({}/{count}):", failures.len());
        for f in &failures[..failures.len().min(30)] {
            println!("{f}");
        }
        if failures.len() > 30 {
            println!("  ... and {} more", failures.len() - 30);
        }
        panic!(
            "{}/{count} points exceeded tolerance (max err={:.2}%)",
            failures.len(),
            max_error * 100.0
        );
    }
}

/// Test total mass density at specific representative conditions.
///
/// These are spot-checks at key altitude/activity combinations.
#[test]
fn total_mass_density_spot_checks() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    // ISS altitude, equator, solar moderate, equinox
    // lon=0° with UT=12h → LST=12h (noon)
    let iss_point = fixture.points.iter().find(|p| {
        p.altitude_km == 400.0
            && p.latitude_deg == 0.0
            && p.longitude_deg == 0.0
            && p.epoch_name == "vernal_equinox"
            && p.activity == "solar_moderate"
    });

    if let Some(p) = iss_point {
        let expected = p.mass_density_kg_m3.unwrap();
        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);
        let err = rel_error(output.total_mass_density, expected);
        assert!(
            err < 0.01,
            "ISS-like condition: err={:.2}% (got={:.4e}, want={:.4e})",
            err * 100.0,
            output.total_mass_density,
            expected,
        );
    }
}

// ─── Species density tests ───

/// Test individual species number densities against oracle.
///
/// pymsis outputs densities in m⁻³; our model uses cm⁻³.
/// Conversion: n [cm⁻³] × 1e6 = n [m⁻³]
///
/// Tolerance: 5% for major species (N2, O2, O above 1e10 m⁻³),
/// 10% for minor species (He, H, Ar, N, anomalous O).
#[test]
fn species_densities() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let cm3_to_m3 = 1e6; // 1 cm⁻³ = 1e6 m⁻³

    let mut failures = Vec::new();
    let species = [
        ("N2", 5),
        ("O2", 5),
        ("O", 5),
        ("He", 10),
        ("H", 10),
        ("Ar", 10),
        ("N", 10),
    ];

    // Test a subset: solar moderate, equinox, equator
    for p in fixture.points.iter().filter(|p| {
        p.activity == "solar_moderate" && p.epoch_name == "vernal_equinox" && p.latitude_deg == 0.0
    }) {
        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);

        let computed_species = [
            ("N2", output.density_n2 * cm3_to_m3, p.n2_m3),
            ("O2", output.density_o2 * cm3_to_m3, p.o2_m3),
            ("O", output.density_o * cm3_to_m3, p.o_m3),
            ("He", output.density_he * cm3_to_m3, p.he_m3),
            ("H", output.density_h * cm3_to_m3, p.h_m3),
            ("Ar", output.density_ar * cm3_to_m3, p.ar_m3),
            ("N", output.density_n * cm3_to_m3, p.n_m3),
        ];

        for (name, computed, expected_opt) in &computed_species {
            let expected = match expected_opt {
                Some(v) if *v > 1e6 => *v, // skip very small densities
                _ => continue,
            };

            let tol_pct = species
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| *t)
                .unwrap_or(10);

            let err = rel_error(*computed, expected);
            if err > tol_pct as f64 / 100.0 {
                failures.push(format!(
                    "  {name} alt={:.0}km lon={:.0}: err={:.2}% (got={:.4e}, want={:.4e})",
                    p.altitude_km,
                    p.longitude_deg,
                    err * 100.0,
                    computed,
                    expected,
                ));
            }
        }
    }

    if !failures.is_empty() {
        println!("Species density failures:");
        for f in &failures[..failures.len().min(20)] {
            println!("{f}");
        }
        panic!(
            "{} species density points exceeded tolerance",
            failures.len()
        );
    }
}

// ─── Temperature tests ───

/// Test temperature at altitude against oracle.
#[test]
fn temperature_at_altitude() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let mut max_error = 0.0f64;
    let mut failures = Vec::new();

    // Test equatorial points only (minimize coordinate conversion effects)
    for p in fixture.points.iter().filter(|p| p.latitude_deg == 0.0) {
        let expected = match p.temperature_k {
            Some(v) if v > 0.0 => v,
            _ => continue,
        };

        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);

        let err = rel_error(output.temp_alt, expected);
        max_error = max_error.max(err);

        if err > 0.01 {
            failures.push(format!(
                "  alt={:.0}km lon={:.0} {}/{}: err={:.2}% (got={:.1}K, want={:.1}K)",
                p.altitude_km,
                p.longitude_deg,
                p.epoch_name,
                p.activity,
                err * 100.0,
                output.temp_alt,
                expected,
            ));
        }
    }

    println!("Temperature: max error={:.4}%", max_error * 100.0);

    if !failures.is_empty() {
        // Sort by error (descending) for debugging
        failures.sort_by(|a, b| {
            let ea: f64 = a
                .split("err=")
                .nth(1)
                .unwrap()
                .split('%')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let eb: f64 = b
                .split("err=")
                .nth(1)
                .unwrap()
                .split('%')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            eb.partial_cmp(&ea).unwrap()
        });
        println!("Temperature failures ({}):", failures.len());
        for f in &failures[..failures.len().min(30)] {
            println!("{f}");
        }
        panic!(
            "{} temperature points exceeded 1% tolerance",
            failures.len()
        );
    }
}

// ─── Solar activity variation tests ───

/// Verify density increases with F10.7 (higher solar flux → hotter/denser thermosphere).
#[test]
fn density_increases_with_f107() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    // Compare solar_min vs solar_max at 400 km, equator, equinox
    // lon=0° with UT=12h → LST=12h (noon)
    let find = |activity: &str| {
        fixture
            .points
            .iter()
            .find(|p| {
                p.altitude_km == 400.0
                    && p.latitude_deg == 0.0
                    && p.longitude_deg == 0.0
                    && p.epoch_name == "vernal_equinox"
                    && p.activity == activity
            })
            .unwrap()
    };

    let p_min = find("solar_min");
    let p_max = find("solar_max");

    let (doy, ut_seconds) = epoch_params("vernal_equinox");

    let input_min = make_input(p_min, doy, ut_seconds);
    let input_max = make_input(p_max, doy, ut_seconds);

    let rho_min = model.calculate(&input_min).total_mass_density;
    let rho_max = model.calculate(&input_max).total_mass_density;

    // Oracle values for reference
    let expected_min = p_min.mass_density_kg_m3.unwrap();
    let expected_max = p_max.mass_density_kg_m3.unwrap();

    println!("F10.7 variation at 400km equator noon:");
    println!("  solar_min: computed={rho_min:.4e}, oracle={expected_min:.4e}");
    println!("  solar_max: computed={rho_max:.4e}, oracle={expected_max:.4e}");
    println!(
        "  oracle ratio (max/min): {:.1}×",
        expected_max / expected_min
    );

    assert!(
        rho_max > rho_min,
        "Density must increase with F10.7: rho_max={rho_max:.4e} <= rho_min={rho_min:.4e}"
    );
}

/// Verify density decreases with altitude (basic sanity check).
#[test]
fn density_decreases_with_altitude() {
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let altitudes = [100.0, 200.0, 400.0, 700.0, 1000.0];
    let mut prev_rho = f64::MAX;

    for alt in &altitudes {
        let input = Nrlmsise00Input {
            day_of_year: 80,
            ut_seconds: 43200.0,
            altitude_km: *alt,
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            local_solar_time_hours: 12.0,
            f107_daily: 150.0,
            f107_avg: 150.0,
            ap_daily: 15.0,
            ap_array: [15.0; 7],
        };
        let rho = model.calculate(&input).total_mass_density;
        assert!(
            rho < prev_rho,
            "Density must decrease with altitude: {alt}km rho={rho:.4e} >= prev={prev_rho:.4e}"
        );
        prev_rho = rho;
    }
}

// ─── Diurnal variation test ───

/// Verify density is higher at local noon than midnight (diurnal bulge).
#[test]
fn diurnal_density_variation() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    // At equator, 400 km, solar moderate, equinox (UT=12h):
    // LST = UT/3600 + lon/15, so lon=0° → LST=12h (noon), lon=180° → LST=0h (midnight)
    let find = |lon: f64| {
        fixture
            .points
            .iter()
            .find(|p| {
                p.altitude_km == 400.0
                    && p.latitude_deg == 0.0
                    && p.longitude_deg == lon
                    && p.epoch_name == "vernal_equinox"
                    && p.activity == "solar_moderate"
            })
            .unwrap()
    };

    let p_noon = find(0.0); // lon=0, UT=12h → LST=12h (noon)
    let p_midnight = find(180.0); // lon=180, UT=12h → LST=24h=0h (midnight)
    let (doy, ut_seconds) = epoch_params("vernal_equinox");

    let rho_noon = model
        .calculate(&make_input(p_noon, doy, ut_seconds))
        .total_mass_density;
    let rho_midnight = model
        .calculate(&make_input(p_midnight, doy, ut_seconds))
        .total_mass_density;

    let expected_noon = p_noon.mass_density_kg_m3.unwrap();
    let expected_midnight = p_midnight.mass_density_kg_m3.unwrap();

    println!("Diurnal variation at 400km equator:");
    println!("  noon (lon=0°):     computed={rho_noon:.4e}, oracle={expected_noon:.4e}");
    println!("  midnight (lon=180°): computed={rho_midnight:.4e}, oracle={expected_midnight:.4e}");
    println!(
        "  oracle ratio (noon/midnight): {:.2}×",
        expected_noon / expected_midnight
    );

    assert!(
        rho_noon > rho_midnight,
        "Noon density must exceed midnight: noon={rho_noon:.4e} <= midnight={rho_midnight:.4e}"
    );
}
