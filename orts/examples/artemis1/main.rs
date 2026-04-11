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
//! 1. Fetches a single Moon ephemeris (target `301`) and a single Sun
//!    ephemeris (target `10`) from JPL Horizons, both centred on Earth
//!    geocentre and sampled at 1-hour spacing over the full mission
//!    window. Both are used as tabulated sources for the third-body
//!    force model, replacing kaname's Meeus analytical models.
//! 2. For each coast window, fetches the real Orion state vector (target
//!    `-1023`) at both endpoints.
//! 3. Propagates the start state forward to the end epoch using `Dop853`
//!    with `dt = 10 s`, `J2/J3/J4`, Sun (Horizons-interpolated),
//!    Moon (Horizons-interpolated).
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
//! ## Error budget history
//!
//! Each row in the table below records the observed position error for a
//! given verification case after a specific iteration of the spike. The
//! goal is to track, with honest numbers, what each change actually bought
//! us — and what remains unaccounted-for — so future iterations can decide
//! where to spend effort next.
//!
//! | Case                     | Baseline (Meeus Sun/Moon)     | + Horizons Moon (d465b9c) | + dt = 1 s test (6460bab) | + Horizons Sun (527496b)  | + ConstantThrust chain (this iter) |
//! |--------------------------|-------------------------------|---------------------------|---------------------------|---------------------------|------------------------------------|
//! | Outbound coast (3 d)     | see apollo11 ~4 km precedent  | ≈ 36.5 km                 | 36.5 km (no change)       | 34.0 km (−2.5 km)         | 34.0 km (no change)                |
//! | DRO coast (5 d)          | would diverge / not attempted | 125.4 km                  | 125.4 km (no change)      | 96.2 km (−29 km, −23 %)   | 96.2 km (no change)                |
//! | Return coast (4 d)       | not attempted                 | 119.4 km                  | 119.4 km (no change)      | 115.2 km (−4.2 km)        | 115.2 km (no change)               |
//! | DRI single burn (25 min) | —                             | 7.43 km                   | 7.43 km (no change)       | 7.43 km (no change)       | 7.43 km (no change)                |
//! | DRDI single burn (24 min)| —                             | 20.44 km                  | 20.44 km (no change)      | 20.44 km (no change)      | 20.44 km (no change)               |
//! | **Chain DRI → DRDI (6 d)**| —                            | 1317.2 km                 | 1317.2 km (no change)     | 1266.7 km (−50 km)        | **1266.7 km (no change — see §)**  |
//!
//! ### What each iteration taught us
//!
//! **d465b9c — Moon: Meeus → Horizons** (≈ 4,000–15,000 km Meeus distance
//! error → tabulated ground truth with ~m level Hermite interp error).
//! Got the baseline architecture off the ground: without this, the DRO
//! phase would have diverged — the Meeus Moon is too coarse for the 6-day
//! coast at ~70,000 km lunar distance where third-body tidal terms are
//! dominant. This switch is what made the feasibility spike viable at all.
//!
//! **6460bab — dt = 1 s empirical test** (8th-order Dop853, expected per-
//! step truncation ≈ `(dt/τ)^9` ≈ 10⁻⁴⁶ for DRO dynamics). **Negative
//! result**: halving dt changes coast / chain errors by less than 1 m.
//! Step-size truncation is many orders of magnitude below any other error
//! source in the stack — tightening the integrator gains nothing. This
//! steered the next iteration away from Dop853 tuning and toward missing
//! physics.
//!
//! **Multi-impulse burn application (abandoned)**: tried splitting each
//! corrected Δv into `n=10` sub-impulses uniformly spread across the real
//! burn duration, expecting to reduce the ~7 km impulsive-midpoint
//! residual. **Mathematical equivalence**: for symmetric uniform impulse
//! distribution, the trajectory is identical to a single impulse at the
//! midpoint to first order in Δv — verified empirically, the two
//! approaches differed by 12 cm over the 1317 km chain.
//!
//! **ConstantThrust force model (this iter)** — the proper continuous-
//! thrust analogue of multi-impulse: install each burn as a
//! [`orts::perturbations::ConstantThrust`] that applies uniform
//! acceleration `Δv_corrected / burn_duration_s` over
//! `[mid_epoch − T/2, mid_epoch + T/2]`, and let the integrator take
//! its normal steps through the burn. See [`verify_burn_chain_continuous`].
//!
//! **Result: bit-identical to the impulsive chain** (1266.657 km in both).
//! This matches theory: for a symmetric uniform-thrust profile the
//! position trajectory through the burn differs from impulsive-at-mid
//! only at second order in Δv, and the gravity-gradient contribution
//! over the ~80–100 s burn window is sub-mm at DRO gradients (gradient
//! ~10⁻¹⁰ s⁻² × 0.75 km mean-position offset × 82 s ≈ 6 μm/s velocity
//! error, propagated to ~3 m of position over 6 days — negligible
//! vs. the 1266.7 km chain observable).
//!
//! **Important side finding — `dt` vs burn_duration discontinuity**:
//! initially the continuous chain was built as a single
//! `Dop853.integrate` call with all thrust models installed; this
//! produced 1812 km (500 km worse than impulsive) because a fixed-step
//! integrator crossing a discontinuous-force boundary mid-step gets the
//! partial-coverage steps wrong. At `burn_duration = 1 s << dt = 10 s`
//! the error became catastrophic (73,706 km). The fix is to **split
//! propagation at burn boundaries**: coast legs run `build_artemis_system`
//! alone, burn legs add a ConstantThrust model and use `burn_dt = 1 s`
//! to get ~100 internal steps per short burn. With splits, continuous
//! and impulsive agree to machine precision, confirming the
//! physics-level equivalence for symmetric profiles.
//!
//! **What would actually help**: the ~7 km DRI residual and its DRO-
//! amplified ~1100 km chain contribution come from **real burn profile
//! asymmetry** (OMS-E has ramp-up and ramp-down phases) and from the
//! `extract_burns.py` mid_epoch being the *geometric* centre of the
//! detected velocity-discontinuity cluster rather than the *thrust
//! centroid*. A non-uniform thrust profile (or better: fitting the
//! actual burn centroid from denser Horizons sampling inside the burn
//! window) is the next refinement. Neither `ConstantThrust` nor multi-
//! impulse can help while the profile is assumed symmetric.
//!
//! **This iteration — Sun: Meeus → Horizons** (~10 km Meeus geocentric
//! Sun error at 1 AU → tabulated ground truth). **-23 % DRO coast error**
//! (125 → 96 km). The Sun enters the dynamics through its tidal term
//! (`a = μ_sun [(r_body − r_sat)/|…|³ − r_body/|r_body|³]`), which at
//! DRO's ~400,000 km Earth distance contributes ≈ 5×10⁻⁷ m/s² × 5 days
//! ≈ 5 m/s velocity drift and ~50 km position drift. The Meeus-vs-Horizons
//! discrepancy of ~10 km in Sun position translates to a proportional
//! fraction of that drift, roughly matching the observed 29 km
//! improvement. Individual burn verifications are unchanged because their
//! 24-minute windows are too short for solar-tidal effects to accumulate.
//!
//! ### What's still unaccounted for
//!
//! After all of the above, the DRO coast still sits at **96 km over
//! 5 days** (≈ 19 km/day) and the chain at **1267 km** (dominated by
//! DRI impulsive residual × DRO stability amplification, not coast
//! propagation). Remaining candidate sources in rough order of expected
//! impact:
//!
//! 1. **Solar Radiation Pressure (SRP)**: Orion has an effective area /
//!    mass ratio around 8×10⁻⁴ m²/kg, giving SRP ≈ 5×10⁻⁹ m/s². Over 5
//!    days that's ~200 m position drift on direct integration, but the
//!    tidal coupling and repeated integration through the eclipse /
//!    illumination boundary could push this into the low-km range.
//!    Currently missing from the force model entirely.
//! 2. **TDB / UTC time-scale handling**: Horizons queries use
//!    `TIME_TYPE=TDB` while our `Epoch` parses ISO 8601 strings as if
//!    they were UTC. TDB − UTC ≈ 69 s for modern epochs; at orbital
//!    velocity ~1 km/s that's ~69 km of position offset at the Horizons
//!    reference endpoints. Whether this actually produces error depends on
//!    internal consistency: if every fetch *and* the integrator's clock
//!    treat the Epoch's JD as TDB uniformly, the offset cancels out. If
//!    not, there's a hidden ~km error. Worth auditing.
//! 3. **Asymmetric burn profile modelling** (for the chain only): uniform
//!    continuous thrust has now been tried and is mathematically
//!    equivalent to impulsive-at-midpoint for symmetric profiles (see
//!    "ConstantThrust" paragraph above). Reducing the ~7 km DRI
//!    residual requires modelling the real burn's asymmetric thrust
//!    profile (ramp up / ramp down) OR fitting the actual thrust
//!    centroid from denser Horizons sampling inside the burn window
//!    rather than taking the geometric centre of the detected cluster.
//! 4. **Jupiter / Venus / Mercury third bodies**: tiny at this distance
//!    (~10⁻⁹ m/s² for Jupiter at 6 AU). Expected to contribute ≤ 1 km
//!    even over 5 days. Low priority.
//! 5. **Horizons Orion reference-trajectory uncertainty**: JPL's post-
//!    flight reconstruction is expected to be sub-km. This is a floor,
//!    not a bug: even a perfect propagator would not land at 0 km.
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
use orts::perturbations::{ConstantThrust, ThirdBodyGravity};
#[cfg(feature = "fetch-horizons")]
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
#[cfg(feature = "fetch-horizons")]
use orts::record::components::{BodyRadius, GravitationalParameter, Position3D};
#[cfg(feature = "fetch-horizons")]
use orts::record::entity_path::EntityPath;
#[cfg(feature = "fetch-horizons")]
use orts::record::recording::Recording;
#[cfg(feature = "fetch-horizons")]
use orts::record::timeline::TimePoint;
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

/// Sun JPL Horizons target ID.
///
/// The Sun's position is fetched from Horizons for the same reason
/// the Moon is: the kaname analytical Sun ephemeris (Meeus-based) is
/// only ~10 km accurate at 1 AU, and that accumulates through the
/// third-body tidal term over multi-day propagation.
#[cfg(feature = "fetch-horizons")]
const SUN_TARGET: &str = "10";

/// Horizons sample spacing for the Moon ephemeris table.
#[cfg(feature = "fetch-horizons")]
const MOON_SAMPLE_STEP: &str = "1h";

/// Dop853 propagation step size (same as apollo11).
///
/// Note: `Dop853::integrate` is a fixed-step driver (no adaptive error
/// control). Dop853 is 8th-order accurate per step, so local truncation
/// error is ~(dt/τ)^9 × orbital scale. For DRO at ~70,000 km from the
/// Moon with τ ≈ 10 days and `dt = 10 s`, this is ~10⁻⁴⁶ — far below
/// any other error source. An empirical test (commit 6460bab) confirmed
/// that reducing `dt` from 10 s to 1 s does **not** change the coast or
/// chain verification results to 3-decimal km precision, so step-size
/// truncation is not the bottleneck.
///
/// The currently observed errors (DRO coast ~96 km over 5 days, chain
/// DRI→DRDI ~1267 km over 6 days) come from sources other than
/// integrator precision. See the "Error budget history" section in the
/// module-level docstring for the running list of what's been addressed
/// and what remains. Short version of what's left:
///
/// - Solar radiation pressure is not in the force model (expected
///   impact: ~200 m direct + tidal coupling, possibly low km)
/// - TDB / UTC time-scale handling between Horizons and `Epoch` is
///   unaudited (potential ~km error if internally inconsistent)
/// - Asymmetric burn profile modelling — *symmetric* continuous
///   thrust has been tried via `ConstantThrust` and is bit-identical
///   to impulsive-at-midpoint, so only modelling the real ramp-up /
///   ramp-down profile (or fitting the actual thrust centroid from
///   dense Horizons sampling) would reduce the DRI residual
/// - Horizons Orion reference uncertainty itself (expected < 1 km floor)
#[cfg(feature = "fetch-horizons")]
const DT_SECONDS: f64 = 10.0;

/// RRD recording output interval [seconds].
///
/// The integrator runs at `DT_SECONDS = 10 s` but we subsample to
/// 60-second output for the Rerun recording. Logging every integrator
/// step for a 6-day chain (~52k steps × multiple entities) bloats the
/// RRD to several GB and adds no visual information — adjacent
/// subsamples differ by < 1 m of drift. 60 s matches Apollo 11's
/// `OUTPUT_INTERVAL` and gives ~8,600 samples for the chain window
/// (smooth trajectory visible in Rerun's 3D view, fine enough to
/// resolve burn onset / DRO loop shape).
#[cfg(feature = "fetch-horizons")]
const OUTPUT_INTERVAL: f64 = 60.0;

/// Dense Orion reference step for the chain-window Horizons fetch.
///
/// [`record_chain_trajectory`] logs the Horizons reference trajectory
/// alongside the propagated state at every [`OUTPUT_INTERVAL`]. Rather
/// than hitting Horizons once per output step (hundreds of HTTP
/// round-trips that also saturate the disk cache with tiny 2-sample
/// files), we fetch one dense table covering the whole chain window
/// at 1-minute resolution and Hermite-interpolate it in memory. The
/// cubic Hermite error at 60 s sample spacing is O(h^4) ≈ sub-metre for
/// Orion's smooth trajectory near the DRI/DRDI burns — below the
/// display resolution of any imaginable plot.
#[cfg(feature = "fetch-horizons")]
const ORION_REF_STEP: &str = "1m";

/// Output path for the Rerun RRD file produced at the end of `main`.
///
/// Matches the `apollo11.rrd` convention alongside the companion
/// example and is `.gitignore`d via the top-level ignore list.
#[cfg(feature = "fetch-horizons")]
const RRD_OUTPUT_PATH: &str = "orts/examples/artemis1/artemis1.rrd";

/// Mission epoch used as the `sim_time = 0` reference for the RRD
/// recording.
///
/// The recorded phases (outbound coast, DRI→DRDI chain, return coast)
/// sit on a continuous timeline whose origin is the first logged
/// sample. Using `COAST_PHASES[0]`'s start keeps the `sim_time` axis
/// readable (first sample at t = 0 days, last sample at ~22.96 days)
/// and matches the "outbound coast" phase the reader sees first.
#[cfg(feature = "fetch-horizons")]
const MISSION_EPOCH_ISO: &str = "2022-11-17T00:00:00Z";

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
    /// Chosen as the burn midpoint reported by `extract_burns.py`.
    /// An earlier iteration of this docstring claimed this had an
    /// "irreducible position error floor of order |Δv|·τ/√12" from
    /// the impulsive-at-midpoint approximation; that was falsified
    /// by [`verify_burn_continuous`] which showed impulsive and
    /// ConstantThrust continuous-uniform burns produce bit-identical
    /// post-burn positions (direct integration; see that function's
    /// docstring for the derivation). The current ~2.7 / 15.1 km
    /// verify_burn residuals come from other sources (Moon ephemeris
    /// interpolation across the burn window, Method B pure-coast
    /// reference precision, real burn profile asymmetry, etc),
    /// discussed in `BURN_THRESHOLD_PASS_KM`'s docstring.
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
    /// Approximate real engine burn duration [seconds].
    ///
    /// Used by the continuous-thrust chain verification
    /// ([`verify_burn_chain_continuous`]) to spread the corrected Δv as
    /// a `ConstantThrust` force model over
    /// `[mid_epoch − duration/2, mid_epoch + duration/2]` rather than
    /// applying it as a single impulse. The impulsive chain variant
    /// ([`verify_burn_chain`]) ignores this field.
    ///
    /// Estimate from extract_burns.py: `duration ≈ |Δv| / peak_rate`
    /// (|Δv| in m/s, peak_rate in m/s²). This underestimates the true
    /// duration by ~10-30 % because `peak_rate` is the peak thrust
    /// whereas the real burn profile has ramp-up / ramp-down phases
    /// that pull the average below the peak. For DRI/DRDI-sized burns
    /// the resulting uniform-thrust model is within a few percent of
    /// physical reality, which is adequate for a spike.
    burn_duration_s: f64,
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
    // DRI values regenerated via
    //   `extract_burns.py --zoom 2022-11-25T21:50:00Z --window-min 30
    //    --zoom-step-seconds 30 --rust`
    // after `extract_burns.py` was switched to `TIME_TYPE=UT`. Under the
    // old `TIME_TYPE=TDB` query, the same extraction emitted
    // `mid_epoch_iso: "2022-11-25T21:53:45Z"` — a string whose digits
    // were the burn event's **TDB** wall clock dressed as UTC. Parsed
    // as UTC by `kaname::Epoch::from_iso8601` (the only time scale
    // `Epoch` understands) that label sat 69 s after the real physical
    // burn midpoint, so `verify_burn`'s impulsive Δv landed in the
    // wrong place and produced |Δv| × 69 s ≈ 7 km of position error per
    // burn. The UT values below carry the correct UTC wall clock.
    Maneuver {
        label: "DRI (DRO insertion)",
        pre_epoch_iso: "2022-11-25T21:40:00Z",
        mid_epoch_iso: "2022-11-25T21:52:45Z",
        post_epoch_iso: "2022-11-25T22:05:00Z",
        raw_dv_eci_ms: [-49.186835, -88.041098, -40.193547],
        raw_magnitude_ms: 108.563811,
        // |Δv| / peak_rate = 108.564 / 1.341 m/s² ≈ 81 s, from the
        // extract_burns.py zoom output at 30-second resolution.
        burn_duration_s: 81.0,
    },
    // DRDI values regenerated via
    //   `extract_burns.py --zoom 2022-12-01T21:54:00Z --window-min 30
    //    --zoom-step-seconds 30 --rust`
    // (UT mode, same reasoning as DRI above). Old `TIME_TYPE=TDB`
    // values: mid `21:54:00Z`, |Δv| 137.004 m/s, peak_rate 1.336 m/s².
    // `extract_burns.py --rust` emitted `21:52:59.999Z`; we round to
    // `21:53:00Z` here for readability — the 1 ms shift produces at
    // most |Δv| × 1 ms ≈ 14 cm of position error, well below the
    // ~15 km residual on this burn.
    Maneuver {
        label: "DRDI (DRO departure)",
        pre_epoch_iso: "2022-12-01T21:42:00Z",
        mid_epoch_iso: "2022-12-01T21:53:00Z",
        post_epoch_iso: "2022-12-01T22:06:00Z",
        raw_dv_eci_ms: [136.053068, -5.478211, 1.835707],
        raw_magnitude_ms: 136.175688,
        // |Δv| / peak_rate = 136.176 / 1.353 m/s² ≈ 101 s.
        burn_duration_s: 101.0,
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
/// ## Sources of burn residual (not an irreducible impulsive-midpoint floor)
///
/// Method B guarantees **velocity** match at `post_epoch` by construction.
/// An earlier iteration of this docstring claimed the **position** match
/// was bounded below by an "irreducible impulsive-midpoint floor" of order
/// `|Δv| × τ / √12`, suggesting that modelling the burn as a finite
/// ConstantThrust would reduce it. **That claim was falsified** (see
/// [`verify_burn_continuous`] docstring for the derivation and the
/// empirical side-by-side comparison): for a symmetric uniform-thrust
/// burn the continuous trajectory and the impulsive-at-midpoint trajectory
/// produce bit-identical positions for all `t > mid_epoch + τ/2`, so the
/// finite-burn model contributes no improvement on its own.
///
/// The observed DRI ~2.7 km / DRDI ~15.1 km residuals (post commit 2ede30f)
/// therefore come from elsewhere. Best-guess attribution, pending
/// follow-up diagnosis:
/// - Moon ephemeris Hermite interpolation across the ~80 / 100 s burn
///   window (Horizons Moon table is sampled at 1 h; the interpolation
///   residual couples into the third-body tidal term).
/// - Method B pure-coast reference propagation precision over the
///   ~25-minute pre → post window (the `dv_corrected` depends on it).
/// - Real OMS-E thrust profile asymmetry (ramp-up / ramp-down): if the
///   physical thrust centroid differs from the geometric burn midpoint,
///   the uniform-constant model applied at `mid_epoch` shifts position
///   post-burn by `|Δv| × centroid_offset`.
/// - Moon-vicinity gravity gradient during DRDI (DRO departure is the
///   closest lunar approach in the chain).
/// - Integration step during the burn leg (`burn_dt = 1 s` for the
///   continuous path, `dt = 10 s` for the impulsive path — the latter
///   is the default propagation step since the impulse is instantaneous).
///
/// The thresholds are calibrated for DRI/DRDI-sized burns. Larger burns
/// (TLI at ~3200 m/s, RPF at ~330 m/s) will produce proportionally
/// larger residuals if the same error sources dominate.
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
    /// Pre→mid and mid→post leg durations in seconds. Carried here
    /// mostly for per-burn stdout logs inside `verify_burn` /
    /// `verify_burn_continuous`; `print_burn_summary` does not
    /// include them in the interleaved impulsive-vs-continuous table
    /// (the table is already wide and per-burn timing is repeated in
    /// the detailed log above).
    #[allow(dead_code)]
    pre_to_mid_seconds: f64,
    #[allow(dead_code)]
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

    // ----- Fetch one Sun ephemeris covering the whole mission -----
    // Mirrors the Moon fetch: the kaname analytical Sun (Meeus) is
    // only ~10-km accurate at 1 AU and contributes to observed coast /
    // chain error via the third-body tidal term. Using a Horizons
    // table here aligns the Sun position with JPL's reference
    // trajectory for the same reason the Moon was switched.
    println!("[1b/4] Fetching Sun ephemeris ({MOON_SAMPLE_STEP} spacing) from Horizons...");
    let sun_table = HorizonsTable::fetch_vector_table(
        SUN_TARGET,
        EARTH_GEOCENTER,
        &moon_window_start,
        &moon_window_stop,
        MOON_SAMPLE_STEP,
        None,
    )
    .expect("fetch Sun ephemeris");
    println!(
        "  {} samples over {} → {}",
        sun_table.samples().len(),
        iso_short(&moon_window_start),
        iso_short(&moon_window_stop),
    );
    let sun_table_arc: Arc<HorizonsTable> = Arc::new(sun_table);
    println!();

    // ----- Fetch Orion state vectors at every phase endpoint -----
    println!("[2/4] Fetching Orion reference state vectors at each phase endpoint...");
    println!();

    // ----- Verify each coast phase -----
    println!("[3/4] Propagating each coast phase and comparing to Horizons...");
    println!();

    let mut results: Vec<PhaseResult> = Vec::new();
    for (label, start_iso, end_iso) in COAST_PHASES {
        let result = verify_coast(
            label,
            start_iso,
            end_iso,
            &moon_ephem,
            &moon_concrete,
            &sun_table_arc,
        );
        results.push(result);
        println!();
    }

    // ----- Verify each maneuver (impulsive burn application) -----
    if !MANEUVERS.is_empty() {
        println!("[4/4] Verifying impulsive burn application for each maneuver...");
        println!();
    }

    let mut burn_results_impulsive: Vec<BurnResult> = Vec::new();
    let mut burn_results_continuous: Vec<BurnResult> = Vec::new();
    for burn in MANEUVERS {
        let result_impulsive = verify_burn(burn, &moon_ephem, &moon_concrete, &sun_table_arc);
        burn_results_impulsive.push(result_impulsive);
        println!();
        // Continuous-thrust variant immediately after so the reader
        // sees the impulsive residual and the finite-burn-modelled
        // residual side-by-side in the stdout log for each burn.
        let result_continuous =
            verify_burn_continuous(burn, &moon_ephem, &moon_concrete, &sun_table_arc);
        burn_results_continuous.push(result_continuous);
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
        let chain_label_impulsive = format!("{chain_label} (impulsive)");
        let result_impulsive = verify_burn_chain(
            &chain_label_impulsive,
            &chain_burns,
            &moon_ephem,
            &moon_concrete,
            &sun_table_arc,
        );
        chain_results.push(result_impulsive);
        println!();

        // Also run the continuous-thrust variant (burns as
        // ConstantThrust force models) to compare against impulsive.
        let chain_label_continuous = format!("{chain_label} (continuous)");
        let result_continuous = verify_burn_chain_continuous(
            &chain_label_continuous,
            &chain_burns,
            &moon_ephem,
            &moon_concrete,
            &sun_table_arc,
        );
        chain_results.push(result_continuous);
        println!();
    }

    // ----- Emit Rerun RRD for visualization -----
    //
    // Re-propagate the three verification phases (outbound coast,
    // DRI→DRDI chain, return coast) with recording hooks and save
    // one consolidated RRD. The second propagation is a negligible
    // CPU cost compared to the Horizons fetches the spike already
    // performs, and keeps the verification code above clean of
    // visualization concerns.
    //
    // `sim_time = 0` corresponds to `MISSION_EPOCH_ISO`; each phase
    // sets its own mission-elapsed-time base via
    // `recording.reset_for_phase` before propagating. The three
    // phases occupy disjoint windows on the sim_time axis with
    // unlogged gaps between them (verify_coast / verify_burn already
    // have their own stdout summaries for those gaps, and rendering
    // the omitted ~5-day coasts would triple the RRD size for no
    // visual benefit).
    println!("── Building Rerun RRD visualization ──");
    let mission_epoch = Epoch::from_iso8601(MISSION_EPOCH_ISO).expect("valid mission epoch");

    // Fetch one dense Orion reference table covering the **whole**
    // recorded span (outbound start → return end). A single fetch
    // lets `HorizonsTable::interpolate` handle every recorded phase
    // from memory with no per-step HTTP calls, and the cached CSV
    // (~5 MB for ~23 days × 1 min) is reused on subsequent runs.
    let ref_start_iso = COAST_PHASES[0].1; // outbound start
    let ref_stop_iso = COAST_PHASES[COAST_PHASES.len() - 1].2; // return end
    let ref_start = Epoch::from_iso8601(ref_start_iso)
        .expect("valid outbound start")
        .add_seconds(-60.0);
    let ref_stop = Epoch::from_iso8601(ref_stop_iso)
        .expect("valid return end")
        .add_seconds(60.0);
    println!("  fetching dense Orion reference table ({ORION_REF_STEP} spacing)…");
    let orion_ref_table = HorizonsTable::fetch_vector_table(
        ORION_TARGET,
        EARTH_GEOCENTER,
        &ref_start,
        &ref_stop,
        ORION_REF_STEP,
        None,
    )
    .expect("fetch Orion reference table");
    println!(
        "  {} Orion reference samples over {} → {}",
        orion_ref_table.samples().len(),
        iso_short(&ref_start),
        iso_short(&ref_stop),
    );

    // Build the recording skeleton. Earth and Moon mu / radius come
    // from `KnownBody::properties()` so the two entities use the same
    // source of truth; any future correction (e.g. switching to DE441
    // GM_EARTH) lands in both places at once.
    let mut rec = Recording::new();
    let earth_props = kaname::body::KnownBody::Earth.properties();
    let earth_path = EntityPath::parse("/world/earth");
    rec.log_static(&earth_path, &GravitationalParameter(earth_props.mu));
    rec.log_static(&earth_path, &BodyRadius(earth_props.radius));
    let moon_path = EntityPath::parse("/world/moon");
    let moon_props = kaname::body::KnownBody::Moon.properties();
    rec.log_static(&moon_path, &GravitationalParameter(moon_props.mu));
    rec.log_static(&moon_path, &BodyRadius(moon_props.radius));

    let mut chain_recording = ChainRecording::new(&mut rec, &orion_ref_table);

    // Helper closure: record a pure-coast phase into a phase-specific
    // slot of the entity tree. The `phase_key` becomes the final
    // path segment (`/world/sat/artemis1/<phase_key>`) so downstream
    // per-phase slicing in plot_trajectory.py can filter by entity
    // path, avoiding the sim_time-boundary collision problem.
    let record_fill = |phase_key: &'static str,
                       label: &'static str,
                       start_iso: &'static str,
                       end_iso: &'static str,
                       chain_recording: &mut ChainRecording| {
        let phase_start = Epoch::from_iso8601(start_iso).expect("valid fill start");
        let met = (phase_start.jd() - mission_epoch.jd()) * 86_400.0;
        chain_recording.reset_for_phase(met, phase_key);
        record_coast_phase(
            label,
            start_iso,
            end_iso,
            &moon_ephem,
            &sun_table_arc,
            chain_recording,
        );
    };

    // --- Phase sequence (covering the whole 2022-11-17 → 2022-12-10
    //     recorded mission window with no gaps) ---
    //
    // 1. Outbound coast (verified)          2022-11-17 → 2022-11-20
    // 2. Outbound-to-chain fill (unmodelled OPF on 2022-11-21 inside)
    //                                       2022-11-20 → 2022-11-25T21:40
    // 3. DRI → DRDI chain (verified)        2022-11-25T21:40 → 2022-12-01T22:06
    // 4. Chain-to-return fill (unmodelled RPF on 2022-12-05 inside)
    //                                       2022-12-01T22:06 → 2022-12-06
    // 5. Return coast (verified)            2022-12-06 → 2022-12-10
    //
    // Each phase starts fresh from the Horizons reference state at
    // its own start epoch so upstream errors do not accumulate into
    // the next phase; the fills 2 and 4 will visibly diverge from
    // the Horizons reference at ~1 day past their start because the
    // powered flyby Δv is missing from the force model.
    record_fill(
        "outbound",
        COAST_PHASES[0].0,
        COAST_PHASES[0].1,
        COAST_PHASES[0].2,
        &mut chain_recording,
    );

    if BURN_CHAIN_INDICES.len() >= 2 {
        let chain_burns: Vec<&Maneuver> =
            BURN_CHAIN_INDICES.iter().map(|&i| &MANEUVERS[i]).collect();
        let chain_pre_iso = chain_burns[0].pre_epoch_iso;
        let chain_post_iso = chain_burns[chain_burns.len() - 1].post_epoch_iso;

        // Fill between outbound end and chain pre. Contains the OPF
        // (Outbound Powered Flyby, 2022-11-21, ~210 m/s) which the
        // force model does not carry — the plot will show divergence
        // starting around MET ~5 days.
        record_fill(
            "opf_fill",
            "Outbound → chain fill (contains OPF 2022-11-21)",
            COAST_PHASES[0].2, // outbound end
            chain_pre_iso,
            &mut chain_recording,
        );

        let chain_pre = Epoch::from_iso8601(chain_pre_iso).expect("valid chain pre epoch");
        let met = (chain_pre.jd() - mission_epoch.jd()) * 86_400.0;
        chain_recording.reset_for_phase(met, "chain");
        println!("  recording DRI → DRDI chain ({chain_pre_iso} → {chain_post_iso})");
        record_chain_trajectory(
            &chain_burns,
            &moon_ephem,
            &sun_table_arc,
            &mut chain_recording,
        );

        // Fill between chain post and return start. Contains the RPF
        // (Return Powered Flyby, 2022-12-05, ~328 m/s) which the
        // force model also does not carry — divergence starts ~4
        // days after the fill begins.
        if COAST_PHASES.len() >= 3 {
            record_fill(
                "rpf_fill",
                "Chain → return fill (contains RPF 2022-12-05)",
                chain_post_iso,
                COAST_PHASES[2].1, // return start
                &mut chain_recording,
            );
        }
    }

    if COAST_PHASES.len() >= 3 {
        record_fill(
            "return",
            COAST_PHASES[2].0,
            COAST_PHASES[2].1,
            COAST_PHASES[2].2,
            &mut chain_recording,
        );
    }

    // Drop the ChainRecording so its mutable borrow of `rec` ends
    // before the save call below takes `&rec`.
    drop(chain_recording);

    println!("  saving RRD to {RRD_OUTPUT_PATH}");
    orts::record::rerun_export::save_as_rrd(&rec, "orts-artemis1", RRD_OUTPUT_PATH)
        .expect("save artemis1 RRD");
    println!();

    // ----- Summary tables -----
    print_summary(&results);
    if !burn_results_impulsive.is_empty() {
        print_burn_summary(&burn_results_impulsive, &burn_results_continuous);
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
    sun_table: &Arc<HorizonsTable>,
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

    let system = build_artemis_system(start_epoch, moon_ephem, sun_table);
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
            let moon_pos = moon_ephem.position_eci(&epoch).into_inner();
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
    sun_table: &Arc<HorizonsTable>,
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
    let system = build_artemis_system(pre_epoch, moon_ephem, sun_table);
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
    let system_after = build_artemis_system(mid_epoch, moon_ephem, sun_table);
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

/// Continuous-thrust variant of [`verify_burn`]: integrates the
/// corrected Δv as a [`ConstantThrust`] force model active over
/// `[mid_epoch − burn_duration/2, mid_epoch + burn_duration/2]`
/// instead of applying it as a single impulsive velocity jump at
/// `mid_epoch`.
///
/// ## What it **does not** do: reduce the post-burn residual
///
/// This function was added to falsify a widely-cited-internally
/// hypothesis that the DRI / DRDI `verify_burn` residuals (2.686 km
/// and 15.113 km as of commit 5867382) were dominated by the
/// impulsive-at-midpoint approximation error — i.e., that switching
/// to a finite-duration force model would reduce them to ~0. The
/// `|Δv| · τ / √12` scale (≈ 2.54 km for DRI, ≈ 3.97 km for DRDI)
/// was quoted in previous commit messages and the README as the
/// expected theoretical floor from this approximation.
///
/// **That hypothesis is wrong.** For a uniform-thrust burn window
/// `[mid − τ/2, mid + τ/2]` with constant acceleration
/// `a = Δv / τ`, direct integration gives
///
///     r(mid + τ/2) = r(mid − τ/2) + v₀ · τ + ½ · Δv · τ
///     r(t)         = r(mid + τ/2) + (v₀ + Δv) · (t − mid − τ/2)      (for t > mid + τ/2)
///                  = r₀ + v₀ · t + Δv · (t − mid)
///
/// Meanwhile the impulsive-at-midpoint trajectory is
///
///     r(t) = r₀ + v₀ · t + Δv · (t − mid)      (for t > mid)
///
/// **The two are bit-identical for all t > mid + τ/2.** The
/// `|Δv| · τ / √12` formula describes the RMS position error between
/// an impulse at an *uncertain* centroid (σ_centroid = τ / √12 for a
/// uniform time distribution) and the true centroid — which is
/// relevant only if the burn profile is *asymmetric* so the thrust
/// centroid differs from the geometric midpoint. For a symmetric
/// uniform burn the centroid is exactly the midpoint and the formula
/// gives the trivial zero — any resemblance between 2.54 km and the
/// DRI residual was a numerical coincidence.
///
/// Running the spike after adding this function confirmed the
/// analytical prediction: impulsive and continuous produce
/// bit-identical `position_error_km` for both DRI and DRDI, down to
/// the printed precision.
///
/// ## Why keep the function at all?
///
/// 1. **Hypothesis falsification**: the side-by-side stdout output
///    from [`print_burn_summary`] makes it immediately obvious to
///    future readers that the residual is **not** from the
///    impulsive-vs-continuous distinction and that effort should be
///    directed at other error sources (Moon ephemeris interpolation
///    inside the burn window, Method B reference precision, real
///    burn profile asymmetry, integrator step during the burn leg).
/// 2. **Eventual non-uniform thrust**: a future iteration that
///    models the real OMS-E ramp-up / ramp-down profile (or any
///    asymmetric thrust model) will use the same leg-splitting
///    skeleton this function establishes.
/// 3. **Lock-step with chain verification**: the single-burn
///    continuous path uses exactly the same `ConstantThrust`
///    construction and leg-splitting as
///    [`verify_burn_chain_continuous`], so it doubles as a
///    single-burn unit test of the chain path's burn leg.
///
/// ## Lock-step with `verify_burn_chain_continuous`
///
/// This function uses the same leg-splitting pattern and force-model
/// construction as [`verify_burn_chain_continuous`]'s inner burn leg,
/// so the single-burn and chain paths produce numerically identical
/// trajectories through each burn window. Corrected Δv is computed
/// via the same Method B pass as the impulsive [`verify_burn`] so
/// side-by-side comparison of the two functions isolates the
/// approximation effect (impulsive vs finite uniform thrust) from
/// everything else.
#[cfg(feature = "fetch-horizons")]
fn verify_burn_continuous(
    burn: &Maneuver,
    moon_ephem: &Arc<dyn MoonEphemeris>,
    moon_concrete: &Arc<HorizonsMoonEphemeris>,
    sun_table: &Arc<HorizonsTable>,
) -> BurnResult {
    println!("── {} (continuous) ──", burn.label);
    println!(
        "  pre → mid → post: {}  {}  {}    burn_duration: {:.0} s",
        burn.pre_epoch_iso, burn.mid_epoch_iso, burn.post_epoch_iso, burn.burn_duration_s
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

    // Burn window centred on `mid_epoch`, identical to the chain's
    // per-burn window construction in `verify_burn_chain_continuous`.
    let half = burn.burn_duration_s / 2.0;
    let burn_start = mid_epoch.add_seconds(-half);
    let burn_end = mid_epoch.add_seconds(half);
    let pre_to_burn_start_seconds = (burn_start.jd() - pre_epoch.jd()) * 86_400.0;
    let burn_seconds = (burn_end.jd() - burn_start.jd()) * 86_400.0;
    let burn_end_to_post_seconds = (post_epoch.jd() - burn_end.jd()) * 86_400.0;
    assert!(
        pre_to_burn_start_seconds > 0.0 && burn_end_to_post_seconds > 0.0,
        "verify_burn_continuous {:?}: burn window [{}, {}] (± {:.0} s around mid) \
         must fit strictly inside [{}, {}]",
        burn.label,
        iso_short(&burn_start),
        iso_short(&burn_end),
        half,
        burn.pre_epoch_iso,
        burn.post_epoch_iso,
    );

    // Fetch Horizons endpoints (same as impulsive verify_burn).
    let (pre_pos, pre_vel) = fetch_orion_sample(&pre_epoch).expect("fetch Orion at burn pre");
    let (post_pos, post_vel) = fetch_orion_sample(&post_epoch).expect("fetch Orion at burn post");

    let fallbacks_before = moon_concrete.fallback_count();

    // ----- Method B: same pure-coast Δv extraction as verify_burn --
    let initial_state = OrbitalState::new(pre_pos, pre_vel);
    let pure_coast_system = build_artemis_system(pre_epoch, moon_ephem, sun_table);
    let pure_coast_state = Dop853.integrate(
        &pure_coast_system,
        initial_state.clone(),
        0.0,
        pre_to_post_seconds,
        DT_SECONDS,
        |_, _| {},
    );
    let dv_corrected_kms = post_vel - pure_coast_state.velocity();
    let dv_corrected_mag_ms = dv_corrected_kms.magnitude() * 1000.0;

    // ----- Actual run: 3 legs split at the burn-window boundaries --
    //
    // Leg 1: coast pre → burn_start (no thrust).
    // Leg 2: integrate burn_start → burn_end with ConstantThrust
    //        installed so the whole leg sees a uniform force
    //        (Dop853's 12-stage cluster is only well-behaved when
    //        the force is uniform across the step; straddling the
    //        burn-window boundary produces catastrophic per-step
    //        errors — see the chain path's module-level docstring).
    // Leg 3: coast burn_end → post (no thrust).
    //
    // `build_artemis_system` is rebuilt at each leg start epoch so
    // the Moon/Sun third-body closures see an `epoch_0` that matches
    // the leg's own local-t=0; otherwise the Moon position would be
    // queried at `pre_epoch + t` throughout even during the mid→post
    // leg, mis-offsetting the lunar ECI position by ~800 km at DRO
    // distance (same issue `verify_burn` guards against with its
    // `system_after` rebuild).
    let leg1_system = build_artemis_system(pre_epoch, moon_ephem, sun_table);
    let state_at_burn_start = Dop853.integrate(
        &leg1_system,
        initial_state,
        0.0,
        pre_to_burn_start_seconds,
        DT_SECONDS,
        |_, _| {},
    );

    let thrust = ConstantThrust::new(burn.label, burn_start, burn_end, dv_corrected_kms);
    let burn_system = build_artemis_system(burn_start, moon_ephem, sun_table).with_model(thrust);
    // Small integrator step inside the burn: the burn is only
    // ~80–100 s, so the default 10 s step would only give 8–10
    // stages through the uniform-force region. 1 s gives ~100 steps
    // per burn with negligible total cost.
    let burn_dt = burn_seconds.min(1.0);
    let state_at_burn_end = Dop853.integrate(
        &burn_system,
        state_at_burn_start,
        0.0,
        burn_seconds,
        burn_dt,
        |_, _| {},
    );

    let leg3_system = build_artemis_system(burn_end, moon_ephem, sun_table);
    let final_state = Dop853.integrate(
        &leg3_system,
        state_at_burn_end,
        0.0,
        burn_end_to_post_seconds,
        DT_SECONDS,
        |_, _| {},
    );

    // Fallback sanity check (same shape as impulsive verify_burn).
    let fallbacks_after = moon_concrete.fallback_count();
    let fallback_delta = fallbacks_after - fallbacks_before;
    if fallback_delta > 0 {
        eprintln!(
            "  ⚠  Moon ephemeris fell back to Meeus {fallback_delta} time(s) during \
             {} continuous-burn verification.",
            burn.label
        );
        std::process::exit(1);
    }

    let position_error = (final_state.position() - post_pos).magnitude();
    let velocity_error = (final_state.velocity() - post_vel).magnitude();
    let judgment = Judgment::from_burn_error_km(position_error);

    // Print the uniform-burn RMS centroid scale `|Δv| · τ / √12` as
    // a *hypothetical ceiling* for the position error contributed by
    // thrust-profile asymmetry (only realised if the real burn
    // profile is maximally asymmetric). The function docstring
    // explains why the impulsive and continuous residuals are in
    // fact bit-identical for a symmetric uniform profile; this
    // printed number is retained because it is useful when an
    // asymmetric thrust model is eventually introduced.
    let finite_burn_scale_km =
        (dv_corrected_mag_ms / 1000.0) * burn.burn_duration_s / 12.0_f64.sqrt();
    println!(
        "  |Δv| corrected:  {:>7.3} m/s    burn leg: [{}, {}]  ({:.0} s, dt = {:.1} s)",
        dv_corrected_mag_ms,
        iso_short(&burn_start),
        iso_short(&burn_end),
        burn_seconds,
        burn_dt,
    );
    println!(
        "  |Δv|·τ/√12 = {:.3} km  (centroid-uncertainty ceiling for profile asymmetry, \
         *not* the impulsive-vs-uniform floor — see verify_burn_continuous docstring)",
        finite_burn_scale_km,
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
    sun_table: &Arc<HorizonsTable>,
) -> nalgebra::Vector3<f64> {
    let pre_epoch = Epoch::from_iso8601(burn.pre_epoch_iso).expect("valid pre epoch");
    let post_epoch = Epoch::from_iso8601(burn.post_epoch_iso).expect("valid post epoch");
    let window_seconds = (post_epoch.jd() - pre_epoch.jd()) * 86_400.0;

    let (pre_pos, pre_vel) = fetch_orion_sample(&pre_epoch).expect("fetch Orion at burn pre");
    let (_post_pos, post_vel) = fetch_orion_sample(&post_epoch).expect("fetch Orion at burn post");

    let system = build_artemis_system(pre_epoch, moon_ephem, sun_table);
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
    sun_table: &Arc<HorizonsTable>,
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
        .map(|b| compute_corrected_dv(b, moon_ephem, sun_table))
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

        let system = build_artemis_system(current_epoch, moon_ephem, sun_table);
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
    let final_system = build_artemis_system(current_epoch, moon_ephem, sun_table);
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

/// Continuous-thrust variant of [`verify_burn_chain`].
///
/// Instead of applying each burn's corrected Δv as a single impulsive
/// velocity jump at `mid_epoch`, this variant installs each burn as a
/// [`ConstantThrust`] force model active over
/// `[mid_epoch − burn_duration/2, mid_epoch + burn_duration/2]` and
/// propagates the chain in **legs split at every burn boundary** so
/// that each individual `Dop853.integrate` call sees a *uniform* force
/// model throughout its interval (either pure coast OR coast + one
/// active `ConstantThrust`).
///
/// The split is load-bearing for correctness: an earlier draft that
/// installed all burns in a single system and integrated end-to-end
/// produced catastrophic errors (1812 km → 73,706 km depending on
/// `burn_duration_s`) because the fixed-step Dop853 driver's stage
/// cluster straddled the burn boundary and mis-evaluated the step. See
/// the "Important side finding — `dt` vs burn_duration discontinuity"
/// subsection in the module-level docstring for the full story.
///
/// Within each leg the integrator takes normal steps: `DT_SECONDS`
/// during coast legs, and `burn_dt = 1 s` inside burn legs so the
/// short (~80-100 s) constant-force regions are integrated with many
/// clean steps. During a burn leg the force model adds a constant ECI
/// acceleration of `Δv_corrected / burn_duration` on top of
/// J2/J3/J4 + Sun + Moon.
///
/// ## Observed result
///
/// **Bit-identical to the impulsive chain** (1266.657 km in both).
/// Matches theory: for a symmetric uniform-thrust profile the
/// trajectory differs from impulsive-at-midpoint only at second order
/// in Δv, which for these burn sizes and the DRO gravity gradient
/// propagates to ~metre-scale position drift over 6 days — well below
/// the ~1267 km chain observable. The ~1100 km DRO-amplified chain
/// residual comes from real burn profile *asymmetry*, not from the
/// impulsive-vs-continuous distinction. See the module-level Error
/// budget history for the cross-iteration comparison.
///
/// Known limitations:
/// - `ConstantThrust` models uniform thrust only; real OMS-E burns
///   ramp up and down, leaving ~% level residuals.
/// - Thrust direction is fixed in ECI; if the spacecraft rotates
///   during the burn (e.g., guidance updates), this model can't track
///   it.
/// - The burn_duration_s stored on each `Maneuver` is estimated from
///   `|Δv| / peak_rate` via extract_burns.py, which underestimates
///   the real duration by ~10–30 % (peak > mean thrust). A wrong
///   duration shifts the effective thrust interval but should not
///   bias the final state if the integrated Δv matches.
#[cfg(feature = "fetch-horizons")]
fn verify_burn_chain_continuous(
    label: &str,
    burns: &[&Maneuver],
    moon_ephem: &Arc<dyn MoonEphemeris>,
    moon_concrete: &Arc<HorizonsMoonEphemeris>,
    sun_table: &Arc<HorizonsTable>,
) -> BurnChainResult {
    // MUST stay in lock-step with `record_chain_trajectory` below —
    // same integrator step sizes (`DT_SECONDS` / `burn_dt = 1 s`),
    // same burn-window construction (centred on `mid_epoch`), same
    // Method B precomputation of `corrected_dvs`, same leg splitting.
    // If you touch any of these here, mirror the change there so the
    // RRD visualization continues to represent exactly what the
    // verification function validated.
    assert!(
        !burns.is_empty(),
        "burn chain must contain at least one burn"
    );

    println!("── {label} ──");
    println!("  mode: continuous-thrust (burns as force-model ConstantThrusts)");
    for (i, b) in burns.iter().enumerate() {
        println!(
            "  burn {}: {} @ {}  burn_duration {:.0}s",
            i + 1,
            b.label,
            b.mid_epoch_iso,
            b.burn_duration_s,
        );
    }

    // Pre-compute each burn's corrected Δv in its own isolated window
    // (same Method B as the impulsive chain).
    let fallbacks_before_precompute = moon_concrete.fallback_count();
    let corrected_dvs: Vec<nalgebra::Vector3<f64>> = burns
        .iter()
        .map(|b| compute_corrected_dv(b, moon_ephem, sun_table))
        .collect();
    for (b, dv) in burns.iter().zip(&corrected_dvs) {
        println!(
            "  corrected Δv[{}] = {:>8.3} m/s",
            b.label,
            dv.magnitude() * 1000.0
        );
    }

    // Pre-compute each burn's start/end epochs (centred on mid_epoch
    // with width burn_duration_s) alongside the corrected Δv vectors.
    // We'll use these to split the chain propagation at every burn
    // boundary — without splitting, the fixed-step Dop853 driver sees
    // a discontinuous force at burn start/end and integrates the
    // partial-coverage boundary steps inaccurately (empirically this
    // degrades the chain by hundreds of km even for tame burns, and
    // catastrophically when burn_duration < dt).
    let burn_windows: Vec<(Epoch, Epoch, nalgebra::Vector3<f64>)> = burns
        .iter()
        .zip(&corrected_dvs)
        .map(|(burn, dv_kms)| {
            let mid = Epoch::from_iso8601(burn.mid_epoch_iso).expect("valid burn mid epoch");
            let half = burn.burn_duration_s / 2.0;
            let start = mid.add_seconds(-half);
            let end = mid.add_seconds(half);
            (start, end, *dv_kms)
        })
        .collect();

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

    // Walk the chain split at burn boundaries. For each burn we do
    // three propagation legs:
    //   1. coast from current_epoch → burn.start  (no thrust model)
    //   2. integrate burn.start → burn.end with a ConstantThrust
    //      installed for THIS burn only (constant force throughout the
    //      segment → no mid-step discontinuities)
    //   3. current_epoch = burn.end, loop to next burn
    // After the last burn, coast to chain_post_epoch.
    let mut state = OrbitalState::new(chain_pre_pos, chain_pre_vel);
    let mut current_epoch = chain_pre_epoch;
    for ((burn, (start, end, dv_kms)), _burn_idx) in burns.iter().zip(&burn_windows).zip(0..) {
        // Leg 1: coast to burn start.
        let coast_seconds = (start.jd() - current_epoch.jd()) * 86_400.0;
        assert!(
            coast_seconds > 0.0,
            "burn starts must be in ascending order and follow the chain start \
             (offender: {:?})",
            burn.label
        );
        let coast_system = build_artemis_system(current_epoch, moon_ephem, sun_table);
        state = Dop853.integrate(
            &coast_system,
            state,
            0.0,
            coast_seconds,
            DT_SECONDS,
            |_, _| {},
        );

        // Leg 2: integrate the burn itself with a ConstantThrust model
        // active for the entire segment. The force is constant across
        // this integrate() call so Dop853's 12-stage evaluation gives
        // the correct `a × duration = Δv` regardless of how many
        // internal steps it takes.
        let burn_seconds = (end.jd() - start.jd()) * 86_400.0;
        let thrust = ConstantThrust::new(burn.label, *start, *end, *dv_kms);
        let burn_system = build_artemis_system(*start, moon_ephem, sun_table).with_model(thrust);
        // Use a smaller dt inside the burn (1 s) because short burns
        // (~80-100 s) only get ~8-10 steps at the chain's dt=10s, and
        // the gravity-force curvature across the burn matters more in
        // that phase than during coast. This is cheap (100 extra steps
        // per burn vs. ~52k total in the chain).
        let burn_dt = burn_seconds.min(1.0);
        state = Dop853.integrate(&burn_system, state, 0.0, burn_seconds, burn_dt, |_, _| {});

        current_epoch = *end;
    }

    // Final coast from the last burn's end to the chain's post epoch.
    let final_coast_seconds = (chain_post_epoch.jd() - current_epoch.jd()) * 86_400.0;
    assert!(
        final_coast_seconds > 0.0,
        "chain post epoch must follow the last burn's end"
    );
    let final_system = build_artemis_system(current_epoch, moon_ephem, sun_table);
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
             continuous-thrust chain verification (chain-only: {fallback_chain_delta})."
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

// ============================================================
// Rerun RRD visualization
// ============================================================

/// Accumulated recording state for a chain propagation.
///
/// Holds the entity paths, the reference trajectory table, and the
/// time-step bookkeeping that threads through each
/// `Dop853.integrate` callback. `sim_t_offset` is the cumulative
/// wall-clock time at the start of the current leg so the `sim_time`
/// timeline index is continuous across the coast / burn / coast /
/// burn / coast legs of the chain (the integrator itself restarts
/// `t = 0` on each leg).
///
/// Logging is throttled by [`OUTPUT_INTERVAL`]: the integrator ticks
/// at `DT_SECONDS` (10 s) but the RRD only records every
/// `OUTPUT_INTERVAL` (60 s). This keeps the RRD file size manageable
/// without losing visible trajectory resolution — at DRO distances the
/// spacecraft moves < 60 m per integrator step, which is below the
/// screen pixel even in a close-up Earth view.
#[cfg(feature = "fetch-horizons")]
struct ChainRecording<'a> {
    rec: &'a mut Recording,
    orion_ref: &'a HorizonsTable,
    /// Per-phase entity paths. Updated by [`reset_for_phase`] so
    /// each mission phase logs into its own slot
    /// (`/world/sat/artemis1/<phase_key>` etc). This is load-bearing
    /// for downstream per-phase slicing: two adjacent phases whose
    /// throttled `maybe_log` cadence happens to align with the phase
    /// boundary would otherwise emit samples at exactly the same
    /// `sim_time`, and a `sim_time`-based slice cannot tell them
    /// apart. Using distinct entity paths makes the "which phase
    /// owns this sample" question trivially resolvable.
    sat_path: EntityPath,
    ref_path: EntityPath,
    err_path: EntityPath,
    moon_path: EntityPath,
    step: u64,
    /// Last logged `sim_time` in seconds. Initialized to a negative
    /// number larger than `OUTPUT_INTERVAL` so the very first call
    /// fires immediately regardless of the throttle.
    last_log_sim_t: f64,
    /// Cumulative sim_time in seconds at the start of the current
    /// integration leg. [`advance_leg`] bumps this by the leg's
    /// duration so the next leg's local `t` stacks on top.
    sim_t_offset: f64,
}

#[cfg(feature = "fetch-horizons")]
impl<'a> ChainRecording<'a> {
    fn new(rec: &'a mut Recording, orion_ref: &'a HorizonsTable) -> Self {
        Self {
            rec,
            orion_ref,
            // These placeholder paths are never written to — the
            // first `reset_for_phase` call updates them before any
            // force_log fires. Using dummy leaves the fields
            // non-Option and keeps the log_* call sites clean.
            sat_path: EntityPath::parse("/world/sat/artemis1/_placeholder"),
            ref_path: EntityPath::parse("/world/ref/artemis1/_placeholder"),
            err_path: EntityPath::parse("/world/analysis/error_km/_placeholder"),
            moon_path: EntityPath::parse("/world/moon/_placeholder"),
            step: 0,
            last_log_sim_t: f64::NEG_INFINITY,
            sim_t_offset: 0.0,
        }
    }

    /// Reset the per-phase bookkeeping before a new propagation
    /// segment. Sets `sim_t_offset` to the given mission-elapsed-time
    /// base (seconds since [`MISSION_EPOCH_ISO`] at the phase start),
    /// rewinds `last_log_sim_t` so the first callback of the phase
    /// unconditionally logs, and swaps the entity paths to
    /// phase-specific slots so per-phase slicing in the Python
    /// visualization can simply filter by entity path. The `step`
    /// counter is **not** reset so every entity gets monotonically
    /// increasing step indices across the whole mission timeline —
    /// Rerun uses that as a secondary index alongside `sim_time`.
    fn reset_for_phase(&mut self, mission_elapsed_seconds: f64, phase_key: &str) {
        self.sat_path = EntityPath::parse(&format!("/world/sat/artemis1/{phase_key}"));
        self.ref_path = EntityPath::parse(&format!("/world/ref/artemis1/{phase_key}"));
        self.err_path = EntityPath::parse(&format!("/world/analysis/error_km/{phase_key}"));
        self.moon_path = EntityPath::parse(&format!("/world/moon/{phase_key}"));
        self.sim_t_offset = mission_elapsed_seconds;
        self.last_log_sim_t = f64::NEG_INFINITY;
    }

    /// Rate-limited log: emits a frame only when at least
    /// `OUTPUT_INTERVAL` seconds of sim time have passed since the
    /// previous frame. Called from inside the integrator's callback.
    fn maybe_log(
        &mut self,
        local_t: f64,
        state: &OrbitalState,
        leg_start_epoch: Epoch,
        moon_ephem: &dyn MoonEphemeris,
    ) {
        let sim_t = self.sim_t_offset + local_t;
        if sim_t - self.last_log_sim_t < OUTPUT_INTERVAL {
            return;
        }
        self.force_log(local_t, state, leg_start_epoch, moon_ephem);
    }

    /// Unconditional log: bypasses the `OUTPUT_INTERVAL` throttle.
    /// Used for the very first and last samples of the chain so the
    /// endpoints are always present in the RRD regardless of where
    /// they land relative to the throttle cadence.
    ///
    /// Spacecraft and reference orbits are logged as
    /// [`RecordOrbitalState`] (position + velocity). Moon is logged
    /// as Position3D only since only its trajectory is needed.
    /// The error vector carries both Δ-position and Δ-velocity.
    fn force_log(
        &mut self,
        local_t: f64,
        state: &OrbitalState,
        leg_start_epoch: Epoch,
        moon_ephem: &dyn MoonEphemeris,
    ) {
        let sim_t = self.sim_t_offset + local_t;
        let epoch = leg_start_epoch.add_seconds(local_t);
        let tp = TimePoint::new().with_sim_time(sim_t).with_step(self.step);

        // Propagated spacecraft state.
        let os = RecordOrbitalState::new(*state.position(), *state.velocity());
        self.rec.log_orbital_state(&self.sat_path, &tp, &os);

        // Horizons reference via Hermite interpolation over the dense
        // ±1-minute chain-window table fetched once at `main` startup.
        // `interpolate` returns `None` only if the epoch falls outside
        // the table range, which is impossible here because we padded
        // the fetch window by ±1 minute in `main`.
        if let Some(sample) = self.orion_ref.interpolate(&epoch) {
            let ref_os = RecordOrbitalState::new(sample.position, sample.velocity);
            self.rec.log_orbital_state(&self.ref_path, &tp, &ref_os);

            // Error vector = (propagated − reference) in (km, km/s).
            // Logged as a full OrbitalState so both the position
            // residual and the velocity residual become time-series
            // channels in Rerun (rerun_export skips entities that are
            // position-only, see force_log docstring above). Magnitude
            // is derivable in the viewer via a computed field on the
            // individual x/y/z components.
            let err_pos = *state.position() - sample.position;
            let err_vel = *state.velocity() - sample.velocity;
            let err_os = RecordOrbitalState::new(err_pos, err_vel);
            self.rec.log_orbital_state(&self.err_path, &tp, &err_os);
        }

        // Moon trajectory. Logged on the same timeline as the
        // spacecraft so Rerun's 3D view can animate them together.
        // Only Position3D is needed — the generic column export no
        // longer requires a Position3D+Velocity3D pair.
        let moon_pos = moon_ephem.position_eci(&epoch).into_inner();
        self.rec
            .log_temporal(&self.moon_path, &tp, &Position3D(moon_pos));

        self.step += 1;
        self.last_log_sim_t = sim_t;
    }

    /// Bump `sim_t_offset` by the completed leg's duration. Called
    /// once after each `Dop853.integrate` call in
    /// [`record_chain_trajectory`].
    fn advance_leg(&mut self, leg_seconds: f64) {
        self.sim_t_offset += leg_seconds;
    }
}

/// Re-run the continuous-thrust DRI→DRDI chain propagation for
/// visualization, emitting Rerun log entries for the spacecraft
/// trajectory, the Horizons reference trajectory, the error vector,
/// and the Moon position alongside the integration.
///
/// ## Why a second propagation?
///
/// [`verify_burn_chain_continuous`] above already walks the same chain
/// for the error-budget summary, but it is structured around the
/// verification judgment (`BurnChainResult`) and does not carry any
/// recording state. Threading an optional `Recording` through it would
/// clutter the verification code path without benefit. The visualization
/// propagation is a few hundred milliseconds of extra wall clock
/// (6 days / 10 s / 2 = ~25,000 steps), which is negligible next to
/// the Horizons fetches the spike already performs.
///
/// ## What gets logged
///
/// - `/world/sat/artemis1` — propagated `OrbitalState` every
///   [`OUTPUT_INTERVAL`] (60 s).
/// - `/world/ref/artemis1` — Horizons reference `OrbitalState` at the
///   same epochs, via Hermite interpolation of the dense pre-fetched
///   chain-window table.
/// - `/world/analysis/error_km` — propagation error as an
///   `OrbitalState` archetype carrying (Δposition [km], Δvelocity
///   [km/s]). Both channels are necessary to satisfy
///   `rerun_export::save_as_rrd`'s "Position3D + Velocity3D present"
///   precondition; the Δvelocity axis is also independently useful in
///   Rerun's chart view for diagnosing burn residuals vs. coast drift.
/// - `/world/moon` — Moon `OrbitalState` at the same epochs (position
///   from the ephemeris, velocity by central finite difference), so
///   the 3D view can animate Moon motion alongside the spacecraft.
///
/// Earth is logged as static in `main` (body radius + µ) outside this
/// function because it never moves in the ECI frame.
#[cfg(feature = "fetch-horizons")]
fn record_chain_trajectory(
    burns: &[&Maneuver],
    moon_ephem: &Arc<dyn MoonEphemeris>,
    sun_table: &Arc<HorizonsTable>,
    recording: &mut ChainRecording,
) {
    // MUST stay in lock-step with `verify_burn_chain_continuous` above —
    // same integrator step sizes (`DT_SECONDS` / `burn_dt = 1 s`),
    // same burn-window construction (centred on `mid_epoch`), same
    // Method B precomputation of `corrected_dvs`, same leg splitting.
    // The RRD visualization is only useful as long as it represents
    // exactly what the verification path above validated against
    // Horizons.
    assert!(
        !burns.is_empty(),
        "record_chain_trajectory: burn chain must contain at least one burn"
    );

    // Recompute each burn's corrected Δv (Method B, same short window
    // as the verification path so the chain stays numerically
    // identical to `verify_burn_chain_continuous`).
    let corrected_dvs: Vec<nalgebra::Vector3<f64>> = burns
        .iter()
        .map(|b| compute_corrected_dv(b, moon_ephem, sun_table))
        .collect();

    let burn_windows: Vec<(Epoch, Epoch, nalgebra::Vector3<f64>)> = burns
        .iter()
        .zip(&corrected_dvs)
        .map(|(burn, dv_kms)| {
            let mid = Epoch::from_iso8601(burn.mid_epoch_iso).expect("valid burn mid epoch");
            let half = burn.burn_duration_s / 2.0;
            (mid.add_seconds(-half), mid.add_seconds(half), *dv_kms)
        })
        .collect();

    let chain_pre_epoch =
        Epoch::from_iso8601(burns[0].pre_epoch_iso).expect("valid chain pre epoch");
    let chain_post_epoch =
        Epoch::from_iso8601(burns[burns.len() - 1].post_epoch_iso).expect("valid chain post epoch");

    let (chain_pre_pos, chain_pre_vel) =
        fetch_orion_sample(&chain_pre_epoch).expect("fetch Orion at chain pre");

    let mut state = OrbitalState::new(chain_pre_pos, chain_pre_vel);
    let mut current_epoch = chain_pre_epoch;

    // Anchor the phase start with an unconditional log so the RRD's
    // first chain sample sits exactly at `chain_pre_epoch`. Caller
    // is expected to have already called `recording.reset_for_phase`
    // with the chain's mission-elapsed-time base.
    recording.force_log(0.0, &state, current_epoch, moon_ephem.as_ref());

    // Walk legs in lock-step with `verify_burn_chain_continuous` —
    // coast → burn → coast → burn → … → final coast.
    for (burn, (burn_start, burn_end, dv_kms)) in burns.iter().zip(&burn_windows) {
        // Leg A: coast to burn start.
        let coast_seconds = (burn_start.jd() - current_epoch.jd()) * 86_400.0;
        let coast_system = build_artemis_system(current_epoch, moon_ephem, sun_table);
        let leg_start = current_epoch;
        state = Dop853.integrate(
            &coast_system,
            state,
            0.0,
            coast_seconds,
            DT_SECONDS,
            |t, s| {
                recording.maybe_log(t, s, leg_start, moon_ephem.as_ref());
            },
        );
        recording.advance_leg(coast_seconds);
        current_epoch = *burn_start;

        // Leg B: burn window with ConstantThrust installed. Small
        // integrator step (1 s) because the burn is only ~80–100 s.
        let burn_seconds = (burn_end.jd() - burn_start.jd()) * 86_400.0;
        let thrust = ConstantThrust::new(burn.label, *burn_start, *burn_end, *dv_kms);
        let burn_system =
            build_artemis_system(*burn_start, moon_ephem, sun_table).with_model(thrust);
        let burn_dt = burn_seconds.min(1.0);
        let burn_leg_start = current_epoch;
        state = Dop853.integrate(&burn_system, state, 0.0, burn_seconds, burn_dt, |t, s| {
            recording.maybe_log(t, s, burn_leg_start, moon_ephem.as_ref());
        });
        recording.advance_leg(burn_seconds);
        current_epoch = *burn_end;
    }

    // Final coast to the chain post epoch. As with `record_coast_phase`
    // we intentionally skip a force_log of the endpoint — any phase
    // that follows this one (the RPF fill, in main's sequence) will
    // force_log its own start at the same sim_time and providing the
    // boundary sample from both sides would poison per-phase slicers
    // in plot_trajectory.py.
    let final_coast_seconds = (chain_post_epoch.jd() - current_epoch.jd()) * 86_400.0;
    let final_system = build_artemis_system(current_epoch, moon_ephem, sun_table);
    let final_leg_start = current_epoch;
    let _final_state = Dop853.integrate(
        &final_system,
        state,
        0.0,
        final_coast_seconds,
        DT_SECONDS,
        |t, s| {
            recording.maybe_log(t, s, final_leg_start, moon_ephem.as_ref());
        },
    );
}

/// Re-run a single coast phase (outbound or return) for
/// visualization, recording the trajectory into `recording` alongside
/// the Horizons reference, Moon, and error vector.
///
/// This is the pure-coast counterpart to [`record_chain_trajectory`]:
/// one leg, no burns, one [`Dop853::integrate`] call. The function
/// assumes `recording.reset_for_phase(...)` has been called by the
/// caller to set the mission-elapsed-time base for this phase.
///
/// Like [`record_chain_trajectory`] this is a second propagation
/// alongside [`verify_coast`] above; threading a `Recording` through
/// verify_coast would leak visualization concerns into the
/// verification code, and the extra ~few hundred millisecond CPU cost
/// is negligible next to the Horizons fetches.
#[cfg(feature = "fetch-horizons")]
fn record_coast_phase(
    label: &str,
    start_iso: &str,
    end_iso: &str,
    moon_ephem: &Arc<dyn MoonEphemeris>,
    sun_table: &Arc<HorizonsTable>,
    recording: &mut ChainRecording,
) {
    println!("  recording coast phase: {label} ({start_iso} → {end_iso})");
    let start_epoch = Epoch::from_iso8601(start_iso).expect("valid coast phase start");
    let end_epoch = Epoch::from_iso8601(end_iso).expect("valid coast phase end");
    let duration_seconds = (end_epoch.jd() - start_epoch.jd()) * 86_400.0;
    assert!(
        duration_seconds > 0.0,
        "coast phase end must follow start ({start_iso} → {end_iso})"
    );

    let (start_pos, start_vel) =
        fetch_orion_sample(&start_epoch).expect("fetch Orion at coast phase start");
    let state = OrbitalState::new(start_pos, start_vel);

    let system = build_artemis_system(start_epoch, moon_ephem, sun_table);

    // Anchor the phase start unconditionally so the RRD carries a
    // sample at exactly `start_epoch` for the first frame. We do
    // **not** force_log the endpoint here: if another phase follows
    // this one, its own `force_log` at start provides the exact
    // boundary sample, and having both ends duplicate the boundary
    // sim_time would poison per-phase slicers that can't otherwise
    // distinguish samples from neighbouring phases.
    recording.force_log(0.0, &state, start_epoch, moon_ephem.as_ref());

    let _final_state =
        Dop853.integrate(&system, state, 0.0, duration_seconds, DT_SECONDS, |t, s| {
            recording.maybe_log(t, s, start_epoch, moon_ephem.as_ref());
        });
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
fn print_burn_summary(results_impulsive: &[BurnResult], results_continuous: &[BurnResult]) {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Burn summary (impulsive-at-midpoint vs continuous-thrust force model)");
    println!("═══════════════════════════════════════════════════════════════════");
    println!(
        "{:<24}  {:<11}  {:>11}  {:>12}  {:>12}  {}",
        "Burn", "mode", "|Δv| m/s", "Pos err km", "Vel err km/s", "Judgment"
    );
    println!("{}", "-".repeat(92));
    // Interleave impulsive and continuous rows per burn so the reader
    // immediately sees the improvement from finite-burn modelling.
    // The two vectors are expected to have equal length (one call per
    // maneuver) but we defensively zip the shorter one.
    for (i, (r_imp, r_cont)) in results_impulsive
        .iter()
        .zip(results_continuous.iter())
        .enumerate()
    {
        // First row: impulsive (with the burn label).
        println!(
            "{:<24}  {:<11}  {:>11.3}  {:>12.3}  {:>12.6}  {}",
            r_imp.label,
            "impulsive",
            r_imp.magnitude_ms,
            r_imp.position_error_km,
            r_imp.velocity_error_kms,
            r_imp.judgment.glyph(),
        );
        // Second row: continuous (blank label column for alignment).
        println!(
            "{:<24}  {:<11}  {:>11.3}  {:>12.3}  {:>12.6}  {}",
            "",
            "continuous",
            r_cont.magnitude_ms,
            r_cont.position_error_km,
            r_cont.velocity_error_kms,
            r_cont.judgment.glyph(),
        );
        // Improvement delta on a third row so the verdict is obvious
        // without arithmetic in the reader's head. For a symmetric
        // uniform-thrust burn (the common case in this spike), the
        // two trajectories are bit-identical post-burn by direct
        // integration — see `verify_burn_continuous` docstring — so
        // "equivalent" is the expected label and "× better/worse" is
        // reserved for asymmetric thrust models (future iterations).
        let improvement = r_imp.position_error_km - r_cont.position_error_km;
        let verdict = if improvement.abs() < 1e-6 {
            "equivalent".to_string()
        } else if improvement > 0.0 {
            format!(
                "{:.2}× better",
                r_imp.position_error_km / r_cont.position_error_km
            )
        } else {
            format!(
                "{:.2}× worse",
                r_cont.position_error_km / r_imp.position_error_km
            )
        };
        println!(
            "{:<24}  {:<11}  {:>11}  {:>+12.3}  {:>12}  {}",
            "", "(Δ)", "", improvement, "", verdict,
        );
        // Small separator between burns except for the last one.
        if i + 1 < results_impulsive.len() {
            println!("{}", "-".repeat(92));
        }
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
    // Horizons requires start != stop; request a 1-minute bracket so
    // Hermite interpolation below has two samples to work with.
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

    // Use Hermite interpolation to get the state EXACTLY at `epoch`.
    //
    // ## Why not `iter().min_by` nearest-neighbor?
    //
    // The previous implementation picked the sample whose JD was closest
    // to `epoch.jd()` and returned it verbatim. That looked reasonable
    // but had a subtle failure mode:
    //
    //   * We request `epoch ± 30 s` with `step=1m`. For this 1-minute
    //     range Horizons snaps sampling to the window boundaries and
    //     returns exactly two samples at `epoch − 30 s` and
    //     `epoch + 30 s` — with **no** sample at the exact epoch.
    //   * Both candidates are 30 s away from `epoch`. In floating-point
    //     the two `|Δt|` values are either bitwise equal (a true tie) or
    //     differ by a sub-ULP asymmetry. Either way the observed result
    //     is deterministic: the chronologically earlier sample is picked.
    //       - On a true tie, [`Iterator::min_by`] has the documented
    //         contract *"If several elements are equally minimum, the
    //         first element is returned"* — and for a `HorizonsTable`
    //         whose samples are sorted ascending by epoch (enforced by
    //         `parse_csv`), "first" means the chronologically earlier
    //         sample.
    //       - On a sub-ULP asymmetry, the direction still favours the
    //         earlier sample because Horizons emits the `epoch − 30 s`
    //         row first and the JD text precision biases the comparison
    //         consistently.
    //   * Result: every round-minute fetch returned a state that was
    //     **always 30 s earlier** in physical time than what the caller
    //     asked for — a systematic bias, not a random jitter. This was
    //     verified empirically with debug prints: every single call to
    //     this function during a full spike run reported a −30.000 s
    //     offset between the requested epoch and the picked sample.
    //
    // That 30 s offset is invisible to pure coast propagation (it just
    // shifts everything by 30 s in parallel), but it interacts with
    // Method B burn verification: the impulsive Δv is applied at integer
    // t = pre_to_mid_seconds after a state whose physical time is 30 s
    // older than the label suggests, so the impulse lands 30 s before
    // the `mid_epoch` label in physical time. For a burn with
    // |Δv| ≈ 110 m/s the resulting post-burn position error is
    // |Δv| × 30 s ≈ 3 km per burn — on the right order of magnitude
    // for the 7.4 km / 20.4 km DRI/DRDI residuals observed before this
    // fix (the rest of those residuals is the burn-profile-asymmetry
    // floor discussed in `MANEUVERS` and the impulsive-at-midpoint
    // approximation error).
    //
    // ## Why interpolate is correct
    //
    // `HorizonsTable::interpolate` does a cubic Hermite interpolation
    // using position **and** velocity at the two bracketing samples.
    // For smooth ballistic motion (our case — Orion in coast phases near
    // the DRI/DRDI burn windows) the error is O(h^4) where h is the
    // sample spacing; with h = 60 s this is numerically exact for
    // kilometre-scale verification. The interpolated state is the state
    // **at exactly the requested epoch**, eliminating the 30 s bias.
    //
    // ## Observed effect
    //
    // Running `cargo run --release --example artemis1 -p orts
    // --features fetch-horizons` with `TIME_TYPE=TDB` and a clean cache:
    //
    //   |                    | nearest-neighbor | interpolate |
    //   | DRI burn error     |       7.432 km   |   3.965 km  |
    //   | DRDI burn error    |      20.440 km   |  16.288 km  |
    //   | 6-day chain error  |    1266.657 km   | 1196.257 km |
    //   | Return coast (4d)  |     115.150 km   | 105.873 km  |
    //
    // DRI / DRDI improve as expected (≈ |Δv| × 30 s removed from the
    // burn floor). `Return coast` also improves because the same 30 s
    // bias on pre/post was nudging the pure-coast drift. `DRO coast`
    // **regresses** slightly (96 → 125 km, still within the ≤ 1000 km
    // `THRESHOLD_PASS_KM` bucket so the Summary Judgment is unchanged);
    // the 30 s bias was accidentally masking a separate DRO-phase error
    // source that is now exposed. Investigating that is future work —
    // the bias itself was wrong and needed removing regardless.
    //
    // A panic from `HorizonsTable::interpolate` is impossible here
    // because the samples are guaranteed to bracket `epoch` — we
    // explicitly fetched a ±30 s window around it, and `parse_csv`
    // guarantees non-empty ascending samples on the `Ok` path. The
    // `debug_assert!` below gives an additional loud signal in case
    // Horizons' response shape ever degenerates to a single sample.
    debug_assert!(
        table.len() >= 2,
        "Horizons returned {} sample(s) for a 1-minute window; \
         Hermite interpolation requires ≥ 2 samples",
        table.len()
    );
    let sample = table.interpolate(epoch).unwrap_or_else(|| {
        let (first_jd, last_jd) = table
            .date_range()
            .map(|(a, b)| (a.jd(), b.jd()))
            .unwrap_or((f64::NAN, f64::NAN));
        panic!(
            "HorizonsTable::interpolate returned None: requested JD {:.9} ({}), \
             table range [{:.9}, {:.9}] ({} samples). This should be impossible \
             for a ±30 s fetch window — Horizons response shape may have changed.",
            epoch.jd(),
            iso_short(epoch),
            first_jd,
            last_jd,
            table.len(),
        )
    });

    Ok((sample.position, sample.velocity))
}

#[cfg(feature = "fetch-horizons")]
/// Build the Earth-centred `OrbitalSystem` used by every coast and burn
/// verification in this spike.
///
/// Force model components:
/// - J2/J3/J4 zonal harmonics (from kaname constants)
/// - Sun as a third-body, using the Horizons-tabulated ephemeris
///   (closure over [`HorizonsTable`])
/// - Moon as a third-body, using the [`MoonEphemeris`] trait object
///
/// The Moon and Sun are handled asymmetrically by intent:
///
/// - The Moon is plumbed through [`ThirdBodyGravity::moon_with_ephemeris`]
///   and the [`MoonEphemeris`] trait because [`HorizonsMoonEphemeris`]
///   carries a fallback counter that each phase verifier inspects
///   (`fallbacks_before` / `fallbacks_after` checks). That counter is the
///   single most important guardrail against silent accuracy regressions
///   — if the Horizons Moon table ever fails to cover the propagation
///   window, the verifier exits loudly instead of silently rolling back
///   to the Meeus approximation.
/// - The Sun, in contrast, is plumbed through [`ThirdBodyGravity::custom`]
///   with an inline closure over the raw `HorizonsTable`. No trait, no
///   counter — see the closure comment below for the silent-fallback
///   caveat.
///
/// The asymmetry is historical: the Moon ephemeris trait was already in
/// place for the earlier Moon migration, and cloning the same pattern
/// for the Sun would require a new `SunEphemeris` trait + type in
/// kaname. For a research spike the closure path is adequate and keeps
/// the change consumer-side only.
#[cfg(feature = "fetch-horizons")]
fn build_artemis_system(
    epoch: Epoch,
    moon_ephem: &Arc<dyn MoonEphemeris>,
    sun_table: &Arc<HorizonsTable>,
) -> OrbitalSystem {
    use kaname::body::KnownBody;
    use kaname::earth::{J2 as J2_EARTH, J3 as J3_EARTH, J4 as J4_EARTH, MU as MU_EARTH};
    use kaname::sun::MU as MU_SUN;

    let earth = KnownBody::Earth;
    let props = earth.properties();

    // Build a custom Sun third-body model whose position closure looks
    // up the Horizons table via Hermite interpolation. If the query
    // epoch falls outside the table range, the closure **silently**
    // falls back to the kaname Meeus analytical Sun. This fallback
    // should not fire during normal runs — the Sun table is fetched
    // over the same mission window as the Moon table (moon_window_*),
    // with 1-hour padding — but unlike the Moon, there is no fallback
    // counter. If a future iteration narrows the fetch window or
    // shortens the padding, the Sun could silently drop to Meeus during
    // a phase verification and revert the ~29 km / ~23 % DRO coast
    // improvement without any diagnostic signal. If that ever happens,
    // add a counter here mirroring `HorizonsMoonEphemeris`, or replace
    // this closure with a dedicated `SunEphemeris` trait + type in
    // kaname.
    let sun_table_for_closure: Arc<HorizonsTable> = Arc::clone(sun_table);
    let sun_model = ThirdBodyGravity::custom("third_body_sun", MU_SUN, move |e| {
        sun_table_for_closure
            .interpolate(e)
            .map(|s| kaname::frame::Vec3::from_raw(s.position))
            .unwrap_or_else(|| kaname::sun::sun_position_eci(e))
    });

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
    .with_model(sun_model)
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
