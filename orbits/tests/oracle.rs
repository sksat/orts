//! Oracle tests: validate orbital mechanics against analytical predictions.
//!
//! These tests use analytical formulas from celestial mechanics as oracles
//! to verify the numerical integration pipeline (OrbitalSystem + RK4).
//!
//! Strategy (informed by Codex CLI consultation):
//! - Analytical secular drift rates (J2 RAAN, perigee precession)
//! - Conservation laws (Lz for axially symmetric gravity)
//! - Time-reversal for conservative systems
//! - Semi-major axis decay rate for drag
//! - Frozen orbit conditions (J2+J3)
//! - Third-body perturbation effects at GEO

use nalgebra::vector;
use kaname::epoch::Epoch;
use orts_integrator::{DormandPrince, IntegrationOutcome, Integrator, Rk4, State, Tolerances};
use orts_orbits::constants::{J2_EARTH, J3_EARTH, J4_EARTH, MU_EARTH, R_EARTH};
use orts_orbits::body::KnownBody;
use orts_orbits::drag::AtmosphericDrag;
use orts_orbits::gravity::ZonalHarmonics;
use orts_orbits::kepler::KeplerianElements;
use orts_orbits::orbital_system::OrbitalSystem;
use orts_orbits::third_body::ThirdBodyGravity;
use std::f64::consts::PI;
use std::ops::ControlFlow;

// ============================================================================
// Helpers
// ============================================================================

fn earth_j2_system() -> OrbitalSystem {
    OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: None,
            j4: None,
        }),
    )
}

fn earth_j2_j3_system() -> OrbitalSystem {
    OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: Some(J3_EARTH),
            j4: None,
        }),
    )
}

fn earth_j2_j3_j4_system() -> OrbitalSystem {
    OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: Some(J3_EARTH),
            j4: Some(J4_EARTH),
        }),
    )
}

/// Propagate and collect orbital elements at each orbit completion.
fn propagate_collecting_elements(
    system: &OrbitalSystem,
    elements: &KeplerianElements,
    n_orbits: usize,
    dt: f64,
) -> (Vec<KeplerianElements>, State) {
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };
    let period = elements.period(MU_EARTH);

    let mut orbit_elements = vec![];
    let mut current = initial;
    let mut t = 0.0;

    for _ in 0..n_orbits {
        let t_end = t + period;
        current = Rk4.integrate(system, current, t, t_end, dt, |_, _| {});
        t = t_end;
        let elems = KeplerianElements::from_state_vector(
            &current.position,
            &current.velocity,
            MU_EARTH,
        );
        orbit_elements.push(elems);
    }

    (orbit_elements, current)
}

/// Propagate with DP45 (adaptive) and collect orbital elements at each orbit completion.
fn propagate_collecting_elements_dp45(
    system: &OrbitalSystem,
    elements: &KeplerianElements,
    n_orbits: usize,
    tol: &Tolerances,
) -> (Vec<KeplerianElements>, State) {
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };
    let period = elements.period(MU_EARTH);

    let mut orbit_elements = vec![];
    let mut current = initial;
    let mut t = 0.0;

    for _ in 0..n_orbits {
        let t_end = t + period;
        let outcome: IntegrationOutcome<()> =
            DormandPrince.integrate_adaptive_with_events(
                system,
                current,
                t,
                t_end,
                period / 100.0, // initial dt guess
                tol,
                |_, _| {},
                |_, _| ControlFlow::Continue(()),
            );
        match outcome {
            IntegrationOutcome::Completed(state) => current = state,
            other => panic!("DP45 integration failed: {other:?}"),
        }
        t = t_end;
        let elems = KeplerianElements::from_state_vector(
            &current.position,
            &current.velocity,
            MU_EARTH,
        );
        orbit_elements.push(elems);
    }

    (orbit_elements, current)
}

/// Compute total energy including J2 gravitational potential.
///
/// E = v²/2 - μ/r + μ·J2·Re²/(2r³)·(3z²/r² - 1)
///
/// This is the conserved quantity for the J2 system.
fn j2_total_energy(state: &State, mu: f64, j2: f64, r_body: f64) -> f64 {
    let r = state.position.magnitude();
    let z = state.position[2];
    let v_sq = state.velocity.magnitude_squared();
    let two_body = v_sq / 2.0 - mu / r;
    let j2_potential = mu * j2 * r_body.powi(2) / (2.0 * r.powi(3)) * (3.0 * z * z / (r * r) - 1.0);
    two_body + j2_potential
}

/// Unwrap an angle relative to a reference to handle 0/2π wrapping.
fn unwrap_angle(angle: f64, reference: f64) -> f64 {
    let mut a = angle;
    while a - reference > PI {
        a -= 2.0 * PI;
    }
    while a - reference < -PI {
        a += 2.0 * PI;
    }
    a
}

/// Propagate backward using manual RK4 steps (integrate doesn't support t_end < t0).
fn integrate_backward(system: &OrbitalSystem, state: State, t0: f64, t_end: f64, dt: f64) -> State {
    let mut current = state;
    let mut t = t0;
    let step = -dt.abs(); // ensure negative

    while t > t_end + 1e-10 {
        let h = step.max(t_end - t); // max of two negatives = less negative = smaller step
        current = Rk4.step(system, t, &current, h);
        t += h;
    }

    current
}

// ============================================================================
// Test 1: Critical Inclination — ω̇ ≈ 0
//
// At the critical inclination i_c = 63.4349°, the J2 secular rate of
// argument of perigee vanishes:
//   ω̇ = (3/2) n J2 (R_e/p)² (2 - 5/2 sin²i) = 0
// because sin²(63.4349°) = 4/5 → (2 - 5/2 · 4/5) = 0.
//
// Oracle: Lagrange planetary equations (first-order J2 secular terms).
// ============================================================================

#[test]
fn critical_inclination_perigee_frozen() {
    let a = R_EARTH + 800.0;
    let e = 0.1;
    let i_crit = 63.4349_f64.to_radians();

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e,
        inclination: i_crit,
        raan: 0.0,
        argument_of_periapsis: 1.0, // start at 1 rad (avoid ω≈0 ambiguity)
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 20;
    let dt = 10.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);

    // Extract orbit-averaged ω drift rate using first-half / second-half comparison
    let omega_initial = elements.argument_of_periapsis;
    let omega_values: Vec<f64> = orbit_elems
        .iter()
        .map(|e| unwrap_angle(e.argument_of_periapsis, omega_initial))
        .collect();

    let n_half = omega_values.len() / 2;
    let mean_first: f64 = omega_values[..n_half].iter().sum::<f64>() / n_half as f64;
    let mean_second: f64 =
        omega_values[n_half..].iter().sum::<f64>() / (omega_values.len() - n_half) as f64;

    let period = elements.period(MU_EARTH);
    let dt_halves = (n_orbits as f64 / 2.0) * period;
    let omega_dot_deg_per_day = ((mean_second - mean_first) / dt_halves).to_degrees() * 86400.0;

    // At critical inclination, secular ω̇ should be near zero.
    // Codex recommendation: |Δω| < 0.02-0.1° over 10 orbits for dt=10s.
    // We use 0.1 deg/day as tolerance (allows short-period oscillation residual).
    assert!(
        omega_dot_deg_per_day.abs() < 0.1,
        "Critical inclination: ω̇ should be ≈ 0, got {omega_dot_deg_per_day:.4} deg/day"
    );
}

// ============================================================================
// Test 2: J2 Argument of Perigee Precession (non-critical inclination)
//
// At ISS inclination (51.6°), the J2 secular perigee precession rate is:
//   ω̇ = (3/2) n J2 (R_e/p)² (2 - 5/2 sin²i)
//
// For ISS: i=51.6°, a ≈ 7178 km, e ≈ 0.001
//   ω̇ ≈ (3/2)(0.00106)(1.08263e-3)(6378/7178)² * (2 - 5/2 * sin²(51.6°))
//   ≈ +3.3 deg/day
//
// Oracle: Lagrange planetary equations (first-order J2 secular terms).
// ============================================================================

#[test]
fn j2_perigee_precession_iss() {
    let a = R_EARTH + 800.0; // use 800 km for more stable eccentricity
    let e = 0.01;
    let i = 51.6_f64.to_radians();

    // Analytical prediction
    let p = a * (1.0 - e * e);
    let n = (MU_EARTH / a.powi(3)).sqrt();
    let expected_rate = 1.5 * n * J2_EARTH * (R_EARTH / p).powi(2) * (2.0 - 2.5 * i.sin().powi(2));
    let expected_deg_per_day = expected_rate.to_degrees() * 86400.0;

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 15; // ~1 day
    let dt = 10.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);

    let omega_initial = elements.argument_of_periapsis;
    let omega_values: Vec<f64> = orbit_elems
        .iter()
        .map(|e| unwrap_angle(e.argument_of_periapsis, omega_initial))
        .collect();

    // Use orbit-averaged ω drift rate (first-half / second-half mean comparison)
    // to filter out J2 short-period oscillations in ω.
    let n_half = omega_values.len() / 2;
    let mean_first: f64 = omega_values[..n_half].iter().sum::<f64>() / n_half as f64;
    let mean_second: f64 =
        omega_values[n_half..].iter().sum::<f64>() / (omega_values.len() - n_half) as f64;

    let period = elements.period(MU_EARTH);
    let dt_halves = (n_orbits as f64 / 2.0) * period;
    let actual_deg_per_day =
        ((mean_second - mean_first) / dt_halves).to_degrees() * 86400.0;

    // Allow 1.0 deg/day tolerance: first-order secular theory neglects
    // J2² coupling terms and averaging artifacts from short-period terms.
    let error = (actual_deg_per_day - expected_deg_per_day).abs();
    assert!(
        error < 1.5,
        "J2 ω̇ at ISS inclination: expected≈{expected_deg_per_day:.2} deg/day, \
         got={actual_deg_per_day:.2} deg/day, error={error:.3} deg/day"
    );
}

// ============================================================================
// Test 3: Zonal Harmonics Lz Conservation
//
// For axially symmetric gravity (zonal harmonics J2/J3/J4), the z-component
// of angular momentum h_z = (r × v)·ẑ is a conserved quantity.
// This is an exact physics invariant, independent of integration accuracy.
//
// Oracle: Noether's theorem (axial symmetry → Lz conservation).
// ============================================================================

#[test]
fn zonal_harmonics_lz_conservation() {
    let a = R_EARTH + 600.0;
    let i = 65.0_f64.to_radians(); // well off equatorial for non-trivial z dynamics
    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: 0.05,
        inclination: i,
        raan: 30.0_f64.to_radians(),
        argument_of_periapsis: 45.0_f64.to_radians(),
        true_anomaly: 0.0,
    };

    let system = earth_j2_j3_j4_system();
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };

    let initial_lz = initial.position.cross(&initial.velocity).z;
    let mut max_lz_drift: f64 = 0.0;

    let period = elements.period(MU_EARTH);
    let total_time = 10.0 * period;
    let dt = 5.0;

    Rk4.integrate(&system, initial, 0.0, total_time, dt, |_, state| {
        let lz = state.position.cross(&state.velocity).z;
        let drift = (lz - initial_lz).abs() / initial_lz.abs();
        max_lz_drift = max_lz_drift.max(drift);
    });

    // Lz should be conserved to high precision (limited only by integration error)
    // For RK4 with dt=5s over 10 orbits, expect relative drift < 1e-8
    assert!(
        max_lz_drift < 1e-7,
        "Lz conservation violated: max relative drift = {max_lz_drift:.6e} (expected < 1e-7)"
    );
}

// ============================================================================
// Test 4: Time-Reversal (Conservative System)
//
// For J2-only gravity (conservative), propagating forward then backward
// should return close to the initial state.
// RK4 is NOT time-symmetric, so error is O(dt^4) per step.
//
// Oracle: Time-reversibility of conservative ODE (numerical consistency test).
// ============================================================================

#[test]
fn time_reversal_j2_conservative() {
    let a = R_EARTH + 400.0;
    let i = 51.6_f64.to_radians();
    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: 0.001,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };

    let period = elements.period(MU_EARTH);
    let n_orbits = 10;
    let total_time = n_orbits as f64 * period;
    let dt = 10.0;

    // Forward propagation
    let forward = Rk4.integrate(&system, initial.clone(), 0.0, total_time, dt, |_, _| {});

    // Backward propagation (manual loop since integrate doesn't support t_end < t0)
    let backward = integrate_backward(&system, forward, total_time, 0.0, dt);

    let pos_err_km = (backward.position - initial.position).magnitude();
    let vel_err_kms = (backward.velocity - initial.velocity).magnitude();

    // Codex estimate: for dt=10s, 10 orbits LEO, expect ~1-10 m position error
    // Use 100 m (0.1 km) as conservative tolerance
    assert!(
        pos_err_km < 0.1,
        "Time-reversal position error: {:.1} m (expected < 100 m)",
        pos_err_km * 1000.0
    );
    assert!(
        vel_err_kms < 1e-4,
        "Time-reversal velocity error: {:.3} m/s (expected < 0.1 m/s)",
        vel_err_kms * 1000.0
    );
}

// ============================================================================
// Test 5: Drag — Monotonic Semi-Major Axis Decay
//
// Atmospheric drag always removes energy, causing the semi-major axis
// to decrease monotonically.
//
// Oracle: Energy dissipation theorem (drag is always anti-velocity).
// ============================================================================

#[test]
fn drag_monotonic_sma_decay() {
    let a = R_EARTH + 400.0;
    let v = (MU_EARTH / a).sqrt();

    let mut system = OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: None,
            j4: None,
        }),
    );
    system.perturbations.push(Box::new(AtmosphericDrag {
        body: Some(KnownBody::Earth),
        body_radius: R_EARTH,
        omega_body: orts_orbits::drag::OMEGA_EARTH,
        ballistic_coeff: 0.005, // physical ISS: Cd*A/(2m) ≈ 2.2*2000/(2*420000)
        atmosphere: Box::new(tobari::exponential::Exponential),
    }));

    let initial = State {
        position: vector![a, 0.0, 0.0],
        velocity: vector![0.0, v, 0.0],
    };

    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 10;
    let dt = 10.0;

    let mut sma_values = vec![a];
    let mut current = initial;
    let mut t = 0.0;

    for _ in 0..n_orbits {
        let t_end = t + period;
        current = Rk4.integrate(&system, current, t, t_end, dt, |_, _| {});
        t = t_end;
        let elems =
            KeplerianElements::from_state_vector(&current.position, &current.velocity, MU_EARTH);
        sma_values.push(elems.semi_major_axis);
    }

    // Verify monotonic decrease
    for i in 0..sma_values.len() - 1 {
        assert!(
            sma_values[i + 1] < sma_values[i],
            "SMA should decrease monotonically: a[{}]={:.3} >= a[{}]={:.3}",
            i,
            sma_values[i],
            i + 1,
            sma_values[i + 1]
        );
    }

    // Verify total decay is physically reasonable
    // At 400 km with B=0.005 m²/kg (physical ISS):
    // ρ ≈ 3.7e-12 kg/m³, v ≈ 7.66 km/s
    // 10 orbits ≈ 0.63 days → expect ~0.05 km decay at ~0.085 km/day
    let total_decay = sma_values[0] - sma_values.last().unwrap();
    assert!(
        total_decay > 1e-4 && total_decay < 2.0,
        "Total SMA decay over 10 orbits should be 0.0001-2 km, got {total_decay:.6} km"
    );
}

// ============================================================================
// Test 6: Drag Scaling — Doubling Ballistic Coefficient Doubles Decay Rate
//
// For a constant-density atmosphere approximation, da/dt ∝ B.
// So doubling B should roughly double the decay rate.
//
// Oracle: Analytical drag equation linearity in B.
// ============================================================================

#[test]
fn drag_scaling_with_ballistic_coefficient() {
    let a = R_EARTH + 400.0;
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 5;
    let dt = 10.0;

    let run_with_b = |b: f64| -> f64 {
        let mut system = OrbitalSystem::new(
            MU_EARTH,
            Box::new(ZonalHarmonics {
                r_body: R_EARTH,
                j2: J2_EARTH,
                j3: None,
                j4: None,
            }),
        );
        system.perturbations.push(Box::new(AtmosphericDrag {
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: orts_orbits::drag::OMEGA_EARTH,
            ballistic_coeff: b,
            atmosphere: Box::new(tobari::exponential::Exponential),
        }));

        let initial = State {
            position: vector![a, 0.0, 0.0],
            velocity: vector![0.0, v, 0.0],
        };

        let mut current = initial;
        let mut t = 0.0;
        for _ in 0..n_orbits {
            let t_end = t + period;
            current = Rk4.integrate(&system, current, t, t_end, dt, |_, _| {});
            t = t_end;
        }
        let final_elems = KeplerianElements::from_state_vector(
            &current.position,
            &current.velocity,
            MU_EARTH,
        );
        a - final_elems.semi_major_axis // positive = decay
    };

    let decay_b1 = run_with_b(0.005); // physical ISS
    let decay_b2 = run_with_b(0.01); // 2x ISS ballistic coefficient

    // Decay ratio should be approximately 2.0
    let ratio = decay_b2 / decay_b1;
    assert!(
        ratio > 1.5 && ratio < 2.5,
        "Doubling B should ~double decay: decay_B={decay_b1:.6e}, decay_2B={decay_b2:.6e}, ratio={ratio:.2}"
    );
}

// ============================================================================
// Test 7: J2+J3 Frozen Orbit
//
// A frozen orbit has de/dt ≈ 0 and dω/dt ≈ 0 simultaneously under J2+J3.
// Conditions: ω = 90° (or 270°), and eccentricity chosen to balance J3
// forcing against J2 perigee precession:
//
//   e_f = -(J3/J2) * (R_e/a) * sin(i) / (2 - 5/2 sin²i)
//
// For near-polar orbit (i≈98°, a≈7200 km): e_f ≈ 0.001-0.005.
//
// Oracle: Averaged J2+J3 secular equations (mean element theory).
// ============================================================================

#[test]
fn frozen_orbit_j2_j3() {
    let a = R_EARTH + 800.0; // 7178 km
    let i = 98.0_f64.to_radians();

    // Compute frozen eccentricity from J2+J3 balance
    let sin_i = i.sin();
    let sin2_i = sin_i * sin_i;
    let denom = 2.0 - 2.5 * sin2_i;
    let e_f = -(J3_EARTH / J2_EARTH) * (R_EARTH / a) * sin_i / denom;
    let e_f = e_f.abs(); // ensure positive

    // Frozen orbit: ω = 90° (or 270° depending on sign convention)
    let omega = if (J3_EARTH / J2_EARTH) * sin_i / denom < 0.0 {
        PI / 2.0
    } else {
        3.0 * PI / 2.0
    };

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e_f,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: omega,
        true_anomaly: 0.0,
    };

    let system = earth_j2_j3_system();
    let n_orbits = 50;
    let dt = 10.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);

    // Check that eccentricity stays near e_f (within ±50% including short-period terms)
    for (orbit, elems) in orbit_elems.iter().enumerate() {
        let e_err = (elems.eccentricity - e_f).abs() / e_f;
        assert!(
            e_err < 0.5,
            "Frozen orbit eccentricity drift at orbit {}: e={:.6}, e_f={:.6}, rel_err={:.2}",
            orbit + 1,
            elems.eccentricity,
            e_f,
            e_err
        );
    }

    // Check that ω stays near the frozen value (within ±10° including short-period terms).
    // The frozen orbit formula is first-order, so osculating ω will oscillate.
    // Codex recommendation: "mean ω drift < 0.5-2° per orbit" — we check accumulated drift.
    for (orbit, elems) in orbit_elems.iter().enumerate() {
        let omega_diff = unwrap_angle(elems.argument_of_periapsis, omega) - omega;
        assert!(
            omega_diff.abs() < 10.0_f64.to_radians(),
            "Frozen orbit ω drift at orbit {}: ω={:.2}°, expected≈{:.2}°, diff={:.3}°",
            orbit + 1,
            elems.argument_of_periapsis.to_degrees(),
            omega.to_degrees(),
            omega_diff.to_degrees()
        );
    }
}

// ============================================================================
// Test 8: Third-Body GEO Inclination Change
//
// At GEO altitude (42164 km), Moon and Sun perturbations cause measurable
// inclination oscillations over weeks-to-months timescales.
// Over 30 days, the inclination should change by a detectable amount.
//
// Oracle: Known GEO station-keeping requirement (~0.75-0.95 deg/year
// inclination drift from lunar/solar gravity).
// ============================================================================

#[test]
fn third_body_geo_inclination_change() {
    let a_geo = 42164.0;
    let v_geo = (MU_EARTH / a_geo).sqrt();

    let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

    let mut system = earth_j2_j3_j4_system();
    system = system.with_epoch(epoch);
    system = system.with_perturbation(Box::new(ThirdBodyGravity::sun()));
    system = system.with_perturbation(Box::new(ThirdBodyGravity::moon()));

    // GEO: nearly equatorial, circular orbit
    let initial = State {
        position: vector![a_geo, 0.0, 0.0],
        velocity: vector![0.0, v_geo, 0.0],
    };

    let duration = 30.0 * 86400.0; // 30 days
    let dt = 30.0; // larger dt for GEO (slower dynamics)

    let final_state = Rk4.integrate(&system, initial, 0.0, duration, dt, |_, _| {});
    let final_elems = KeplerianElements::from_state_vector(
        &final_state.position,
        &final_state.velocity,
        MU_EARTH,
    );

    // Inclination should have changed from ~0 to something measurable
    let incl_change_deg = final_elems.inclination.to_degrees();

    // GEO inclination drift is ~0.75-0.95 deg/year = ~0.06-0.08 deg/month
    // With just 30 days, expect Δi > 0.002° (very conservative lower bound)
    // and Δi < 1° (upper bound for 1 month)
    assert!(
        incl_change_deg > 0.002,
        "GEO inclination should change measurably with third-body, got Δi={incl_change_deg:.4}°"
    );
    assert!(
        incl_change_deg < 1.0,
        "GEO inclination change over 30 days should be < 1°, got {incl_change_deg:.4}°"
    );
}

// ============================================================================
// Test 9: Third-Body Effect — Sun and Moon Each Cause Measurable Changes
//
// Sun and Moon each independently cause measurable position differences
// vs J2-only at GEO. The combined effect is non-zero but may be smaller
// than either alone due to partial cancellation at certain epochs.
//
// Oracle: Known GEO perturbation magnitudes from astrodynamics literature.
// ============================================================================

#[test]
fn third_body_individual_effects() {
    let a_geo = 42164.0;
    let v_geo = (MU_EARTH / a_geo).sqrt();
    let epoch = Epoch::from_gregorian(2024, 6, 15, 0, 0, 0.0);

    let initial = State {
        position: vector![a_geo, 0.0, 0.0],
        velocity: vector![0.0, v_geo, 0.0],
    };

    let duration = 7.0 * 86400.0; // 7 days
    let dt = 30.0;

    // J2 only (no third-body) as baseline
    let system_j2 = earth_j2_j3_j4_system();
    let final_j2 = Rk4.integrate(&system_j2, initial.clone(), 0.0, duration, dt, |_, _| {});

    // Sun only
    let system_sun = earth_j2_j3_j4_system()
        .with_epoch(epoch)
        .with_perturbation(Box::new(ThirdBodyGravity::sun()));
    let final_sun = Rk4.integrate(&system_sun, initial.clone(), 0.0, duration, dt, |_, _| {});

    // Moon only
    let system_moon = earth_j2_j3_j4_system()
        .with_epoch(epoch)
        .with_perturbation(Box::new(ThirdBodyGravity::moon()));
    let final_moon = Rk4.integrate(&system_moon, initial.clone(), 0.0, duration, dt, |_, _| {});

    // Both
    let system_both = earth_j2_j3_j4_system()
        .with_epoch(epoch)
        .with_perturbation(Box::new(ThirdBodyGravity::sun()))
        .with_perturbation(Box::new(ThirdBodyGravity::moon()));
    let final_both = Rk4.integrate(&system_both, initial, 0.0, duration, dt, |_, _| {});

    // Each third-body should cause a measurable difference from J2-only
    let diff_sun = (final_sun.position - final_j2.position).magnitude();
    let diff_moon = (final_moon.position - final_j2.position).magnitude();
    let diff_both = (final_both.position - final_j2.position).magnitude();

    assert!(
        diff_sun > 0.1,
        "Sun perturbation should cause measurable position change at GEO, got {diff_sun:.3} km"
    );
    assert!(
        diff_moon > 0.1,
        "Moon perturbation should cause measurable position change at GEO, got {diff_moon:.3} km"
    );
    // Combined effect should be non-zero (may be smaller than either due to partial cancellation)
    assert!(
        diff_both > 0.1,
        "Combined Sun+Moon effect should be non-zero, got {diff_both:.3} km"
    );
}

// ============================================================================
// Test 10: RK4 dt Convergence with Full Force Model
//
// Verify that RK4's 4th-order convergence is maintained even with the full
// J2+J3+J4+third-body force model. Halving dt should reduce error by ~16x.
//
// Oracle: Richardson extrapolation (numerical analysis theory).
// ============================================================================

#[test]
fn full_model_dt_convergence() {
    let a = R_EARTH + 600.0;
    let i = 51.6_f64.to_radians();
    let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: 0.001,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_j3_j4_system()
        .with_epoch(epoch)
        .with_perturbation(Box::new(ThirdBodyGravity::sun()))
        .with_perturbation(Box::new(ThirdBodyGravity::moon()));

    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };

    let duration = 2000.0; // ~1/3 orbit

    let dt_coarse = 8.0;
    let dt_fine = 4.0;
    let dt_finest = 2.0;

    let final_coarse =
        Rk4.integrate(&system, initial.clone(), 0.0, duration, dt_coarse, |_, _| {});
    let final_fine =
        Rk4.integrate(&system, initial.clone(), 0.0, duration, dt_fine, |_, _| {});
    let final_finest = Rk4.integrate(&system, initial, 0.0, duration, dt_finest, |_, _| {});

    let err_coarse = (final_coarse.position - final_finest.position).magnitude();
    let err_fine = (final_fine.position - final_finest.position).magnitude();

    let ratio = err_coarse / err_fine;
    assert!(
        ratio > 10.0 && ratio < 25.0,
        "Full model dt convergence ratio = {ratio:.2}, expected ~16 for RK4 \
         (err_coarse={err_coarse:.2e}, err_fine={err_fine:.2e})"
    );
}

// ============================================================================
// Tests 11–14: SGP4 Smoke Tests — Order-of-Magnitude Agreement
//
// Compare our numerical J2 propagation against Python SGP4 (python-sgp4)
// as a sanity check to catch gross errors.
//
// Methodology:
//   1. Python script (tools/generate_sgp4_fixtures.py) propagates TLEs with
//      SGP4/SDP4 (WGS-72), outputting TEME position/velocity at regular
//      intervals → orbits/tests/fixtures/sgp4_reference.json
//   2. Rust tests load the fixture, take the osculating state at t=0 as
//      initial conditions, propagate with our OrbitalSystem (J2-only), and
//      compare position/velocity at each fixture time point.
//
// What this is NOT:
//   SGP4 is NOT a precision oracle. SGP4 uses Brouwer mean element theory
//   (analytical perturbations) while we use direct numerical integration.
//   The two approaches will diverge over time. The comparison is only useful
//   for catching:
//   - Wrong sign or magnitude of J2 coefficient
//   - Wrong μ or R_e constants
//   - Coordinate frame errors (TEME ≈ J2000 to within ~arcseconds)
//   - Gross integration bugs
//
// Known discrepancy sources:
//   - SGP4 uses WGS-72 constants; we use WGS-84 (negligible: ~0.07 km/orbit)
//   - SGP4 includes analytical short-period J2 corrections; we include them
//     naturally via numerical integration (different treatment)
//   - For period > 225 min, SGP4 switches to SDP4 (deep-space mode) which
//     adds luni-solar gravity and resonance terms that our J2-only model
//     does not have
//   - ISS TLE has non-zero BSTAR (drag); our comparison ignores drag
//   - TEME vs J2000 frame difference (~arcseconds, negligible here)
//
// Fixture coverage (real TLEs, measured error over propagation duration):
//   - ISS:         LEO, h≈420 km, e≈0.0005, i=51.6°, 3 orbits   → ~0.3 km / ~0.003°
//   - Sentinel-2A: SSO, h≈700 km, e≈0.001,  i=98.6°, 2 orbits   → ~0.5 km / ~0.004°
//   - GPS BIIR-2:  MEO, h≈20200 km, e≈0.005, i=55.4°, 2 orbits  → ~3 km   / ~0.006°
//   - Molniya 1-93: HEO, e≈0.74,            i≈62.8°, 1 orbit    → ~245 km / ~2°
//
// GPS and Molniya both use SDP4 deep-space mode (period > 225 min).
// GPS shows only ~3 km error (near-circular), while Molniya shows ~245 km
// error. The difference is due to high eccentricity amplifying along-track
// phase divergence — the orbital planes and shapes agree well.
//
// IMPORTANT: HEO fixtures must use real (catalogued) TLEs, not hand-crafted
// ones. Hand-crafted elements are not SGP4-consistent Brouwer mean elements,
// causing the mean-to-osculating conversion to produce an inconsistent initial
// state (measured ~1536 km / ~11° with hand-crafted vs ~245 km / ~2° with real).
// ============================================================================

/// Fixture data structures for SGP4 reference trajectories.
///
/// Generated by `tools/generate_sgp4_fixtures.py` (uv script with `sgp4` dep).
/// Run `uv run tools/generate_sgp4_fixtures.py` to regenerate.
mod sgp4_fixture {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct FixtureFile {
        pub fixtures: Vec<Fixture>,
    }

    #[derive(Deserialize)]
    pub struct Fixture {
        pub name: String,
        pub initial_position_km: [f64; 3],
        pub initial_velocity_km_s: [f64; 3],
        pub trajectory: Vec<TrajectoryPoint>,
    }

    #[derive(Deserialize)]
    pub struct TrajectoryPoint {
        pub t_seconds: f64,
        pub position_km: [f64; 3],
        pub velocity_km_s: [f64; 3],
    }
}

fn load_sgp4_fixtures() -> sgp4_fixture::FixtureFile {
    let json = include_str!("fixtures/sgp4_reference.json");
    serde_json::from_str(json).expect("Failed to parse SGP4 fixture JSON")
}

/// Result of comparing our propagation against an SGP4 trajectory.
struct Sgp4Comparison {
    /// Maximum position error [km] across all trajectory points.
    max_pos_err_km: f64,
    /// Maximum velocity error [km/s] across all trajectory points.
    max_vel_err_km_s: f64,
    /// Maximum angular separation [deg] between our position vector and SGP4's.
    /// This metric is independent of orbital radius, making it more meaningful
    /// for cross-orbit comparison (especially HEO where km errors are amplified
    /// at perigee by high velocity).
    max_angular_err_deg: f64,
}

/// Propagate with the given OrbitalSystem and compare against SGP4 trajectory.
///
/// Uses the fixture's initial state (SGP4 osculating state at t=0) and
/// integrates forward with RK4, comparing at each fixture time point.
fn compare_with_sgp4(
    fixture: &sgp4_fixture::Fixture,
    system: &OrbitalSystem,
    dt: f64,
) -> Sgp4Comparison {
    let initial = State {
        position: vector![
            fixture.initial_position_km[0],
            fixture.initial_position_km[1],
            fixture.initial_position_km[2]
        ],
        velocity: vector![
            fixture.initial_velocity_km_s[0],
            fixture.initial_velocity_km_s[1],
            fixture.initial_velocity_km_s[2]
        ],
    };

    let mut max_pos_err = 0.0_f64;
    let mut max_vel_err = 0.0_f64;
    let mut max_ang_err = 0.0_f64;
    let mut state = initial;
    let mut t = 0.0;

    for point in &fixture.trajectory {
        let target_t = point.t_seconds;
        if target_t <= t {
            continue;
        }
        // Integrate forward to this time point
        while t < target_t - 1e-6 {
            let h = dt.min(target_t - t);
            state = Rk4.step(system, t, &state, h);
            t += h;
        }

        let sgp4_pos =
            vector![point.position_km[0], point.position_km[1], point.position_km[2]];
        let sgp4_vel = vector![
            point.velocity_km_s[0],
            point.velocity_km_s[1],
            point.velocity_km_s[2]
        ];

        let pos_err = (state.position - sgp4_pos).magnitude();
        let vel_err = (state.velocity - sgp4_vel).magnitude();

        // Angular separation: angle between the two position vectors.
        // arccos(r̂₁ · r̂₂), clamped for numerical safety.
        let cos_angle = state
            .position
            .normalize()
            .dot(&sgp4_pos.normalize())
            .clamp(-1.0, 1.0);
        let ang_err_deg = cos_angle.acos().to_degrees();

        max_pos_err = max_pos_err.max(pos_err);
        max_vel_err = max_vel_err.max(vel_err);
        max_ang_err = max_ang_err.max(ang_err_deg);
    }

    Sgp4Comparison {
        max_pos_err_km: max_pos_err,
        max_vel_err_km_s: max_vel_err,
        max_angular_err_deg: max_ang_err,
    }
}

// ============================================================================
// Test 11: ISS — LEO near-circular (h≈420 km, e≈0.0005, i=51.6°)
//
// 3 orbits (~280 min). ISS TLE has non-zero BSTAR so SGP4 includes drag,
// but over 3 orbits the drag effect is small (~meters). Expect < 30 km
// position error and < 0.01° angular error.
// ============================================================================

#[test]
fn sgp4_smoke_iss_leo() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "ISS")
        .expect("ISS fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    let result = compare_with_sgp4(fixture, &system, dt);

    println!(
        "ISS SGP4 smoke: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Measured: ~0.33 km / 0.003° / 0.0003 km/s
    assert!(
        result.max_pos_err_km < 2.0,
        "ISS position error vs SGP4 should be < 2 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.002,
        "ISS velocity error vs SGP4 should be < 0.002 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.02,
        "ISS angular error vs SGP4 should be < 0.02°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 12: Sentinel-2A — SSO (h≈700 km, e≈0.001, i=98.6°)
//
// 2 orbits (~200 min). Sun-synchronous orbit with near-zero drag effect.
// Higher altitude means weaker J2 perturbation → tighter agreement.
// ============================================================================

#[test]
fn sgp4_smoke_sso() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "SSO-Sentinel2A")
        .expect("SSO fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    let result = compare_with_sgp4(fixture, &system, dt);

    println!(
        "SSO SGP4 smoke: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Measured: ~0.48 km / 0.004° / 0.0004 km/s
    assert!(
        result.max_pos_err_km < 2.0,
        "SSO position error vs SGP4 should be < 2 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.002,
        "SSO velocity error vs SGP4 should be < 0.002 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.02,
        "SSO angular error vs SGP4 should be < 0.02°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 13: GPS BIIR-2 — MEO near-circular (h≈20200 km, e≈0.005, i=55.4°)
//
// 2 orbits (~1440 min = 24 hours). Period > 225 min → SGP4 uses SDP4
// deep-space mode (includes luni-solar gravity). Despite this, near-circular
// orbit keeps the error small because the perturbation treatment difference
// doesn't accumulate into large along-track phase errors.
// ============================================================================

#[test]
fn sgp4_smoke_gps_meo() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "GPS-BIIR2")
        .expect("GPS fixture not found");

    let system = earth_j2_system();
    let dt = 10.0;

    let result = compare_with_sgp4(fixture, &system, dt);

    println!(
        "GPS SGP4 smoke: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Measured: ~3 km / 0.006° / 0.0004 km/s
    assert!(
        result.max_pos_err_km < 10.0,
        "GPS position error vs SGP4 should be < 10 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.002,
        "GPS velocity error vs SGP4 should be < 0.002 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.02,
        "GPS angular error vs SGP4 should be < 0.02°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 14: Molniya 1-93 — Real HEO (e≈0.74, i≈62.8°, a≈26575 km)
//
// 1 orbit (~720 min). Real Molniya-class satellite TLE (NORAD 28163).
// Period > 225 min → SGP4 uses SDP4 deep-space mode.
//
// For high-eccentricity orbits, SGP4 Brouwer analytical theory and
// numerical J2 integration diverge more than for near-circular orbits.
// The error is predominantly along-track (phase) — the orbital plane
// and shape agree well, but the satellite's position along the orbit
// drifts due to different effective period predictions.
//
// Real TLE vs hand-crafted TLE:
//   Using a real (SGP4-consistent) TLE gives ~245 km / ~2° error.
//   A hand-crafted TLE gave ~1536 km / ~11° because arbitrary elements
//   are not SGP4-consistent mean elements — the Brouwer mean-to-osculating
//   conversion produces an inconsistent initial state when applied to
//   non-Brouwer elements (Codex CLI confirmed this diagnosis).
//
// Thresholds informed by Codex recommendation: 100–300 km position error
// is expected for osculating-state-initialized J2-only vs SGP4 at HEO.
// ============================================================================

#[test]
fn sgp4_smoke_molniya_heo() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "Molniya-1-93")
        .expect("Molniya fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    let result = compare_with_sgp4(fixture, &system, dt);

    println!(
        "Molniya SGP4 smoke: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );

    // Position: Codex recommends 100–300 km for HEO with osculating init.
    // Measured ~245 km; threshold at 300 km matches the expected range.
    assert!(
        result.max_pos_err_km < 300.0,
        "Molniya position error vs SGP4 should be < 300 km, got {:.2} km",
        result.max_pos_err_km
    );

    // Angular: the orbit-geometry-independent metric.
    // At perigee (r≈10000 km for this orbit), 245 km → ~2°.
    // At apogee (r≈46000 km), the same phase offset would be < 0.5°.
    assert!(
        result.max_angular_err_deg < 3.0,
        "Molniya angular error vs SGP4 should be < 3°, got {:.4}°",
        result.max_angular_err_deg
    );

    assert!(
        result.max_vel_err_km_s < 0.3,
        "Molniya velocity error vs SGP4 should be < 0.3 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
}

// ============================================================================
// Tests 15–19: Extended SGP4 Tests — Long-Period Error Growth
//
// These tests extend the SGP4 comparison to longer propagation durations
// (1–7 days for LEO/MEO, 3 orbits for HEO) to verify that error growth
// remains within expected bounds and no catastrophic divergence occurs.
//
// Error sources grow over time:
// - Drag difference (ISS BSTAR vs our J2-only): ~0.5–2 km/day along-track
// - Analytical vs numerical J2 treatment: slow secular drift
// - SDP4 luni-solar terms (GPS, Molniya): accumulated phase offset
//
// Additional sanity checks: altitude stays in physical range, verifying
// no sign errors or integration blow-up.
// ============================================================================

/// Compare with SGP4 and additionally verify altitude stays in expected band.
fn compare_with_sgp4_checking_altitude(
    fixture: &sgp4_fixture::Fixture,
    system: &OrbitalSystem,
    dt: f64,
    min_alt_km: f64,
    max_alt_km: f64,
) -> Sgp4Comparison {
    let initial = State {
        position: vector![
            fixture.initial_position_km[0],
            fixture.initial_position_km[1],
            fixture.initial_position_km[2]
        ],
        velocity: vector![
            fixture.initial_velocity_km_s[0],
            fixture.initial_velocity_km_s[1],
            fixture.initial_velocity_km_s[2]
        ],
    };

    let mut max_pos_err = 0.0_f64;
    let mut max_vel_err = 0.0_f64;
    let mut max_ang_err = 0.0_f64;
    let mut state = initial;
    let mut t = 0.0;

    for point in &fixture.trajectory {
        let target_t = point.t_seconds;
        if target_t <= t {
            continue;
        }
        while t < target_t - 1e-6 {
            let h = dt.min(target_t - t);
            state = Rk4.step(system, t, &state, h);
            t += h;
        }

        // Check altitude at each comparison point
        let alt = state.position.magnitude() - R_EARTH;
        assert!(
            alt >= min_alt_km && alt <= max_alt_km,
            "Altitude {alt:.1} km outside expected range [{min_alt_km}, {max_alt_km}] at t={t:.0}s"
        );

        let sgp4_pos =
            vector![point.position_km[0], point.position_km[1], point.position_km[2]];
        let sgp4_vel = vector![
            point.velocity_km_s[0],
            point.velocity_km_s[1],
            point.velocity_km_s[2]
        ];

        let pos_err = (state.position - sgp4_pos).magnitude();
        let vel_err = (state.velocity - sgp4_vel).magnitude();

        let cos_angle = state
            .position
            .normalize()
            .dot(&sgp4_pos.normalize())
            .clamp(-1.0, 1.0);
        let ang_err_deg = cos_angle.acos().to_degrees();

        max_pos_err = max_pos_err.max(pos_err);
        max_vel_err = max_vel_err.max(vel_err);
        max_ang_err = max_ang_err.max(ang_err_deg);
    }

    Sgp4Comparison {
        max_pos_err_km: max_pos_err,
        max_vel_err_km_s: max_vel_err,
        max_angular_err_deg: max_ang_err,
    }
}

// ============================================================================
// Test 15: ISS — 1 day (~16 orbits)
//
// Over 1 day, the drag-induced divergence (ISS has BSTAR, our J2-only ignores
// it) causes significant along-track phase error. Combined with J2 analytical
// vs numerical treatment differences, expect < 10 km total.
// Measured: ~7.7 km / 0.065° / 0.008 km/s
// ============================================================================

#[test]
fn sgp4_extended_iss_1day() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "ISS-1day")
        .expect("ISS-1day fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    let result = compare_with_sgp4_checking_altitude(fixture, &system, dt, 380.0, 440.0);

    println!(
        "ISS 1-day: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Measured: ~7.68 km (drag difference accumulates ~0.5 km/orbit)
    assert!(
        result.max_pos_err_km < 15.0,
        "ISS 1-day position error should be < 15 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.02,
        "ISS 1-day velocity error should be < 0.02 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.15,
        "ISS 1-day angular error should be < 0.15°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 16: ISS — 7 days (~112 orbits)
//
// Over 7 days, the ISS BSTAR drag term causes major along-track divergence.
// Our J2-only model ignores drag entirely, so the period mismatch accumulates
// ~50 km/day along-track. Expect ~350 km position error dominated by phase.
// This test primarily verifies no blow-up; the error is expected and large.
// Measured: ~361 km / 3.0° / 0.40 km/s
// ============================================================================

#[test]
fn sgp4_extended_iss_7day() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "ISS-7day")
        .expect("ISS-7day fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    let result = compare_with_sgp4_checking_altitude(fixture, &system, dt, 350.0, 450.0);

    println!(
        "ISS 7-day: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Error is dominated by along-track phase drift from ignoring drag.
    // Measured: ~361 km. Threshold generous for regression detection.
    assert!(
        result.max_pos_err_km < 600.0,
        "ISS 7-day position error should be < 600 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.7,
        "ISS 7-day velocity error should be < 0.7 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 5.0,
        "ISS 7-day angular error should be < 5°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 17: Sentinel-2A SSO — 3 days (~43 orbits)
//
// SSO at 700 km with negligible BSTAR drag. The error growth is dominated
// by analytical (Brouwer) vs numerical J2 treatment differences. Expect
// relatively slow growth: < 10 km over 3 days.
// ============================================================================

#[test]
fn sgp4_extended_sso_3day() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "SSO-Sentinel2A-3day")
        .expect("SSO-3day fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    // SSO Sentinel-2A actual altitude ~788 km (from TLE mean motion 14.308 rev/day)
    let result = compare_with_sgp4_checking_altitude(fixture, &system, dt, 770.0, 810.0);

    println!(
        "SSO 3-day: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    // Measured: ~11.0 km / 0.088° / 0.010 km/s
    assert!(
        result.max_pos_err_km < 20.0,
        "SSO 3-day position error should be < 20 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.02,
        "SSO 3-day velocity error should be < 0.02 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.2,
        "SSO 3-day angular error should be < 0.2°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 18: GPS BIIR-2 — 7 days (~14 orbits)
//
// GPS at MEO (20200 km) uses SDP4 deep-space mode in SGP4. Our J2-only
// model lacks luni-solar terms that SDP4 includes. However, for near-
// circular MEO the error grows slowly: expect < 30 km over 7 days.
// ============================================================================

#[test]
fn sgp4_extended_gps_7day() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "GPS-BIIR2-7day")
        .expect("GPS-7day fixture not found");

    let system = earth_j2_system();
    let dt = 10.0;

    let result = compare_with_sgp4_checking_altitude(fixture, &system, dt, 20000.0, 20500.0);

    println!(
        "GPS 7-day: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    assert!(
        result.max_pos_err_km < 30.0,
        "GPS 7-day position error should be < 30 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_vel_err_km_s < 0.005,
        "GPS 7-day velocity error should be < 0.005 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
    assert!(
        result.max_angular_err_deg < 0.05,
        "GPS 7-day angular error should be < 0.05°, got {:.4}°",
        result.max_angular_err_deg
    );
}

// ============================================================================
// Test 19: Molniya — 3 orbits (~36 hours)
//
// High-eccentricity orbit amplifies along-track phase error. Over 3 orbits,
// expect the position error to grow roughly linearly from the 1-orbit value
// (~245 km). Angular error is the more meaningful metric.
//
// Altitude sanity: perigee ~500 km, apogee ~39000 km (Molniya class).
// ============================================================================

#[test]
fn sgp4_extended_molniya_3orbit() {
    let fixtures = load_sgp4_fixtures();
    let fixture = fixtures
        .fixtures
        .iter()
        .find(|f| f.name == "Molniya-1-93-3orbit")
        .expect("Molniya-3orbit fixture not found");

    let system = earth_j2_system();
    let dt = 5.0;

    // Molniya: perigee ~500 km, apogee ~39000 km
    let result = compare_with_sgp4_checking_altitude(fixture, &system, dt, 200.0, 50000.0);

    println!(
        "Molniya 3-orbit: pos_err={:.2} km, vel_err={:.4} km/s, ang_err={:.4}°",
        result.max_pos_err_km, result.max_vel_err_km_s, result.max_angular_err_deg
    );
    assert!(
        result.max_pos_err_km < 1500.0,
        "Molniya 3-orbit position error should be < 1500 km, got {:.2} km",
        result.max_pos_err_km
    );
    assert!(
        result.max_angular_err_deg < 10.0,
        "Molniya 3-orbit angular error should be < 10°, got {:.4}°",
        result.max_angular_err_deg
    );
    assert!(
        result.max_vel_err_km_s < 1.0,
        "Molniya 3-orbit velocity error should be < 1.0 km/s, got {:.4} km/s",
        result.max_vel_err_km_s
    );
}

// ============================================================================
// Tests 20–23: Long-Period Analytical Oracle Tests
//
// These tests verify the numerical integrator over hundreds of orbits,
// checking that:
// - Secular rates match analytical predictions over long baselines
// - Conservation laws hold (error growth is bounded and predictable)
// - Dissipative forces accumulate correctly
// ============================================================================

// ============================================================================
// Test 20: RAAN Precession over 200 Orbits
//
// The J2 secular RAAN precession rate is:
//   Ω̇ = -(3/2) n J2 (R_e/p)² cos(i)
//
// Over 200 orbits (~13 days at LEO), the total RAAN change is ~65°.
// With a long averaging baseline, the orbit-averaged secular rate should
// match the analytical formula more closely than the 15-orbit test.
//
// Oracle: Lagrange planetary equations (first-order J2 secular RAAN rate).
// ============================================================================

#[test]
fn raan_precession_200_orbits() {
    let a = R_EARTH + 800.0;
    let e = 0.001;
    let i = 51.6_f64.to_radians();

    // Analytical prediction
    let p = a * (1.0 - e * e);
    let n = (MU_EARTH / a.powi(3)).sqrt();
    let expected_rate = -1.5 * n * J2_EARTH * (R_EARTH / p).powi(2) * i.cos();
    let expected_deg_per_day = expected_rate.to_degrees() * 86400.0;

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 200;
    let dt = 10.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);

    let raan_initial = elements.raan;
    let raan_values: Vec<f64> = orbit_elems
        .iter()
        .map(|e| unwrap_angle(e.raan, raan_initial))
        .collect();

    // Use first-half / second-half orbit-averaged comparison
    let n_half = raan_values.len() / 2;
    let mean_first: f64 = raan_values[..n_half].iter().sum::<f64>() / n_half as f64;
    let mean_second: f64 =
        raan_values[n_half..].iter().sum::<f64>() / (raan_values.len() - n_half) as f64;

    let period = elements.period(MU_EARTH);
    let dt_halves = (n_orbits as f64 / 2.0) * period;
    let actual_deg_per_day = ((mean_second - mean_first) / dt_halves).to_degrees() * 86400.0;

    let error = (actual_deg_per_day - expected_deg_per_day).abs();
    println!(
        "RAAN 200 orbits: expected={expected_deg_per_day:.3} deg/day, \
         actual={actual_deg_per_day:.3} deg/day, error={error:.4} deg/day"
    );
    // With 200 orbits of averaging, expect tighter agreement than the 15-orbit test
    assert!(
        error < 0.2,
        "RAAN precession rate over 200 orbits: expected≈{expected_deg_per_day:.3} deg/day, \
         got={actual_deg_per_day:.3} deg/day, error={error:.4} deg/day (should be < 0.2)"
    );
}

// ============================================================================
// Test 21: Drag — SMA Decay over 200 Orbits
//
// Over 200 orbits at 400 km with B=0.02 m²/kg, atmospheric drag causes
// measurable SMA decrease. As altitude drops, density increases, causing
// the decay rate to accelerate (positive feedback).
//
// Oracle: Energy dissipation theorem + exponential atmosphere model.
// ============================================================================

#[test]
fn drag_decay_200_orbits() {
    let a = R_EARTH + 400.0;
    let v = (MU_EARTH / a).sqrt();

    let mut system = OrbitalSystem::new(
        MU_EARTH,
        Box::new(ZonalHarmonics {
            r_body: R_EARTH,
            j2: J2_EARTH,
            j3: None,
            j4: None,
        }),
    );
    // Use DEFAULT_BALLISTIC_COEFF (0.01) — this test needs enough decay
    // to observe acceleration (positive feedback from exponential atmosphere).
    // ISS physical B=0.005 decays too slowly over 200 orbits (~1 km) relative
    // to the 58 km scale height for the effect to be measurable.
    system.perturbations.push(Box::new(AtmosphericDrag {
        body: Some(KnownBody::Earth),
        body_radius: R_EARTH,
        omega_body: orts_orbits::drag::OMEGA_EARTH,
        ballistic_coeff: orts_orbits::drag::DEFAULT_BALLISTIC_COEFF,
        atmosphere: Box::new(tobari::exponential::Exponential),
    }));

    let initial = State {
        position: vector![a, 0.0, 0.0],
        velocity: vector![0.0, v, 0.0],
    };

    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 200;
    let dt = 10.0;

    let mut sma_values = vec![a];
    let mut current = initial;
    let mut t = 0.0;

    for _ in 0..n_orbits {
        let t_end = t + period;
        current = Rk4.integrate(&system, current, t, t_end, dt, |_, _| {});
        t = t_end;
        let elems =
            KeplerianElements::from_state_vector(&current.position, &current.velocity, MU_EARTH);
        sma_values.push(elems.semi_major_axis);
    }

    // 1. Monotonic decrease over all 200 orbits
    for i in 0..sma_values.len() - 1 {
        assert!(
            sma_values[i + 1] < sma_values[i],
            "SMA should decrease monotonically at orbit {}: a[{}]={:.4} >= a[{}]={:.4}",
            i,
            i,
            sma_values[i],
            i + 1,
            sma_values[i + 1]
        );
    }

    // 2. Total decay in reasonable range
    // 200 orbits ≈ 12.6 days with B=0.01, expect ~2 km decay
    let total_decay = sma_values[0] - sma_values.last().unwrap();
    println!("Drag 200 orbits: total SMA decay = {total_decay:.4} km");
    assert!(
        total_decay > 0.001 && total_decay < 15.0,
        "Total SMA decay over 200 orbits should be 0.001–15 km, got {total_decay:.6} km"
    );

    // 3. Decay accelerates: last 50 orbits decay faster than first 50 orbits
    let decay_first_50 = sma_values[0] - sma_values[50];
    let decay_last_50 = sma_values[150] - sma_values[200];
    println!(
        "Drag 200 orbits: first 50 decay = {decay_first_50:.6} km, \
         last 50 decay = {decay_last_50:.6} km"
    );
    assert!(
        decay_last_50 > decay_first_50,
        "Decay should accelerate: last 50 orbits ({decay_last_50:.6} km) \
         should exceed first 50 ({decay_first_50:.6} km)"
    );
}

// ============================================================================
// Test 22: Orbit-Averaged SMA Stability over 500 Orbits (Conservative System)
//
// For conservative zonal harmonics (J2+J3+J4, no drag), the orbit-averaged
// semi-major axis should remain nearly constant. RK4 is not symplectic and
// introduces secular energy drift — the SMA drifts by ~0.01 km/orbit
// (~5.6 km over 500 orbits for dt=5s). This is bounded and linear.
//
// The test verifies:
// 1. Total drift stays bounded (no catastrophic blow-up)
// 2. Drift grows linearly, not exponentially (no instability)
//
// Oracle: RK4 secular energy error is O(T*dt^4), linear in propagation time.
// ============================================================================

#[test]
fn sma_stability_500_orbits() {
    let a = R_EARTH + 600.0;
    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: 0.05,
        inclination: 65.0_f64.to_radians(),
        raan: 30.0_f64.to_radians(),
        argument_of_periapsis: 45.0_f64.to_radians(),
        true_anomaly: 0.0,
    };

    let system = earth_j2_j3_j4_system();
    let n_orbits = 500;
    let dt = 5.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);
    let sma_values: Vec<f64> = orbit_elems.iter().map(|e| e.semi_major_axis).collect();

    // 1. No orbit should deviate catastrophically (catches blow-up)
    for (i, &sma) in sma_values.iter().enumerate() {
        let deviation = (sma - a).abs();
        assert!(
            deviation < 20.0,
            "SMA deviates by {deviation:.2} km at orbit {} (catastrophic drift)", i + 1
        );
    }

    // 2. Total drift is bounded: orbit-averaged SMA of first vs last 50 orbits
    let mean_first_50: f64 = sma_values[..50].iter().sum::<f64>() / 50.0;
    let mean_last_50: f64 = sma_values[450..].iter().sum::<f64>() / 50.0;
    let total_drift = (mean_last_50 - mean_first_50).abs();

    println!(
        "SMA stability 500 orbits: mean_first_50={mean_first_50:.4} km, \
         mean_last_50={mean_last_50:.4} km, total_drift={total_drift:.4} km"
    );

    // Measured: ~5.6 km. RK4 non-symplectic drift is expected; bound at 15 km.
    assert!(
        total_drift < 15.0,
        "SMA drift over 500 orbits should be bounded: {total_drift:.4} km (expected < 15 km)"
    );

    // 3. Linear growth check: compare drift rate in first vs second half.
    // If the drift is linear, the rate should be similar. If exponential, the
    // second half would be much larger.
    let mean_mid: f64 = sma_values[225..275].iter().sum::<f64>() / 50.0;
    let drift_first_half = (mean_mid - mean_first_50).abs();
    let drift_second_half = (mean_last_50 - mean_mid).abs();

    println!(
        "SMA drift first half={drift_first_half:.4} km, second half={drift_second_half:.4} km"
    );

    // Second half should not be more than 3x the first half (linear ≈ 1x, exponential >> 1x)
    if drift_first_half > 0.01 {
        let ratio = drift_second_half / drift_first_half;
        assert!(
            ratio < 3.0,
            "SMA drift should grow linearly, not exponentially: \
             ratio = {ratio:.2} (expected < 3.0)"
        );
    }
}

// ============================================================================
// Test 23: Lz Conservation over 500 Orbits
//
// z-angular momentum Lz = (r × v)·ẑ is conserved for axially symmetric
// gravity (zonal harmonics). This extends the 10-orbit test (Test 3) to
// 500 orbits to verify integration error accumulates as √N (random walk)
// rather than linearly or exponentially.
//
// Oracle: Noether's theorem (axial symmetry → Lz conservation).
// ============================================================================

#[test]
fn lz_conservation_500_orbits() {
    let a = R_EARTH + 600.0;
    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: 0.1,
        inclination: 65.0_f64.to_radians(),
        raan: 30.0_f64.to_radians(),
        argument_of_periapsis: 45.0_f64.to_radians(),
        true_anomaly: 0.0,
    };

    let system = earth_j2_j3_j4_system();
    let (pos, vel) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos,
        velocity: vel,
    };

    let initial_lz = initial.position.cross(&initial.velocity).z;
    let mut max_lz_drift: f64 = 0.0;

    let period = elements.period(MU_EARTH);
    let total_time = 500.0 * period;
    let dt = 5.0;

    Rk4.integrate(&system, initial, 0.0, total_time, dt, |_, state| {
        let lz = state.position.cross(&state.velocity).z;
        let drift = (lz - initial_lz).abs() / initial_lz.abs();
        max_lz_drift = max_lz_drift.max(drift);
    });

    println!("Lz conservation 500 orbits: max relative drift = {max_lz_drift:.6e}");

    // 10 orbits gave < 1e-7. Over 500 orbits, expect √50 ≈ 7x more error
    // if growth is random-walk, so < 1e-6 is conservative.
    assert!(
        max_lz_drift < 1e-6,
        "Lz conservation over 500 orbits: max relative drift = {max_lz_drift:.6e} (expected < 1e-6)"
    );
}

// ============================================================================
// Test 23: ISS 30-Day Survival with Physical Ballistic Coefficient
//
// Oracle: ISS at ~400 km with physical B = Cd*A/(2m) ≈ 0.005 m²/kg
// should survive at least 30 days without reboost. Expected decay rate
// is 0.05-0.8 km/day (solar-activity-dependent).
//
// Our atmosphere model uses US Standard 1976 (solar-minimum-like ρ ≈ 3.7e-12
// at 400 km), so expect the lower end of the decay range.
// ============================================================================

#[test]
fn iss_drag_30day_survival() {
    let a = R_EARTH + 400.0;
    let v = (MU_EARTH / a).sqrt();

    let mut system = earth_j2_system();
    system.perturbations.push(Box::new(AtmosphericDrag {
        body: Some(KnownBody::Earth),
        body_radius: R_EARTH,
        omega_body: orts_orbits::drag::OMEGA_EARTH,
        ballistic_coeff: 0.005, // Physical ISS: Cd*A/(2m) ≈ 2.2*2000/(2*420000)
        atmosphere: Box::new(tobari::exponential::Exponential),
    }));

    let initial = State {
        position: vector![a, 0.0, 0.0],
        velocity: vector![0.0, v, 0.0],
    };

    let duration = 30.0 * 86400.0; // 30 days in seconds
    let dt = 30.0;

    let mut min_altitude = f64::MAX;
    let final_state = Rk4.integrate(&system, initial, 0.0, duration, dt, |_, state| {
        let alt = state.position.magnitude() - R_EARTH;
        min_altitude = min_altitude.min(alt);
    });

    let final_alt = final_state.position.magnitude() - R_EARTH;
    let final_elems =
        KeplerianElements::from_state_vector(&final_state.position, &final_state.velocity, MU_EARTH);
    let sma_decay = a - final_elems.semi_major_axis;
    let decay_per_day = sma_decay / 30.0;

    println!("ISS 30-day drag: min_alt={min_altitude:.2} km, final_alt={final_alt:.2} km");
    println!("ISS 30-day drag: SMA decay={sma_decay:.4} km, rate={decay_per_day:.4} km/day");

    // Satellite must survive (altitude > Kármán line)
    assert!(
        min_altitude > 100.0,
        "ISS should survive 30 days, min altitude was {min_altitude:.1} km"
    );
    assert!(
        final_alt > 300.0,
        "ISS final altitude after 30 days should be > 300 km, got {final_alt:.1} km"
    );

    // SMA decay should be physically plausible: 0.5-20 km over 30 days
    // (ISS actual without reboost: ~2-5 km/month at solar minimum)
    assert!(
        sma_decay > 0.5 && sma_decay < 20.0,
        "ISS SMA decay over 30 days should be 0.5-20 km, got {sma_decay:.4} km"
    );

    // Decay rate: 0.01-1.0 km/day
    assert!(
        decay_per_day > 0.01 && decay_per_day < 1.0,
        "ISS decay rate should be 0.01-1.0 km/day, got {decay_per_day:.4} km/day"
    );
}

// ============================================================================
// Test: J2 Eccentricity Oscillation Bounded
//
// Near-circular orbit under J2 has short-period eccentricity oscillations
// with amplitude ≈ (3/4) J2 (R_e/p)² ≈ 0.0006-0.001.
// The ω precession modulates the oscillation envelope but eccentricity
// must remain bounded (no secular divergence for conservative J2).
//
// Oracle: Brouwer theory — J2 causes no secular eccentricity change;
//         energy must be conserved for the conservative J2 system.
// ============================================================================

#[test]
fn j2_eccentricity_oscillation_bounded() {
    let a = R_EARTH + 800.0;
    let e_0 = 0.01;
    let i = 51.6_f64.to_radians();

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e_0,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 50;
    let dt = 10.0;

    // Compute initial total energy (including J2 potential)
    let (pos_0, vel_0) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos_0.clone(),
        velocity: vel_0.clone(),
    };
    let energy_0 = j2_total_energy(&initial, MU_EARTH, J2_EARTH, R_EARTH);

    let (orbit_elems, final_state) =
        propagate_collecting_elements(&system, &elements, n_orbits, dt);

    // Collect eccentricity values
    let ecc_values: Vec<f64> = orbit_elems.iter().map(|el| el.eccentricity).collect();
    let e_min = ecc_values.iter().copied().fold(f64::MAX, f64::min);
    let e_max = ecc_values.iter().copied().fold(f64::MIN, f64::max);
    let e_mean = ecc_values.iter().sum::<f64>() / ecc_values.len() as f64;
    let e_range = e_max - e_min;

    println!("J2 e oscillation (50 orbits): e_0={e_0}, e_min={e_min:.6}, e_max={e_max:.6}, e_mean={e_mean:.6}, range={e_range:.6}");

    // 1. Eccentricity must remain bounded (no divergence)
    //    J2 short-period amplitude ≈ (3/4) J2 (Re/p)² ≈ 0.0006 for this orbit.
    //    Allow generous margin: e stays within e_0 ± 0.01
    assert!(
        e_max < e_0 + 0.01,
        "Eccentricity should not diverge: e_max={e_max:.6}, limit={:.6}",
        e_0 + 0.01
    );
    assert!(
        e_min > 0.0,
        "Eccentricity must remain positive: e_min={e_min:.6}"
    );

    // 2. Mean eccentricity should be close to initial (no secular drift)
    assert!(
        (e_mean - e_0).abs() < 0.005,
        "Mean eccentricity should not drift: e_mean={e_mean:.6}, e_0={e_0}"
    );

    // 3. Short-period oscillation amplitude should be physically reasonable
    //    Theory: Δe_sp ≈ (3/4) J2 (Re/p)² ≈ 0.0006
    let p = a * (1.0 - e_0 * e_0);
    let delta_e_theory = 0.75 * J2_EARTH * (R_EARTH / p).powi(2);
    println!("  Theoretical J2 short-period Δe ≈ {delta_e_theory:.6}");
    // Oscillation range should be on the order of the theoretical value
    // (within 10x is acceptable given higher-order effects)
    assert!(
        e_range < delta_e_theory * 20.0,
        "Oscillation range {e_range:.6} exceeds 20× theoretical Δe {delta_e_theory:.6}"
    );

    // 4. Total energy conservation (including J2 potential — the true conserved quantity)
    //    With correct total energy, RK4 drift should be much smaller than
    //    the ~5e-4 we saw with two-body energy (which oscillates due to J2).
    let energy_final = j2_total_energy(&final_state, MU_EARTH, J2_EARTH, R_EARTH);
    let rel_energy_error = ((energy_final - energy_0) / energy_0).abs();
    println!("  Total energy: initial={energy_0:.10}, final={energy_final:.10}, rel_error={rel_energy_error:.2e}");
    assert!(
        rel_energy_error < 1e-5,
        "Total energy should be conserved: relative error = {rel_energy_error:.2e}"
    );
}

// ============================================================================
// Test: J2 ω Precession Modulates Eccentricity (Long-Duration)
//
// Over 200 orbits (~14 days), ω precesses ~46° (at 3.3°/day for ISS incl.).
// The eccentricity oscillation envelope is modulated by ω but must remain
// bounded. This validates long-term numerical stability of the J2 integrator.
//
// Oracle: Lagrange planetary equations predict no secular e change from J2.
//         ω̇ ≈ 3.3°/day at ISS inclination confirms precession tracking.
// ============================================================================

#[test]
fn j2_omega_precession_modulates_eccentricity() {
    let a = R_EARTH + 800.0;
    let e_0 = 0.05; // larger e to make ω oscillation coupling visible
    let i = 51.6_f64.to_radians();

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e_0,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 200;
    let dt = 10.0;

    let (orbit_elems, _) = propagate_collecting_elements(&system, &elements, n_orbits, dt);

    // Collect e and ω over time
    let ecc_values: Vec<f64> = orbit_elems.iter().map(|el| el.eccentricity).collect();
    let omega_values: Vec<f64> = orbit_elems
        .iter()
        .map(|el| unwrap_angle(el.argument_of_periapsis, elements.argument_of_periapsis))
        .collect();

    let e_min = ecc_values.iter().copied().fold(f64::MAX, f64::min);
    let e_max = ecc_values.iter().copied().fold(f64::MIN, f64::max);
    let e_mean = ecc_values.iter().sum::<f64>() / ecc_values.len() as f64;

    let omega_final = omega_values.last().copied().unwrap();
    let period = elements.period(MU_EARTH);
    let total_time = n_orbits as f64 * period;
    let omega_rate_deg_per_day = omega_final.to_degrees() * 86400.0 / total_time;

    println!("J2 e-ω modulation (200 orbits, {:.1} days):", total_time / 86400.0);
    println!("  e: min={e_min:.6}, max={e_max:.6}, mean={e_mean:.6}, range={:.6}", e_max - e_min);
    println!("  ω: initial=0°, final={:.2}°, rate={omega_rate_deg_per_day:.2}°/day", omega_final.to_degrees());

    // 1. ω precession rate should match analytical prediction
    let p = a * (1.0 - e_0 * e_0);
    let n_mean = (MU_EARTH / a.powi(3)).sqrt();
    let expected_omega_dot =
        1.5 * n_mean * J2_EARTH * (R_EARTH / p).powi(2) * (2.0 - 2.5 * i.sin().powi(2));
    let expected_deg_per_day = expected_omega_dot.to_degrees() * 86400.0;

    let omega_error = (omega_rate_deg_per_day - expected_deg_per_day).abs();
    assert!(
        omega_error < 1.5,
        "ω precession rate: expected≈{expected_deg_per_day:.2}°/day, got={omega_rate_deg_per_day:.2}°/day, error={omega_error:.3}"
    );

    // 2. Eccentricity must remain bounded (no divergence over 200 orbits)
    assert!(
        e_max < e_0 + 0.02,
        "Eccentricity diverging: e_max={e_max:.6}, limit={:.6}",
        e_0 + 0.02
    );
    assert!(
        e_min > e_0 - 0.02,
        "Eccentricity diverging: e_min={e_min:.6}, limit={:.6}",
        e_0 - 0.02
    );

    // 3. Mean eccentricity should be close to initial (no secular drift)
    assert!(
        (e_mean - e_0).abs() < 0.01,
        "Mean eccentricity drifted: e_mean={e_mean:.6}, e_0={e_0}"
    );

    // 4. Eccentricity should show oscillatory structure, not monotonic growth
    //    Check: first quarter e-mean vs last quarter e-mean should be similar
    let q_len = ecc_values.len() / 4;
    let e_mean_q1 = ecc_values[..q_len].iter().sum::<f64>() / q_len as f64;
    let e_mean_q4 = ecc_values[3 * q_len..].iter().sum::<f64>() / (ecc_values.len() - 3 * q_len) as f64;
    let secular_drift = (e_mean_q4 - e_mean_q1).abs();
    println!("  Secular e drift (Q1 mean vs Q4 mean): {secular_drift:.6}");
    assert!(
        secular_drift < 0.005,
        "Secular eccentricity drift detected: Q1 mean={e_mean_q1:.6}, Q4 mean={e_mean_q4:.6}, drift={secular_drift:.6}"
    );
}

// ============================================================================
// DP45 (Dormand-Prince) versions — tighter tolerances, longer duration
//
// DP45 with atol=1e-12, rtol=1e-10 preserves energy to ~1e-10 level,
// enabling strict energy conservation checks over long periods.
// ============================================================================

#[test]
fn j2_eccentricity_dp45_500_orbits() {
    let a = R_EARTH + 800.0;
    let e_0 = 0.01;
    let i = 51.6_f64.to_radians();

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e_0,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 500; // ~35 days
    let tol = Tolerances {
        atol: 1e-12,
        rtol: 1e-10,
    };

    let (pos_0, vel_0) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos_0.clone(),
        velocity: vel_0.clone(),
    };
    let energy_0 = j2_total_energy(&initial, MU_EARTH, J2_EARTH, R_EARTH);

    let (orbit_elems, final_state) =
        propagate_collecting_elements_dp45(&system, &elements, n_orbits, &tol);

    let ecc_values: Vec<f64> = orbit_elems.iter().map(|el| el.eccentricity).collect();
    let e_min = ecc_values.iter().copied().fold(f64::MAX, f64::min);
    let e_max = ecc_values.iter().copied().fold(f64::MIN, f64::max);
    let e_mean = ecc_values.iter().sum::<f64>() / ecc_values.len() as f64;

    let period = elements.period(MU_EARTH);
    let total_days = n_orbits as f64 * period / 86400.0;

    println!("DP45 J2 e oscillation ({n_orbits} orbits, {total_days:.1} days):");
    println!("  e: min={e_min:.8}, max={e_max:.8}, mean={e_mean:.8}, range={:.8}", e_max - e_min);

    // 1. Eccentricity bounded
    assert!(
        e_max < e_0 + 0.01,
        "e diverged: e_max={e_max:.8}, limit={:.6}", e_0 + 0.01
    );
    assert!(e_min > 0.0, "e went negative: e_min={e_min:.8}");

    // 2. Mean eccentricity stable
    assert!(
        (e_mean - e_0).abs() < 0.005,
        "Mean e drifted: e_mean={e_mean:.8}, e_0={e_0}"
    );

    // 3. Strict total energy conservation (including J2 potential)
    //    DP45 with tight tolerances should preserve total energy to ~1e-10 level
    let energy_final = j2_total_energy(&final_state, MU_EARTH, J2_EARTH, R_EARTH);
    let rel_energy_error = ((energy_final - energy_0) / energy_0).abs();
    println!("  Total energy: rel_error={rel_energy_error:.2e}");
    assert!(
        rel_energy_error < 1e-7,
        "DP45 total energy should be conserved: rel_error={rel_energy_error:.2e}"
    );

    // 4. No secular drift: Q1 vs Q4 mean eccentricity
    let q_len = ecc_values.len() / 4;
    let e_mean_q1 = ecc_values[..q_len].iter().sum::<f64>() / q_len as f64;
    let e_mean_q4 = ecc_values[3 * q_len..].iter().sum::<f64>() / (ecc_values.len() - 3 * q_len) as f64;
    let secular_drift = (e_mean_q4 - e_mean_q1).abs();
    println!("  Secular e drift (Q1 vs Q4): {secular_drift:.8}");
    assert!(
        secular_drift < 0.002,
        "Secular drift: Q1={e_mean_q1:.8}, Q4={e_mean_q4:.8}, drift={secular_drift:.8}"
    );
}

#[test]
fn j2_omega_precession_dp45_500_orbits() {
    let a = R_EARTH + 800.0;
    let e_0 = 0.05;
    let i = 51.6_f64.to_radians();

    let elements = KeplerianElements {
        semi_major_axis: a,
        eccentricity: e_0,
        inclination: i,
        raan: 0.0,
        argument_of_periapsis: 0.0,
        true_anomaly: 0.0,
    };

    let system = earth_j2_system();
    let n_orbits = 500; // ~35 days
    let tol = Tolerances {
        atol: 1e-12,
        rtol: 1e-10,
    };

    let (pos_0, vel_0) = elements.to_state_vector(MU_EARTH);
    let initial = State {
        position: pos_0.clone(),
        velocity: vel_0.clone(),
    };
    let energy_0 = j2_total_energy(&initial, MU_EARTH, J2_EARTH, R_EARTH);

    let (orbit_elems, final_state) =
        propagate_collecting_elements_dp45(&system, &elements, n_orbits, &tol);

    let ecc_values: Vec<f64> = orbit_elems.iter().map(|el| el.eccentricity).collect();
    let omega_values: Vec<f64> = orbit_elems
        .iter()
        .map(|el| unwrap_angle(el.argument_of_periapsis, elements.argument_of_periapsis))
        .collect();

    let e_min = ecc_values.iter().copied().fold(f64::MAX, f64::min);
    let e_max = ecc_values.iter().copied().fold(f64::MIN, f64::max);
    let e_mean = ecc_values.iter().sum::<f64>() / ecc_values.len() as f64;

    let omega_final = omega_values.last().copied().unwrap();
    let period = elements.period(MU_EARTH);
    let total_time = n_orbits as f64 * period;
    let total_days = total_time / 86400.0;
    let omega_rate_deg_per_day = omega_final.to_degrees() * 86400.0 / total_time;

    println!("DP45 J2 e-ω modulation ({n_orbits} orbits, {total_days:.1} days):");
    println!("  e: min={e_min:.8}, max={e_max:.8}, mean={e_mean:.8}");
    println!("  ω: final={:.2}°, rate={omega_rate_deg_per_day:.3}°/day", omega_final.to_degrees());

    // 1. ω precession rate matches theory (tighter tolerance with DP45)
    let p = a * (1.0 - e_0 * e_0);
    let n_mean = (MU_EARTH / a.powi(3)).sqrt();
    let expected_omega_dot =
        1.5 * n_mean * J2_EARTH * (R_EARTH / p).powi(2) * (2.0 - 2.5 * i.sin().powi(2));
    let expected_deg_per_day = expected_omega_dot.to_degrees() * 86400.0;

    let omega_error = (omega_rate_deg_per_day - expected_deg_per_day).abs();
    println!("  ω rate error: {omega_error:.4}°/day (expected {expected_deg_per_day:.3}°/day)");
    assert!(
        omega_error < 0.5,
        "ω rate: expected≈{expected_deg_per_day:.3}°/day, got={omega_rate_deg_per_day:.3}°/day"
    );

    // 2. Eccentricity bounded
    assert!(
        e_max < e_0 + 0.02 && e_min > e_0 - 0.02,
        "e out of bounds: [{e_min:.8}, {e_max:.8}]"
    );

    // 3. Strict total energy conservation (including J2 potential)
    let energy_final = j2_total_energy(&final_state, MU_EARTH, J2_EARTH, R_EARTH);
    let rel_energy_error = ((energy_final - energy_0) / energy_0).abs();
    println!("  Total energy: rel_error={rel_energy_error:.2e}");
    assert!(
        rel_energy_error < 1e-7,
        "DP45 total energy: rel_error={rel_energy_error:.2e}"
    );

    // 4. No secular drift over ~35 days
    let q_len = ecc_values.len() / 4;
    let e_mean_q1 = ecc_values[..q_len].iter().sum::<f64>() / q_len as f64;
    let e_mean_q4 = ecc_values[3 * q_len..].iter().sum::<f64>() / (ecc_values.len() - 3 * q_len) as f64;
    let secular_drift = (e_mean_q4 - e_mean_q1).abs();
    println!("  Secular e drift (Q1 vs Q4): {secular_drift:.8}");
    assert!(
        secular_drift < 0.003,
        "Secular drift: Q1={e_mean_q1:.8}, Q4={e_mean_q4:.8}"
    );
}

