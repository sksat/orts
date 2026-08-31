//! Oracle tests for NRLMSISE-00 against pymsis (official NRL Fortran, version=0).
//!
//! Fixture: `tobari/tests/fixtures/nrlmsise00_reference.json`
//! Generator: `tools/generate_nrlmsise00_fixtures.py`
//!
//! pymsis wraps the actual NRL Fortran code, so these are direct comparisons
//! against the official implementation — not a third-party reimplementation.

use serde::Deserialize;
use tobari::nrlmsise00::{ApMode, Nrlmsise00Input};
use tobari::{ConstantWeather, Nrlmsise00};

// Fixture structures

#[derive(Deserialize)]
struct FixtureData {
    #[allow(dead_code)]
    generator: String,
    #[allow(dead_code)]
    oracle: String,
    points: Vec<DensityPoint>,
    #[allow(dead_code)]
    exospheric_temperature_points: Vec<ExoTempPoint>,
    /// Points below the 72.5 km spline floor, daily-Ap mode.
    lower_atmosphere_points: Vec<DensityPoint>,
    /// Points at and above the 72.5 km split, below the thermosphere grid's
    /// 100 km floor.
    middle_atmosphere_points: Vec<DensityPoint>,
    /// Points generated with the reference's 3-hourly Ap switch enabled.
    ap_history_points: Vec<ApHistoryPoint>,
    #[allow(dead_code)]
    summary: serde_json::Value,
}

/// A point generated with the 3-hourly Ap formulation (reference switch 9 = -1).
///
/// `ap_array[0]` is the same in every point, so reproducing these values
/// requires reading the history in `ap_array[1..]`.
#[derive(Deserialize)]
struct ApHistoryPoint {
    epoch_name: String,
    ap_history_name: String,
    ap_array: [f64; 7],
    f107: f64,
    f107a: f64,
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: f64,
    mass_density_kg_m3: Option<f64>,
    temperature_k: Option<f64>,
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
///
/// A non-finite `computed` also yields `f64::MAX`: the difference would be NaN,
/// and every `NaN > tolerance` comparison is false, so a NaN result would slip
/// through every tolerance check in this file unnoticed.
fn rel_error(computed: f64, expected: f64) -> f64 {
    if !computed.is_finite() {
        return f64::MAX;
    }
    if expected == 0.0 {
        if computed == 0.0 { 0.0 } else { f64::MAX }
    } else {
        (computed - expected).abs() / expected.abs()
    }
}

// Total mass density tests

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

// Species density tests

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

// Temperature tests

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

// Solar activity variation tests

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

// Diurnal variation test

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

// Lower atmosphere (below the 72.5 km spline floor)

/// Total mass density and temperature from the surface to just under 72.5 km.
///
/// Below 72.5 km the model leaves the thermosphere formulation and integrates
/// the mesosphere/stratosphere/troposphere temperature splines. Clamping to the
/// 72.5 km profile instead makes the whole band a flat shelf — 6.3e-5 kg/m³ and
/// 212 K at sea level against the oracle's 1.18 kg/m³ and 300 K, so this test
/// fails by four orders of magnitude if the lower-atmosphere path is skipped.
#[test]
fn lower_atmosphere_density_and_temperature() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    assert!(
        !fixture.lower_atmosphere_points.is_empty(),
        "fixture has no lower-atmosphere points"
    );

    let mut max_rho_err = 0.0f64;
    let mut max_temp_err = 0.0f64;
    let mut failures = Vec::new();

    for p in &fixture.lower_atmosphere_points {
        assert!(
            p.altitude_km < 72.5,
            "lower-atmosphere section must stay below the spline floor: {}",
            p.altitude_km
        );

        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);

        let expected_rho = p.mass_density_kg_m3.expect("mass density in fixture");
        let rho_err = rel_error(output.total_mass_density, expected_rho);
        max_rho_err = max_rho_err.max(rho_err);

        let expected_t = p.temperature_k.expect("temperature in fixture");
        let temp_err = rel_error(output.temp_alt, expected_t);
        max_temp_err = max_temp_err.max(temp_err);

        if rho_err > 0.01 || temp_err > 0.01 {
            failures.push(format!(
                "  alt={:.1}km lat={:.0} lon={:.0} {}/{}: rho_err={:.3}% temp_err={:.3}% \
                 (rho got={:.4e} want={:.4e}, T got={:.2} want={:.2})",
                p.altitude_km,
                p.latitude_deg,
                p.longitude_deg,
                p.epoch_name,
                p.activity,
                rho_err * 100.0,
                temp_err * 100.0,
                output.total_mass_density,
                expected_rho,
                output.temp_alt,
                expected_t,
            ));
        }
    }

    println!(
        "Lower atmosphere: {} points, max rho error={:.4}%, max temp error={:.4}%",
        fixture.lower_atmosphere_points.len(),
        max_rho_err * 100.0,
        max_temp_err * 100.0
    );

    if !failures.is_empty() {
        for f in failures.iter().take(20) {
            println!("{f}");
        }
        panic!(
            "{}/{} lower-atmosphere points exceeded 1% tolerance",
            failures.len(),
            fixture.lower_atmosphere_points.len()
        );
    }
}

/// Mixed-species number densities below 72.5 km, and the absence of the
/// diffusive-only species there.
///
/// The reference reports only the fully mixed species (N₂, O₂, Ar, He) below
/// 72.5 km and leaves O, H, N and anomalous O at zero; the fixture stores those
/// as `null`.
#[test]
fn lower_atmosphere_species() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let cm3_to_m3 = 1e6;
    let mut failures = Vec::new();

    for p in &fixture.lower_atmosphere_points {
        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let out = model.calculate(&input);

        for (name, computed, expected) in [
            ("N2", out.density_n2 * cm3_to_m3, p.n2_m3),
            ("O2", out.density_o2 * cm3_to_m3, p.o2_m3),
            ("Ar", out.density_ar * cm3_to_m3, p.ar_m3),
            ("He", out.density_he * cm3_to_m3, p.he_m3),
        ] {
            let expected = expected.expect("mixed species present below 72.5 km");
            let err = rel_error(computed, expected);
            if err > 0.02 {
                failures.push(format!(
                    "  {name} alt={:.1}km lat={:.0}: err={:.3}% (got={computed:.4e}, want={expected:.4e})",
                    p.altitude_km,
                    p.latitude_deg,
                    err * 100.0
                ));
            }
        }

        // O, H, N and anomalous O are not modelled below 72.5 km.
        assert_eq!(p.o_m3, None, "oracle reports O below 72.5 km");
        assert_eq!(p.h_m3, None, "oracle reports H below 72.5 km");
        assert_eq!(p.n_m3, None, "oracle reports N below 72.5 km");
        for (name, v) in [
            ("O", out.density_o),
            ("H", out.density_h),
            ("N", out.density_n),
            ("anomalous O", out.density_anomalous_o),
        ] {
            assert_eq!(v, 0.0, "{name} must be zero below 72.5 km, got {v:.4e}");
        }
    }

    if !failures.is_empty() {
        for f in failures.iter().take(20) {
            println!("{f}");
        }
        panic!(
            "{} lower-atmosphere species points exceeded 2% tolerance",
            failures.len()
        );
    }
}

/// Hydrostatic scale height through the troposphere and stratosphere.
///
/// Needs no oracle: for an atmosphere in hydrostatic equilibrium with a
/// ~200-300 K temperature profile, `-d(ln ρ)/dh` must be near 1/7 km⁻¹. A model
/// that returns a constant density over this band gives ~0 instead.
#[test]
fn hydrostatic_scale_height_below_the_thermosphere() {
    let model = Nrlmsise00::new(Box::new(ConstantWeather::solar_moderate()));

    let rho = |alt: f64| {
        model
            .calculate(&Nrlmsise00Input {
                day_of_year: 80,
                ut_seconds: 43200.0,
                altitude_km: alt,
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                local_solar_time_hours: 12.0,
                f107_daily: 150.0,
                f107_avg: 150.0,
                ap_daily: 15.0,
                ap_array: [15.0; 7],
            })
            .total_mass_density
    };

    let mut alt = 0.0;
    while alt < 70.0 {
        let (lo, hi) = (rho(alt), rho(alt + 2.0));
        assert!(lo > hi, "density must fall with altitude at {alt} km");
        let inv_scale_height = (lo / hi).ln() / 2.0;
        assert!(
            (0.08..0.30).contains(&inv_scale_height),
            "-d(ln rho)/dh = {inv_scale_height:.4} km^-1 over {alt}-{} km \
             is outside the hydrostatic range 0.08-0.30",
            alt + 2.0
        );
        alt += 2.0;
    }
}

/// Cross-check against the crate's own US Standard Atmosphere table.
///
/// The two models are independent implementations of the same mean atmosphere,
/// so below the thermosphere they must agree to within a factor of two. This
/// mirrors the fallback that `HarrisPriester` uses below its own table floor.
#[test]
fn agrees_with_exponential_model_below_the_thermosphere() {
    let model = Nrlmsise00::new(Box::new(ConstantWeather::solar_moderate()));

    for alt in [0.0, 10.0, 20.0, 30.0, 50.0, 70.0, 72.5, 90.0] {
        let msis = model
            .calculate(&Nrlmsise00Input {
                day_of_year: 80,
                ut_seconds: 43200.0,
                altitude_km: alt,
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                local_solar_time_hours: 12.0,
                f107_daily: 150.0,
                f107_avg: 150.0,
                ap_daily: 15.0,
                ap_array: [15.0; 7],
            })
            .total_mass_density;
        let us76 = tobari::exponential::density(alt);
        let ratio = msis / us76;
        assert!(
            (0.5..2.0).contains(&ratio),
            "NRLMSISE-00 / US76 = {ratio:.4e} at {alt} km (msis={msis:.4e}, us76={us76:.4e})"
        );
    }
}

/// Every output is finite at and around every temperature-spline node.
///
/// Needs no oracle. The lower atmosphere is built from two splines whose node
/// altitudes (72.5, 55, 45, 32.5, 20, 15, 10, 0 km) are exactly the altitudes
/// where an interpolation weight or a segment integral collapses to zero, so
/// they are where an unpopulated node or a degenerate interval shows up. A NaN
/// here is invisible to a relative-error tolerance, since every comparison
/// against NaN is false.
#[test]
fn output_is_finite_at_every_spline_node() {
    let model = Nrlmsise00::new(Box::new(ConstantWeather::solar_moderate()));

    let mut altitudes: Vec<f64> = Vec::new();
    for node in [72.5, 62.5, 55.0, 45.0, 32.5, 20.0, 15.0, 10.0, 0.0] {
        for delta in [-1e-3, -1e-9, 0.0, 1e-9, 1e-3] {
            altitudes.push(node + delta);
        }
    }
    // Plus a coarse sweep so nothing between the nodes is missed.
    let mut alt = 0.0;
    while alt <= 120.0 {
        altitudes.push(alt);
        alt += 0.25;
    }

    for alt in altitudes {
        for latitude_deg in [-85.0, -45.0, 0.0, 45.0, 85.0] {
            for day_of_year in [1u32, 80, 173, 356] {
                let out = model.calculate(&Nrlmsise00Input {
                    day_of_year,
                    ut_seconds: 43200.0,
                    altitude_km: alt,
                    latitude_deg,
                    longitude_deg: 30.0,
                    local_solar_time_hours: 14.0,
                    f107_daily: 150.0,
                    f107_avg: 150.0,
                    ap_daily: 15.0,
                    ap_array: [15.0; 7],
                });
                for (name, v) in [
                    ("total_mass_density", out.total_mass_density),
                    ("temp_alt", out.temp_alt),
                    ("temp_exo", out.temp_exo),
                    ("n2", out.density_n2),
                    ("o2", out.density_o2),
                    ("ar", out.density_ar),
                    ("he", out.density_he),
                    ("o", out.density_o),
                    ("h", out.density_h),
                    ("n", out.density_n),
                    ("anomalous_o", out.density_anomalous_o),
                ] {
                    assert!(
                        v.is_finite(),
                        "{name} = {v} at alt={alt} lat={latitude_deg} doy={day_of_year}"
                    );
                }
                assert!(
                    out.total_mass_density > 0.0,
                    "density {} at alt={alt} lat={latitude_deg} doy={day_of_year}",
                    out.total_mass_density
                );
            }
        }
    }
}

// 3-hourly Ap mode

/// Density and temperature in the 3-hourly Ap formulation.
///
/// The oracle points hold the daily Ap fixed at 4.0 and vary only the history,
/// so a model that reads `ap_daily` here — or ignores the history — cannot
/// reproduce them.
#[test]
fn ap_history_mode_matches_oracle() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)))
        .with_ap_mode(ApMode::ThreeHourly);

    assert!(
        !fixture.ap_history_points.is_empty(),
        "fixture has no 3-hourly Ap points"
    );

    let mut max_rho_err = 0.0f64;
    let mut max_temp_err = 0.0f64;
    let mut failures = Vec::new();

    for p in &fixture.ap_history_points {
        assert_eq!(
            p.ap_array[0], 4.0,
            "daily Ap must be held fixed across the 3-hourly section"
        );

        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let lst = ((ut_seconds / 3600.0 + p.longitude_deg / 15.0) % 24.0 + 24.0) % 24.0;
        let input = Nrlmsise00Input {
            day_of_year: doy,
            ut_seconds,
            altitude_km: p.altitude_km,
            latitude_deg: p.latitude_deg,
            longitude_deg: p.longitude_deg,
            local_solar_time_hours: lst,
            f107_daily: p.f107,
            f107_avg: p.f107a,
            // Deliberately absurd: this mode must not read it.
            ap_daily: 0.0,
            ap_array: p.ap_array,
        };
        let out = model.calculate(&input);

        let expected_rho = p.mass_density_kg_m3.expect("mass density in fixture");
        let rho_err = rel_error(out.total_mass_density, expected_rho);
        max_rho_err = max_rho_err.max(rho_err);

        let expected_t = p.temperature_k.expect("temperature in fixture");
        let temp_err = rel_error(out.temp_alt, expected_t);
        max_temp_err = max_temp_err.max(temp_err);

        if rho_err > 0.01 || temp_err > 0.01 {
            failures.push(format!(
                "  alt={:.1}km lat={:.0} {}/{}: rho_err={:.3}% temp_err={:.3}% \
                 (rho got={:.4e} want={:.4e})",
                p.altitude_km,
                p.latitude_deg,
                p.epoch_name,
                p.ap_history_name,
                rho_err * 100.0,
                temp_err * 100.0,
                out.total_mass_density,
                expected_rho,
            ));
        }
    }

    println!(
        "3-hourly Ap: {} points, max rho error={:.4}%, max temp error={:.4}%",
        fixture.ap_history_points.len(),
        max_rho_err * 100.0,
        max_temp_err * 100.0
    );

    if !failures.is_empty() {
        for f in failures.iter().take(20) {
            println!("{f}");
        }
        panic!(
            "{}/{} 3-hourly Ap points exceeded 1% tolerance",
            failures.len(),
            fixture.ap_history_points.len()
        );
    }
}

/// Each mode reads exactly the geomagnetic input it documents.
///
/// Needs no oracle. In `ThreeHourly` the history must move the density and
/// `ap_daily` must not; in `Daily` it is the other way round.
#[test]
fn each_ap_mode_reads_only_its_own_input() {
    let make = |ap_daily: f64, ap_array: [f64; 7]| Nrlmsise00Input {
        day_of_year: 80,
        ut_seconds: 43200.0,
        altitude_km: 400.0,
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        local_solar_time_hours: 12.0,
        f107_daily: 150.0,
        f107_avg: 150.0,
        ap_daily,
        ap_array,
    };
    let quiet = [4.0; 7];
    let storm = [4.0, 400.0, 400.0, 400.0, 400.0, 400.0, 400.0];

    let three_hourly = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)))
        .with_ap_mode(ApMode::ThreeHourly);
    let rho_quiet = three_hourly.calculate(&make(4.0, quiet)).total_mass_density;
    let rho_storm = three_hourly.calculate(&make(4.0, storm)).total_mass_density;
    let change = (rho_storm - rho_quiet) / rho_quiet;
    assert!(
        change > 0.05,
        "3-hourly mode must respond to the Ap history: quiet={rho_quiet:.4e}, \
         storm={rho_storm:.4e} ({:.2}%)",
        change * 100.0
    );
    assert_eq!(
        three_hourly
            .calculate(&make(400.0, quiet))
            .total_mass_density,
        rho_quiet,
        "3-hourly mode must not read ap_daily"
    );

    let daily = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));
    let daily_quiet = daily.calculate(&make(4.0, quiet)).total_mass_density;
    assert_eq!(
        daily.calculate(&make(4.0, storm)).total_mass_density,
        daily_quiet,
        "daily mode must not read the Ap history"
    );
    assert!(
        daily.calculate(&make(400.0, quiet)).total_mass_density > daily_quiet * 1.05,
        "daily mode must respond to ap_daily"
    );
}

/// Density and temperature across the 72.5 km split, where the two
/// formulations meet.
///
/// The lower band stops at 72.4 km and the thermosphere grid starts at 100 km,
/// so this stretch had nothing to compare against. It is also where the
/// temperature splines are anchored on a `gts7` evaluation at 72.5 km: a wrong
/// branch, or a spline anchored on the wrong value, still passes a finiteness
/// check and a US76 sanity band.
#[test]
fn middle_atmosphere_density_and_temperature() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    assert!(
        !fixture.middle_atmosphere_points.is_empty(),
        "fixture has no middle-atmosphere points"
    );

    let mut max_rho_err = 0.0f64;
    let mut max_temp_err = 0.0f64;
    let mut failures = Vec::new();
    let mut saw_split = false;

    for p in &fixture.middle_atmosphere_points {
        assert!(
            p.altitude_km >= 72.5 && p.altitude_km < 100.0,
            "middle-atmosphere section spans [72.5, 100) km: {}",
            p.altitude_km
        );
        saw_split |= (p.altitude_km - 72.5).abs() < 1e-9;

        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let output = model.calculate(&input);

        let expected_rho = p.mass_density_kg_m3.expect("mass density in fixture");
        let rho_err = rel_error(output.total_mass_density, expected_rho);
        max_rho_err = max_rho_err.max(rho_err);

        let expected_t = p.temperature_k.expect("temperature in fixture");
        let temp_err = rel_error(output.temp_alt, expected_t);
        max_temp_err = max_temp_err.max(temp_err);

        if rho_err > 0.01 || temp_err > 0.01 {
            failures.push(format!(
                "  alt={:.1}km lat={:.0} lon={:.0} {}/{}: rho_err={:.3}% temp_err={:.3}% \
                 (rho got={:.4e} want={:.4e}, T got={:.2} want={:.2})",
                p.altitude_km,
                p.latitude_deg,
                p.longitude_deg,
                p.epoch_name,
                p.activity,
                rho_err * 100.0,
                temp_err * 100.0,
                output.total_mass_density,
                expected_rho,
                output.temp_alt,
                expected_t,
            ));
        }
    }

    assert!(
        saw_split,
        "the split itself (72.5 km) has to be among the sampled altitudes"
    );

    println!(
        "Middle atmosphere: {} points, max rho error={:.4}%, max temp error={:.4}%",
        fixture.middle_atmosphere_points.len(),
        max_rho_err * 100.0,
        max_temp_err * 100.0
    );

    if !failures.is_empty() {
        for f in failures.iter().take(20) {
            println!("{f}");
        }
        panic!("{} middle-atmosphere points exceed 1%", failures.len());
    }
}

/// The species the reference reports at and above the split.
///
/// Below 72.5 km only the fully mixed species are reported; from 72.5 km up the
/// reference also returns O, H, N and anomalous O. Measured with pymsis 0.12.0:
/// those four are absent at 72.4 km and present at 72.5 km. The lower-band test
/// pins the absence, and this one pins the presence, so the split cannot be
/// off by a step without one of them failing.
#[test]
fn middle_atmosphere_species() {
    let fixture = load_fixture();
    let model = Nrlmsise00::new(Box::new(ConstantWeather::new(150.0, 15.0)));

    let cm3_to_m3 = 1e6;
    let mut failures = Vec::new();

    for p in &fixture.middle_atmosphere_points {
        let (doy, ut_seconds) = epoch_params(&p.epoch_name);
        let input = make_input(p, doy, ut_seconds);
        let out = model.calculate(&input);

        for (name, computed, expected) in [
            ("N2", out.density_n2 * cm3_to_m3, p.n2_m3),
            ("O2", out.density_o2 * cm3_to_m3, p.o2_m3),
            ("Ar", out.density_ar * cm3_to_m3, p.ar_m3),
            ("He", out.density_he * cm3_to_m3, p.he_m3),
            ("O", out.density_o * cm3_to_m3, p.o_m3),
            ("H", out.density_h * cm3_to_m3, p.h_m3),
            ("N", out.density_n * cm3_to_m3, p.n_m3),
        ] {
            let expected = expected.expect("the reference reports this species above the split");
            let err = rel_error(computed, expected);
            if err > 0.02 {
                failures.push(format!(
                    "  {name} alt={:.1}km lat={:.0}: err={:.3}% (got={computed:.4e}, want={expected:.4e})",
                    p.altitude_km,
                    p.latitude_deg,
                    err * 100.0
                ));
            }
        }
    }

    if !failures.is_empty() {
        for f in failures.iter().take(20) {
            println!("{f}");
        }
        panic!("{} middle-atmosphere species exceed 2%", failures.len());
    }
}
