//! Artemis 1 simulation example — currently a multi-phase coast feasibility
//! spike.
//!
//! ## Current status (Phase 1.5)
//!
//! This example is a **feasibility spike** for a future full Artemis 1
//! mission reconstruction. Before committing to the full mission scaffold
//! (25 days, ~15 burns, OPF/DRI/DRDI/RPF targeting), we need to answer one
//! concrete question: can an Earth-centric `Dop853` integrator with a
//! `J2/J3/J4 + Sun + HorizonsMoon` force model propagate Orion's three
//! major coast phases to within acceptable accuracy of the real Horizons
//! trajectory?
//!
//! The DRO regime is where this architecture is most stressed: the
//! spacecraft orbits the Moon at ~70,000 km while Earth-centric coordinates
//! treat the Moon as a third-body perturber — but at DRO distances the
//! Moon's gravity is comparable to Earth's, violating the usual "third
//! body is far" assumption. Apollo 11, which this example's template is
//! based on, only spends ~2.5 days in low lunar orbit and already documents
//! degraded accuracy near the Moon.
//!
//! ## Coast phases exercised
//!
//! | Phase    | Window (UTC, round epochs)         | Regime                                |
//! |----------|------------------------------------|---------------------------------------|
//! | Outbound | 2022-11-17 00:00 → 2022-11-20 00:00 | Trans-lunar cruise, far from Moon     |
//! | DRO      | 2022-11-26 00:00 → 2022-12-01 00:00 | Retrograde loop at ~70,000 km lunar   |
//! | Return   | 2022-12-06 00:00 → 2022-12-10 00:00 | Trans-Earth cruise, moving away       |
//!
//! Each window is chosen to sit **between major burns** so the result
//! isolates integrator-plus-ephemeris accuracy from the big propulsive
//! events. Rough Artemis 1 burn epochs for context:
//!
//! - TLI: 2022-11-16 ~08:24 UTC
//! - OPF (Outbound Powered Flyby): 2022-11-21 ~12:44 UTC
//! - DRI (DRO Insertion): 2022-11-25 ~22:52 UTC
//! - DRDI (DRO Departure): 2022-12-01 ~21:27 UTC
//! - RPF (Return Powered Flyby): 2022-12-05 ~16:42 UTC
//! - EI (Entry Interface): 2022-12-11 ~17:20 UTC
//!
//! **Caveat**: small (<1 m/s) trajectory-correction burns (OTC-2..4 during
//! outbound, RTC-1..n during return) may fall inside the Outbound and
//! Return windows. Their integrated effect on position is O(10 km) over a
//! few days — large enough to see in the error budget but far below the
//! 1000 km PASS threshold, so the overall judgment is unaffected. The DRO
//! window is clean (no scheduled station-keeping during Artemis 1's 6-day
//! retrograde loop).
//!
//! ## What this example does
//!
//! 1. Fetches a single Moon ephemeris (target `301`) from JPL Horizons that
//!    covers all three coast windows, at 1-hour spacing.
//! 2. For each coast window, fetches the real Orion state vector (target
//!    `-1023`) at both endpoints.
//! 3. Propagates the start state forward to the end epoch using `Dop853`
//!    with `dt = 10 s`, `J2/J3/J4`, Sun (Meeus), Moon (Horizons-interpolated).
//! 4. Compares the propagated final state to the Horizons reference.
//! 5. Asserts that no Horizons-Moon table lookups fell back to Meeus — a
//!    silent fallback would hide the whole point of this spike.
//! 6. Prints a summary table with each window's error envelope and a
//!    overall pass/conditional/fail judgment.
//!
//! ## Judgment criteria
//!
//! - `< 1000 km`: **Pass** — proceed with Earth-centric architecture.
//! - `1000-10000 km`: **Conditional** — re-run with tighter tolerances.
//! - `>= 10000 km`: **Fail** — switch to Moon-centric SOI switching,
//!   higher-order integrator, or reduce scope (drop the failing phase).
//!
//! The overall judgment is the worst of the three phases.
//!
//! ## Running
//!
//! The HTTP fetch requires the `fetch-horizons` feature:
//!
//! ```bash
//! cargo run --example artemis1 -p orts --features fetch-horizons
//! ```
//!
//! Without the feature the example exits with a helpful message — the
//! spike cannot run offline because it needs the Horizons reference data.
//! (Future iterations will bundle a pre-fetched CSV for offline runs.)

#[cfg(feature = "fetch-horizons")]
use std::sync::Arc;

#[cfg(feature = "fetch-horizons")]
use kaname::epoch::Epoch;
#[cfg(feature = "fetch-horizons")]
use kaname::horizons::HorizonsTable;
#[cfg(feature = "fetch-horizons")]
use kaname::moon::{HorizonsMoonEphemeris, MoonEphemeris};
#[cfg(feature = "fetch-horizons")]
use orts::OrbitalState;
#[cfg(feature = "fetch-horizons")]
use orts::orbital::OrbitalSystem;
#[cfg(feature = "fetch-horizons")]
use orts::orbital::gravity::ZonalHarmonics;
#[cfg(feature = "fetch-horizons")]
use orts::perturbations::ThirdBodyGravity;
#[cfg(feature = "fetch-horizons")]
use utsuroi::{Dop853, Integrator};

// ============================================================
// Mission constants
// ============================================================

/// Orion spacecraft JPL Horizons target ID.
#[cfg(feature = "fetch-horizons")]
const ORION_TARGET: &str = "-1023";

/// Earth geocenter in Horizons center syntax.
#[cfg(feature = "fetch-horizons")]
const EARTH_GEOCENTER: &str = "500@399";

/// Moon JPL Horizons target ID.
#[cfg(feature = "fetch-horizons")]
const MOON_TARGET: &str = "301";

/// Horizons sample spacing for the Moon ephemeris table.
#[cfg(feature = "fetch-horizons")]
const MOON_SAMPLE_STEP: &str = "1h";

/// Dop853 propagation step size (same as apollo11).
#[cfg(feature = "fetch-horizons")]
const DT_SECONDS: f64 = 10.0;

/// Moon ephemeris window covering all three coast phases, padded ±1 h for
/// the Hermite interpolator's bracketing requirement.
#[cfg(feature = "fetch-horizons")]
const MOON_WINDOW_START_ISO: &str = "2022-11-16T23:00:00Z";
#[cfg(feature = "fetch-horizons")]
const MOON_WINDOW_STOP_ISO: &str = "2022-12-11T01:00:00Z";

/// Coast phases to verify. Each is a `(label, start_iso, end_iso)` tuple,
/// deliberately chosen to sit between burns so the result isolates coast
/// accuracy from burn-application accuracy.
#[cfg(feature = "fetch-horizons")]
const COAST_PHASES: &[(&str, &str, &str)] = &[
    (
        "Outbound (trans-lunar)",
        "2022-11-17T00:00:00Z",
        "2022-11-20T00:00:00Z",
    ),
    (
        "DRO (retrograde loop)",
        "2022-11-26T00:00:00Z",
        "2022-12-01T00:00:00Z",
    ),
    (
        "Return (trans-Earth)",
        "2022-12-06T00:00:00Z",
        "2022-12-10T00:00:00Z",
    ),
];

/// Thresholds for the per-phase judgment (km of final position error).
#[cfg(feature = "fetch-horizons")]
const THRESHOLD_PASS_KM: f64 = 1000.0;
#[cfg(feature = "fetch-horizons")]
const THRESHOLD_CONDITIONAL_KM: f64 = 10_000.0;

// ============================================================
// Phase result (used by the summary table)
// ============================================================

#[cfg(feature = "fetch-horizons")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Judgment {
    Pass,
    Conditional,
    Fail,
}

#[cfg(feature = "fetch-horizons")]
impl Judgment {
    fn from_error_km(position_error_km: f64) -> Self {
        if position_error_km < THRESHOLD_PASS_KM {
            Self::Pass
        } else if position_error_km < THRESHOLD_CONDITIONAL_KM {
            Self::Conditional
        } else {
            Self::Fail
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "✓ PASS",
            Self::Conditional => "? COND",
            Self::Fail => "✗ FAIL",
        }
    }
}

#[cfg(feature = "fetch-horizons")]
struct PhaseResult {
    label: &'static str,
    duration_days: f64,
    position_error_km: f64,
    velocity_error_kms: f64,
    min_moon_distance_km: f64,
    max_earth_distance_km: f64,
    judgment: Judgment,
}

// ============================================================
// Main
// ============================================================

#[cfg(not(feature = "fetch-horizons"))]
fn main() {
    eprintln!(
        "artemis1 example: the DRO feasibility spike requires live JPL Horizons data.\n\
         \n\
         Rerun with the `fetch-horizons` feature enabled:\n\
         \n\
             cargo run --example artemis1 -p orts --features fetch-horizons\n\
         \n\
         (Offline mode with bundled reference CSV is a future enhancement.)"
    );
    std::process::exit(1);
}

#[cfg(feature = "fetch-horizons")]
fn main() {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Artemis 1 Coast Feasibility Spike");
    println!("═══════════════════════════════════════════════════════════════════");
    println!();
    println!("Goal: determine whether an Earth-centric Dop853 integrator with");
    println!("J2/J3/J4 + Sun + Horizons-Moon can propagate Orion's three coast");
    println!("phases (outbound, DRO, return) to within 1000 km of Horizons.");
    println!();

    // ----- Fetch one Moon ephemeris covering the whole mission -----
    println!("[1/3] Fetching Moon ephemeris ({MOON_SAMPLE_STEP} spacing) from Horizons...");
    let moon_window_start =
        Epoch::from_iso8601(MOON_WINDOW_START_ISO).expect("valid Moon window start");
    let moon_window_stop =
        Epoch::from_iso8601(MOON_WINDOW_STOP_ISO).expect("valid Moon window stop");
    let moon_table = HorizonsTable::fetch_vector_table(
        MOON_TARGET,
        EARTH_GEOCENTER,
        &moon_window_start,
        &moon_window_stop,
        MOON_SAMPLE_STEP,
        None,
    )
    .expect("fetch Moon ephemeris");
    println!(
        "  {} samples over {} → {}",
        moon_table.samples().len(),
        iso_short(&moon_window_start),
        iso_short(&moon_window_stop),
    );
    // Concretely-typed handle so `fallback_count` is readable after each
    // phase; the dynamically-typed handle is what the force model consumes.
    let moon_concrete: Arc<HorizonsMoonEphemeris> =
        Arc::new(HorizonsMoonEphemeris::from_table(moon_table));
    let moon_ephem: Arc<dyn MoonEphemeris> = moon_concrete.clone();
    println!();

    // ----- Fetch Orion state vectors at every phase endpoint -----
    println!("[2/3] Fetching Orion reference state vectors at each phase endpoint...");
    println!();

    // ----- Verify each coast phase -----
    println!("[3/3] Propagating each coast phase and comparing to Horizons...");
    println!();

    let mut results: Vec<PhaseResult> = Vec::new();
    for (label, start_iso, end_iso) in COAST_PHASES {
        let result = verify_coast(label, start_iso, end_iso, &moon_ephem, &moon_concrete);
        results.push(result);
        println!();
    }

    // ----- Summary table -----
    print_summary(&results);
}

// ============================================================
// Coast verification
// ============================================================

/// Propagate a single coast phase from `start_iso` to `end_iso`, compare
/// to the Horizons reference at `end_iso`, and return a `PhaseResult`.
///
/// `moon_concrete` is the same underlying ephemeris as `moon_ephem`; it's
/// passed separately so we can read `fallback_count` after integration to
/// detect silent drop-through to Meeus.
#[cfg(feature = "fetch-horizons")]
fn verify_coast(
    label: &'static str,
    start_iso: &str,
    end_iso: &str,
    moon_ephem: &Arc<dyn MoonEphemeris>,
    moon_concrete: &Arc<HorizonsMoonEphemeris>,
) -> PhaseResult {
    println!("── {label} ──");
    println!("  window: {start_iso}  →  {end_iso}");

    let start_epoch = Epoch::from_iso8601(start_iso).expect("valid phase start");
    let end_epoch = Epoch::from_iso8601(end_iso).expect("valid phase end");
    let duration_seconds = (end_epoch.jd() - start_epoch.jd()) * 86_400.0;
    let duration_days = duration_seconds / 86_400.0;

    let (start_pos, start_vel) =
        fetch_orion_sample(&start_epoch).expect("fetch Orion at phase start");
    let (end_pos, end_vel) = fetch_orion_sample(&end_epoch).expect("fetch Orion at phase end");

    // Record the fallback count before this phase so the post-propagation
    // delta is attributable to this call alone (not the whole mission).
    let fallbacks_before = moon_concrete.fallback_count();

    let system = build_artemis_system(start_epoch, moon_ephem);
    let initial_state = OrbitalState::new(start_pos, start_vel);

    let mut min_moon_distance = f64::MAX;
    let mut max_earth_distance: f64 = 0.0;
    let final_state = Dop853.integrate(
        &system,
        initial_state,
        0.0,
        duration_seconds,
        DT_SECONDS,
        |t, state| {
            let epoch = start_epoch.add_seconds(t);
            let moon_pos = moon_ephem.position_eci(&epoch);
            let moon_distance = (state.position() - moon_pos).magnitude();
            if moon_distance < min_moon_distance {
                min_moon_distance = moon_distance;
            }
            let earth_distance = state.position().magnitude();
            if earth_distance > max_earth_distance {
                max_earth_distance = earth_distance;
            }
        },
    );

    // Fallback sanity check — this phase must not have silently dropped
    // through to Meeus. Any nonzero delta means the Moon ephemeris window
    // was too narrow for the propagation.
    let fallbacks_after = moon_concrete.fallback_count();
    let fallback_delta = fallbacks_after - fallbacks_before;
    if fallback_delta > 0 {
        eprintln!(
            "  ⚠  Moon ephemeris fell back to Meeus {fallback_delta} time(s) during {label} \
             — result is contaminated by analytical Moon. Widen MOON_WINDOW_* and rerun."
        );
        std::process::exit(1);
    }

    let position_error = (final_state.position() - end_pos).magnitude();
    let velocity_error = (final_state.velocity() - end_vel).magnitude();
    let judgment = Judgment::from_error_km(position_error);

    println!(
        "  duration: {:.2} days    samples: dt = {:.0}s",
        duration_days, DT_SECONDS
    );
    println!(
        "  closest Moon approach: {:10.1} km    max Earth distance: {:10.1} km",
        min_moon_distance, max_earth_distance
    );
    println!(
        "  position error:  {:10.3} km         velocity error: {:.6} km/s   {}",
        position_error,
        velocity_error,
        judgment.glyph()
    );

    PhaseResult {
        label,
        duration_days,
        position_error_km: position_error,
        velocity_error_kms: velocity_error,
        min_moon_distance_km: min_moon_distance,
        max_earth_distance_km: max_earth_distance,
        judgment,
    }
}

#[cfg(feature = "fetch-horizons")]
fn print_summary(results: &[PhaseResult]) {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Summary");
    println!("═══════════════════════════════════════════════════════════════════");
    println!(
        "{:<24}  {:>8}  {:>12}  {:>12}  {:>11}  {:>11}  {}",
        "Phase", "Days", "Pos err km", "Vel err km/s", "Min moon km", "Max earth km", "Judgment"
    );
    println!("{}", "-".repeat(100));
    for r in results {
        println!(
            "{:<24}  {:>8.2}  {:>12.3}  {:>12.6}  {:>11.0}  {:>11.0}  {}",
            r.label,
            r.duration_days,
            r.position_error_km,
            r.velocity_error_kms,
            r.min_moon_distance_km,
            r.max_earth_distance_km,
            r.judgment.glyph(),
        );
    }
    println!();

    // Overall judgment = worst per-phase.
    let overall = results
        .iter()
        .map(|r| r.judgment)
        .max_by_key(|j| match j {
            Judgment::Pass => 0,
            Judgment::Conditional => 1,
            Judgment::Fail => 2,
        })
        .unwrap_or(Judgment::Pass);

    print!("Overall judgment: ");
    match overall {
        Judgment::Pass => println!(
            "✓ PASS — all coast phases under {THRESHOLD_PASS_KM:.0} km. \
             Proceed with Earth-centric architecture for the full Artemis 1 example."
        ),
        Judgment::Conditional => println!(
            "? CONDITIONAL — at least one phase between {THRESHOLD_PASS_KM:.0} km and \
             {THRESHOLD_CONDITIONAL_KM:.0} km. Try tighter integrator tolerances before \
             proceeding."
        ),
        Judgment::Fail => println!(
            "✗ FAIL — at least one phase exceeds {THRESHOLD_CONDITIONAL_KM:.0} km. \
             Architecture change required (SOI switching, higher-order integrator, or \
             scope reduction)."
        ),
    }
    println!();
}

// ============================================================
// Helpers (only compiled with fetch-horizons)
// ============================================================

#[cfg(feature = "fetch-horizons")]
fn fetch_orion_sample(
    epoch: &Epoch,
) -> Result<(nalgebra::Vector3<f64>, nalgebra::Vector3<f64>), kaname::horizons::HorizonsError> {
    // Horizons requires start != stop; request a 1-minute bracket and pick
    // the sample closest to the requested epoch.
    let start = epoch.add_seconds(-30.0);
    let stop = epoch.add_seconds(30.0);
    let table = HorizonsTable::fetch_vector_table(
        ORION_TARGET,
        EARTH_GEOCENTER,
        &start,
        &stop,
        "1m",
        None,
    )?;

    // `parse_csv` already errors out on an empty ephemeris block, so this
    // branch is defensive-only.
    let samples = table.samples();
    if samples.is_empty() {
        return Err(kaname::horizons::HorizonsError::NoData);
    }

    // Pick the sample whose JD is closest to the requested epoch. For round
    // epochs Horizons snaps to a step boundary and the first sample is at
    // the requested time; for non-round epochs the nearest-sample picks the
    // best of the ≤ 2 candidates.
    let sample = samples
        .iter()
        .min_by(|a, b| {
            let da = (a.epoch.jd() - epoch.jd()).abs();
            let db = (b.epoch.jd() - epoch.jd()).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("non-empty samples");

    // Alignment assertion: fail loudly if Horizons returned a sample outside
    // the ±30 s window we requested. This would indicate that step-boundary
    // snapping is more aggressive than expected or the query parameters are
    // being misinterpreted.
    let dt_seconds = (sample.epoch.jd() - epoch.jd()).abs() * 86_400.0;
    assert!(
        dt_seconds < 60.0,
        "Horizons sample at JD {:.9} is {:.1} s away from requested epoch JD {:.9}",
        sample.epoch.jd(),
        dt_seconds,
        epoch.jd(),
    );

    Ok((sample.position, sample.velocity))
}

#[cfg(feature = "fetch-horizons")]
fn build_artemis_system(epoch: Epoch, moon_ephem: &Arc<dyn MoonEphemeris>) -> OrbitalSystem {
    use kaname::body::KnownBody;
    use kaname::constants::{J2_EARTH, J3_EARTH, J4_EARTH, MU_EARTH};

    let earth = KnownBody::Earth;
    let props = earth.properties();

    OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: props.radius,
            j2: J2_EARTH,
            j3: Some(J3_EARTH),
            j4: Some(J4_EARTH),
        }),
    )
    .with_epoch(epoch)
    .with_model(ThirdBodyGravity::sun())
    .with_model(ThirdBodyGravity::moon_with_ephemeris(Arc::clone(
        moon_ephem,
    )))
    .with_body_radius(props.radius)
}

#[cfg(feature = "fetch-horizons")]
fn iso_short(epoch: &Epoch) -> String {
    // Use `DateTime`'s `Display` impl so sub-microsecond JD round-off does
    // not produce "HH:59:60"-style nonsense. The Display output is
    // `YYYY-MM-DDTHH:MM:SSZ`; swap `T` for a space and drop the trailing
    // `Z` for readability.
    let dt = epoch.to_datetime();
    format!("{dt}")
        .replace('T', " ")
        .trim_end_matches('Z')
        .to_string()
}
