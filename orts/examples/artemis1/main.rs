//! Artemis 1 simulation example — currently a DRO feasibility spike.
//!
//! ## Current status (Phase 1.5)
//!
//! This example is a **feasibility spike** for a future full Artemis 1
//! mission reconstruction. Before committing to the full mission scaffold
//! (25 days, ~15 burns, OPF/DRI/DRDI/RPF targeting), we need to answer one
//! concrete question: can an Earth-centric `Dop853` integrator with a
//! `J2 + Sun + HorizonsMoon` force model propagate a Distant Retrograde
//! Orbit (DRO) for several days to within acceptable accuracy of the real
//! Orion trajectory?
//!
//! The DRO regime is where this architecture is most stressed: the
//! spacecraft orbits the Moon at ~70,000 km while Earth-centric coordinates
//! treat the Moon as a third-body perturber — but at DRO distances the
//! Moon's gravity is comparable to Earth's, violating the usual "third
//! body is far" assumption. Apollo 11, which this example's template is
//! based on, only spends ~2.5 days in low lunar orbit and already documents
//! degraded accuracy near the Moon.
//!
//! ## What this example does
//!
//! 1. Fetches the real Orion spacecraft state vector from JPL Horizons
//!    (target `-1023`, center `500@399` Earth geocenter) at the DRO
//!    insertion epoch and the DRO departure epoch.
//! 2. Fetches the Moon ephemeris (target `301`) over the ~5-day window at
//!    1-hour spacing to drive the third-body force model.
//! 3. Propagates forward from the DRI state using `Dop853` with
//!    `dt = 10 s`, `J2_EARTH + J3_EARTH + J4_EARTH`, Sun third-body
//!    (Meeus), and Moon third-body (Horizons-interpolated).
//! 4. Compares the propagated final state to the Horizons DRDI reference.
//! 5. Reports the position error envelope and issues a judgment about
//!    whether to proceed with Earth-centric propagation for the full
//!    Artemis 1 mission.
//!
//! ## Judgment criteria (see plan in `.claude/plans/zippy-pondering-pearl.md`)
//!
//! - `< 1000 km`: **Pass** — proceed with Earth-centric architecture.
//! - `1000-10000 km`: **Conditional** — re-run with tighter tolerances.
//! - `>= 10000 km`: **Fail** — switch to Moon-centric SOI switching,
//!   higher-order integrator, or reduce scope (drop DRO phase).
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
// Mission constants (Artemis 1, NASA Artemis 1 press kit)
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

/// DRO Insertion epoch (DRI, 2022-11-25 22:52 UTC — post-burn).
///
/// Source: NASA Artemis 1 DRO insertion burn was executed at roughly
/// this time; for a feasibility spike we use a round epoch that falls
/// cleanly within the DRO coast phase.
#[cfg(feature = "fetch-horizons")]
const DRI_EPOCH_ISO: &str = "2022-11-26T00:00:00Z";

/// DRO Departure epoch approximation (real DRDI was 2022-12-01 21:27 UTC).
///
/// Target for the propagation comparison. We use `00:00:00` rather than the
/// exact departure time to work with clean round epochs; the 5-day coast
/// window this produces is enough to stress-test the architecture. The
/// eventual full-mission example will extend to the real 2022-12-01T21:27Z
/// departure and include the RPF powered flyby that follows.
#[cfg(feature = "fetch-horizons")]
const DRDI_EPOCH_ISO: &str = "2022-12-01T00:00:00Z";

/// Horizons sample spacing for the Moon ephemeris table.
#[cfg(feature = "fetch-horizons")]
const MOON_SAMPLE_STEP: &str = "1h";

/// Dop853 propagation step size (same as apollo11).
#[cfg(feature = "fetch-horizons")]
const DT_SECONDS: f64 = 10.0;

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
    println!("Artemis 1 DRO Feasibility Spike");
    println!("═══════════════════════════════════════════════════════════════════");
    println!();
    println!("Goal: determine whether an Earth-centric Dop853 integrator with");
    println!("J2/J3/J4 + Sun + Horizons-Moon can propagate a multi-day DRO coast");
    println!("to within 1000 km of the real Orion trajectory.");
    println!();

    // ----- Parse epoch bounds -----
    let dri_epoch = Epoch::from_iso8601(DRI_EPOCH_ISO).expect("valid DRI epoch");
    let drdi_epoch = Epoch::from_iso8601(DRDI_EPOCH_ISO).expect("valid DRDI epoch");
    let duration_seconds = (drdi_epoch.jd() - dri_epoch.jd()) * 86_400.0;
    println!(
        "Propagation window: {DRI_EPOCH_ISO} → {DRDI_EPOCH_ISO}  ({:.2} days)",
        duration_seconds / 86_400.0
    );
    println!();

    // ----- Fetch Orion reference at DRI and DRDI -----
    println!("[1/3] Fetching Orion reference state vectors from Horizons...");
    let orion_dri = fetch_orion_sample(&dri_epoch).expect("fetch Orion at DRI");
    let orion_drdi = fetch_orion_sample(&drdi_epoch).expect("fetch Orion at DRDI");
    print_state("  DRI initial", &dri_epoch, &orion_dri.0, &orion_dri.1);
    print_state("  DRDI target", &drdi_epoch, &orion_drdi.0, &orion_drdi.1);
    println!();

    // ----- Fetch Moon ephemeris over the window -----
    println!("[2/3] Fetching Moon ephemeris ({MOON_SAMPLE_STEP} spacing) from Horizons...");
    // Horizons requires stop > start + step, so pad the window by one step on
    // each side to give the Hermite interpolator a clean interval.
    let moon_start = dri_epoch.add_seconds(-3600.0);
    let moon_stop = drdi_epoch.add_seconds(3600.0);
    let moon_table = HorizonsTable::fetch_vector_table(
        MOON_TARGET,
        EARTH_GEOCENTER,
        &moon_start,
        &moon_stop,
        MOON_SAMPLE_STEP,
        None,
    )
    .expect("fetch Moon ephemeris");
    println!(
        "  {} samples over {} → {}",
        moon_table.samples().len(),
        iso_short(&moon_start),
        iso_short(&moon_stop),
    );
    // Keep a concretely-typed handle so we can read `fallback_count` after
    // propagation. The dynamically-typed `Arc<dyn MoonEphemeris>` is what
    // the force model and helpers consume.
    let moon_concrete: Arc<HorizonsMoonEphemeris> =
        Arc::new(HorizonsMoonEphemeris::from_table(moon_table));
    let moon_ephem: Arc<dyn MoonEphemeris> = moon_concrete.clone();
    println!();

    // ----- Build force model and propagate -----
    println!(
        "[3/3] Propagating DRI state for {:.2} days...",
        duration_seconds / 86_400.0
    );
    let system = build_dro_system(dri_epoch, &moon_ephem);
    let initial_state = OrbitalState::new(orion_dri.0, orion_dri.1);

    // Track closest Moon approach and maximum Earth distance during the coast.
    let mut min_moon_distance = f64::MAX;
    let mut max_earth_distance: f64 = 0.0;
    let final_state = Dop853.integrate(
        &system,
        initial_state,
        0.0,
        duration_seconds,
        DT_SECONDS,
        |t, state| {
            let epoch = dri_epoch.add_seconds(t);
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
    println!("  closest Moon approach: {:10.1} km", min_moon_distance);
    println!("  max Earth distance:    {:10.1} km", max_earth_distance);

    // Sanity check: the propagation should stay inside the Horizons Moon
    // table range. Any out-of-range query silently falls back to the Meeus
    // analytical model (`MoonEphemeris` trait doc), which would hide the
    // whole point of this spike. Assert the counter is still zero.
    let fallbacks = moon_concrete.fallback_count();
    if fallbacks > 0 {
        eprintln!(
            "  ⚠  Moon ephemeris fell back to Meeus {fallbacks} time(s) — \
             spike result is contaminated by analytical Moon. Widen the \
             Horizons table padding and rerun."
        );
        std::process::exit(1);
    }
    println!("  Horizons Moon fallbacks: 0 (table fully covered the propagation)");
    println!();

    // ----- Compare against Horizons reference -----
    let position_error = (final_state.position() - orion_drdi.0).magnitude();
    let velocity_error = (final_state.velocity() - orion_drdi.1).magnitude();

    println!("═══════════════════════════════════════════════════════════════════");
    println!("Results");
    println!("═══════════════════════════════════════════════════════════════════");
    print_state(
        "Propagated (DRDI)",
        &drdi_epoch,
        final_state.position(),
        final_state.velocity(),
    );
    print_state(
        "Horizons   (DRDI)",
        &drdi_epoch,
        &orion_drdi.0,
        &orion_drdi.1,
    );
    println!();
    println!("Position error: {:12.3} km", position_error);
    println!("Velocity error: {:12.6} km/s", velocity_error);
    println!();

    // ----- Judgment -----
    print!("Judgment: ");
    if position_error < 1000.0 {
        println!("✓ PASS  (error < 1000 km — proceed with Earth-centric architecture)");
    } else if position_error < 10_000.0 {
        println!(
            "? CONDITIONAL  ({:.0} km — retry with tighter tolerances / investigate)",
            position_error
        );
    } else {
        println!(
            "✗ FAIL  ({:.0} km — architecture change required: Moon-centric SOI switch, \
             higher-order integrator, or scope reduction)",
            position_error
        );
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
fn build_dro_system(epoch: Epoch, moon_ephem: &Arc<dyn MoonEphemeris>) -> OrbitalSystem {
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
fn print_state(
    label: &str,
    epoch: &Epoch,
    position: &nalgebra::Vector3<f64>,
    velocity: &nalgebra::Vector3<f64>,
) {
    println!("  {} @ {}", label, iso_short(epoch));
    println!(
        "    r = [{:12.3}, {:12.3}, {:12.3}] km    |r| = {:.3} km",
        position.x,
        position.y,
        position.z,
        position.magnitude()
    );
    println!(
        "    v = [{:12.6}, {:12.6}, {:12.6}] km/s  |v| = {:.6} km/s",
        velocity.x,
        velocity.y,
        velocity.z,
        velocity.magnitude()
    );
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
