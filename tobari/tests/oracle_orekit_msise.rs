//! Density-level cross-validation of NRLMSISE-00 against Orekit.
//!
//! Compares total mass density at sampled (lat, lon, alt, epoch) points with
//! constant solar activity. This isolates atmosphere model differences (coordinate
//! conversions, LST approximation) from integration/force-model differences.
//!
//! Fixture: `tobari/tests/fixtures/orekit_msise_density_reference.json`
//! Generator: `tools/generate_orekit_msise_density_fixtures.py`
//!
//! Known differences:
//!   - LST: Orekit uses precise solar time; Rust uses Meeus EoT correction (residual < 1 min).
//!   - Coordinates: both use WGS-84 geodetic (after geo.rs fix).

use arika::epoch::Epoch;
use serde::Deserialize;
use tobari::{ConstantWeather, CssiData, CssiSpaceWeather, Nrlmsise00};

// ─── Fixture structures ───

#[derive(Deserialize)]
struct FixtureData {
    #[allow(dead_code)]
    generator: String,
    #[allow(dead_code)]
    note: String,
    #[allow(dead_code)]
    known_differences: Vec<String>,
    points: Vec<DensityPoint>,
}

#[derive(Deserialize)]
struct DensityPoint {
    epoch_utc: String,
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: f64,
    f107: f64,
    ap: f64,
    #[allow(dead_code)]
    weather_label: String,
    density_kg_m3: f64,
}

// ─── Helpers ───

fn load_fixture() -> FixtureData {
    let json = include_str!("fixtures/orekit_msise_density_reference.json");
    serde_json::from_str(json).expect("Failed to parse Orekit MSISE density fixture")
}

fn parse_epoch(epoch_utc: &str) -> Epoch {
    // "2024-03-20T12:00:00Z" → Epoch
    let s = epoch_utc.trim_end_matches('Z');
    let parts: Vec<&str> = s.split('T').collect();
    let date_parts: Vec<i32> = parts[0].split('-').map(|x| x.parse().unwrap()).collect();
    let time_parts: Vec<&str> = parts[1].split(':').collect();
    Epoch::from_gregorian(
        date_parts[0],
        date_parts[1] as u32,
        date_parts[2] as u32,
        time_parts[0].parse().unwrap(),
        time_parts[1].parse().unwrap(),
        time_parts[2].parse::<f64>().unwrap(),
    )
}

/// Compute density via ECI round-trip (exercises geo.rs eci_to_geodetic_latlon).
fn compute_density_via_eci(
    model: &Nrlmsise00,
    lat_deg: f64,
    lon_deg: f64,
    alt_km: f64,
    epoch: &Epoch,
) -> f64 {
    // Convert geodetic → ECEF → ECI → geodetic (round-trip to exercise the full path)
    let gmst = epoch.gmst();
    let geod = arika::earth::Geodetic {
        latitude: lat_deg.to_radians(),
        longitude: lon_deg.to_radians(),
        altitude: alt_km,
    };
    let ecef = arika::SimpleEcef::from(geod);
    let eci =
        arika::frame::Rotation::<arika::frame::SimpleEcef, arika::frame::SimpleEci>::from_era(gmst)
            .transform(&ecef);

    // Round-trip: ECI → geodetic via geo module
    let (rt_lat_deg, rt_lon_deg) =
        tobari::nrlmsise00::geo::simple_eci_to_geodetic_latlon(&eci, epoch);
    let rt_geod = arika::earth::Geodetic {
        latitude: rt_lat_deg.to_radians(),
        longitude: rt_lon_deg.to_radians(),
        altitude: arika::earth::geodetic_altitude(eci.inner()),
    };

    model
        .density_with_composition(&rt_geod, epoch)
        .total_mass_density
}

// ─── Tests ───

/// All density points: compare Orekit vs Rust NRLMSISE-00 via ECI path.
///
/// With EoT correction, residual error is dominated by Meeus vs Orekit precise
/// solar position difference (~1 arcmin).
#[test]
fn orekit_msise_density_all_points() {
    let fixture = load_fixture();

    let mut max_rel_err = 0.0_f64;
    let mut sum_rel_err = 0.0_f64;
    let mut n_exceed_5pct = 0;
    let mut worst_point = String::new();

    for (i, p) in fixture.points.iter().enumerate() {
        let epoch = parse_epoch(&p.epoch_utc);
        let model = Nrlmsise00::new(Box::new(ConstantWeather::new(p.f107, p.ap)));

        let our_density = compute_density_via_eci(
            &model,
            p.latitude_deg,
            p.longitude_deg,
            p.altitude_km,
            &epoch,
        );

        let rel_err = if p.density_kg_m3.abs() > 1e-30 {
            (our_density - p.density_kg_m3).abs() / p.density_kg_m3
        } else {
            (our_density - p.density_kg_m3).abs()
        };

        sum_rel_err += rel_err;
        if rel_err > max_rel_err {
            max_rel_err = rel_err;
            worst_point = format!(
                "#{i}: epoch={} lat={} lon={} alt={} f107={} ap={} orekit={:.4e} ours={:.4e} err={:.2}%",
                p.epoch_utc,
                p.latitude_deg,
                p.longitude_deg,
                p.altitude_km,
                p.f107,
                p.ap,
                p.density_kg_m3,
                our_density,
                rel_err * 100.0
            );
        }
        if rel_err > 0.05 {
            n_exceed_5pct += 1;
        }
    }

    let mean_rel_err = sum_rel_err / fixture.points.len() as f64;

    println!(
        "NRLMSISE-00 Orekit cross-validation: {} points",
        fixture.points.len()
    );
    println!("  max relative error: {:.4}%", max_rel_err * 100.0);
    println!("  mean relative error: {:.4}%", mean_rel_err * 100.0);
    println!("  points > 5% error: {n_exceed_5pct}");
    println!("  worst: {worst_point}");

    // Measured: max 0.32%, mean 0.05% (after EoT correction)
    assert!(
        max_rel_err < 0.005,
        "max relative error {:.4}% exceeds 0.5% threshold\n  worst: {worst_point}",
        max_rel_err * 100.0,
    );
    assert!(
        mean_rel_err < 0.001,
        "mean relative error {:.4}% exceeds 0.1% threshold",
        mean_rel_err * 100.0,
    );
}

/// Density comparison at equator — minimal geocentric/geodetic difference.
/// Should show tighter agreement than high-latitude points.
#[test]
fn orekit_msise_density_equatorial_tight() {
    let fixture = load_fixture();

    let equatorial: Vec<&DensityPoint> = fixture
        .points
        .iter()
        .filter(|p| p.latitude_deg.abs() < 1.0)
        .collect();

    assert!(!equatorial.is_empty(), "No equatorial points in fixture");

    let mut max_rel_err = 0.0_f64;

    for p in &equatorial {
        let epoch = parse_epoch(&p.epoch_utc);
        let model = Nrlmsise00::new(Box::new(ConstantWeather::new(p.f107, p.ap)));
        let our_density = compute_density_via_eci(
            &model,
            p.latitude_deg,
            p.longitude_deg,
            p.altitude_km,
            &epoch,
        );

        let rel_err = (our_density - p.density_kg_m3).abs() / p.density_kg_m3;
        if rel_err > max_rel_err {
            max_rel_err = rel_err;
        }
    }

    println!(
        "Equatorial points: {}, max error: {:.4}%",
        equatorial.len(),
        max_rel_err * 100.0
    );

    // Equatorial should be tighter (no latitude conversion issue)
    // Measured: max 0.11% (after EoT correction)
    assert!(
        max_rel_err < 0.002,
        "equatorial max error {:.4}% exceeds 0.2%",
        max_rel_err * 100.0,
    );
}

// ─── CSSI real weather density tests ───
//
// Uses the same trimmed CSSI fixture file as the Rust propagation tests.
// Both Orekit and Rust read the same CSSI data, so differences isolate:
// - CSSI parser implementation (binary search, 3-hour Ap interpolation)
// - LST approximation (UT+lon/15 vs Orekit precise solar time)

#[derive(Deserialize)]
struct CssiFixtureData {
    #[allow(dead_code)]
    generator: String,
    #[allow(dead_code)]
    note: String,
    #[allow(dead_code)]
    known_differences: Vec<String>,
    points: Vec<CssiDensityPoint>,
}

#[derive(Deserialize)]
struct CssiDensityPoint {
    epoch_utc: String,
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_km: f64,
    density_kg_m3: f64,
}

/// All CSSI density points: compare Orekit vs Rust NRLMSISE-00 with real weather.
///
/// A single Nrlmsise00 instance with CssiSpaceWeather provides time-varying
/// F10.7 and Ap from the CSSI fixture file. This validates that our CSSI
/// parser's binary search and 3-hour Ap history match Orekit's implementation.
#[test]
fn orekit_msise_cssi_density_all_points() {
    let json = include_str!("fixtures/orekit_cssi_density_reference.json");
    let fixture: CssiFixtureData =
        serde_json::from_str(json).expect("Failed to parse Orekit CSSI density fixture");

    let cssi_text = include_str!("fixtures/cssi_test_weather.txt");
    let cssi_data = CssiData::parse(cssi_text).expect("Failed to parse CSSI fixture");
    let weather = Box::new(CssiSpaceWeather::new(cssi_data));
    let model = Nrlmsise00::new(weather);

    let mut max_rel_err = 0.0_f64;
    let mut sum_rel_err = 0.0_f64;
    let mut n_exceed_5pct = 0;
    let mut worst_point = String::new();

    for (i, p) in fixture.points.iter().enumerate() {
        let epoch = parse_epoch(&p.epoch_utc);
        let our_density = compute_density_via_eci(
            &model,
            p.latitude_deg,
            p.longitude_deg,
            p.altitude_km,
            &epoch,
        );

        let rel_err = if p.density_kg_m3.abs() > 1e-30 {
            (our_density - p.density_kg_m3).abs() / p.density_kg_m3
        } else {
            (our_density - p.density_kg_m3).abs()
        };

        sum_rel_err += rel_err;
        if rel_err > max_rel_err {
            max_rel_err = rel_err;
            worst_point = format!(
                "#{i}: epoch={} lat={} lon={} alt={} orekit={:.4e} ours={:.4e} err={:.2}%",
                p.epoch_utc,
                p.latitude_deg,
                p.longitude_deg,
                p.altitude_km,
                p.density_kg_m3,
                our_density,
                rel_err * 100.0
            );
        }
        if rel_err > 0.05 {
            n_exceed_5pct += 1;
        }
    }

    let mean_rel_err = sum_rel_err / fixture.points.len() as f64;

    println!(
        "NRLMSISE-00 CSSI cross-validation: {} points",
        fixture.points.len()
    );
    println!("  max relative error: {:.4}%", max_rel_err * 100.0);
    println!("  mean relative error: {:.4}%", mean_rel_err * 100.0);
    println!("  points > 5% error: {n_exceed_5pct}");
    println!("  worst: {worst_point}");

    // CSSI has additional parser/interpolation differences on top of LST residual.
    // Measured: max 2.85%, mean 0.54% (after EoT correction)
    assert!(
        max_rel_err < 0.035,
        "max relative error {:.4}% exceeds 3.5% threshold\n  worst: {worst_point}",
        max_rel_err * 100.0,
    );
    assert!(
        mean_rel_err < 0.008,
        "mean relative error {:.4}% exceeds 0.8% threshold",
        mean_rel_err * 100.0,
    );
}
