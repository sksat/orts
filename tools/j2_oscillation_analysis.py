#!/usr/bin/env python3
"""
Analytical analysis of J2 eccentricity / altitude oscillation.

PURPOSE: Compute analytical predictions for J2-perturbed orbit behavior
and output test thresholds for the Rust oracle tests.

PHYSICS:
  J2 causes NO secular eccentricity change (Brouwer theory).
  The osculating eccentricity has:
    - Short-period oscillation (orbital frequency): Δe_sp ≈ (3/4) J2 (Re/p)²
    - Long-period oscillation (ω frequency): δe_lp ∝ J2² (second-order)
      For first-order J2: δe_lp = -(J2/2)(Re/p)² e (1 - 5/4 sin²i) cos(2ω)
      This is NEGLIGIBLE for near-circular orbits (∝ e).

  The ALTITUDE oscillation is modulated by two effects:
    1. Keplerian eccentricity → Δr ≈ 2ae (once per orbit, phase = ω)
    2. J2 radial perturbation → Δr ≈ J2/2 (Re/r)² r (3cos²θ-1) (twice per orbit)
  As ω precesses, these two signals go in/out of phase, causing the
  altitude oscillation ENVELOPE to vary with period = π/|ω̇|.
  This is bounded and periodic — NOT divergent.

  Secular rates:
    ω̇ = (3/2) n J2 (Re/p)² (2 - 5/2 sin²i)
    Ω̇ = -(3/2) n J2 (Re/p)² cos(i)

References:
  - Brouwer, D. (1959). "Solution of the Problem of Artificial Satellite
    Theory Without Drag"
  - Schaub & Junkins, "Analytical Mechanics of Space Systems", Ch. 9
"""

import json
import sys
import numpy as np

# Constants (must match Rust orts constants)
MU_EARTH = 398600.4418   # km³/s²
R_EARTH = 6378.137       # km
J2 = 1.08263e-3
J3 = -2.5327e-6


def orbit_params(a: float, e: float, i_deg: float):
    """Compute derived orbital parameters."""
    i = np.radians(i_deg)
    p = a * (1 - e**2)
    n = np.sqrt(MU_EARTH / a**3)
    T = 2 * np.pi / n
    return i, p, n, T


def omega_dot(a: float, e: float, i_deg: float) -> float:
    """J2 secular ω precession rate [rad/s]."""
    i, p, n, _ = orbit_params(a, e, i_deg)
    return 1.5 * n * J2 * (R_EARTH / p)**2 * (2 - 2.5 * np.sin(i)**2)


def raan_dot(a: float, e: float, i_deg: float) -> float:
    """J2 secular Ω precession rate [rad/s]."""
    i, p, n, _ = orbit_params(a, e, i_deg)
    return -1.5 * n * J2 * (R_EARTH / p)**2 * np.cos(i)


def delta_e_sp_amplitude(a: float, e: float, i_deg: float) -> float:
    """
    Short-period eccentricity oscillation amplitude (per orbit).
    Δe_sp ≈ (3/4) J2 (Re/p)²
    """
    _, p, _, _ = orbit_params(a, e, i_deg)
    return 0.75 * J2 * (R_EARTH / p)**2


def delta_e_lp_coefficient(a: float, e: float, i_deg: float) -> float:
    """
    Long-period eccentricity modulation coefficient.
    δe_lp = coeff * cos(2ω)
    coeff = -(J2/2)(Re/p)² e (1 - 5/4 sin²i)

    NOTE: Proportional to e, so NEGLIGIBLE for near-circular orbits.
    """
    i, p, _, _ = orbit_params(a, e, i_deg)
    return -0.5 * J2 * (R_EARTH / p)**2 * e * (1 - 1.25 * np.sin(i)**2)


def j2_radial_perturbation_amplitude(a: float, i_deg: float) -> float:
    """
    Amplitude of J2-induced radial oscillation (twice per orbit).
    For a satellite at inclination i, the radial distance varies as the
    satellite moves between equator and max latitude.

    Δr_J2 ≈ (3/2) J2 (Re²/a) sin²i  [approximate]
    """
    i = np.radians(i_deg)
    return 1.5 * J2 * R_EARTH**2 / a * np.sin(i)**2


def analyze_orbit(name: str, a: float, e_0: float, i_deg: float,
                  duration_days: float) -> dict:
    """Full analysis for one orbit configuration."""
    i, p, n, T = orbit_params(a, e_0, i_deg)

    print(f"\n{'='*70}")
    print(f"  {name}")
    print(f"  a = {a:.1f} km, e = {e_0}, i = {i_deg}°")
    print(f"{'='*70}")

    # Secular rates
    w_dot = omega_dot(a, e_0, i_deg)
    O_dot = raan_dot(a, e_0, i_deg)
    w_dot_deg_day = np.degrees(w_dot) * 86400
    O_dot_deg_day = np.degrees(O_dot) * 86400
    n_orbits = duration_days * 86400 / T

    print(f"\nORBITAL PARAMETERS:")
    print(f"  Period T = {T:.1f} s ({T/60:.1f} min)")
    print(f"  Duration = {duration_days:.0f} days ({n_orbits:.0f} orbits)")
    print(f"  ω̇ = {w_dot_deg_day:+.4f} °/day")
    print(f"  Ω̇ = {O_dot_deg_day:+.4f} °/day")

    # Periods
    omega_full_period = abs(2 * np.pi / w_dot) / 86400
    omega_half_period = abs(np.pi / w_dot) / 86400
    print(f"  ω full period = {omega_full_period:.1f} days")
    print(f"  Altitude modulation period (= ω half period) = {omega_half_period:.1f} days")

    # Eccentricity oscillations
    de_sp = delta_e_sp_amplitude(a, e_0, i_deg)
    de_lp = abs(delta_e_lp_coefficient(a, e_0, i_deg))
    print(f"\nECCENTRICITY OSCILLATION:")
    print(f"  Short-period amplitude Δe_sp = {de_sp:.6f}")
    print(f"  Long-period coefficient |δe_lp| = {de_lp:.8f}")
    print(f"  Ratio Δe_sp / e_0 = {de_sp / e_0 if e_0 > 0 else 'inf':.4f}")
    print(f"  Note: δe_lp ∝ e, so {'NEGLIGIBLE' if de_lp < 1e-5 else 'significant'}")

    # Eccentricity bounds (theoretical)
    # Max osculating e = e_0 + Δe_sp + |δe_lp|
    e_max_theory = e_0 + de_sp + de_lp
    e_min_theory = max(0, e_0 - de_sp - de_lp)
    e_range_theory = e_max_theory - e_min_theory
    print(f"  Theoretical e range: [{e_min_theory:.6f}, {e_max_theory:.6f}]")
    print(f"  Theoretical e oscillation range: {e_range_theory:.6f}")

    # Altitude oscillations
    # 1. Keplerian altitude oscillation from eccentricity: Δalt_ecc ≈ 2ae
    alt_ecc_max = 2 * a * e_max_theory
    alt_ecc_min = 2 * a * e_min_theory
    # 2. J2 radial perturbation
    dr_j2 = j2_radial_perturbation_amplitude(a, i_deg)

    print(f"\nALTITUDE OSCILLATION COMPONENTS:")
    print(f"  Keplerian (from e): Δalt = {2*a*e_0:.2f} km (nominal)")
    print(f"                       max = {alt_ecc_max:.2f} km, min = {alt_ecc_min:.2f} km")
    print(f"  J2 radial (latitude): Δr_J2 = {dr_j2:.2f} km")
    print(f"  Combined max envelope ≈ {alt_ecc_max + dr_j2:.2f} km")

    # Modulation: as ω precesses, ecc and J2 oscillations go in/out of phase
    # Worst case: both add constructively
    # Best case: they partially cancel
    alt_total_max = alt_ecc_max + dr_j2
    alt_total_min = max(0, alt_ecc_min - dr_j2)
    print(f"\n  ALTITUDE OSCILLATION ENVELOPE:")
    print(f"    Max amplitude (constructive): {alt_total_max:.2f} km")
    print(f"    Min amplitude (destructive):  {alt_total_min:.2f} km")
    if alt_total_min > 0:
        print(f"    Envelope ratio: {alt_total_max / alt_total_min:.2f}x")
    else:
        print(f"    Envelope ratio: ∞ (min ≈ 0)")
    print(f"    Modulation period: {omega_half_period:.1f} days")

    # ω advance over duration
    omega_advance = abs(w_dot_deg_day * duration_days)
    print(f"\n  Over {duration_days:.0f} days: ω advances {omega_advance:.1f}°")

    # Test thresholds
    # Use generous margins (2× theory) for numerical tests
    e_bound_margin = 2.0  # 2× theoretical range for safety
    e_upper = e_0 + (de_sp + de_lp) * e_bound_margin
    e_lower = max(0, e_0 - (de_sp + de_lp) * e_bound_margin)
    alt_bound = alt_total_max * 1.5  # 1.5× for test margin

    thresholds = {
        'name': name,
        'a': a,
        'e_0': e_0,
        'i_deg': i_deg,
        'period_s': T,
        'omega_dot_deg_day': w_dot_deg_day,
        'raan_dot_deg_day': O_dot_deg_day,
        'omega_half_period_days': omega_half_period,
        'omega_full_period_days': omega_full_period,
        'de_sp': de_sp,
        'de_lp_coeff': de_lp,
        'e_range_theory': e_range_theory,
        'e_max_theory': e_max_theory,
        'e_min_theory': e_min_theory,
        'alt_ecc_max_km': alt_ecc_max,
        'dr_j2_km': dr_j2,
        'alt_envelope_max_km': alt_total_max,
        'alt_envelope_min_km': alt_total_min,
        # Test thresholds (with margin)
        'test_e_upper': e_upper,
        'test_e_lower': e_lower,
        'test_alt_max_km': alt_bound,
        'test_energy_rel_tol_rk4': 1e-5,
        'test_energy_rel_tol_dp45': 1e-7,
        'test_secular_drift_tol': 0.005,
    }

    print(f"\n  TEST THRESHOLDS (with margins):")
    print(f"    e bounds: [{e_lower:.6f}, {e_upper:.6f}]")
    print(f"    Alt bound: < {alt_bound:.1f} km")
    print(f"    Energy tol (RK4): < {thresholds['test_energy_rel_tol_rk4']:.0e}")
    print(f"    Energy tol (DP45): < {thresholds['test_energy_rel_tol_dp45']:.0e}")
    print(f"    Secular drift tol: < {thresholds['test_secular_drift_tol']}")

    return thresholds


def design_tests(configs: list[dict]):
    """Output test design based on analytical results."""
    print(f"\n{'='*70}")
    print(f"  RECOMMENDED ORACLE TESTS")
    print(f"{'='*70}")

    for cfg in configs:
        name = cfg['name']
        a = cfg['a']
        e = cfg['e_0']
        i = cfg['i_deg']
        T = cfg['period_s']
        w_hp = cfg['omega_half_period_days']

        n_orbits_half = int(w_hp * 86400 / T)
        n_orbits_full = int(2 * w_hp * 86400 / T)

        print(f"\n  --- {name} ---")
        print(f"  Config: a={a:.0f} km, e={e}, i={i}°")
        print()

        print(f"  Test: j2_bounded_{n_orbits_half}_orbits")
        print(f"    Duration: {w_hp:.0f} days ({n_orbits_half} orbits = 1 modulation period)")
        print(f"    Assertions:")
        print(f"      e ∈ [{cfg['test_e_lower']:.6f}, {cfg['test_e_upper']:.6f}]")
        print(f"      ω̇ = {cfg['omega_dot_deg_day']:.3f} ± 0.5 °/day")
        print(f"      Total energy rel error < {cfg['test_energy_rel_tol_dp45']:.0e} (DP45)")
        print(f"      |mean(e, first quarter) - mean(e, last quarter)| < {cfg['test_secular_drift_tol']}")
        print()

        print(f"  Test: j2_periodic_{n_orbits_full}_orbits")
        print(f"    Duration: {2*w_hp:.0f} days ({n_orbits_full} orbits = 2 modulation periods)")
        print(f"    Assertions:")
        print(f"      Same bounds as above")
        print(f"      e(t=0) ≈ e(t={2*w_hp:.0f} days) within {cfg['de_sp']*3:.6f}")
        print(f"      Proves oscillation is PERIODIC, not divergent")

    # Output JSON for programmatic use
    print(f"\n{'='*70}")
    print(f"  JSON THRESHOLDS (for test parametrization)")
    print(f"{'='*70}")
    for cfg in configs:
        # Remove non-serializable items and output
        out = {k: v for k, v in cfg.items() if isinstance(v, (int, float, str))}
        print(json.dumps(out, indent=2))


if __name__ == "__main__":
    print("J2 Eccentricity / Altitude Oscillation Analysis")
    print("Computes analytical predictions and test thresholds")

    configs = []

    # 1. Oracle test orbit (e=0.01, i=51.6°) — matches existing RK4 test
    configs.append(analyze_orbit(
        "Oracle e=0.01 (existing test)",
        a=R_EARTH + 800,
        e_0=0.01,
        i_deg=51.6,
        duration_days=35,
    ))

    # 2. Oracle test orbit (e=0.05, i=51.6°) — matches existing DP45 test
    configs.append(analyze_orbit(
        "Oracle e=0.05 (existing test)",
        a=R_EARTH + 800,
        e_0=0.05,
        i_deg=51.6,
        duration_days=120,
    ))

    # 3. Near-circular SSO (viewer simulation condition)
    configs.append(analyze_orbit(
        "SSO 800km near-circular (viewer)",
        a=R_EARTH + 800,
        e_0=0.0001,  # essentially circular
        i_deg=98.6,
        duration_days=120,
    ))

    # 4. Near-circular ISS (viewer simulation condition)
    configs.append(analyze_orbit(
        "ISS 400km near-circular (viewer)",
        a=R_EARTH + 400,
        e_0=0.0001,  # essentially circular
        i_deg=51.6,
        duration_days=120,
    ))

    design_tests(configs)

    # Summary: key finding
    print(f"\n{'='*70}")
    print("  KEY FINDINGS")
    print(f"{'='*70}")
    print("""
  1. J2 long-period eccentricity variation (δe_lp) is proportional to e.
     For near-circular orbits (e < 0.001), it is NEGLIGIBLE.

  2. The altitude oscillation GROWTH seen in the viewer over 30 days is
     caused by the PHASE INTERFERENCE between:
       a) Keplerian eccentricity oscillation (once per orbit)
       b) J2 radial perturbation (twice per orbit, latitude-dependent)
     As ω precesses, these go in/out of phase → amplitude modulation.

  3. This modulation is BOUNDED and PERIODIC with period = π/|ω̇|
     (~55-60 days for ISS inclination, ~62 days for SSO).

  4. The total energy (including J2 potential) is the conserved quantity.
     Two-body energy oscillates with amplitude ≈ J2 — this is NOT drift.

  5. To prove non-divergence numerically:
     - Run for ≥ 1 full modulation period (π/|ω̇| days)
     - Verify e returns to near-initial envelope
     - Verify total energy conserved to integrator precision
""")
