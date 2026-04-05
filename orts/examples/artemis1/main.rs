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
//! 7. **For each modelled burn**: applies an impulsive Δv derived via
//!    Method B (see below) at the burn midpoint and verifies the
//!    post-burn state against Horizons.
//! 8. **Runs a multi-burn chain**: propagates `DRI → 6-day DRO coast →
//!    DRDI` end-to-end with each burn's pre-computed Method B Δv
//!    applied at the right midpoint, then verifies the chain's final
//!    state against Horizons. See [`verify_burn_chain`] for details.
//!
//! The "Overall judgment" printed by [`print_summary`] covers the
//! **coast phases only**. The individual-burn and chain summary tables
//! are independent quality reports, so a Conditional burn/chain result
//! does not invalidate a Pass overall judgment.
//!
//! ## Method B: reverse-engineering the propulsive Δv
//!
//! The sibling `extract_burns.py` script reports a raw `v_post − v_pre`
//! endpoint difference for each detected burn. This raw value includes
//! both the propulsive contribution and the gravitational velocity change
//! accumulated during the burn window (for DRI that's ~3 m/s of gravity
//! contamination and a ~1.3° direction error over a 25-minute window).
//! Applying the raw Δv directly as an impulsive jump would double-count
//! gravity because the integrator already integrates gravitational
//! acceleration on both sides of the jump.
//!
//! Instead, for each [`Maneuver`], [`verify_burn`] first runs a pure
//! coast from `pre_epoch` to `post_epoch` (no burn) to obtain
//! `v_pure_coast(post)`, then derives the true propulsive contribution:
//!
//!     Δv_corrected = v_horizons(post) − v_pure_coast(post)
//!
//! and applies that at `mid_epoch`. This makes the verification frame-
//! consistent even over longer windows, and — critically — makes the
//! technique chainable: each burn's Δv is purely propulsive, so velocity
//! error does not compound from one burn to the next. The per-burn log
//! prints both the raw magnitude and the corrected magnitude plus the
//! raw→corrected angular error so the user can see how much
//! gravitational contamination the extractor carried.
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
// Maneuver (impulsive burn applied during propagation)
// ============================================================

/// An impulsive maneuver scheduled for the mission.
///
/// ## Semantics: raw fields are advisory, Method B computes the truth at runtime
///
/// [`Maneuver::raw_dv_eci_ms`] and [`Maneuver::raw_magnitude_ms`] store the
/// **raw extractor output** from `extract_burns.py`, i.e.
/// `v_horizons(post) − v_horizons(pre)` including both the propulsive
/// contribution and the gravitational velocity change accumulated during
/// the burn window. These fields are **advisory only**:
/// they are printed in the per-burn log for comparison, but they are
/// *not* the Δv actually applied by the simulator.
///
/// At runtime [`verify_burn`] uses **Method B**: it runs a pure-coast pass
/// from `pre_epoch` to `post_epoch`, computes
/// `Δv_corrected = v_horizons(post) − v_pure_coast(post)`, and applies
/// *that* at `mid_epoch`. The corrected value is the pure propulsive
/// contribution because the integrator already handles gravitational
/// drift on both sides of the impulsive jump. The per-burn log reports
/// both the raw magnitude and the corrected magnitude plus the angular
/// difference between them so the user can see how much gravitational
/// contamination the extractor picked up.
///
/// For future burns this field may be removed entirely; for now it is
/// kept as a sanity-check cross-reference against the Python extractor.
#[cfg(feature = "fetch-horizons")]
#[derive(Debug, Clone, Copy)]
struct Maneuver {
    /// Human-readable label (e.g., "DRI").
    label: &'static str,
    /// ISO-8601 epoch where the impulsive Δv is applied.
    ///
    /// Chosen as the burn midpoint reported by `extract_burns.py`. The
    /// impulsive-at-midpoint approximation has an irreducible position
    /// error floor — see [`BURN_THRESHOLD_PASS_KM`] for the bound.
    mid_epoch_iso: &'static str,
    /// ISO-8601 epoch to use as the initial state (Horizons fetch point
    /// that predates the burn by several minutes so the integrator can
    /// coast in to the impulsive application site).
    pre_epoch_iso: &'static str,
    /// ISO-8601 epoch to use as the verification target (Horizons fetch
    /// point that post-dates the burn by several minutes).
    post_epoch_iso: &'static str,
    /// **Advisory** — raw Δv vector in ECI/J2000 [m/s] as reported by
    /// `extract_burns.py --rust`. Not applied by the simulator; see
    /// struct-level docs. The simulator derives the actual Δv from the
    /// Horizons pre/post states via Method B.
    raw_dv_eci_ms: [f64; 3],
    /// **Advisory** — magnitude of [`Maneuver::raw_dv_eci_ms`].
    raw_magnitude_ms: f64,
}

/// Artemis 1 burns hardcoded from the `extract_burns.py` zoom output.
///
/// ## Current scope (iteration 1)
///
/// Only the DRO Insertion (DRI) burn is modelled. The pre/post epochs
/// are chosen ±12 minutes around the burn midpoint so the window contains
/// the ~5-minute real burn plus enough surrounding coast to verify the
/// integrator's re-entry into steady state.
///
/// Future iterations will add TLI, OPF, DRDI, RPF, and EI, plus the
/// smaller OTC/RTC corrections. When that happens `extract_burns.py`
/// should emit the pre/post boundaries directly so this constant can be
/// auto-generated.
///
/// **Method B is used**: the raw Δv fields are advisory; the simulator
/// reconstructs the true propulsive Δv at runtime. See [`Maneuver`] and
/// [`verify_burn`] for details.
#[cfg(feature = "fetch-horizons")]
const MANEUVERS: &[Maneuver] = &[
    Maneuver {
        label: "DRI (DRO insertion)",
        pre_epoch_iso: "2022-11-25T21:40:00Z",
        mid_epoch_iso: "2022-11-25T21:53:45Z",
        post_epoch_iso: "2022-11-25T22:05:00Z",
        raw_dv_eci_ms: [-48.686652, -87.093937, -39.758539],
        raw_magnitude_ms: 107.408033,
    },
    Maneuver {
        label: "DRDI (DRO departure)",
        pre_epoch_iso: "2022-12-01T21:42:00Z",
        mid_epoch_iso: "2022-12-01T21:54:00Z",
        post_epoch_iso: "2022-12-01T22:06:00Z",
        raw_dv_eci_ms: [136.880516, -5.517049, 1.842703],
        raw_magnitude_ms: 137.004047,
    },
];

/// Burn chain: verify DRI + DRO coast (~6 days) + DRDI as a single
/// continuous propagation.
///
/// The chain stresses the multi-burn pipeline end-to-end: two impulsive
/// burns with a multi-day coast segment in between. Each burn's corrected
/// Δv is computed via Method B in its own isolated short window (so the
/// computation stays clean even when the overall chain spans ~6 days).
/// The chain propagation then applies both at the appropriate midpoints
/// and compares the final state to Horizons.
///
/// ## Observed error breakdown (~6-day chain)
///
/// Running this chain produces ~1300 km total position error, roughly
/// decomposed as:
///
/// - ~7 km initial position offset from DRI's impulsive-midpoint
///   approximation
/// - That 7 km grows into ~1100 km over 6 days of DRO propagation
///   because the DRO phase space amplifies small perturbations along
///   unstable eigendirections (DRO is a marginally stable orbit in
///   three-body mechanics, not a perfectly stable attractor)
/// - ~125 km baseline DRO coast error from the integrator + ephemeris
///   (measured independently in the DRO coast phase verification)
/// - ~20 km from DRDI's impulsive-midpoint approximation
/// - Residual velocity error ~14 m/s at the chain endpoint: the
///   per-burn Method B is only exact at each burn's own narrow window,
///   so applying the pre-computed Δv inside a chain does not guarantee
///   velocity match at the chain's terminal epoch.
///
/// The chain lands at ~1300 km, just over the 1000 km Pass threshold
/// (result: Conditional). The architecture is viable; reducing the
/// error further would require modelling the burns as finite-duration
/// thrust rather than instantaneous impulses, which is a separate
/// iteration.
///
/// ## Thresholds
///
/// Uses the coast thresholds (1000 km Pass / 10000 km Conditional)
/// rather than the tight burn thresholds, because the chain
/// accumulates coast drift on top of each impulsive residual.
#[cfg(feature = "fetch-horizons")]
const BURN_CHAIN_INDICES: &[usize] = &[0, 1];

/// Thresholds for burn verification (km of position error at post_epoch).
///
/// These are tighter than the coast thresholds because the post-verification
/// window is intentionally short (~10 minutes), so any position error has
/// not had time to accumulate from e.g. ephemeris inaccuracies.
///
/// ## Irreducible impulsive-midpoint floor
///
/// Method B guarantees **velocity** match at `post_epoch` by construction,
/// but the **position** match is bounded below by the error of treating a
/// real multi-minute burn as a point event at `mid_epoch`. For an ideally
/// symmetric constant-thrust burn the first-order endpoint offset is zero;
/// the observed residual comes from (a) thrust-profile asymmetry shifting
/// the effective centroid away from `mid_epoch`, (b) ECI frame rotation
/// during the burn, and (c) position-dependence of the gravity field over
/// the burn's spatial extent. Empirically this is of order `V × Δt / 10`
/// km per burn, where `V` is the Δv magnitude in km/s and `Δt` is the
/// burn duration in seconds. For DRI (|Δv| ≈ 110 m/s, Δt ≈ 300 s) the
/// observed residual is ~7 km.
///
/// The thresholds are calibrated for DRI-sized burns. Larger burns
/// (TLI at ~3200 m/s, RPF at ~330 m/s) will produce proportionally
/// larger residuals; `BURN_THRESHOLD_PASS_KM` may need to be raised
/// or the burn modelled as a finite thrust to stay inside bounds.
#[cfg(feature = "fetch-horizons")]
const BURN_THRESHOLD_PASS_KM: f64 = 10.0;
#[cfg(feature = "fetch-horizons")]
const BURN_THRESHOLD_CONDITIONAL_KM: f64 = 100.0;

#[cfg(feature = "fetch-horizons")]
struct BurnResult {
    label: &'static str,
    /// Corrected (Method B) Δv magnitude [m/s] — the propulsive-only
    /// contribution actually applied by the simulator at `mid_epoch`,
    /// *not* the raw `v_post − v_pre` endpoint difference from the
    /// extractor. The raw magnitude is reported separately in the
    /// per-burn log but not retained in this struct.
    magnitude_ms: f64,
    pre_to_mid_seconds: f64,
    mid_to_post_seconds: f64,
    position_error_km: f64,
    velocity_error_kms: f64,
    judgment: Judgment,
}

#[cfg(feature = "fetch-horizons")]
impl Judgment {
    fn from_burn_error_km(position_error_km: f64) -> Self {
        if position_error_km < BURN_THRESHOLD_PASS_KM {
            Self::Pass
        } else if position_error_km < BURN_THRESHOLD_CONDITIONAL_KM {
            Self::Conditional
        } else {
            Self::Fail
        }
    }
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
    println!("[1/4] Fetching Moon ephemeris ({MOON_SAMPLE_STEP} spacing) from Horizons...");
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
    println!("[2/4] Fetching Orion reference state vectors at each phase endpoint...");
    println!();

    // ----- Verify each coast phase -----
    println!("[3/4] Propagating each coast phase and comparing to Horizons...");
    println!();

    let mut results: Vec<PhaseResult> = Vec::new();
    for (label, start_iso, end_iso) in COAST_PHASES {
        let result = verify_coast(label, start_iso, end_iso, &moon_ephem, &moon_concrete);
        results.push(result);
        println!();
    }

    // ----- Verify each maneuver (impulsive burn application) -----
    if !MANEUVERS.is_empty() {
        println!("[4/4] Verifying impulsive burn application for each maneuver...");
        println!();
    }

    let mut burn_results: Vec<BurnResult> = Vec::new();
    for burn in MANEUVERS {
        let result = verify_burn(burn, &moon_ephem, &moon_concrete);
        burn_results.push(result);
        println!();
    }

    // ----- Verify burn chains (end-to-end multi-burn propagation) -----
    let mut chain_results: Vec<BurnChainResult> = Vec::new();
    if BURN_CHAIN_INDICES.len() >= 2 {
        println!("── Burn chain verification ──");
        println!();
        let chain_burns: Vec<&Maneuver> =
            BURN_CHAIN_INDICES.iter().map(|&i| &MANEUVERS[i]).collect();
        let chain_label = chain_burns
            .iter()
            .map(|b| b.label.split_whitespace().next().unwrap_or(b.label))
            .collect::<Vec<_>>()
            .join(" → ");
        let result = verify_burn_chain(&chain_label, &chain_burns, &moon_ephem, &moon_concrete);
        chain_results.push(result);
        println!();
    }

    // ----- Summary tables -----
    print_summary(&results);
    if !burn_results.is_empty() {
        print_burn_summary(&burn_results);
    }
    if !chain_results.is_empty() {
        print_chain_summary(&chain_results);
    }
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

// ============================================================
// Burn verification
// ============================================================

/// Propagate a single maneuver: coast from `pre_epoch` to `mid_epoch`,
/// apply the impulsive Δv, coast from `mid_epoch` to `post_epoch`, and
/// compare to the Horizons reference at `post_epoch`.
///
/// The Δv applied at the midpoint is **not** the raw
/// `v_post_horizons − v_pre_horizons` endpoint difference — that would
/// double-count the gravitational velocity change the integrator already
/// accumulates during the pre→mid and mid→post coast segments. Instead
/// we back out the *propulsive-only* component by running a first pass
/// that coasts the entire pre→post window with no burn, and setting
///
///     Δv_propulsive = v_post_horizons − v_post_pure_coast
///
/// This is the impulsive-equivalent contribution the engine actually
/// added. Applying it at the midpoint is still an approximation (the
/// real burn acts over several minutes), but it isolates the direction
/// and magnitude so the result is frame-consistent even over longer
/// verification windows.
///
/// The raw Δv from `extract_burns.py` is retained in [`Maneuver`] as an
/// advisory starting point and printed alongside the corrected value so
/// the user can see how much gravitational contamination the raw
/// extractor data carried.
#[cfg(feature = "fetch-horizons")]
fn verify_burn(
    burn: &Maneuver,
    moon_ephem: &Arc<dyn MoonEphemeris>,
    moon_concrete: &Arc<HorizonsMoonEphemeris>,
) -> BurnResult {
    println!("── {} ──", burn.label);
    println!(
        "  pre → mid → post: {}  {}  {}",
        burn.pre_epoch_iso, burn.mid_epoch_iso, burn.post_epoch_iso
    );

    let pre_epoch = Epoch::from_iso8601(burn.pre_epoch_iso).expect("valid pre epoch");
    let mid_epoch = Epoch::from_iso8601(burn.mid_epoch_iso).expect("valid mid epoch");
    let post_epoch = Epoch::from_iso8601(burn.post_epoch_iso).expect("valid post epoch");
    let pre_to_mid_seconds = (mid_epoch.jd() - pre_epoch.jd()) * 86_400.0;
    let mid_to_post_seconds = (post_epoch.jd() - mid_epoch.jd()) * 86_400.0;
    let pre_to_post_seconds = pre_to_mid_seconds + mid_to_post_seconds;
    assert!(
        pre_to_mid_seconds > 0.0 && mid_to_post_seconds > 0.0,
        "burn epochs out of order: pre={} mid={} post={}",
        burn.pre_epoch_iso,
        burn.mid_epoch_iso,
        burn.post_epoch_iso,
    );

    // Fetch Horizons endpoints.
    let (pre_pos, pre_vel) = fetch_orion_sample(&pre_epoch).expect("fetch Orion at burn pre");
    let (post_pos, post_vel) = fetch_orion_sample(&post_epoch).expect("fetch Orion at burn post");

    // Record fallback count so we can attribute any drop-through to this burn.
    let fallbacks_before = moon_concrete.fallback_count();

    // ----- Pure-coast reference pass (pre → post, no burn) -----
    // Used to derive the propulsive Δv by comparing to the Horizons post
    // state: the difference is exactly what the burn contributed above and
    // beyond the gravitational drift the integrator already captures.
    let system = build_artemis_system(pre_epoch, moon_ephem);
    let initial_state = OrbitalState::new(pre_pos, pre_vel);
    let pure_coast_state = Dop853.integrate(
        &system,
        initial_state.clone(),
        0.0,
        pre_to_post_seconds,
        DT_SECONDS,
        |_, _| {},
    );
    let dv_corrected_kms = post_vel - pure_coast_state.velocity();
    let dv_corrected_ms = dv_corrected_kms * 1000.0;
    let dv_corrected_mag_ms = dv_corrected_ms.magnitude();

    // Compare against the raw extractor direction (advisory).
    let dv_raw_ms = nalgebra::Vector3::new(
        burn.raw_dv_eci_ms[0],
        burn.raw_dv_eci_ms[1],
        burn.raw_dv_eci_ms[2],
    );
    let dv_raw_mag_ms = dv_raw_ms.magnitude();
    // Consistency guard: the hardcoded `raw_magnitude_ms` must match the
    // magnitude recomputed from the vector. Catches copy-paste errors
    // when a new maneuver is added by hand (or regenerated from
    // `extract_burns.py` with inconsistent fields).
    debug_assert!(
        (burn.raw_magnitude_ms - dv_raw_mag_ms).abs() < 1.0e-3,
        "Maneuver {:?}: raw_magnitude_ms ({}) differs from |raw_dv_eci_ms| ({})",
        burn.label,
        burn.raw_magnitude_ms,
        dv_raw_mag_ms,
    );
    let raw_vs_corrected_angle_deg = angle_between_deg(&dv_raw_ms, &dv_corrected_ms);
    let raw_vs_corrected_mag_diff_ms = dv_raw_mag_ms - dv_corrected_mag_ms;

    // ----- Actual run: coast pre → mid → apply(corrected Δv) → mid → post -----
    let state_at_mid = Dop853.integrate(
        &system,
        initial_state,
        0.0,
        pre_to_mid_seconds,
        DT_SECONDS,
        |_, _| {},
    );
    let state_after_burn = state_at_mid.apply_delta_v(dv_corrected_kms);

    // Rebuild the system anchored at `mid_epoch`. This is **required for
    // correctness**, not just style: `OrbitalSystem` stores its reference
    // epoch in `epoch_0` and passes `epoch_0.add_seconds(t)` to each
    // force model's `eval()`. The mid→post integration restarts `t` from
    // 0, so without re-anchoring the Moon/Sun third-body closures would
    // be queried at `pre_epoch + t` during the mid→post leg — i.e., they
    // would return Moon positions from ~13 minutes earlier, offsetting
    // the lunar ECI position by ~800 km at DRO distance.
    let system_after = build_artemis_system(mid_epoch, moon_ephem);
    let final_state = Dop853.integrate(
        &system_after,
        state_after_burn,
        0.0,
        mid_to_post_seconds,
        DT_SECONDS,
        |_, _| {},
    );

    // Fallback sanity check.
    let fallbacks_after = moon_concrete.fallback_count();
    let fallback_delta = fallbacks_after - fallbacks_before;
    if fallback_delta > 0 {
        eprintln!(
            "  ⚠  Moon ephemeris fell back to Meeus {fallback_delta} time(s) during \
             {} burn verification.",
            burn.label
        );
        std::process::exit(1);
    }

    // Compare against Horizons.
    let position_error = (final_state.position() - post_pos).magnitude();
    let velocity_error = (final_state.velocity() - post_vel).magnitude();
    let judgment = Judgment::from_burn_error_km(position_error);

    println!(
        "  |Δv| raw (extractor):  {:>7.3} m/s    pre→mid: {:.0}s    mid→post: {:.0}s",
        dv_raw_mag_ms, pre_to_mid_seconds, mid_to_post_seconds
    );
    println!(
        "  |Δv| corrected (true): {:>7.3} m/s    raw→corrected: Δmag {:+.3} m/s, angle {:.3}°",
        dv_corrected_mag_ms, raw_vs_corrected_mag_diff_ms, raw_vs_corrected_angle_deg,
    );
    println!(
        "  position error:  {:10.3} km         velocity error: {:.6} km/s   {}",
        position_error,
        velocity_error,
        judgment.glyph()
    );

    BurnResult {
        label: burn.label,
        magnitude_ms: dv_corrected_mag_ms,
        pre_to_mid_seconds,
        mid_to_post_seconds,
        position_error_km: position_error,
        velocity_error_kms: velocity_error,
        judgment,
    }
}

// ============================================================
// Burn chain verification
// ============================================================

/// Helper that pre-computes a single maneuver's corrected Δv in its own
/// isolated Method B pass. Used by [`verify_burn_chain`] so each burn's
/// Δv is derived from a tight window around that burn even when the
/// chain as a whole spans multiple days.
#[cfg(feature = "fetch-horizons")]
fn compute_corrected_dv(
    burn: &Maneuver,
    moon_ephem: &Arc<dyn MoonEphemeris>,
) -> nalgebra::Vector3<f64> {
    let pre_epoch = Epoch::from_iso8601(burn.pre_epoch_iso).expect("valid pre epoch");
    let post_epoch = Epoch::from_iso8601(burn.post_epoch_iso).expect("valid post epoch");
    let window_seconds = (post_epoch.jd() - pre_epoch.jd()) * 86_400.0;

    let (pre_pos, pre_vel) = fetch_orion_sample(&pre_epoch).expect("fetch Orion at burn pre");
    let (_post_pos, post_vel) = fetch_orion_sample(&post_epoch).expect("fetch Orion at burn post");

    let system = build_artemis_system(pre_epoch, moon_ephem);
    let pure_coast_state = Dop853.integrate(
        &system,
        OrbitalState::new(pre_pos, pre_vel),
        0.0,
        window_seconds,
        DT_SECONDS,
        |_, _| {},
    );
    // Return in km/s (same units as OrbitalState).
    post_vel - pure_coast_state.velocity()
}

#[cfg(feature = "fetch-horizons")]
struct BurnChainResult {
    label: String,
    n_burns: usize,
    duration_days: f64,
    position_error_km: f64,
    velocity_error_kms: f64,
    judgment: Judgment,
}

/// Propagate a chain of maneuvers end-to-end through multiple coast
/// segments and verify the final state against Horizons.
///
/// Each burn's corrected Δv is computed independently via Method B using
/// its own tight pre/post window (see [`compute_corrected_dv`]) so the
/// Δv values stay clean even when the chain as a whole spans many days.
///
/// The actual chain propagation then:
///
/// 1. Fetches the initial state at `burns[0].pre_epoch_iso`.
/// 2. Coasts forward to `burns[0].mid_epoch_iso` and applies the
///    pre-computed Δv for burn 0.
/// 3. Coasts to `burns[1].mid_epoch_iso` and applies burn 1, etc.
/// 4. After the last burn, coasts to `burns[last].post_epoch_iso`.
/// 5. Compares the final state to Horizons at that epoch.
///
/// Uses the coast-phase thresholds (`THRESHOLD_PASS_KM` = 1000 km) rather
/// than the tight burn thresholds because the chain accumulates
/// coast-propagation error over multi-day segments on top of each
/// burn's impulsive residual.
#[cfg(feature = "fetch-horizons")]
fn verify_burn_chain(
    label: &str,
    burns: &[&Maneuver],
    moon_ephem: &Arc<dyn MoonEphemeris>,
    moon_concrete: &Arc<HorizonsMoonEphemeris>,
) -> BurnChainResult {
    assert!(
        !burns.is_empty(),
        "burn chain must contain at least one burn"
    );

    println!("── {label} ──");
    for (i, b) in burns.iter().enumerate() {
        println!(
            "  burn {}: {} @ {}  →  post {}",
            i + 1,
            b.label,
            b.mid_epoch_iso,
            b.post_epoch_iso
        );
    }

    // Pre-compute each burn's corrected Δv in its own isolated window.
    let fallbacks_before_precompute = moon_concrete.fallback_count();
    let corrected_dvs: Vec<nalgebra::Vector3<f64>> = burns
        .iter()
        .map(|b| compute_corrected_dv(b, moon_ephem))
        .collect();
    for (b, dv) in burns.iter().zip(&corrected_dvs) {
        println!(
            "  corrected Δv[{}] = {:>8.3} m/s",
            b.label,
            dv.magnitude() * 1000.0
        );
    }

    // Chain endpoints.
    let chain_pre_epoch =
        Epoch::from_iso8601(burns[0].pre_epoch_iso).expect("valid chain pre epoch");
    let chain_post_epoch =
        Epoch::from_iso8601(burns[burns.len() - 1].post_epoch_iso).expect("valid chain post epoch");
    let total_seconds = (chain_post_epoch.jd() - chain_pre_epoch.jd()) * 86_400.0;
    let total_days = total_seconds / 86_400.0;
    println!(
        "  chain window: {} → {}  ({:.2} days)",
        burns[0].pre_epoch_iso,
        burns[burns.len() - 1].post_epoch_iso,
        total_days,
    );

    let (chain_pre_pos, chain_pre_vel) =
        fetch_orion_sample(&chain_pre_epoch).expect("fetch Orion at chain pre");
    let (chain_post_pos, chain_post_vel) =
        fetch_orion_sample(&chain_post_epoch).expect("fetch Orion at chain post");

    let fallbacks_before_chain = moon_concrete.fallback_count();

    // Walk the chain: coast to each burn's mid, apply corrected Δv, rebuild
    // system with the new reference epoch for the next segment.
    let mut state = OrbitalState::new(chain_pre_pos, chain_pre_vel);
    let mut current_epoch = chain_pre_epoch;
    for (burn, dv_kms) in burns.iter().zip(&corrected_dvs) {
        let mid_epoch = Epoch::from_iso8601(burn.mid_epoch_iso).expect("valid burn mid epoch");
        let coast_seconds = (mid_epoch.jd() - current_epoch.jd()) * 86_400.0;
        assert!(
            coast_seconds > 0.0,
            "burns must be in ascending mid_epoch order (offender: {:?})",
            burn.label
        );

        let system = build_artemis_system(current_epoch, moon_ephem);
        state = Dop853.integrate(&system, state, 0.0, coast_seconds, DT_SECONDS, |_, _| {});
        state = state.apply_delta_v(*dv_kms);
        current_epoch = mid_epoch;
    }

    // Final coast from the last burn's midpoint to the chain's post epoch.
    let final_coast_seconds = (chain_post_epoch.jd() - current_epoch.jd()) * 86_400.0;
    assert!(
        final_coast_seconds > 0.0,
        "chain post epoch must follow last burn mid"
    );
    let final_system = build_artemis_system(current_epoch, moon_ephem);
    let final_state = Dop853.integrate(
        &final_system,
        state,
        0.0,
        final_coast_seconds,
        DT_SECONDS,
        |_, _| {},
    );

    let fallbacks_after = moon_concrete.fallback_count();
    let fallback_delta = fallbacks_after - fallbacks_before_precompute;
    let fallback_chain_delta = fallbacks_after - fallbacks_before_chain;
    if fallback_delta > 0 {
        eprintln!(
            "  ⚠  Moon ephemeris fell back to Meeus {fallback_delta} time(s) during \
             chain verification (chain-only: {fallback_chain_delta}). The Moon window \
             does not fully cover the chain span."
        );
        std::process::exit(1);
    }

    let position_error = (final_state.position() - chain_post_pos).magnitude();
    let velocity_error = (final_state.velocity() - chain_post_vel).magnitude();
    let judgment = Judgment::from_error_km(position_error);

    println!(
        "  position error:  {:10.3} km         velocity error: {:.6} km/s   {}",
        position_error,
        velocity_error,
        judgment.glyph()
    );

    BurnChainResult {
        label: label.to_string(),
        n_burns: burns.len(),
        duration_days: total_days,
        position_error_km: position_error,
        velocity_error_kms: velocity_error,
        judgment,
    }
}

#[cfg(feature = "fetch-horizons")]
fn print_chain_summary(results: &[BurnChainResult]) {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Burn chain summary");
    println!("═══════════════════════════════════════════════════════════════════");
    println!(
        "{:<28}  {:>7}  {:>8}  {:>12}  {:>12}  {}",
        "Chain", "#burns", "Days", "Pos err km", "Vel err km/s", "Judgment"
    );
    println!("{}", "-".repeat(90));
    for r in results {
        println!(
            "{:<28}  {:>7}  {:>8.2}  {:>12.3}  {:>12.6}  {}",
            r.label,
            r.n_burns,
            r.duration_days,
            r.position_error_km,
            r.velocity_error_kms,
            r.judgment.glyph(),
        );
    }
    println!();
}

/// Return the unsigned angle between two vectors in degrees. Uses a
/// numerically stable dot-product clamp.
#[cfg(feature = "fetch-horizons")]
fn angle_between_deg(a: &nalgebra::Vector3<f64>, b: &nalgebra::Vector3<f64>) -> f64 {
    let na = a.magnitude();
    let nb = b.magnitude();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    let cos = (a.dot(b) / (na * nb)).clamp(-1.0, 1.0);
    cos.acos().to_degrees()
}

#[cfg(feature = "fetch-horizons")]
fn print_burn_summary(results: &[BurnResult]) {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Burn summary");
    println!("═══════════════════════════════════════════════════════════════════");
    println!(
        "{:<24}  {:>11}  {:>11}  {:>11}  {:>12}  {:>12}  {}",
        "Burn", "|Δv| m/s", "pre→mid s", "mid→post s", "Pos err km", "Vel err km/s", "Judgment"
    );
    println!("{}", "-".repeat(105));
    for r in results {
        println!(
            "{:<24}  {:>11.3}  {:>11.0}  {:>11.0}  {:>12.3}  {:>12.6}  {}",
            r.label,
            r.magnitude_ms,
            r.pre_to_mid_seconds,
            r.mid_to_post_seconds,
            r.position_error_km,
            r.velocity_error_kms,
            r.judgment.glyph(),
        );
    }
    println!();
    println!(
        "Burn thresholds: < {:.0} km Pass | < {:.0} km Conditional | else Fail",
        BURN_THRESHOLD_PASS_KM, BURN_THRESHOLD_CONDITIONAL_KM,
    );
    println!();
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
