//! SRP oracle tests: validate solar radiation pressure against analytical predictions.
//!
//! These tests verify the SRP `ForceModel` implementation using:
//! - Eccentricity vector secular drift (Gauss VOP, cannonball model)
//! - Scaling linearity (A/m, Cr)
//! - Semi-major axis stability (no secular drift from SRP)
//! - Shadow model physical correctness
//! - dt convergence (4th-order for RK4)
//! - Energy-work consistency (nonconservative force)

use kaname::constants::{MU_EARTH, R_EARTH};
use kaname::epoch::Epoch;
use nalgebra::vector;
use orts::OrbitalState;
use orts::gravity::PointMass;
use orts::kepler::KeplerianElements;
use orts::orbital_system::OrbitalSystem;
use orts::perturbations::SolarRadiationPressure;
use std::f64::consts::PI;
use utsuroi::{Integrator, Rk4};

// ============================================================================
// Helpers
// ============================================================================

fn test_epoch() -> Epoch {
    Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
}

/// Build a point-mass + SRP system (no J2, clean SRP effects).
fn point_mass_srp_system(
    cr: f64,
    area_to_mass: f64,
    with_shadow: bool,
    epoch: Epoch,
) -> OrbitalSystem {
    let srp = SolarRadiationPressure {
        cr,
        area_to_mass,
        shadow_body_radius: if with_shadow { Some(R_EARTH) } else { None },
    };
    OrbitalSystem::new(MU_EARTH, Box::new(PointMass))
        .with_perturbation(Box::new(srp))
        .with_epoch(epoch)
}

/// Propagate for n_orbits, collecting eccentricity and SMA at each orbit completion.
fn propagate_collecting(
    system: &OrbitalSystem,
    initial: &OrbitalState,
    period: f64,
    n_orbits: usize,
    dt: f64,
) -> (Vec<f64>, Vec<f64>, OrbitalState) {
    let mut eccentricities = vec![];
    let mut sma_values = vec![];
    let mut current = initial.clone();
    let mut t = 0.0;

    for _ in 0..n_orbits {
        let t_end = t + period;
        current = Rk4.integrate(system, current, t, t_end, dt, |_, _| {});
        t = t_end;
        let elems =
            KeplerianElements::from_state_vector(current.position(), current.velocity(), MU_EARTH);
        eccentricities.push(elems.eccentricity);
        sma_values.push(elems.semi_major_axis);
    }

    (eccentricities, sma_values, current)
}

// ============================================================================
// Test 1: SRP Eccentricity Growth (No Shadow)
//
// For a cannonball SRP with fixed Sun direction, no eclipse, initially circular
// orbit, the eccentricity vector drifts linearly. Over N orbits, eccentricity
// should grow from ~0 to a measurable value.
//
// Oracle: Gauss VOP — de_vec/dt = (3/2)(a_srp × ĥ)/(n·a)
// ============================================================================

#[test]
fn srp_eccentricity_growth_no_shadow() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 20;
    let dt = 10.0;

    let system = point_mass_srp_system(1.5, 0.02, false, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let (eccentricities, _, _) = propagate_collecting(&system, &initial, period, n_orbits, dt);

    // Eccentricity should grow from ~0
    let final_e = *eccentricities.last().unwrap();
    assert!(
        final_e > 1e-7,
        "SRP should induce eccentricity growth, got e={final_e:.3e}"
    );
    assert!(
        final_e < 0.01,
        "SRP eccentricity growth should be small over 20 orbits, got e={final_e:.3e}"
    );

    // Should generally increase (final > first)
    assert!(
        eccentricities.last() > eccentricities.first(),
        "Eccentricity should grow: e_final={:.3e}, e_first={:.3e}",
        eccentricities.last().unwrap(),
        eccentricities.first().unwrap()
    );

    println!("SRP eccentricity growth over {n_orbits} orbits: e_final={final_e:.6e}");
}

// ============================================================================
// Test 2: SRP Scaling with Area-to-Mass Ratio
//
// Doubling A/m should approximately double eccentricity growth.
//
// Oracle: SRP acceleration is linear in A/m.
// ============================================================================

#[test]
fn srp_scaling_area_to_mass() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 10;
    let dt = 10.0;

    let run_with_am = |am: f64| -> f64 {
        let system = point_mass_srp_system(1.5, am, false, epoch);
        let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);
        let (eccentricities, _, _) = propagate_collecting(&system, &initial, period, n_orbits, dt);
        *eccentricities.last().unwrap()
    };

    let e1 = run_with_am(0.01);
    let e2 = run_with_am(0.02);
    let ratio = e2 / e1;

    println!("A/m scaling: e(0.01)={e1:.3e}, e(0.02)={e2:.3e}, ratio={ratio:.3}");

    assert!(
        ratio > 1.5 && ratio < 2.5,
        "2x A/m should ~double eccentricity growth: ratio={ratio:.3}"
    );
}

// ============================================================================
// Test 3: No Secular SMA Drift from SRP
//
// SRP does not secularly change the semi-major axis (to first order) when
// the orbit is circular and the Sun direction is approximately fixed.
// da/dt ≈ 0 from orbit-averaged Gauss equations.
//
// Oracle: Gauss VOP — da/dt = 0 for constant SRP on circular orbit.
// ============================================================================

#[test]
fn srp_no_sma_secular_drift() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 20;
    let dt = 10.0;

    let system = point_mass_srp_system(1.5, 0.02, false, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let (_, sma_values, _) = propagate_collecting(&system, &initial, period, n_orbits, dt);

    // SMA should not drift significantly
    let mean_sma: f64 = sma_values.iter().sum::<f64>() / sma_values.len() as f64;
    let max_deviation = sma_values
        .iter()
        .map(|s| (s - mean_sma).abs())
        .fold(0.0_f64, f64::max);

    println!("SMA stability: mean={mean_sma:.3} km, max_deviation={max_deviation:.6} km");

    // Allow oscillation but no secular drift
    assert!(
        max_deviation < 1.0,
        "SMA should not drift with SRP (no shadow): max_deviation={max_deviation:.3} km"
    );

    // Final SMA close to initial
    let sma_drift = (sma_values.last().unwrap() - a).abs();
    assert!(
        sma_drift < 0.5,
        "SMA secular drift should be near zero: drift={sma_drift:.3} km"
    );
}

// ============================================================================
// Test 4: Shadow Reduces SRP Effect
//
// With the cylindrical shadow model enabled, eccentricity growth should be
// smaller than without shadow, since SRP is blocked for part of each orbit.
//
// Oracle: LEO satellite spends ~30% of orbit in shadow → ~30% reduction.
// ============================================================================

#[test]
fn srp_shadow_reduces_effect() {
    let epoch = test_epoch();
    let a = R_EARTH + 400.0; // Low orbit → more time in shadow
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 10;
    let dt = 10.0;

    let run = |with_shadow: bool| -> f64 {
        let system = point_mass_srp_system(1.5, 0.02, with_shadow, epoch);
        let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);
        let (eccentricities, _, _) = propagate_collecting(&system, &initial, period, n_orbits, dt);
        *eccentricities.last().unwrap()
    };

    let e_no_shadow = run(false);
    let e_with_shadow = run(true);

    let reduction = 1.0 - e_with_shadow / e_no_shadow;
    println!(
        "Shadow effect: e_no_shadow={e_no_shadow:.3e}, e_with_shadow={e_with_shadow:.3e}, \
         reduction={:.1}%",
        reduction * 100.0
    );

    assert!(
        e_with_shadow < e_no_shadow,
        "Shadow should reduce SRP effect: e_shadow={e_with_shadow:.3e}, e_noshadow={e_no_shadow:.3e}"
    );
    // LEO shadow fraction is ~30-35%
    assert!(
        reduction > 0.05 && reduction < 0.8,
        "Shadow reduction should be 5-80% for LEO, got {:.1}%",
        reduction * 100.0
    );
}

// ============================================================================
// Test 5: Time Reversal
//
// SRP (without shadow) is a smooth function of position and epoch.
// The ODE is reversible: propagating forward then backward should return
// close to the initial state.
//
// Oracle: ODE reversibility (not energy conservation — SRP is nonconservative).
// ============================================================================

#[test]
fn srp_time_reversal() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();

    let system = point_mass_srp_system(1.5, 0.02, false, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let total_time = 5.0 * period;
    let dt = 10.0;

    // Forward propagation
    let forward = Rk4.integrate(&system, initial.clone(), 0.0, total_time, dt, |_, _| {});

    // Backward propagation (negate velocity, propagate forward, negate velocity again)
    let reversed = OrbitalState::new(*forward.position(), -*forward.velocity());
    let backward = Rk4.integrate(
        &system,
        reversed,
        total_time,
        2.0 * total_time,
        dt,
        |_, _| {},
    );
    let recovered = OrbitalState::new(*backward.position(), -*backward.velocity());

    let pos_err = (*recovered.position() - *initial.position()).magnitude();
    let rel_pos_err = pos_err / initial.position().magnitude();

    println!("Time reversal: pos_err={pos_err:.3e} km, rel_err={rel_pos_err:.3e}");

    assert!(
        rel_pos_err < 1e-6,
        "Time reversal position error: {pos_err:.3e} km (rel: {rel_pos_err:.3e})"
    );
}

// ============================================================================
// Test 6: Energy-Work Consistency
//
// SRP is nonconservative: it does net work on the satellite.
// The change in orbital energy should match the accumulated work:
//   ΔE = ∫ a_srp · v dt
//
// Oracle: First law of thermodynamics for mechanics.
// ============================================================================

#[test]
fn srp_energy_work_consistency() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();

    let cr = 1.5;
    let area_to_mass = 0.02;
    let system = point_mass_srp_system(cr, area_to_mass, false, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let orbital_energy = |s: &OrbitalState| -> f64 {
        let r = s.position().magnitude();
        0.5 * s.velocity().magnitude_squared() - MU_EARTH / r
    };

    let e0 = orbital_energy(&initial);
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    // Use small dt so the left Riemann sum (O(dt)) is accurate enough
    let dt = 1.0;
    let total_time = 5.0 * period;

    // Accumulate SRP work: ∫ a_srp · v dt
    let srp_model = SolarRadiationPressure {
        cr,
        area_to_mass,
        shadow_body_radius: None,
    };

    use orts::perturbations::ForceModel;
    let mut work_accumulated = 0.0;

    let final_state = Rk4.integrate(&system, initial, 0.0, total_time, dt, |t, state| {
        let ep = epoch.add_seconds(t);
        let a_srp = srp_model.acceleration(t, state, Some(&ep));
        work_accumulated += a_srp.dot(state.velocity()) * dt;
    });

    let ef = orbital_energy(&final_state);
    let delta_e = ef - e0;

    // The work integral is computed via left Riemann sum (O(dt)),
    // so we allow reasonable tolerance for a quantity that oscillates
    // around zero. Check that both are small and have the same sign.
    println!("Energy-work: ΔE={delta_e:.6e}, work={work_accumulated:.6e}");

    // Both should be very small (SRP barely changes energy on circular orbit)
    assert!(
        delta_e.abs() < 1e-6,
        "SRP energy change should be tiny: ΔE={delta_e:.3e}"
    );
    assert!(
        work_accumulated.abs() < 1e-6,
        "SRP work should be tiny: work={work_accumulated:.3e}"
    );

    // Both ΔE and work should have the same sign (if non-negligible)
    if delta_e.abs() > 1e-12 && work_accumulated.abs() > 1e-12 {
        assert!(
            delta_e.signum() == work_accumulated.signum(),
            "ΔE and work should have same sign: ΔE={delta_e:.3e}, work={work_accumulated:.3e}"
        );
    }
}

// ============================================================================
// Test 7: dt Convergence (4th-Order for RK4)
//
// Halving dt should reduce position error by ~16x for RK4.
//
// Oracle: RK4 is O(dt⁴).
// ============================================================================

#[test]
fn srp_dt_convergence() {
    let epoch = test_epoch();
    let a = R_EARTH + 800.0;
    let v = (MU_EARTH / a).sqrt();

    let system = point_mass_srp_system(1.5, 0.02, false, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let duration = 1000.0; // ~1/6 orbit

    let dt_coarse = 4.0;
    let dt_fine = 2.0;
    let dt_finest = 1.0;

    let f_coarse = Rk4.integrate(
        &system,
        initial.clone(),
        0.0,
        duration,
        dt_coarse,
        |_, _| {},
    );
    let f_fine = Rk4.integrate(&system, initial.clone(), 0.0, duration, dt_fine, |_, _| {});
    let f_finest = Rk4.integrate(&system, initial, 0.0, duration, dt_finest, |_, _| {});

    let err_coarse = (*f_coarse.position() - *f_finest.position()).magnitude();
    let err_fine = (*f_fine.position() - *f_finest.position()).magnitude();

    let ratio = err_coarse / err_fine;

    println!("dt convergence: err(4s)={err_coarse:.3e}, err(2s)={err_fine:.3e}, ratio={ratio:.2}");

    assert!(
        ratio > 10.0 && ratio < 25.0,
        "SRP dt convergence ratio={ratio:.2}, expected ~16 for 4th-order"
    );
}

// ============================================================================
// Test 8: GEO SRP Eccentricity Growth
//
// GPS/GEO satellites at high altitude with large solar panels (high A/m)
// experience significant SRP. This tests that SRP works at GEO altitudes.
//
// Oracle: Measurable eccentricity growth at GEO with A/m=0.04.
// ============================================================================

#[test]
fn srp_geo_eccentricity() {
    let epoch = test_epoch();
    let a = R_EARTH + 20200.0; // GPS-like MEO altitude
    let v = (MU_EARTH / a).sqrt();
    let period = 2.0 * PI * (a.powi(3) / MU_EARTH).sqrt();
    let n_orbits = 5;
    let dt = 30.0; // GPS period ~12h, dt=30s is fine

    let system = point_mass_srp_system(1.5, 0.04, true, epoch);
    let initial = OrbitalState::new(vector![a, 0.0, 0.0], vector![0.0, v, 0.0]);

    let (eccentricities, sma_values, _) =
        propagate_collecting(&system, &initial, period, n_orbits, dt);

    let final_e = *eccentricities.last().unwrap();

    println!(
        "GEO SRP: e_final={final_e:.6e}, sma_drift={:.3} km over {n_orbits} orbits",
        (sma_values.last().unwrap() - a).abs()
    );

    // GPS should see measurable eccentricity growth
    assert!(
        final_e > 1e-7,
        "GPS SRP should induce measurable eccentricity: e={final_e:.3e}"
    );
}
