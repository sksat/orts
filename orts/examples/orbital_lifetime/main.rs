//! Orbital lifetime analysis example: AE1b (YODAKA) decay reproduction.
//!
//! Demonstrates using orts libraries (kaname, utsuroi, tobari, orts) to predict
//! how long a LEO satellite stays in orbit before atmospheric reentry.
//!
//! Compares multiple atmosphere models and ballistic coefficients against the
//! observed lifetime of ArkEdge Space's 6U CubeSat AE1b (YODAKA, NORAD 62295),
//! deployed from the ISS on 2024-12-09 and decayed on 2025-02-25.
//!
//! Run:
//!   cargo run --example orbital_lifetime -p orts
//!   cargo run --example orbital_lifetime -p orts --features fetch-weather
//!
//! Test:
//!   cargo test --example orbital_lifetime -p orts

use kaname::constants::{J2_EARTH, MU_EARTH, R_EARTH};
use kaname::epoch::Epoch;
use nalgebra::Vector3;
use orts::OrbitalState;
use orts::events::SimulationEvent;
use orts::orbital::OrbitalSystem;
use orts::orbital::gravity::ZonalHarmonics;
use orts::orbital::kepler::KeplerianElements;
use orts::perturbations::AtmosphericDrag;
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
use orts::record::components::{BodyRadius, GravitationalParameter};
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;
use tobari::{ConstantWeather, Exponential, HarrisPriester, Nrlmsise00};
use utsuroi::{DormandPrince, Tolerances};

// ============================================================
// AE1b (YODAKA) mission reference
// ============================================================
//
// NORAD Catalog Number: 62295
// COSPAR ID: 1998-067XB (ISS-associated object)
//
// Sources:
//   [1] CelesTrak SATCAT
//       https://celestrak.org/satcat/records.php?CATNR=62295
//       Owner: Japan, Object Type: Payload, Decay: 2025-02-25
//       Last orbit: 185x195 km, 88.29 min, i=51.61 deg
//   [2] SatNOGS DB
//       https://db.satnogs.org/satellite/PZIL-3361-3557-2301-4863/
//       Deployed: 2024-12-09 08:15 UTC (J-SSOD), Decay: 2025-02-28
//   [3] Space-Track SATCAT (USSPACECOM)
//       https://www.space-track.org/
//       Authoritative TLE source for NORAD 62295

/// J-SSOD deployment from ISS [2].
const DEPLOY_EPOCH_ISO: &str = "2024-12-09T08:15:00Z";

/// Observed orbital lifetime [days].
/// Decay date: CelesTrak reports 2025-02-25 [1], SatNOGS reports 2025-02-28 [2].
/// Using CelesTrak (USSPACECOM-derived) as primary reference.
const OBSERVED_LIFETIME_DAYS: f64 = 78.0; // 2024-12-09 -> 2025-02-25

// ============================================================
// Initial orbit
// ============================================================
// CubeSat deployed from ISS via J-SSOD: orbit matches ISS at deployment.
// Source: ISS TLE from Space-Track (NORAD 25544) near 2024-12-09.
//   ISS maintained ~410-420 km altitude in late 2024.
//   Inclination: 51.64 deg (SATCAT [1] confirms 51.61 deg for YODAKA).

const INITIAL_ALT_KM: f64 = 415.0;
const INCLINATION_DEG: f64 = 51.64;

// ============================================================
// 6U CubeSat physical parameters
// ============================================================
// Form factor: 6U standard (approximately 10x20x34 cm).
// Source: CubeSat Design Specification Rev. 14.1, Cal Poly SLO
//   https://www.cubesat.org/cubesatinfo
//
// Ballistic coefficient B = Cd*A/(2*m) [m^2/kg]
//
// Cd ~ 2.2: Vallado, "Fundamentals of Astrodynamics and Applications"
//           4th ed., Section 8.6.2
//
// Tumbling-average cross-section A_avg = S_total / 4
//   (Sentman, "Free molecule flow theory and its application to the
//    determination of aerodynamic forces", LMSC-448514, 1961)
//
// Bus body (10 x 20 x 34 cm):
//   S_body = 2*(0.10*0.20 + 0.10*0.34 + 0.20*0.34) = 0.244 m^2
//   A_body = S_body / 4 = 0.061 m^2
//
// Deployed solar panels (typical 6U: 2 wings, each ~20 x 33 cm):
//   Source: EnduroSat 6U Deployable Solar Array datasheet (209 x 342 mm)
//     https://www.endurosat.com/products/6u-deployable-solar-array/
//   Each wing is a thin plate, area A_face ~ 0.071 m^2
//   Tumbling average for thin plate: A_face / 2 = 0.036 m^2 per wing
//   Two wings: A_panels = 2 * 0.036 = 0.071 m^2
//
// A_total ~ 0.061 + 0.071 = 0.132 m^2
//
// Mass: ArkEdge Space "10 kg class" for 6U standard bus
//   Source: https://arkedgespace.com/en/news/multipurpose6usatellite
//   CubeSat Design Specification Rev. 14.1 max: 12 kg
//
// B = Cd * A_total / (2 * m) = 2.2 * 0.132 / (2 * 10) = 0.015 m^2/kg
//
// Sensitivity range (mass uncertainty +-20%):
//   m=12 kg: B = 2.2 * 0.132 / 24 = 0.012
//   m=10 kg: B = 2.2 * 0.132 / 20 = 0.015  (nominal)
//   m= 8 kg: B = 2.2 * 0.132 / 16 = 0.018

#[cfg_attr(not(feature = "fetch-weather"), allow(dead_code))]
const BALLISTIC_COEFF_LOW: f64 = 0.012;
const BALLISTIC_COEFF_MID: f64 = 0.015;
#[cfg_attr(not(feature = "fetch-weather"), allow(dead_code))]
const BALLISTIC_COEFF_HIGH: f64 = 0.018;

/// Reentry detection threshold [km].
/// Karman line: 100 km altitude (IAF/FAI definition).
const REENTRY_ALT: f64 = 100.0;

/// Maximum simulation duration [days].
const MAX_DURATION_DAYS: f64 = 365.0;

// ============================================================
// Space weather for predictive (Group 1) scenarios
// ============================================================
// Source: NOAA SWPC Solar Cycle 25 Progression
//   https://www.swpc.noaa.gov/products/solar-cycle-progression
//   Smoothed monthly F10.7 peaked at 160.8 SFU in October 2024.
//   Monthly values Dec 2024 - Feb 2025 ranged ~136-180 SFU.
//   Geomagnetic activity was moderate (Ap ~10-20) through this period.

const F107_PREDICTED: f64 = 170.0;
const AP_PREDICTED: f64 = 15.0;

/// Daily output interval for stdout [days].
const PRINT_INTERVAL_DAYS: usize = 10;

// ============================================================
// Data structures
// ============================================================

#[allow(dead_code)]
struct DailySample {
    day: f64,
    altitude_km: f64,
    sma_km: f64,
}

#[allow(dead_code)]
struct ScenarioResult {
    name: String,
    group: &'static str,
    atmosphere: String,
    weather: String,
    ballistic_coeff: f64,
    lifetime_days: f64,
    samples: Vec<DailySample>,
}

// ============================================================
// Helper functions
// ============================================================

/// Create initial OrbitalState from circular orbit parameters.
fn initial_state(altitude_km: f64, inclination_deg: f64) -> OrbitalState {
    let elements = KeplerianElements {
        semi_major_axis: R_EARTH + altitude_km,
        eccentricity: 0.0,
        inclination: inclination_deg.to_radians(),
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    OrbitalState::new(pos, vel)
}

/// Build OrbitalSystem with J2 gravity + atmospheric drag.
fn build_system(
    epoch: Epoch,
    ballistic_coeff: f64,
    atmosphere: Box<dyn tobari::AtmosphereModel>,
) -> OrbitalSystem {
    let gravity = ZonalHarmonics {
        r_body: R_EARTH,
        j2: J2_EARTH,
        j3: None,
        j4: None,
    };
    OrbitalSystem::new(MU_EARTH, Box::new(gravity))
        .with_epoch(epoch)
        .with_body_radius(R_EARTH)
        .with_model(AtmosphericDrag::for_earth(Some(ballistic_coeff)).with_atmosphere(atmosphere))
}

/// Compute osculating SMA from Cartesian state: a = -mu/(2*energy).
fn osculating_sma(pos: &Vector3<f64>, vel: &Vector3<f64>, mu: f64) -> f64 {
    let r = pos.magnitude();
    let v2 = vel.magnitude_squared();
    let energy = v2 / 2.0 - mu / r;
    -mu / (2.0 * energy)
}

/// Run a single scenario. Returns ScenarioResult.
#[allow(clippy::too_many_arguments)]
fn run_scenario(
    name: &str,
    group: &'static str,
    atmosphere_name: &str,
    weather_name: &str,
    system: OrbitalSystem,
    initial: &OrbitalState,
    ballistic_coeff: f64,
    rec: &mut Recording,
    entity_path: &EntityPath,
) -> ScenarioResult {
    let tol = Tolerances {
        atol: 1e-10,
        rtol: 1e-8,
    };
    let dp = DormandPrince;
    let mut stepper = dp.stepper(&system, initial.clone(), 0.0, 10.0, tol);

    let max_t = MAX_DURATION_DAYS * 86400.0;
    let mut samples = Vec::new();
    let mut lifetime_days = MAX_DURATION_DAYS;
    let mut day = 0u64;
    let mut step = 0u64;

    // Record initial state
    let sma0 = osculating_sma(initial.position(), initial.velocity(), MU_EARTH);
    let alt0 = initial.position().magnitude() - R_EARTH;
    samples.push(DailySample {
        day: 0.0,
        altitude_km: alt0,
        sma_km: sma0,
    });
    let tp = TimePoint::new().with_sim_time(0.0).with_step(step);
    let os = RecordOrbitalState::new(*initial.position(), *initial.velocity());
    rec.log_orbital_state(entity_path, &tp, &os);
    step += 1;

    println!("--- {name} ---");
    println!(
        "  Day {:>5.0}: alt = {:>7.1} km, SMA = {:>8.1} km",
        0.0, alt0, sma0
    );

    let event_check = orts::events::collision_check(R_EARTH, Some(REENTRY_ALT));

    loop {
        day += 1;
        let t_target = day as f64 * 86400.0;
        if t_target > max_t {
            break;
        }

        let result =
            stepper.advance_to(t_target, |_t, _state| {}, |t, state| event_check(t, state));

        match result {
            Ok(utsuroi::AdvanceOutcome::Reached) => {
                let pos = stepper.state().position();
                let vel = stepper.state().velocity();
                let alt = pos.magnitude() - R_EARTH;
                let sma = osculating_sma(pos, vel, MU_EARTH);

                samples.push(DailySample {
                    day: day as f64,
                    altitude_km: alt,
                    sma_km: sma,
                });

                // Record to RRD
                let tp = TimePoint::new().with_sim_time(stepper.t()).with_step(step);
                let os = RecordOrbitalState::new(*pos, *vel);
                rec.log_orbital_state(entity_path, &tp, &os);
                step += 1;

                if (day as usize).is_multiple_of(PRINT_INTERVAL_DAYS) {
                    println!(
                        "  Day {:>5}: alt = {:>7.1} km, SMA = {:>8.1} km",
                        day, alt, sma
                    );
                }
            }
            Ok(utsuroi::AdvanceOutcome::Event {
                reason: SimulationEvent::AtmosphericEntry { altitude_km },
            })
            | Ok(utsuroi::AdvanceOutcome::Event {
                reason: SimulationEvent::Collision { altitude_km },
            }) => {
                lifetime_days = stepper.t() / 86400.0;
                println!(
                    "  ** Reentry at day {:.1} (alt = {:.1} km) **",
                    lifetime_days, altitude_km
                );

                // Record final state
                let tp = TimePoint::new().with_sim_time(stepper.t()).with_step(step);
                let os = RecordOrbitalState::new(
                    *stepper.state().position(),
                    *stepper.state().velocity(),
                );
                rec.log_orbital_state(entity_path, &tp, &os);
                break;
            }
            Err(e) => {
                eprintln!("  Integration error: {e:?}");
                lifetime_days = stepper.t() / 86400.0;
                break;
            }
        }
    }

    println!();

    ScenarioResult {
        name: name.to_string(),
        group,
        atmosphere: atmosphere_name.to_string(),
        weather: weather_name.to_string(),
        ballistic_coeff,
        lifetime_days,
        samples,
    }
}

// ============================================================
// Main
// ============================================================

fn main() {
    let epoch = Epoch::from_iso8601(DEPLOY_EPOCH_ISO).unwrap();
    let initial = initial_state(INITIAL_ALT_KM, INCLINATION_DEG);

    let sma0 = osculating_sma(initial.position(), initial.velocity(), MU_EARTH);
    let period_min = 2.0 * std::f64::consts::PI * (sma0.powi(3) / MU_EARTH).sqrt() / 60.0;

    println!("=== Orbital Lifetime Analysis: AE1b (YODAKA) ===");
    println!();
    println!("  NORAD 62295 / COSPAR 1998-067XB");
    println!("  Initial orbit: {INITIAL_ALT_KM:.0} km circular, {INCLINATION_DEG:.2} deg incl");
    println!("  Period: {period_min:.1} min, SMA: {sma0:.1} km");
    println!("  Deployment: {DEPLOY_EPOCH_ISO}");
    println!("  Observed decay: 2025-02-25 ({OBSERVED_LIFETIME_DAYS:.0} days) [CelesTrak SATCAT]");
    println!();

    // Build recording
    let mut rec = Recording::new();
    let earth_path = EntityPath::parse("/world/earth");
    rec.log_static(&earth_path, &GravitationalParameter(MU_EARTH));
    rec.log_static(&earth_path, &BodyRadius(R_EARTH));

    let mut results: Vec<ScenarioResult> = Vec::new();

    // ================================================================
    // Group 1: Predictive (pre-launch information only)
    // ================================================================
    println!("=== Group 1: Predictive (pre-launch information only) ===");
    println!();

    // Scenario A: Exponential atmosphere
    results.push(run_scenario(
        "A: Exponential, B=0.015",
        "Pred",
        "Exponential",
        "-",
        build_system(epoch, BALLISTIC_COEFF_MID, Box::new(Exponential)),
        &initial,
        BALLISTIC_COEFF_MID,
        &mut rec,
        &EntityPath::parse("/world/sat/scenario_a"),
    ));

    // Scenario B: Harris-Priester
    results.push(run_scenario(
        "B: Harris-Priester, B=0.015",
        "Pred",
        "H-P",
        "-",
        build_system(epoch, BALLISTIC_COEFF_MID, Box::new(HarrisPriester::new())),
        &initial,
        BALLISTIC_COEFF_MID,
        &mut rec,
        &EntityPath::parse("/world/sat/scenario_b"),
    ));

    // Scenario C: NRLMSISE-00 + ConstantWeather
    results.push(run_scenario(
        &format!("C: NRLMSISE-00 (F10.7={F107_PREDICTED}, Ap={AP_PREDICTED}), B=0.015"),
        "Pred",
        "NRLMSISE-00",
        &format!("Const({F107_PREDICTED:.0})"),
        build_system(
            epoch,
            BALLISTIC_COEFF_MID,
            Box::new(Nrlmsise00::new(Box::new(ConstantWeather::new(
                F107_PREDICTED,
                AP_PREDICTED,
            )))),
        ),
        &initial,
        BALLISTIC_COEFF_MID,
        &mut rec,
        &EntityPath::parse("/world/sat/scenario_c"),
    ));

    // ================================================================
    // Group 2: Launch-day prediction (CSSI data available at launch)
    // ================================================================
    // CSSI data is truncated at the deployment epoch so only data that
    // would have been available on launch day is used.  After the cutoff
    // the CssiSpaceWeather provider clamps to the last known record,
    // effectively assuming "conditions stay the same" — the simplest
    // operationally-available forecast.
    #[cfg(feature = "fetch-weather")]
    {
        println!("=== Group 2: Launch-day prediction (CSSI cutoff at deployment) ===");
        println!();

        let make_cutoff_atmo = || -> Box<dyn tobari::AtmosphereModel> {
            let cssi_full = tobari::CssiSpaceWeather::fetch_default().unwrap();
            let cssi_data = cssi_full.into_data();
            let cutoff_data = cssi_data.truncate_after(&epoch);
            if let Some((_, last)) = cutoff_data.date_range() {
                eprintln!(
                    "  CSSI data cutoff: last record = {} ({} records)",
                    last.jd(),
                    cutoff_data.len()
                );
            }
            let cssi_cutoff = tobari::CssiSpaceWeather::new(cutoff_data);
            Box::new(Nrlmsise00::new(Box::new(cssi_cutoff)))
        };

        // Scenario D: launch-day CSSI + mid B
        results.push(run_scenario(
            "D: NRLMSISE-00 + CSSI@launch, B=0.015",
            "Launch",
            "NRLMSISE-00",
            "CSSI@launch",
            build_system(epoch, BALLISTIC_COEFF_MID, make_cutoff_atmo()),
            &initial,
            BALLISTIC_COEFF_MID,
            &mut rec,
            &EntityPath::parse("/world/sat/scenario_d"),
        ));

        // Scenario E: launch-day CSSI + low B
        results.push(run_scenario(
            "E: NRLMSISE-00 + CSSI@launch, B=0.012",
            "Launch",
            "NRLMSISE-00",
            "CSSI@launch",
            build_system(epoch, BALLISTIC_COEFF_LOW, make_cutoff_atmo()),
            &initial,
            BALLISTIC_COEFF_LOW,
            &mut rec,
            &EntityPath::parse("/world/sat/scenario_e"),
        ));

        // Scenario F: launch-day CSSI + high B
        results.push(run_scenario(
            "F: NRLMSISE-00 + CSSI@launch, B=0.018",
            "Launch",
            "NRLMSISE-00",
            "CSSI@launch",
            build_system(epoch, BALLISTIC_COEFF_HIGH, make_cutoff_atmo()),
            &initial,
            BALLISTIC_COEFF_HIGH,
            &mut rec,
            &EntityPath::parse("/world/sat/scenario_f"),
        ));
    }

    // ================================================================
    // Group 3: Retrospective (full CSSI observed data, post-decay)
    // ================================================================
    #[cfg(feature = "fetch-weather")]
    {
        println!("=== Group 3: Retrospective (full CSSI observed data) ===");
        println!();

        let make_full_atmo = || -> Box<dyn tobari::AtmosphereModel> {
            let cssi = tobari::CssiSpaceWeather::fetch_default().unwrap();
            Box::new(Nrlmsise00::new(Box::new(cssi)))
        };

        // Scenario G: full CSSI + mid B
        results.push(run_scenario(
            "G: NRLMSISE-00 + CSSI(full), B=0.015",
            "Retro",
            "NRLMSISE-00",
            "CSSI(full)",
            build_system(epoch, BALLISTIC_COEFF_MID, make_full_atmo()),
            &initial,
            BALLISTIC_COEFF_MID,
            &mut rec,
            &EntityPath::parse("/world/sat/scenario_g"),
        ));

        // Scenario H: full CSSI + high B
        results.push(run_scenario(
            "H: NRLMSISE-00 + CSSI(full), B=0.018",
            "Retro",
            "NRLMSISE-00",
            "CSSI(full)",
            build_system(epoch, BALLISTIC_COEFF_HIGH, make_full_atmo()),
            &initial,
            BALLISTIC_COEFF_HIGH,
            &mut rec,
            &EntityPath::parse("/world/sat/scenario_h"),
        ));
    }

    // ================================================================
    // Comparison table
    // ================================================================
    println!("=== Comparison ===");
    println!();
    println!(
        "  {:>5} | {:>1} | {:>11} | {:>10} | {:>9} | {:>8} | {:>11}",
        "Group", "#", "Atmosphere", "Weather", "B [m2/kg]", "Lifetime", "vs Observed"
    );
    println!(
        "  {:->5}-+-{:->1}-+-{:->11}-+-{:->10}-+-{:->9}-+-{:->8}-+-{:->11}",
        "", "", "", "", "", "", ""
    );

    for r in &results {
        let letter = r.name.chars().next().unwrap_or('?');
        let diff = r.lifetime_days - OBSERVED_LIFETIME_DAYS;
        let diff_str = if diff >= 0.0 {
            format!("+{diff:.0} days")
        } else {
            format!("{diff:.0} days")
        };
        println!(
            "  {:>5} | {} | {:>11} | {:>10} | {:>9.3} | {:>5.0} days | {:>11}",
            r.group, letter, r.atmosphere, r.weather, r.ballistic_coeff, r.lifetime_days, diff_str
        );
    }

    println!(
        "  {:->5}-+-{:->1}-+-{:->11}-+-{:->10}-+-{:->9}-+-{:->8}-+-{:->11}",
        "", "", "", "", "", "", ""
    );
    println!(
        "  {:>5}   {:>1}   {:>11}   {:>10}   {:>9}   {:>5.0} days",
        "", "", "Observed", "", "", OBSERVED_LIFETIME_DAYS
    );
    println!();

    // ================================================================
    // Save RRD
    // ================================================================
    let rrd_path = "orts/examples/orbital_lifetime/orbital_lifetime.rrd";
    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: Some(epoch.jd()),
        mu: Some(MU_EARTH),
        body_radius: Some(R_EARTH),
        body_name: Some("Earth".to_string()),
        altitude: Some(INITIAL_ALT_KM),
        period: None,
    };
    orts::record::rerun_export::save_as_rrd(&rec, "orts-orbital-lifetime", rrd_path).unwrap();
    println!("Saved to {rrd_path} (open with: rerun {rrd_path})");

    // ================================================================
    // Assertions (active during `cargo test --example`)
    // ================================================================
    // Group 1: all scenarios must terminate with reentry
    for r in &results {
        assert!(
            r.lifetime_days < MAX_DURATION_DAYS,
            "{}: did not reenter within {MAX_DURATION_DAYS} days (lifetime={:.0})",
            r.name,
            r.lifetime_days,
        );
        assert!(
            r.lifetime_days > 0.0,
            "{}: lifetime must be positive",
            r.name,
        );
    }

    // Group 2+3 assertions (only when fetch-weather is enabled)
    #[cfg(feature = "fetch-weather")]
    {
        // Ballistic coefficient ordering within each CSSI group: higher B -> shorter lifetime
        for group_name in &["Launch", "Retro"] {
            let group: Vec<&ScenarioResult> =
                results.iter().filter(|r| r.group == *group_name).collect();
            if group.len() >= 2 {
                let mut sorted = group.clone();
                sorted.sort_by(|a, b| a.ballistic_coeff.partial_cmp(&b.ballistic_coeff).unwrap());
                for w in sorted.windows(2) {
                    assert!(
                        w[0].lifetime_days >= w[1].lifetime_days,
                        "{}: {} (B={:.3}, {:.0}d) should live longer than {} (B={:.3}, {:.0}d)",
                        group_name,
                        w[0].name,
                        w[0].ballistic_coeff,
                        w[0].lifetime_days,
                        w[1].name,
                        w[1].ballistic_coeff,
                        w[1].lifetime_days,
                    );
                }
            }
        }

        // At least one scenario within 50% of observed
        let any_close = results.iter().any(|r| {
            let ratio = r.lifetime_days / OBSERVED_LIFETIME_DAYS;
            (0.5..=1.5).contains(&ratio)
        });
        assert!(
            any_close,
            "No scenario within +-50% of observed {OBSERVED_LIFETIME_DAYS} days"
        );
    }
}
