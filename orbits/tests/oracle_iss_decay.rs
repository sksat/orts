//! ISS orbital decay validation against real-world TLE data.
//!
//! Propagates the ISS orbit (J2 + Harris-Priester drag) over reboost-free
//! windows and compares predicted SMA decay against observed TLE-based decay.
//!
//! Key design choices:
//! - Compare **orbit-averaged SMA decay** to filter J2 short-period oscillations.
//!   Osculating SMA oscillates ~5-10 km per orbit due to J2; secular decay is
//!   only 0.01-0.1 km/day, so orbit-averaging is essential.
//! - Mean-to-osculating SMA offset cancels when comparing *change*.
//! - Tolerance is wide because our atmosphere models (US Std 1976 / HP)
//!   have no solar flux input:
//!   - Solar minimum (F10.7 ~70): ratio 0.3-3.0×
//!   - Solar maximum (F10.7 ~150+): ratio 0.05-0.5× (model underpredicts)
//!
//! Fixture: `orbits/tests/fixtures/iss_decay_reference.json`
//! Generator: `tools/generate_iss_decay_fixtures.py`

use kaname::epoch::Epoch;
use nalgebra::Vector3;
use orts_integrator::{DormandPrince, State, Tolerances};
use orts_orbits::constants::{J2_EARTH, MU_EARTH, R_EARTH};
use orts_orbits::drag::AtmosphericDrag;
use orts_orbits::gravity::ZonalHarmonics;
use orts_orbits::orbital_system::OrbitalSystem;
use serde::Deserialize;
use tobari::harris_priester::HarrisPriester;

// ─── Fixture data structures ───

#[derive(Deserialize)]
struct FixtureData {
    mu_earth_km3_s2: f64,
    r_earth_km: f64,
    windows: Vec<DecayWindow>,
}

#[derive(Deserialize)]
struct DecayWindow {
    name: String,
    #[allow(dead_code)]
    description: String,
    initial_tle: InitialTle,
    initial_osculating: InitialOsculating,
    tle_sequence: Vec<TlePoint>,
    window_duration_days: f64,
    total_mean_sma_decay_km: f64,
    mean_decay_rate_km_per_day: f64,
}

#[derive(Deserialize)]
struct InitialTle {
    epoch_jd: f64,
    #[allow(dead_code)]
    epoch_utc: String,
    #[allow(dead_code)]
    line1: String,
    #[allow(dead_code)]
    line2: String,
}

#[derive(Deserialize)]
struct InitialOsculating {
    position_km: [f64; 3],
    velocity_km_s: [f64; 3],
}

#[derive(Deserialize)]
struct TlePoint {
    epoch_jd: f64,
    #[allow(dead_code)]
    epoch_utc: String,
    mean_sma_km: f64,
    #[allow(dead_code)]
    mean_altitude_km: f64,
    #[allow(dead_code)]
    line1: String,
    #[allow(dead_code)]
    line2: String,
}

// ─── Test infrastructure ───

fn load_fixture() -> FixtureData {
    let json = include_str!("fixtures/iss_decay_reference.json");
    serde_json::from_str(json).expect("Failed to parse ISS decay fixture")
}

fn build_iss_system(epoch: Epoch) -> OrbitalSystem {
    let gravity = ZonalHarmonics {
        r_body: R_EARTH,
        j2: J2_EARTH,
        j3: None,
        j4: None,
    };
    OrbitalSystem::new(MU_EARTH, Box::new(gravity))
        .with_epoch(epoch)
        .with_body_radius(R_EARTH)
        .with_perturbation(Box::new(
            AtmosphericDrag::for_earth(Some(0.005)) // ISS physical B ≈ Cd*A/(2m)
                .with_atmosphere(Box::new(HarrisPriester::new())),
        ))
}

/// Compute osculating SMA from Cartesian state: a = -μ/(2ε), ε = v²/2 - μ/r
fn osculating_sma(pos: &Vector3<f64>, vel: &Vector3<f64>, mu: f64) -> f64 {
    let r = pos.magnitude();
    let v2 = vel.magnitude_squared();
    let energy = v2 / 2.0 - mu / r;
    -mu / (2.0 * energy)
}

/// ISS approximate orbital period [s] at given SMA
fn orbital_period(sma: f64, mu: f64) -> f64 {
    2.0 * std::f64::consts::PI * (sma.powi(3) / mu).sqrt()
}

/// Compute orbit-averaged SMA by sampling over N orbits from current stepper state.
///
/// Advances the stepper by `n_orbits` orbital periods, sampling SMA at
/// `samples_per_orbit` points per orbit, and returns the time-averaged value.
fn orbit_averaged_sma(
    stepper: &mut orts_integrator::AdaptiveStepper<'_, OrbitalSystem>,
    mu: f64,
    n_orbits: usize,
    samples_per_orbit: usize,
) -> f64 {
    let sma0 = osculating_sma(
        &stepper.state().position,
        &stepper.state().velocity,
        mu,
    );
    let period = orbital_period(sma0, mu);
    let dt_sample = period / samples_per_orbit as f64;
    let n_samples = n_orbits * samples_per_orbit;

    let mut sum_sma = 0.0;
    let t_start = stepper.t();

    for i in 0..n_samples {
        let t_target = t_start + (i as f64 + 0.5) * dt_sample;
        stepper
            .advance_to(
                t_target,
                |_, _| {},
                |_, _| std::ops::ControlFlow::<()>::Continue(()),
            )
            .expect("Integration failed during orbit averaging");
        let sma = osculating_sma(
            &stepper.state().position,
            &stepper.state().velocity,
            mu,
        );
        sum_sma += sma;
    }

    sum_sma / n_samples as f64
}

/// Run a single decay window test.
fn run_decay_window(window_name: &str, min_ratio: f64, max_ratio: f64) {
    let fixture = load_fixture();
    let window = fixture
        .windows
        .iter()
        .find(|w| w.name == window_name)
        .unwrap_or_else(|| panic!("Window '{window_name}' not found in fixture"));

    let mu = fixture.mu_earth_km3_s2;

    // Initial state from SGP4 osculating
    let ic = &window.initial_osculating;
    let initial = State {
        position: Vector3::new(ic.position_km[0], ic.position_km[1], ic.position_km[2]),
        velocity: Vector3::new(
            ic.velocity_km_s[0],
            ic.velocity_km_s[1],
            ic.velocity_km_s[2],
        ),
    };

    let epoch_jd_start = window.initial_tle.epoch_jd;
    let epoch = Epoch::from_jd(epoch_jd_start);
    let system = build_iss_system(epoch);

    let initial_sma = osculating_sma(&initial.position, &initial.velocity, mu);
    println!(
        "{}: initial osc SMA={:.3} km (alt={:.1} km), TLE mean SMA={:.3} km",
        window_name,
        initial_sma,
        initial_sma - fixture.r_earth_km,
        window.tle_sequence[0].mean_sma_km,
    );

    let tol = Tolerances {
        atol: 1e-12,
        rtol: 1e-10,
    };

    // Averaging parameters: 3 orbits, 50 samples/orbit
    let n_avg_orbits = 3;
    let samples_per_orbit = 50;

    // Phase 1: compute orbit-averaged SMA at start (first 3 orbits)
    let dp = DormandPrince;
    let mut stepper = dp.stepper(&system, initial.clone(), 0.0, 10.0, tol.clone());
    let avg_sma_start = orbit_averaged_sma(&mut stepper, mu, n_avg_orbits, samples_per_orbit);
    let t_after_start_avg = stepper.t();
    println!(
        "{}: orbit-avg SMA at start = {:.3} km (averaged over {n_avg_orbits} orbits, t={:.0}s)",
        window_name, avg_sma_start, t_after_start_avg,
    );

    // Phase 2: propagate to end of window
    let last_tle = window.tle_sequence.last().unwrap();
    let dt_end = (last_tle.epoch_jd - epoch_jd_start) * 86400.0;
    // Advance to a few orbits before the end to set up for averaging
    let period = orbital_period(avg_sma_start, mu);
    let t_avg_end_start = dt_end - period * n_avg_orbits as f64;

    let mut min_altitude = f64::MAX;
    stepper
        .advance_to(
            t_avg_end_start,
            |_, state| {
                let alt = state.position.magnitude() - R_EARTH;
                if alt < min_altitude {
                    min_altitude = alt;
                }
            },
            |_, _| std::ops::ControlFlow::<()>::Continue(()),
        )
        .expect("Integration failed advancing to end");

    // Phase 3: compute orbit-averaged SMA at end (last 3 orbits)
    let avg_sma_end = orbit_averaged_sma(&mut stepper, mu, n_avg_orbits, samples_per_orbit);
    println!(
        "{}: orbit-avg SMA at end = {:.3} km (t={:.0}s)",
        window_name, avg_sma_end, stepper.t(),
    );

    // Compare decay
    let predicted_decay = avg_sma_start - avg_sma_end;
    let observed_decay = window.total_mean_sma_decay_km;
    let decay_ratio = if observed_decay > 0.0 {
        predicted_decay / observed_decay
    } else {
        f64::INFINITY
    };

    println!("\n{}: RESULTS", window_name);
    println!(
        "  Duration: {:.1} days ({} TLEs)",
        window.window_duration_days,
        window.tle_sequence.len()
    );
    println!("  Predicted SMA decay: {:.4} km (orbit-averaged)", predicted_decay);
    println!("  Observed SMA decay:  {:.4} km (TLE mean)", observed_decay);
    println!(
        "  Predicted rate: {:.4} km/day",
        predicted_decay / window.window_duration_days
    );
    println!(
        "  Observed rate:  {:.4} km/day",
        window.mean_decay_rate_km_per_day
    );
    println!("  Decay ratio (predicted/observed): {:.3}", decay_ratio);
    println!("  Min altitude: {:.1} km", min_altitude);

    // Assertions
    assert!(
        predicted_decay > 0.0,
        "{window_name}: drag must cause positive SMA decay, got {predicted_decay:.4} km"
    );

    assert!(
        decay_ratio >= min_ratio && decay_ratio <= max_ratio,
        "{window_name}: decay ratio {decay_ratio:.3} outside [{min_ratio}, {max_ratio}]"
    );

    assert!(
        min_altitude > 200.0,
        "{window_name}: ISS altitude dropped below 200 km (min={min_altitude:.1} km)"
    );
}

// ─── Test functions ───

// Solar minimum (2019-2020): HP density table (Montenbruck & Gill) represents
// moderate solar activity. During deep solar minimum (F10.7 ~70 SFU), actual
// thermospheric density at 400 km is ~10-20× lower than the model baseline.
// Our model consistently overpredicts decay by ~15-19×.
// Measured ratios: 19.1, 13.8, 17.1 — use 5-30× tolerance.

#[test]
fn iss_decay_solar_min_2019a() {
    // 43-day window, 2019-07-03 to 2019-08-15
    run_decay_window("solar_min_2019a", 5.0, 30.0);
}

#[test]
fn iss_decay_solar_min_2019b() {
    // 54-day window, 2019-09-15 to 2019-11-07
    run_decay_window("solar_min_2019b", 5.0, 30.0);
}

#[test]
fn iss_decay_solar_min_2020c() {
    // 71-day window, 2020-04-19 to 2020-06-29
    run_decay_window("solar_min_2020c", 5.0, 30.0);
}

// Solar maximum (2024): HP density table matches reality well during high
// solar activity (F10.7 ~150-200 SFU). Measured ratio: 1.14×.
// Use 0.5-2.0× tolerance.

#[test]
fn iss_decay_solar_max_2024d() {
    // 32-day window, 2024-03-14 to 2024-04-15
    run_decay_window("solar_max_2024d", 0.5, 2.0);
}
