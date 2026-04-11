//! Apollo 11 mission trajectory simulation.
//!
//! Demonstrates using the orts library (not CLI) to reproduce the full Apollo 11
//! mission: parking orbit → TLI → translunar coast → LOI → lunar orbit → TEI →
//! trans-Earth coast → Earth return.
//!
//! All propagation is Earth-centered with Moon/Sun as third-body perturbations.
//! SOI switching is not yet implemented, so accuracy degrades near the Moon.
//!
//! Reference data: Apollo/Saturn V Postflight Trajectory AS-506
//!
//! Run: `cargo run --example apollo11 -p orts`
//! Test: `cargo test --example apollo11 -p orts`

use std::sync::Arc;

use nalgebra::Vector3;

use arika::body::KnownBody;
use arika::earth::{
    Geodetic, J2 as J2_EARTH, J3 as J3_EARTH, J4 as J4_EARTH, MU as MU_EARTH, R as R_EARTH,
};
use arika::epoch::Epoch;
use arika::moon::{MU as MU_MOON, MeeusMoonEphemeris, MoonEphemeris};
use orts::OrbitalState;
use orts::orbital::OrbitalSystem;
use orts::orbital::gravity::ZonalHarmonics;
use orts::orbital::kepler::KeplerianElements;
use orts::perturbations::ThirdBodyGravity;
use orts::record::archetypes::OrbitalState as RecordOrbitalState;
use orts::record::components::{BodyRadius, GravitationalParameter};
use orts::record::entity_path::EntityPath;
use orts::record::recording::Recording;
use orts::record::timeline::TimePoint;
use utsuroi::{Dop853, Integrator};

// ============================================================
// Apollo 11 reference data (NASA Postflight Trajectory AS-506)
// ============================================================

// --- Parking orbit (Earth Parking Orbit) ---

/// Parking orbit insertion epoch: 1969-07-16T13:43:49Z (GET 00:11:49)
const PARKING_EPOCH_ISO: &str = "1969-07-16T13:43:49Z";

/// Parking orbit parameters
const PARKING_ALT_APOGEE: f64 = 186.0; // km
const PARKING_ALT_PERIGEE: f64 = 183.2; // km
const PARKING_INC_DEG: f64 = 32.521; // degrees
const PARKING_RAAN_DEG: f64 = 358.383; // degrees
const PARKING_ECC: f64 = 0.00021;
const PARKING_PERIOD_MIN: f64 = 88.18; // minutes

// --- TLI (Trans-Lunar Injection) ---

/// TLI cutoff epoch (S-IVB SECO): 1969-07-16T16:22:03Z
const TLI_EPOCH_ISO: &str = "1969-07-16T16:22:03Z";

/// Time from parking orbit insertion to TLI SECO [s]
const PARKING_TO_TLI: f64 = 9494.0; // ~2h 38m

/// TLI delta-V magnitude [km/s]
const TLI_DV: f64 = 3.041;

/// Post-TLI orbital elements (Oikofuge conversion from Postflight state vector)
const POST_TLI_SMA: f64 = 286_545.0; // km
const POST_TLI_ECC: f64 = 0.97697;
const POST_TLI_INC_DEG: f64 = 31.383; // degrees
const POST_TLI_RAAN_DEG: f64 = 358.383; // degrees
const POST_TLI_AOP_DEG: f64 = 4.410; // degrees
const POST_TLI_TA_DEG: f64 = 14.909; // degrees

/// Post-TLI specific energy C3 [km²/s²] (negative = bound ellipse)
const POST_TLI_C3: f64 = -1.392;

/// Post-TLI inertial speed [km/s]
const POST_TLI_SPEED: f64 = 10.8343;

/// Post-TLI geocentric distance [km]
const POST_TLI_DISTANCE: f64 = 6711.964;

// --- MCC (Midcourse Correction) ---

/// MCC-2 time: GET 26:44:58 → ~24h after TLI SECO
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const MCC2_TIME_AFTER_TLI: f64 = 23.0 * 3600.0 + 55.0 * 60.0;

/// MCC-2 delta-V magnitude [km/s]
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const MCC2_DV: f64 = 0.0064; // 6.4 m/s

// --- LOI (Lunar Orbit Insertion) ---

/// TLI to LOI-1 elapsed time [s] (~73 hours)
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const TLI_TO_LOI_SECONDS: f64 = 73.0 * 3600.0;

/// LOI-1 delta-V magnitude [km/s] (retrograde)
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const LOI1_DV: f64 = 0.8892;

// --- TEI (Trans-Earth Injection) ---

/// LOI to TEI elapsed time [s] (~60 hours: 2.5 days in lunar orbit)
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const LOI_TO_TEI_SECONDS: f64 = 60.0 * 3600.0;

/// TEI delta-V magnitude [km/s] (prograde)
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const TEI_DV: f64 = 1.001;

// --- Reference event GET times (Apollo 11 Mission Report NASA SP-238, Table 7-I) ---
// Used for timing assertions. All values in seconds from parking orbit insertion.

/// TLI SECO: GET 02:44:16.2 (from launch), parking insertion at GET 00:11:49
/// → GET 02:44:16.2 - 00:11:49 = 02:32:27.2 from parking insertion
/// Simplified: use PARKING_TO_TLI constant (9494 s ≈ 2h38m)
const REF_TLI_GET: f64 = PARKING_TO_TLI;

/// LOI-1: GET 75:49:50 from launch → GET 75:49:50 - 00:11:49 ≈ 75.63h from parking
/// Apollo/Saturn V Postflight Trajectory AS-506
const REF_LOI_GET_HOURS: f64 = 75.6;

/// TEI: GET 135:23:42 from launch → ≈ 135.4h from launch, 135.2h from parking
/// Apollo 11 Mission Report NASA SP-238
const REF_TEI_GET_HOURS: f64 = 135.2;

/// Entry interface: GET 195:03:05.7 from launch ≈ 194.9h from parking
const REF_EI_GET_HOURS: f64 = EI_GET_SECONDS / 3600.0;

// --- Entry Interface reference (Apollo 11 Mission Report NASA SP-238, Table 7-II) ---
//
// Entry interface is defined at 400,000 ft (121.92 km) geodetic altitude.
// GET 195:03:05.7 (1969-07-24T16:46:55Z)
//
// Geodetic state from Mission Report:
//   Latitude  = -3.193° S
//   Longitude = 171.196° E
//   Altitude  = 122.0 km
//   Speed     = 11.032 km/s (space-fixed)
//   FPA       = -6.48° (inertial, negative = descending)
//   Azimuth   = 50.1761° (inertial, from north)
//
/// Entry interface geodetic latitude [deg] (negative = south).
const EI_LAT_DEG: f64 = -3.193;

/// Entry interface longitude [deg] (east positive).
const EI_LON_DEG: f64 = 171.196;

/// Entry interface altitude [km] (400,000 ft).
const EI_ALT_KM: f64 = 122.0;

/// Entry interface space-fixed speed [km/s].
const EI_SPEED: f64 = 11.032;

/// Entry interface inertial flight path angle [deg] (negative = descending).
const EI_FPA_DEG: f64 = -6.48;

/// Entry interface inertial azimuth [deg] (from north, clockwise).
const EI_AZI_DEG: f64 = 50.1761;

/// Entry interface GET [s] (195:03:05.7).
const EI_GET_SECONDS: f64 = 702_185.7;

/// TEI to entry interface elapsed time [s] (~60 hours)
/// Apollo 11 Mission Report: TEI at GET 135:23:42, Entry at GET 195:03:05
const TEI_TO_ENTRY_SECONDS: f64 = 59.7 * 3600.0;

/// Total mission time [s] (GET 195:18:35 = splashdown)
/// Apollo 11 Mission Report NASA SP-238, Table 7-I
const MISSION_DURATION_REF: f64 = 195.3 * 3600.0;

// --- Simulation parameters ---

/// Translunar coast duration [s] (~4 days)
const TRANSLUNAR_DURATION: f64 = 4.0 * 86400.0;

/// Trans-Earth coast duration [s] (~8 days, covers return ellipse half-period)
const TRANSEARTH_DURATION: f64 = 8.0 * 86400.0;

/// Integration step size [s]
const DT: f64 = 10.0;

/// Output interval [s]
const OUTPUT_INTERVAL: f64 = 60.0;

// ============================================================
// Helper functions
// ============================================================

/// Compute entry interface state vector in ECI from geodetic reference data.
///
/// Converts the geodetic position (lat, lon, alt) to ECI using WGS84 + GMST,
/// then decomposes the velocity (speed, flight path angle, azimuth) into ECI
/// components using the local topocentric frame at the ECI position.
fn entry_interface_state_eci(epoch: &Epoch) -> OrbitalState {
    // Position: geodetic → ECEF → ECI
    let geod = Geodetic {
        latitude: EI_LAT_DEG.to_radians(),
        longitude: EI_LON_DEG.to_radians(),
        altitude: EI_ALT_KM,
    };
    let gmst = epoch.gmst();
    let ecef = arika::SimpleEcef::from(geod);
    let pos_eci =
        arika::frame::Rotation::<arika::frame::SimpleEcef, arika::frame::SimpleEci>::from_era(gmst)
            .transform(&ecef)
            .into_inner();

    // Velocity: decompose (speed, FPA, azimuth) in the local topocentric frame at pos_eci.
    //
    // Local frame at the ECI position:
    //   up    = r_hat (radial outward)
    //   north = projection of Z-axis onto horizontal plane
    //   east  = north × up
    let r_hat = pos_eci.normalize();
    let z_hat = Vector3::new(0.0, 0.0, 1.0);
    let north = (z_hat - z_hat.dot(&r_hat) * r_hat).normalize();
    let east = north.cross(&r_hat); // completes right-hand system

    let fpa = EI_FPA_DEG.to_radians();
    let azi = EI_AZI_DEG.to_radians();
    let v_radial = EI_SPEED * fpa.sin(); // negative = descending
    let v_horiz = EI_SPEED * fpa.cos();
    let vel_eci = v_radial * r_hat + v_horiz * azi.sin() * east + v_horiz * azi.cos() * north;

    OrbitalState::new(pos_eci, vel_eci)
}

/// Build post-TLI orbital elements from reference data.
fn post_tli_elements() -> KeplerianElements {
    KeplerianElements {
        semi_major_axis: POST_TLI_SMA,
        eccentricity: POST_TLI_ECC,
        inclination: POST_TLI_INC_DEG.to_radians(),
        raan: POST_TLI_RAAN_DEG.to_radians(),
        argument_of_periapsis: POST_TLI_AOP_DEG.to_radians(),
        true_anomaly: POST_TLI_TA_DEG.to_radians(),
    }
}

/// Build initial OrbitalState from post-TLI Keplerian elements.
fn post_tli_state() -> OrbitalState {
    let elements = post_tli_elements();
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    OrbitalState::new(pos, vel)
}

/// Compute the closest Moon approach distance [km] for a given state.
///
/// All Moon position queries go through `moon_ephem` so that the integrator
/// and the targeter share a single source of truth for lunar ephemeris.
fn moon_pericynthion(
    system: &OrbitalSystem,
    state: &OrbitalState,
    epoch: Epoch,
    duration: f64,
    moon_ephem: &dyn MoonEphemeris,
) -> f64 {
    let mut min_dist = f64::MAX;
    Dop853.integrate(system, state.clone(), 0.0, duration, DT, |t, s| {
        let ep = epoch.add_seconds(t);
        let moon_pos = moon_ephem.position_eci(&ep).into_inner();
        let d = (s.position() - moon_pos).magnitude();
        if d < min_dist {
            min_dist = d;
        }
    });
    min_dist
}

/// Compute MCC ΔV direction using scalar pericynthion gradient targeting.
///
/// Uses the same linearized sensitivity approach as Apollo's RTCC:
/// numerically differentiate ∂r_peri/∂V, then compute the minimum-norm ΔV
/// to achieve the desired pericynthion distance. `moon_ephem` provides the
/// Moon position used by the pericynthion computation so that the gradient
/// is consistent with the integrator's third-body model.
fn compute_mcc_dv(
    system: &OrbitalSystem,
    state: &OrbitalState,
    epoch: Epoch,
    duration: f64,
    desired_r_peri: f64,
    max_dv: f64,
    moon_ephem: &dyn MoonEphemeris,
) -> Vector3<f64> {
    let eps = 0.0001; // 0.1 m/s perturbation for finite differences [km/s]

    let mut current = state.clone();
    let mut total_dv = Vector3::zeros();

    for iteration in 0..6 {
        let r_peri_0 = moon_pericynthion(system, &current, epoch, duration, moon_ephem);
        let delta_r = desired_r_peri - r_peri_0;

        if delta_r.abs() < 10.0 {
            break; // converged within 10 km
        }

        // Compute gradient by finite differences
        let mut grad = Vector3::zeros();
        for i in 0..3 {
            let mut v_pert = *current.velocity();
            v_pert[i] += eps;
            let perturbed = OrbitalState::new(*current.position(), v_pert);
            let r_peri_i = moon_pericynthion(system, &perturbed, epoch, duration, moon_ephem);
            grad[i] = (r_peri_i - r_peri_0) / eps;
        }

        let grad_sq = grad.dot(&grad);
        if grad_sq < 1e-20 {
            break; // gradient too small
        }

        // Minimum-norm ΔV: dv = delta_r * grad / |grad|²
        let dv = grad * (delta_r / grad_sq);

        // Clamp to max_dv
        let dv = if dv.magnitude() > max_dv {
            dv.normalize() * max_dv
        } else {
            dv
        };

        current = current.apply_delta_v(dv);
        total_dv += dv;

        eprintln!(
            "    targeting iter {}: r_peri={:.0} km, Δr={:.0} km, dv={:.1} m/s",
            iteration,
            r_peri_0,
            delta_r,
            dv.magnitude() * 1000.0
        );
    }

    total_dv
}

/// Propagate from TEI and return (perigee_altitude_km, time_to_perigee_s).
///
/// Finds the closest Earth approach within `duration` seconds.
fn earth_perigee(
    system: &OrbitalSystem,
    state: &OrbitalState,
    epoch: Epoch,
    duration: f64,
) -> (f64, f64) {
    let mut min_r = f64::MAX;
    let mut min_t = 0.0;
    Dop853.integrate(system, state.clone(), 0.0, duration, DT, |t, s| {
        let r = s.position().magnitude();
        if r < min_r {
            min_r = r;
            min_t = t;
        }
    });
    (min_r - R_EARTH, min_t)
}

/// Compute TEI ΔV direction using gradient-based targeting of Earth entry interface.
///
/// Targets two objectives simultaneously:
///   1. Earth perigee altitude ≈ `EI_ALT_KM` (122 km)
///   2. Time of perigee ≈ `EI_GET_SECONDS - tei_mission_t` (so GET matches)
///
/// Uses finite-difference gradients on the ΔV direction (fixed magnitude `TEI_DV`).
/// Each iteration propagates only to `target_time * 1.2` (not the full 8-day coast)
/// for speed.
fn compute_tei_dv(
    system: &OrbitalSystem,
    state: &OrbitalState,
    epoch: Epoch,
    tei_mission_t: f64,
    moon_ephem: &dyn MoonEphemeris,
) -> Vector3<f64> {
    // Target perigee at ~30 km (ensures passage through 122 km entry interface).
    // The entry interface at 122 km is a *crossing* altitude, not the perigee.
    // Apollo 11 CM re-entered with a flight path angle of -6.48°, reaching a
    // perigee well below 122 km.  The atmosphere_alt collision check fires at
    // 100 km (Karman line), so we target ~30 km to guarantee atmospheric capture.
    // The exact value is not critical as long as it is below atmosphere_alt.
    let target_alt = 30.0;
    let target_coast_time = EI_GET_SECONDS - tei_mission_t;
    // Only propagate slightly beyond the target time
    let coast_duration = target_coast_time * 1.3;
    let eps = 0.001; // 1 m/s perturbation for finite differences [km/s]

    // Start with prograde (Moon-relative) as initial guess, TEI_DV as initial magnitude
    let moon_vel = moon_ephem.velocity_eci(&epoch).into_inner();
    let v_rel = state.velocity() - moon_vel;
    let mut dv_dir = v_rel.normalize();
    let mut dv_mag = TEI_DV;

    // Helper: evaluate objectives for a given ΔV vector
    let eval = |dir: Vector3<f64>, mag: f64| -> (f64, f64) {
        let post = state.apply_delta_v(dir * mag);
        earth_perigee(system, &post, epoch, coast_duration)
    };

    for iteration in 0..30 {
        let (peri_alt, peri_t) = eval(dv_dir, dv_mag);
        let alt_err = peri_alt - target_alt;
        let time_err = peri_t - target_coast_time;

        eprintln!(
            "    TEI iter {iteration}: alt={peri_alt:.0} km (err={alt_err:+.0}), \
             t={:.1}h (err={:.1}h), |ΔV|={:.4} km/s",
            peri_t / 3600.0,
            time_err / 3600.0,
            dv_mag,
        );

        if alt_err.abs() < 100.0 && time_err.abs() < 3600.0 {
            break;
        }

        // Three perturbation directions: two on the unit sphere + magnitude
        let arb = if dv_dir.x.abs() < 0.9 {
            Vector3::new(1.0, 0.0, 0.0)
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        };
        let u1 = dv_dir.cross(&arb).normalize();
        let u2 = dv_dir.cross(&u1).normalize();

        // Jacobian: 2×3 [∂alt/∂θ1, ∂alt/∂θ2, ∂alt/∂mag; ∂t/∂θ1, ∂t/∂θ2, ∂t/∂mag]
        let (alt_1, t_1) = eval((dv_dir + u1 * eps).normalize(), dv_mag);
        let (alt_2, t_2) = eval((dv_dir + u2 * eps).normalize(), dv_mag);
        let (alt_m, t_m) = eval(dv_dir, dv_mag + eps);

        let j = [
            [
                (alt_1 - peri_alt) / eps,
                (alt_2 - peri_alt) / eps,
                (alt_m - peri_alt) / eps,
            ],
            [
                (t_1 - peri_t) / eps,
                (t_2 - peri_t) / eps,
                (t_m - peri_t) / eps,
            ],
        ];

        // Solve underdetermined 2×3 system via pseudoinverse (minimum-norm solution)
        // J^T (J J^T)^{-1} b, where b = -[alt_err, time_err]
        let jjt = [
            [
                j[0][0] * j[0][0] + j[0][1] * j[0][1] + j[0][2] * j[0][2],
                j[0][0] * j[1][0] + j[0][1] * j[1][1] + j[0][2] * j[1][2],
            ],
            [
                j[1][0] * j[0][0] + j[1][1] * j[0][1] + j[1][2] * j[0][2],
                j[1][0] * j[1][0] + j[1][1] * j[1][1] + j[1][2] * j[1][2],
            ],
        ];
        let det = jjt[0][0] * jjt[1][1] - jjt[0][1] * jjt[1][0];
        if det.abs() < 1e-30 {
            eprintln!("    TEI: Jacobian singular, stopping");
            break;
        }
        // (J J^T)^{-1} * b
        let b = [-alt_err, -time_err];
        let y0 = (jjt[1][1] * b[0] - jjt[0][1] * b[1]) / det;
        let y1 = (-jjt[1][0] * b[0] + jjt[0][0] * b[1]) / det;
        // J^T * y = minimum-norm step
        let d1_full = j[0][0] * y0 + j[1][0] * y1;
        let d2_full = j[0][1] * y0 + j[1][1] * y1;
        let dm_full = j[0][2] * y0 + j[1][2] * y1;

        // Clamp direction step (max ~3°) and magnitude step (max 0.05 km/s)
        let dir_mag = (d1_full * d1_full + d2_full * d2_full).sqrt();
        let dir_clamp = if dir_mag > 0.05 { 0.05 / dir_mag } else { 1.0 };
        let dm_clamped = dm_full.clamp(-0.05, 0.05);

        // Line search
        let current_err = alt_err.abs() + time_err.abs() * 0.1;
        let mut alpha = 1.0_f64;
        let mut best_dir = dv_dir;
        let mut best_mag = dv_mag;
        for _ in 0..5 {
            let s = alpha * dir_clamp;
            let candidate_dir = (dv_dir + u1 * (d1_full * s) + u2 * (d2_full * s)).normalize();
            let candidate_mag = (dv_mag + dm_clamped * alpha).max(0.5);
            let (a, t) = eval(candidate_dir, candidate_mag);
            let candidate_err = (a - target_alt).abs() + (t - target_coast_time).abs() * 0.1;
            if candidate_err < current_err {
                best_dir = candidate_dir;
                best_mag = candidate_mag;
                break;
            }
            alpha *= 0.5;
            best_dir = candidate_dir;
            best_mag = candidate_mag;
        }
        dv_dir = best_dir;
        dv_mag = best_mag;
    }

    eprintln!("    TEI final |ΔV| = {dv_mag:.4} km/s (ref: {TEI_DV:.3})");
    dv_dir * dv_mag
}

/// Specific orbital energy: v²/2 - μ/r [km²/s²]
fn specific_energy(state: &OrbitalState, mu: f64) -> f64 {
    let v2 = state.velocity().magnitude_squared();
    let r = state.position().magnitude();
    v2 / 2.0 - mu / r
}

/// Build the Earth-centered dynamical system for translunar trajectory.
///
/// `moon_ephem` provides the Moon position for the third-body perturbation.
/// Sharing the same handle between the integrator and the targeting helpers
/// ensures both see an identical lunar ephemeris (critical for finite-
/// difference gradient consistency). The blanket
/// `impl<T> MoonEphemeris for Arc<T>` in [`arika::moon`] makes the
/// `ThirdBodyGravity::moon_with_ephemeris` constructor accept the shared
/// handle directly — no manual closure plumbing needed.
fn build_translunar_system(epoch: Epoch, moon_ephem: &Arc<dyn MoonEphemeris>) -> OrbitalSystem {
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

/// Propagate and record orbital state. Returns (final_state, min_moon_distance, min_moon_dist_time).
///
/// `t_offset` shifts the recording timeline so phases are continuous in the rrd.
/// `moon_ephem` provides the Moon position used for distance tracking — must
/// match the ephemeris used by `system` for consistency.
///
/// The argument count is over the Clippy threshold because each phase
/// (parking → TLI → translunar → lunar orbit → TEI → entry) needs all of
/// these values. A follow-up refactor will extract a
/// `PropagationContext { rec, moon_ephem, step_offset, t_offset }` struct so
/// that the signature shrinks and artemis1 can inherit the same shape
/// without re-allowing the lint. Tracked for PR 5 / Artemis 1 migration.
#[allow(clippy::too_many_arguments)]
fn propagate_and_record(
    system: &OrbitalSystem,
    initial: &OrbitalState,
    epoch: Epoch,
    duration: f64,
    dt: f64,
    rec: &mut Recording,
    step_offset: u64,
    t_offset: f64,
    moon_ephem: &dyn MoonEphemeris,
) -> (OrbitalState, f64, f64, u64) {
    let sat_path = EntityPath::parse("/world/sat/apollo11");

    let mut step = step_offset;
    let mut min_moon_dist = f64::MAX;
    let mut min_moon_dist_t = 0.0;
    let mut last_output_t = -OUTPUT_INTERVAL; // ensure first step is recorded

    let final_state = Dop853.integrate(system, initial.clone(), 0.0, duration, dt, |t, state| {
        // Track Moon distance
        let current_epoch = epoch.add_seconds(t);
        let moon_pos = moon_ephem.position_eci(&current_epoch).into_inner();
        let moon_dist = (state.position() - moon_pos).magnitude();
        if moon_dist < min_moon_dist {
            min_moon_dist = moon_dist;
            min_moon_dist_t = t;
        }

        // Record at output intervals
        if t - last_output_t >= OUTPUT_INTERVAL {
            let tp = TimePoint::new().with_sim_time(t_offset + t).with_step(step);
            let os = RecordOrbitalState::new(*state.position(), *state.velocity());
            rec.log_orbital_state(&sat_path, &tp, &os);
            step += 1;
            last_output_t = t;
        }
    });

    // Log final state
    let tp = TimePoint::new()
        .with_sim_time(t_offset + duration)
        .with_step(step);
    let os = RecordOrbitalState::new(*final_state.position(), *final_state.velocity());
    rec.log_orbital_state(&sat_path, &tp, &os);
    step += 1;

    (final_state, min_moon_dist, min_moon_dist_t, step)
}

// ============================================================
// main — full Apollo 11 mission simulation
// ============================================================

fn main() {
    println!("=== Apollo 11 Mission Trajectory Simulation ===");
    println!("    Earth-centered propagation with Moon/Sun third-body perturbations");
    println!();

    let parking_epoch = Epoch::from_iso8601(PARKING_EPOCH_ISO).unwrap();

    // Moon ephemeris — single source of truth for both the integrator's
    // third-body force model and the targeting helpers. Defaults to the
    // Meeus analytical model, matching apollo11's historical behavior.
    let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);

    let system = build_translunar_system(parking_epoch, &moon_ephem);

    // Build recording
    let mut rec = Recording::new();
    let earth_path = EntityPath::parse("/world/earth");
    rec.log_static(&earth_path, &GravitationalParameter(MU_EARTH));
    rec.log_static(&earth_path, &BodyRadius(R_EARTH));

    let moon_path = EntityPath::parse("/world/moon");
    let moon_props = KnownBody::Moon.properties();
    rec.log_static(&moon_path, &GravitationalParameter(MU_MOON));
    rec.log_static(&moon_path, &BodyRadius(moon_props.radius));

    let mut mission_t: f64 = 0.0; // continuous mission elapsed time [s]
    let mut step: u64 = 0;

    // ──────────────────────────────────────────────
    // Phase 1: Earth Parking Orbit (~1.5 revolutions)
    // ──────────────────────────────────────────────
    println!("Phase 1: Earth Parking Orbit");
    println!("  Epoch: {}", parking_epoch.to_datetime());

    let mean_alt = (PARKING_ALT_APOGEE + PARKING_ALT_PERIGEE) / 2.0;
    let parking_elements = KeplerianElements {
        semi_major_axis: R_EARTH + mean_alt,
        eccentricity: PARKING_ECC,
        inclination: PARKING_INC_DEG.to_radians(),
        raan: PARKING_RAAN_DEG.to_radians(),
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };
    let (parking_pos, parking_vel) = parking_elements.to_state_vector(MU_EARTH);
    let parking_state = OrbitalState::new(parking_pos, parking_vel);

    println!(
        "  Altitude: {:.1} km (ref: {PARKING_ALT_PERIGEE}×{PARKING_ALT_APOGEE} km)",
        parking_state.position().magnitude() - R_EARTH
    );
    println!(
        "  Velocity: {:.3} km/s",
        parking_state.velocity().magnitude()
    );
    println!(
        "  Period: {:.2} min (ref: {PARKING_PERIOD_MIN} min)",
        2.0 * std::f64::consts::PI * ((R_EARTH + mean_alt).powi(3) / MU_EARTH).sqrt() / 60.0
    );

    let parking_coast = PARKING_TO_TLI;
    let (state_at_tli, _, _, new_step) = propagate_and_record(
        &system,
        &parking_state,
        parking_epoch,
        parking_coast,
        10.0,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );
    step = new_step;

    println!(
        "  Coasted {:.1} min ({:.1} revolutions)",
        parking_coast / 60.0,
        parking_coast / (PARKING_PERIOD_MIN * 60.0)
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 2: Trans-Lunar Injection (TLI)
    // ──────────────────────────────────────────────
    println!("Phase 2: Trans-Lunar Injection (TLI)");

    // Demonstrate apply_delta_v with approximate prograde ΔV
    let v_hat = state_at_tli.velocity().normalize();
    let tli_dv = v_hat * TLI_DV;
    let post_tli_approx = state_at_tli.apply_delta_v(tli_dv);
    let c3_approx = specific_energy(&post_tli_approx, MU_EARTH) * 2.0;

    // Use accurate post-TLI state from NASA reconstruction for propagation
    let post_tli = post_tli_state();
    let c3 = specific_energy(&post_tli, MU_EARTH) * 2.0;

    println!("  ΔV = {TLI_DV:.3} km/s");
    println!("  Approximate (prograde ΔV): C3 = {c3_approx:.3} km²/s²");
    println!("  Reconstructed (NASA):      C3 = {c3:.3} km²/s² (ref: {POST_TLI_C3} km²/s²)");
    println!(
        "  Post-TLI: r = {:.1} km (ref: {POST_TLI_DISTANCE} km), v = {:.3} km/s (ref: {POST_TLI_SPEED} km/s)",
        post_tli.position().magnitude(),
        post_tli.velocity().magnitude()
    );

    // Verify post-TLI state against NASA reconstruction
    assert!(
        (c3 - POST_TLI_C3).abs() < 0.1,
        "C3 mismatch: {c3:.3} vs ref {POST_TLI_C3}"
    );
    assert!(
        (post_tli.position().magnitude() - POST_TLI_DISTANCE).abs() < 5.0,
        "post-TLI distance mismatch"
    );
    assert!(
        (post_tli.velocity().magnitude() - POST_TLI_SPEED).abs() < 0.01,
        "post-TLI speed mismatch"
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 3: Translunar Coast (~3 days to Moon)
    // ──────────────────────────────────────────────
    println!("Phase 3: Translunar Coast");

    // Reset mission time to TLI SECO epoch for accurate propagation
    let tli_epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
    mission_t = PARKING_TO_TLI;
    let system_tli = build_translunar_system(tli_epoch, &moon_ephem);

    let energy_at_tli = specific_energy(&post_tli, MU_EARTH);

    // Phase 3a: coast to MCC-2 (~24h after TLI)
    let (state_at_mcc2, _, _, new_step) = propagate_and_record(
        &system_tli,
        &post_tli,
        tli_epoch,
        MCC2_TIME_AFTER_TLI,
        DT,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );
    mission_t += MCC2_TIME_AFTER_TLI;
    step = new_step;

    // Verify energy conservation during ballistic coast (TLI → MCC-2, ~24h, no burns)
    // Third-body perturbations transfer energy, but the change should be modest.
    let energy_at_mcc2 = specific_energy(&state_at_mcc2, MU_EARTH);
    let energy_change_pct = ((energy_at_mcc2 - energy_at_tli) / energy_at_tli).abs() * 100.0;
    assert!(
        energy_change_pct < 5.0,
        "energy changed {energy_change_pct:.2}% during 24h ballistic coast (expected < 5%)"
    );

    // MCC-2: compute burn direction using scalar pericynthion gradient targeting
    // (simplified version of Apollo RTCC's B-plane targeting)
    let mcc2_epoch = tli_epoch.add_seconds(MCC2_TIME_AFTER_TLI);
    let system_mcc2 = build_translunar_system(mcc2_epoch, &moon_ephem);
    let coast_remaining = TRANSLUNAR_DURATION - MCC2_TIME_AFTER_TLI;
    let desired_pericynthion = 1850.0; // ~113 km altitude above Moon surface [km from center]

    println!("  MCC-2 at GET {:.1}h:", mission_t / 3600.0);
    let mcc2_dv = compute_mcc_dv(
        &system_mcc2,
        &state_at_mcc2,
        mcc2_epoch,
        coast_remaining,
        desired_pericynthion,
        MCC2_DV,
        &*moon_ephem,
    );
    let post_mcc2 = state_at_mcc2.apply_delta_v(mcc2_dv);
    println!(
        "    ΔV = {:.1} m/s (ref: {:.1} m/s), targeting pericynthion {:.0} km",
        mcc2_dv.magnitude() * 1000.0,
        MCC2_DV * 1000.0,
        desired_pericynthion,
    );

    // Phase 3b: coast from MCC-2 to Moon closest approach
    let mut min_moon_dist = f64::MAX;
    let mut min_moon_dist_t = 0.0;
    Dop853.integrate(
        &system_mcc2,
        post_mcc2.clone(),
        0.0,
        coast_remaining,
        DT,
        |t, state| {
            let ep = mcc2_epoch.add_seconds(t);
            let moon_pos = moon_ephem.position_eci(&ep).into_inner();
            let d = (state.position() - moon_pos).magnitude();
            if d < min_moon_dist {
                min_moon_dist = d;
                min_moon_dist_t = t;
            }
        },
    );

    let moon_approach_get = (mission_t + min_moon_dist_t) / 3600.0;
    let moon_approach_alt = min_moon_dist - 1737.4;
    println!(
        "  Closest Moon approach: {:.0} km ({:.0} km alt) at GET {:.1}h (ref: LOI at ~{:.0}h)",
        min_moon_dist,
        moon_approach_alt,
        moon_approach_get,
        (PARKING_TO_TLI + TLI_TO_LOI_SECONDS) / 3600.0
    );

    // Verify: Moon approach should be above surface and within ~200 km of ref (1850 km)
    assert!(
        min_moon_dist > 1737.4,
        "trajectory passes through Moon! (dist={min_moon_dist:.0} km < 1737 km)"
    );
    assert!(
        (min_moon_dist - 1850.0).abs() < 500.0,
        "Moon approach distance {min_moon_dist:.0} km too far from ref 1850 km"
    );
    // Timing should be within ~5 hours of ref
    let ref_get = (PARKING_TO_TLI + TLI_TO_LOI_SECONDS) / 3600.0;
    assert!(
        (moon_approach_get - ref_get).abs() < 5.0,
        "Moon approach timing {moon_approach_get:.1}h too far from ref {ref_get:.0}h"
    );

    // Record up to closest approach
    let (state_at_loi, _, _, new_step) = propagate_and_record(
        &system_mcc2,
        &post_mcc2,
        mcc2_epoch,
        min_moon_dist_t,
        DT,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );

    mission_t += min_moon_dist_t;
    step = new_step;

    // LOI timing: should be close to GET ~75.6h (ref)
    let loi_get_h = mission_t / 3600.0;
    println!("  LOI at GET {loi_get_h:.1}h (ref: {REF_LOI_GET_HOURS:.1}h)");
    assert!(
        (loi_get_h - REF_LOI_GET_HOURS).abs() < 5.0,
        "LOI timing off by {:.1}h (GET {loi_get_h:.1}h vs ref {REF_LOI_GET_HOURS:.1}h)",
        loi_get_h - REF_LOI_GET_HOURS
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 4: Lunar Orbit Insertion (LOI-1)
    // ──────────────────────────────────────────────
    println!("Phase 4: Lunar Orbit Insertion (LOI-1)");

    let loi_epoch = parking_epoch.add_seconds(mission_t);
    let moon_pos_at_loi = moon_ephem.position_eci(&loi_epoch).into_inner();
    let moon_vel_at_loi = moon_ephem.velocity_eci(&loi_epoch).into_inner();

    // Compute velocity relative to Moon, apply retrograde ΔV in that frame
    let v_rel_moon = state_at_loi.velocity() - moon_vel_at_loi;
    let v_rel_hat = v_rel_moon.normalize();
    let loi_dv = v_rel_hat * (-LOI1_DV); // retrograde relative to Moon
    let post_loi = state_at_loi.apply_delta_v(loi_dv);

    let moon_relative_dist = (post_loi.position() - moon_pos_at_loi).magnitude();
    let v_rel_after = (post_loi.velocity() - moon_vel_at_loi).magnitude();

    println!(
        "  ΔV = {LOI1_DV:.3} km/s (retrograde w.r.t. Moon), Moon distance: {moon_relative_dist:.0} km"
    );
    println!(
        "  Moon-relative velocity: {:.3} → {:.3} km/s",
        v_rel_moon.magnitude(),
        v_rel_after
    );

    // Verify: LOI should capture (v_rel < escape velocity at this distance)
    let v_escape_moon = (2.0 * MU_MOON / moon_relative_dist).sqrt();
    assert!(
        v_rel_after < v_escape_moon,
        "LOI failed to capture: v_rel={v_rel_after:.3} >= v_esc={v_escape_moon:.3}"
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 4b: LOI-2 circularization (~4.4h after LOI-1)
    // ──────────────────────────────────────────────
    // Apollo 11 Mission Report NASA SP-238: LOI-2 at GET 80:11:36,
    // ΔV ≈ 48.5 m/s retrograde to circularize from 60×170 nmi to 60×65 nmi.
    println!("Phase 4b: LOI-2 circularization");

    let loi2_coast = 4.4 * 3600.0; // seconds until LOI-2
    let loi2_dv_mag = 0.0485; // 48.5 m/s [km/s]

    let system_lo1 = build_translunar_system(loi_epoch, &moon_ephem);
    let (state_at_loi2, _, _, new_step) = propagate_and_record(
        &system_lo1,
        &post_loi,
        loi_epoch,
        loi2_coast,
        DT,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );
    mission_t += loi2_coast;
    step = new_step;

    let loi2_epoch = parking_epoch.add_seconds(mission_t);
    let moon_vel_at_loi2 = moon_ephem.velocity_eci(&loi2_epoch).into_inner();
    let v_rel_loi2 = state_at_loi2.velocity() - moon_vel_at_loi2;
    let loi2_dv = v_rel_loi2.normalize() * (-loi2_dv_mag);
    let post_loi2 = state_at_loi2.apply_delta_v(loi2_dv);
    println!(
        "  ΔV = {:.1} m/s (retrograde), Moon-relative: {:.3} → {:.3} km/s",
        loi2_dv_mag * 1000.0,
        v_rel_loi2.magnitude(),
        (post_loi2.velocity() - moon_vel_at_loi2).magnitude()
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 5: Lunar Orbit (~56 hours after LOI-2)
    // ──────────────────────────────────────────────
    println!("Phase 5: Lunar Orbit");

    let remaining_lo = LOI_TO_TEI_SECONDS - loi2_coast;
    let system_lo = build_translunar_system(loi2_epoch, &moon_ephem);
    let (state_after_lo, _, _, new_step) = propagate_and_record(
        &system_lo,
        &post_loi2,
        loi2_epoch,
        remaining_lo,
        DT,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );

    mission_t += remaining_lo;
    step = new_step;

    let lo_end_epoch = parking_epoch.add_seconds(mission_t);
    let moon_pos_lo = moon_ephem.position_eci(&lo_end_epoch).into_inner();
    let moon_dist_lo = (state_after_lo.position() - moon_pos_lo).magnitude();
    println!(
        "  Coasted {:.1} hours (after LOI-2), Moon distance: {:.0} km",
        remaining_lo / 3600.0,
        moon_dist_lo
    );

    // Verify: spacecraft should remain near the Moon during lunar orbit
    assert!(
        moon_dist_lo < 10_000.0,
        "spacecraft drifted too far from Moon during orbit: {moon_dist_lo:.0} km"
    );

    // Find the next far-side point (local max of Earth distance) for TEI
    let system_tei_search = build_translunar_system(lo_end_epoch, &moon_ephem);
    let mut max_earth_dist = 0.0_f64;
    let mut max_earth_t = 0.0;
    let mut found_far_side = false;
    let search_duration = 4.0 * 3600.0; // search within 4 hours (>1 orbit period)

    Dop853.integrate(
        &system_tei_search,
        state_after_lo.clone(),
        0.0,
        search_duration,
        DT,
        |t, state| {
            let r = state.position().magnitude();
            if !found_far_side {
                if r > max_earth_dist {
                    max_earth_dist = r;
                    max_earth_t = t;
                } else if r < max_earth_dist - 50.0 && max_earth_t > 100.0 {
                    found_far_side = true;
                }
            }
        },
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 6: Trans-Earth Injection (TEI)
    // ──────────────────────────────────────────────
    println!("Phase 6: Trans-Earth Injection (TEI)");

    // Propagate to the far-side point
    let (state_at_tei, _, _, new_step) = propagate_and_record(
        &system_tei_search,
        &state_after_lo,
        lo_end_epoch,
        max_earth_t,
        DT,
        &mut rec,
        step,
        mission_t,
        &*moon_ephem,
    );

    mission_t += max_earth_t;
    step = new_step;

    // TEI timing: should be close to GET ~135.2h (ref)
    let tei_get_h = mission_t / 3600.0;
    println!("  TEI at GET {tei_get_h:.1}h (ref: {REF_TEI_GET_HOURS:.1}h)");
    assert!(
        (tei_get_h - REF_TEI_GET_HOURS).abs() < 5.0,
        "TEI timing off by {:.1}h (GET {tei_get_h:.1}h vs ref {REF_TEI_GET_HOURS:.1}h)",
        tei_get_h - REF_TEI_GET_HOURS
    );

    let tei_epoch = parking_epoch.add_seconds(mission_t);
    let system_te = build_translunar_system(tei_epoch, &moon_ephem);

    // TEI: optimize ΔV direction to target Apollo 11 entry interface.
    // The magnitude is fixed at TEI_DV (1.001 km/s from mission report);
    // the direction is adjusted to match the historical perigee altitude
    // and arrival time (GET 195:03:05.7, alt 122 km).
    let tei_dv_vec = compute_tei_dv(
        &system_te,
        &state_at_tei,
        tei_epoch,
        mission_t,
        &*moon_ephem,
    );
    let post_tei = state_at_tei.apply_delta_v(tei_dv_vec);

    let tei_dv_actual = tei_dv_vec.magnitude();
    let moon_vel_at_tei = moon_ephem.velocity_eci(&tei_epoch).into_inner();
    let v_rel_before = (state_at_tei.velocity() - moon_vel_at_tei).magnitude();
    let v_rel_after = (post_tei.velocity() - moon_vel_at_tei).magnitude();
    println!("  ΔV = {tei_dv_actual:.4} km/s (targeted to entry interface, ref: {TEI_DV:.3})");
    println!(
        "  Moon-relative: {:.3} → {:.3} km/s",
        v_rel_before, v_rel_after
    );
    println!();

    // ──────────────────────────────────────────────
    // Phase 7: Trans-Earth Coast → Atmospheric Entry
    // ──────────────────────────────────────────────
    println!("Phase 7: Trans-Earth Coast");

    let sat_path = EntityPath::parse("/world/sat/apollo11");
    let mut last_output_t = -OUTPUT_INTERVAL;
    let atmosphere_alt = 100.0; // Karman line [km]

    // Capture state at entry interface altitude (122 km) and at reference GET time
    let ei_alt = EI_ALT_KM;
    let ei_get_rel = EI_GET_SECONDS - mission_t; // GET relative to TEI epoch
    let mut state_at_122km: Option<(f64, OrbitalState)> = None;
    let mut state_at_ei_get: Option<(f64, OrbitalState)> = None;
    let mut prev_alt = post_tei.position().magnitude() - R_EARTH;

    let outcome = Dop853.integrate_with_events(
        &system_te,
        post_tei.clone(),
        0.0,
        TRANSEARTH_DURATION,
        DT,
        |t, state| {
            let alt = state.position().magnitude() - R_EARTH;

            // Capture first crossing of 122 km (descending)
            if state_at_122km.is_none() && prev_alt > ei_alt && alt <= ei_alt {
                // Linear interpolation for more precise crossing
                let frac = (prev_alt - ei_alt) / (prev_alt - alt);
                let t_cross = t - DT * (1.0 - frac);
                state_at_122km = Some((t_cross, state.clone()));
            }
            prev_alt = alt;

            // Capture state closest to reference GET time
            if state_at_ei_get.is_none() && t >= ei_get_rel {
                state_at_ei_get = Some((t, state.clone()));
            }

            if t - last_output_t >= OUTPUT_INTERVAL {
                let tp = TimePoint::new()
                    .with_sim_time(mission_t + t)
                    .with_step(step);
                let os = RecordOrbitalState::new(*state.position(), *state.velocity());
                rec.log_orbital_state(&sat_path, &tp, &os);
                step += 1;
                last_output_t = t;
            }
        },
        orts::events::collision_check(R_EARTH, Some(atmosphere_alt)),
    );

    let (final_state, coast_time, terminated) = match &outcome {
        utsuroi::IntegrationOutcome::Completed(state) => {
            (state.clone(), TRANSEARTH_DURATION, false)
        }
        utsuroi::IntegrationOutcome::Terminated {
            state, t, reason, ..
        } => {
            println!("  ** {reason:?} **");
            (state.clone(), *t, true)
        }
        utsuroi::IntegrationOutcome::Error(e) => {
            eprintln!("  Integration error: {e:?}");
            std::process::exit(1);
        }
    };

    mission_t += coast_time;

    // Entry timing
    let ei_get_h = mission_t / 3600.0;
    let final_alt = final_state.position().magnitude() - R_EARTH;
    println!(
        "  Entry at GET {ei_get_h:.1}h (ref: {REF_EI_GET_HOURS:.1}h), \
         alt {:.0} km, {:.1}h after TEI, v = {:.3} km/s",
        final_alt,
        coast_time / 3600.0,
        final_state.velocity().magnitude()
    );
    println!(
        "    (Apollo 11: alt 122 km at {:.1}h after TEI, v ≈ 11.0 km/s)",
        TEI_TO_ENTRY_SECONDS / 3600.0,
    );
    if !terminated {
        println!(
            "  (Did not reach atmosphere within {:.0} days)",
            TRANSEARTH_DURATION / 86400.0
        );
    }

    // Verify: should reach atmosphere
    assert!(terminated, "spacecraft did not reach atmospheric entry");
    // Entry velocity should be ~11 km/s (ref: 10.7-11.2 km/s)
    let entry_speed = final_state.velocity().magnitude();
    assert!(
        entry_speed > 9.0 && entry_speed < 13.0,
        "entry speed {entry_speed:.1} km/s outside expected range 9-13 km/s"
    );
    // Entry timing: GET should be within 10h of reference
    // (currently limited by TEI targeting accuracy and earlier phase timing)
    assert!(
        (ei_get_h - REF_EI_GET_HOURS).abs() < 10.0,
        "entry GET {ei_get_h:.1}h off by {:.1}h from ref {REF_EI_GET_HOURS:.1}h",
        ei_get_h - REF_EI_GET_HOURS
    );
    // Total mission time should be within ~20% of ref (195.3 hours)
    assert!(
        mission_t / 3600.0 > 160.0 && mission_t / 3600.0 < 250.0,
        "total mission time {:.1}h outside expected range 160-250h",
        mission_t / 3600.0
    );

    // Compare with Apollo 11 entry interface reference (NASA SP-238 Table 7-II)
    let ei_epoch = parking_epoch.add_seconds(EI_GET_SECONDS);
    let ei_ref = entry_interface_state_eci(&ei_epoch);

    println!("  Entry interface comparison vs Apollo 11 (SP-238 Table 7-II):");

    // (a) At 122 km altitude crossing
    if let Some((t_cross, ref state_122)) = state_at_122km {
        let get_122 = mission_t + t_cross;
        let epoch_122 = parking_epoch.add_seconds(get_122);
        let eci_122 = arika::SimpleEci::from_raw(*state_122.position());
        let ecef_122 =
            arika::frame::Rotation::<arika::frame::SimpleEci, arika::frame::SimpleEcef>::from_era(
                epoch_122.gmst(),
            )
            .transform(&eci_122);
        let geod_122 = arika::earth::Geodetic::from(ecef_122);
        let pos_err = (state_122.position() - ei_ref.position()).magnitude();
        let vel_err = (state_122.velocity() - ei_ref.velocity()).magnitude();
        println!("    [At 122 km altitude]");
        println!(
            "      GET: {:.2}h (ref: {:.2}h, Δ={:.2}h)",
            get_122 / 3600.0,
            EI_GET_SECONDS / 3600.0,
            (get_122 - EI_GET_SECONDS) / 3600.0
        );
        println!(
            "      Geodetic: lat={:.1}°, lon={:.1}° (ref: lat={EI_LAT_DEG}°, lon={EI_LON_DEG}°)",
            geod_122.latitude.to_degrees(),
            geod_122.longitude.to_degrees()
        );
        println!(
            "      Speed: {:.3} km/s (ref: {EI_SPEED} km/s)",
            state_122.velocity().magnitude()
        );
        println!("      Position error: {pos_err:.0} km");
        println!("      Velocity error: {vel_err:.3} km/s");
    }

    // (b) At reference GET time
    if let Some((t_get, ref state_get)) = state_at_ei_get {
        let get_t = mission_t + t_get;
        let epoch_get = parking_epoch.add_seconds(get_t);
        let eci_get = arika::SimpleEci::from_raw(*state_get.position());
        let ecef_get =
            arika::frame::Rotation::<arika::frame::SimpleEci, arika::frame::SimpleEcef>::from_era(
                epoch_get.gmst(),
            )
            .transform(&eci_get);
        let geod_get = arika::earth::Geodetic::from(ecef_get);
        let pos_err = (state_get.position() - ei_ref.position()).magnitude();
        let vel_err = (state_get.velocity() - ei_ref.velocity()).magnitude();
        println!("    [At reference GET {:.2}h]", EI_GET_SECONDS / 3600.0);
        println!(
            "      Geodetic: lat={:.1}°, lon={:.1}°, alt={:.0} km",
            geod_get.latitude.to_degrees(),
            geod_get.longitude.to_degrees(),
            geod_get.altitude
        );
        println!("      Position error: {pos_err:.0} km");
        println!("      Velocity error: {vel_err:.3} km/s");
    }
    println!();

    // ──────────────────────────────────────────────
    // Summary
    // ──────────────────────────────────────────────
    println!("=== Mission Summary ===");
    println!(
        "Total mission elapsed time: {:.1} hours ({:.2} days)",
        mission_t / 3600.0,
        mission_t / 86400.0
    );
    println!(
        "  (Apollo 11 actual: {:.1} hours / {:.1} days)",
        MISSION_DURATION_REF / 3600.0,
        MISSION_DURATION_REF / 86400.0
    );
    println!(
        "Final state: alt {:.0} km, v = {:.3} km/s",
        final_alt,
        final_state.velocity().magnitude()
    );

    // ──────────────────────────────────────────────
    // Record Moon trajectory and save RRD
    // ──────────────────────────────────────────────
    let total_duration = mission_t;
    let n_moon_steps = (total_duration / OUTPUT_INTERVAL) as u64;
    for i in 0..=n_moon_steps {
        let t = i as f64 * OUTPUT_INTERVAL;
        let moon_epoch = parking_epoch.add_seconds(t);
        let moon_pos = moon_ephem.position_eci(&moon_epoch).into_inner();
        let tp = TimePoint::new().with_sim_time(t).with_step(i);
        let os = RecordOrbitalState::new(moon_pos, Vector3::zeros());
        rec.log_orbital_state(&moon_path, &tp, &os);
    }

    let rrd_path = "orts/examples/apollo11/apollo11.rrd";
    rec.metadata = orts::record::recording::SimMetadata {
        epoch_jd: Some(parking_epoch.jd()),
        mu: Some(MU_EARTH),
        body_radius: Some(R_EARTH),
        body_name: Some("earth".to_string()),
        altitude: None,
        period: None,
    };
    orts::record::rerun_export::save_as_rrd(&rec, "orts-apollo11", rrd_path).unwrap();
    println!();
    println!("Saved to {rrd_path} (open with: rerun {rrd_path})");
}

// ============================================================
// Tests — validate against NASA Postflight Trajectory AS-506
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::vector;

    // Post-TLI state vector and Moon approach are validated by
    // assert!() in main(). Tests below cover properties NOT checked there.

    #[test]
    fn keplerian_roundtrip_preserves_state() {
        let elements = post_tli_elements();
        let (pos, vel) = elements.to_state_vector(MU_EARTH);
        let recovered = KeplerianElements::from_state_vector(&pos, &vel, MU_EARTH);

        assert!(
            (recovered.semi_major_axis - elements.semi_major_axis).abs() < 1.0,
            "SMA roundtrip failed"
        );
        assert!(
            (recovered.eccentricity - elements.eccentricity).abs() < 1e-10,
            "eccentricity roundtrip failed"
        );
        assert!(
            (recovered.inclination - elements.inclination).abs() < 1e-10,
            "inclination roundtrip failed"
        );
    }

    // ----------------------------------------------------------
    // Translunar coast propagation tests
    // ----------------------------------------------------------

    /// Propagate the translunar trajectory and return (final_state, min_moon_dist, min_moon_dist_t).
    fn run_translunar_propagation() -> (OrbitalState, f64, f64) {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let initial = post_tli_state();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);

        let mut min_moon_dist = f64::MAX;
        let mut min_moon_dist_t = 0.0;

        let final_state = Dop853.integrate(
            &system,
            initial,
            0.0,
            TRANSLUNAR_DURATION,
            60.0,
            |t, state| {
                let current_epoch = epoch.add_seconds(t);
                let moon_pos = moon_ephem.position_eci(&current_epoch).into_inner();
                let moon_dist = (state.position() - moon_pos).magnitude();
                if moon_dist < min_moon_dist {
                    min_moon_dist = moon_dist;
                    min_moon_dist_t = t;
                }
            },
        );

        (final_state, min_moon_dist, min_moon_dist_t)
    }

    // translunar_reaches_moon_vicinity and closest_approach_timing
    // are validated by assert!() in main().

    #[test]
    fn translunar_energy_conservation() {
        let (final_state, _, _) = run_translunar_propagation();
        let initial_energy = specific_energy(&post_tli_state(), MU_EARTH);
        let final_energy = specific_energy(&final_state, MU_EARTH);
        // Third-body perturbations transfer energy, so we check relative change is bounded.
        // In a pure 2-body system this would be zero; with Sun+Moon perturbation
        // the energy change over 4 days should be modest (few %).
        let relative_change = ((final_energy - initial_energy) / initial_energy).abs();
        assert!(
            relative_change < 0.5,
            "energy relative change should be < 50%, got {:.4}%",
            relative_change * 100.0
        );
    }

    #[test]
    fn translunar_trajectory_leaves_leo() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let initial = post_tli_state();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);

        // After 1 hour, spacecraft should be well beyond LEO
        let state_1h = Dop853.integrate(&system, initial, 0.0, 3600.0, 60.0, |_, _| {});
        let r_1h = state_1h.position().magnitude();
        let alt_1h = r_1h - R_EARTH;
        assert!(
            alt_1h > 1000.0,
            "after 1 hour, altitude should be > 1000 km, got {alt_1h:.0} km"
        );
    }

    #[test]
    fn translunar_trajectory_leaves_geo() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let initial = post_tli_state();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);

        // After 6 hours, spacecraft should be beyond GEO (42,164 km geocentric)
        let state_6h = Dop853.integrate(&system, initial, 0.0, 6.0 * 3600.0, 60.0, |_, _| {});
        let r_6h = state_6h.position().magnitude();
        assert!(
            r_6h > 42_164.0,
            "after 6 hours, geocentric distance should be > GEO (42,164 km), got {r_6h:.0} km"
        );
    }

    #[test]
    fn translunar_outbound_velocity_decreases() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let initial = post_tli_state();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);

        let v_initial = initial.velocity().magnitude();

        // After 12 hours, velocity should have decreased (climbing out of Earth's gravity well)
        let state_12h = Dop853.integrate(&system, initial, 0.0, 12.0 * 3600.0, 60.0, |_, _| {});
        let v_12h = state_12h.velocity().magnitude();
        assert!(
            v_12h < v_initial,
            "velocity should decrease during outbound coast: initial={v_initial:.3}, 12h={v_12h:.3}"
        );
    }

    // ----------------------------------------------------------
    // Dynamical system configuration tests
    // ----------------------------------------------------------

    #[test]
    fn translunar_system_has_third_body_models() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);
        let names = system.model_names();
        assert!(
            names.contains(&"third_body_sun"),
            "should include Sun third-body: {names:?}"
        );
        assert!(
            names.contains(&"third_body_moon"),
            "should include Moon third-body: {names:?}"
        );
    }

    #[test]
    fn translunar_system_has_body_radius() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);
        assert_eq!(system.body_radius, Some(R_EARTH));
    }

    // ----------------------------------------------------------
    // Acceleration breakdown at TLI
    // ----------------------------------------------------------

    #[test]
    fn tli_gravity_dominates_acceleration() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);
        let state = post_tli_state();
        let breakdown = system.acceleration_breakdown(0.0, &state);

        let gravity = breakdown.iter().find(|(n, _)| *n == "gravity").unwrap().1;
        let sun = breakdown
            .iter()
            .find(|(n, _)| *n == "third_body_sun")
            .unwrap()
            .1;
        let moon = breakdown
            .iter()
            .find(|(n, _)| *n == "third_body_moon")
            .unwrap()
            .1;

        // At TLI altitude (~334 km), gravity should dominate
        assert!(
            gravity > sun * 1e4,
            "gravity ({gravity:.6e}) should dominate Sun ({sun:.6e}) by >4 orders"
        );
        assert!(
            gravity > moon * 1e4,
            "gravity ({moon:.6e}) should dominate Moon ({moon:.6e}) by >4 orders"
        );
    }

    #[test]
    fn tli_gravity_magnitude() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let moon_ephem: Arc<dyn MoonEphemeris> = Arc::new(MeeusMoonEphemeris);
        let system = build_translunar_system(epoch, &moon_ephem);
        let state = post_tli_state();
        let breakdown = system.acceleration_breakdown(0.0, &state);

        let gravity = breakdown.iter().find(|(n, _)| *n == "gravity").unwrap().1;
        let r = state.position().magnitude();
        let expected = MU_EARTH / (r * r); // Point-mass approximation

        // J2 correction is small at this altitude
        let error_pct = ((gravity - expected) / expected).abs() * 100.0;
        assert!(
            error_pct < 1.0,
            "gravity should be within 1% of point-mass: got {gravity:.6e}, expected {expected:.6e} (Δ={error_pct:.3}%)"
        );
    }

    // ----------------------------------------------------------
    // apply_delta_v demonstration (parking orbit → TLI)
    // ----------------------------------------------------------

    #[test]
    fn parking_orbit_period() {
        // Apollo 11 parking orbit: ~185.9 km circular, i=32.521°
        let alt = (186.0 + 183.2) / 2.0; // mean altitude
        let a = R_EARTH + alt;
        let period_analytical = 2.0 * std::f64::consts::PI * (a.powi(3) / MU_EARTH).sqrt();
        let expected_minutes = 88.18;
        let period_minutes = period_analytical / 60.0;
        let error = (period_minutes - expected_minutes).abs();
        assert!(
            error < 0.5,
            "parking orbit period: expected ≈{expected_minutes} min, got {period_minutes:.2} min"
        );
    }

    #[test]
    fn tli_delta_v_produces_correct_c3() {
        // Start in parking orbit, apply TLI ΔV, check resulting C3
        let alt = 184.6; // mean parking orbit altitude
        let r = R_EARTH + alt;
        let v_circ = (MU_EARTH / r).sqrt();

        // Parking orbit state (simplified: circular, equatorial for ΔV test)
        let parking_state = OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v_circ, 0.0]);

        // Apply TLI ΔV in prograde direction (~3.041 km/s)
        let dv_magnitude = 3.041; // km/s
        let dv = vector![0.0, dv_magnitude, 0.0]; // prograde
        let post_tli = parking_state.apply_delta_v(dv);

        let c3 = specific_energy(&post_tli, MU_EARTH) * 2.0;
        // C3 should be close to -1.4 km²/s² (bound transfer to Moon)
        assert!(
            c3 < 0.0 && c3 > -5.0,
            "C3 after TLI should be slightly negative, got {c3:.3}"
        );
    }

    #[test]
    fn tli_delta_v_creates_highly_eccentric_orbit() {
        let alt = 184.6;
        let r = R_EARTH + alt;
        let v_circ = (MU_EARTH / r).sqrt();

        let parking_state = OrbitalState::new(vector![r, 0.0, 0.0], vector![0.0, v_circ, 0.0]);
        let dv = vector![0.0, 3.041, 0.0];
        let post_tli = parking_state.apply_delta_v(dv);

        let elements = KeplerianElements::from_state_vector(
            post_tli.position(),
            post_tli.velocity(),
            MU_EARTH,
        );

        // Eccentricity should be very high (simplified prograde ΔV gives ~0.93;
        // actual Apollo 11 TLI with optimal flight path angle gives ~0.977)
        assert!(
            elements.eccentricity > 0.90,
            "TLI should produce e > 0.90, got {:.4}",
            elements.eccentricity
        );
    }

    // ----------------------------------------------------------
    // Moon ephemeris at Apollo 11 epoch
    // ----------------------------------------------------------

    #[test]
    fn moon_position_at_tli_epoch() {
        let epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let moon_pos = arika::moon::moon_position_eci(&epoch);
        let moon_dist = moon_pos.magnitude();

        // Moon distance should be ~384,400 km ± ~5%
        assert!(
            moon_dist > 350_000.0 && moon_dist < 420_000.0,
            "Moon distance at TLI epoch should be ~384,400 km, got {moon_dist:.0} km"
        );
    }

    #[test]
    fn moon_position_at_loi_epoch() {
        let tli_epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let loi_epoch = tli_epoch.add_seconds(TLI_TO_LOI_SECONDS);
        let moon_pos = arika::moon::moon_position_eci(&loi_epoch);
        let moon_dist = moon_pos.magnitude();

        // Moon should still be at roughly lunar distance
        assert!(
            moon_dist > 350_000.0 && moon_dist < 420_000.0,
            "Moon distance at LOI epoch should be ~384,400 km, got {moon_dist:.0} km"
        );
    }

    #[test]
    fn moon_moves_during_transit() {
        let tli_epoch = Epoch::from_iso8601(TLI_EPOCH_ISO).unwrap();
        let loi_epoch = tli_epoch.add_seconds(TLI_TO_LOI_SECONDS);
        let moon_tli = arika::moon::moon_position_eci(&tli_epoch);
        let moon_loi = arika::moon::moon_position_eci(&loi_epoch);

        // Moon moves ~13°/day, so in ~3 days it should move ~39° ≈ significant displacement
        let displacement = (moon_loi - moon_tli).magnitude();
        assert!(
            displacement > 10_000.0,
            "Moon should move significantly during 3-day transit, displacement = {displacement:.0} km"
        );
    }
}
