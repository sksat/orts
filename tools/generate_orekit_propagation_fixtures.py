# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit numerical propagation reference fixtures.

Cross-validates the Rust orbit propagator (OrbitalSystem + DormandPrince)
against Orekit's NumericalPropagator with matched force models.

Tiered scenarios:
  Tier 1: Gravity-only (J2, J2+J3+J4) — no ephemeris difference
  Tier 2: Gravity + third-body (Sun, Moon)
  Tier 3: Gravity + SRP
  Tier 4: Gravity + Harris-Priester drag
  Tier 5: Full force model

Known differences from our Rust implementation:
  - Sun/Moon position: Orekit DE405 vs our Meeus/analytical
  - Altitude for drag: Orekit geodetic (WGS-84) vs our spherical (r - R)
  - Gravity: Orekit HolmesFeatherstone vs our explicit J2/J3/J4

Output: orbits/tests/fixtures/orekit_propagation_reference.json
Run:   uv run tools/generate_orekit_propagation_fixtures.py
"""

import json
import math
import sys
from pathlib import Path


# ─── Constants matching our Rust code exactly ───

MU_EARTH_KM3_S2 = 398600.4418       # WGS84
R_EARTH_KM = 6378.137               # WGS84 equatorial
J2_EARTH = 1.08263e-3               # WGS84/EGM96
J3_EARTH = -2.5356e-6
J4_EARTH = -1.6199e-6
MU_SUN_KM3_S2 = 132712440018.0
MU_MOON_KM3_S2 = 4902.800066       # from ThirdBodyGravity::moon()
OMEGA_EARTH = 7.2921159e-5          # rad/s
SOLAR_RADIATION_PRESSURE = 4.5396e-6  # N/m² at 1 AU
DEFAULT_CR = 1.5
DEFAULT_AREA_TO_MASS = 0.02         # m²/kg
DEFAULT_BALLISTIC_COEFF = 0.01      # m²/kg


def setup_orekit():
    """Initialize Orekit JVM and data."""
    import orekit_jpype as orekit

    orekit.initVM()

    from orekit_jpype.pyhelpers import (
        download_orekit_data_curdir,
        setup_orekit_curdir,
    )

    data_dir = Path("orekit-data")
    if not data_dir.exists():
        print("Downloading Orekit data (~30 MB)...")
        download_orekit_data_curdir()
    setup_orekit_curdir()


def make_date(epoch_str: str):
    """Parse ISO 8601 epoch string to Orekit AbsoluteDate."""
    from org.orekit.time import AbsoluteDate, TimeScalesFactory

    utc = TimeScalesFactory.getUTC()
    return AbsoluteDate(epoch_str.rstrip("Z"), utc)


def verify_orekit_constants():
    """Verify Orekit's constants match ours."""
    from org.orekit.utils import Constants

    mu_orekit = Constants.WGS84_EARTH_MU / 1e9  # m³/s² → km³/s²
    re_orekit = Constants.WGS84_EARTH_EQUATORIAL_RADIUS / 1e3  # m → km

    print(f"  μ_Earth: ours={MU_EARTH_KM3_S2}, Orekit={mu_orekit:.4f}, diff={abs(mu_orekit - MU_EARTH_KM3_S2):.6e}")
    print(f"  R_Earth: ours={R_EARTH_KM}, Orekit={re_orekit:.6f}, diff={abs(re_orekit - R_EARTH_KM):.6e}")

    assert abs(mu_orekit - MU_EARTH_KM3_S2) < 0.01, f"μ_Earth mismatch: {mu_orekit} vs {MU_EARTH_KM3_S2}"
    assert abs(re_orekit - R_EARTH_KM) < 0.001, f"R_Earth mismatch: {re_orekit} vs {R_EARTH_KM}"


def keplerian_to_cartesian(a_km, e, i_deg, raan_deg, omega_deg, nu_deg, mu_km3_s2):
    """Convert Keplerian elements to Cartesian (position km, velocity km/s) in ECI."""
    i = math.radians(i_deg)
    raan = math.radians(raan_deg)
    omega = math.radians(omega_deg)
    nu = math.radians(nu_deg)

    # Semi-latus rectum
    p = a_km * (1.0 - e * e)

    # Position and velocity in perifocal frame
    r = p / (1.0 + e * math.cos(nu))
    r_pqw = [r * math.cos(nu), r * math.sin(nu), 0.0]
    v_pqw = [
        -math.sqrt(mu_km3_s2 / p) * math.sin(nu),
        math.sqrt(mu_km3_s2 / p) * (e + math.cos(nu)),
        0.0,
    ]

    # Rotation matrix: perifocal → ECI
    cos_raan = math.cos(raan)
    sin_raan = math.sin(raan)
    cos_omega = math.cos(omega)
    sin_omega = math.sin(omega)
    cos_i = math.cos(i)
    sin_i = math.sin(i)

    # Direction cosine matrix rows
    l1 = cos_raan * cos_omega - sin_raan * sin_omega * cos_i
    l2 = -cos_raan * sin_omega - sin_raan * cos_omega * cos_i
    m1 = sin_raan * cos_omega + cos_raan * sin_omega * cos_i
    m2 = -sin_raan * sin_omega + cos_raan * cos_omega * cos_i
    n1 = sin_omega * sin_i
    n2 = cos_omega * sin_i

    pos = [
        l1 * r_pqw[0] + l2 * r_pqw[1],
        m1 * r_pqw[0] + m2 * r_pqw[1],
        n1 * r_pqw[0] + n2 * r_pqw[1],
    ]
    vel = [
        l1 * v_pqw[0] + l2 * v_pqw[1],
        m1 * v_pqw[0] + m2 * v_pqw[1],
        n1 * v_pqw[0] + n2 * v_pqw[1],
    ]

    return pos, vel


def create_propagator(pos_km, vel_km_s, epoch_date, scenario):
    """Create NumericalPropagator with force models matching the scenario config."""
    from org.hipparchus.ode.nonstiff import DormandPrince853Integrator
    from org.hipparchus.geometry.euclidean.threed import Vector3D
    from org.orekit.frames import FramesFactory
    from org.orekit.orbits import CartesianOrbit, OrbitType
    from org.orekit.propagation import SpacecraftState
    from org.orekit.propagation.numerical import NumericalPropagator
    from org.orekit.utils import PVCoordinates

    eci = FramesFactory.getEME2000()
    mu_si = MU_EARTH_KM3_S2 * 1e9  # km³/s² → m³/s²

    # Initial state in meters
    pos_m = Vector3D(pos_km[0] * 1e3, pos_km[1] * 1e3, pos_km[2] * 1e3)
    vel_m_s = Vector3D(vel_km_s[0] * 1e3, vel_km_s[1] * 1e3, vel_km_s[2] * 1e3)
    pv = PVCoordinates(pos_m, vel_m_s)
    orbit = CartesianOrbit(pv, eci, epoch_date, mu_si)

    # Tight integrator tolerances
    min_step = 0.001   # s
    max_step = 300.0   # s
    integrator = DormandPrince853Integrator(min_step, max_step, 1e-14, 1e-12)

    propagator = NumericalPropagator(integrator)
    propagator.setOrbitType(OrbitType.CARTESIAN)
    # Use mass=1.0 kg so that area-to-mass ratios in IsotropicDrag/IsotropicRadiation
    # work correctly (our Rust code uses unit-mass conventions: B = Cd*A/(2m))
    propagator.setInitialState(SpacecraftState(orbit, 1.0))

    # Add force models
    fm = scenario["force_model"]
    _add_gravity(propagator, fm["gravity"], eci)

    if fm.get("third_body_sun"):
        _add_third_body_sun(propagator)
    if fm.get("third_body_moon"):
        _add_third_body_moon(propagator)
    if fm.get("srp"):
        _add_srp(propagator, scenario["satellite"], fm["srp"])
    if fm.get("drag"):
        _add_drag(propagator, scenario["satellite"], fm["drag"], eci)

    return propagator


def _add_gravity(propagator, grav_config, eci):
    """Add gravity field model matching our ZonalHarmonics.

    IMPORTANT: Our Rust code computes J2/J3/J4 acceleration using the J2000 Z-axis
    as the Earth's pole. To match this, we pass EME2000 as the body frame for
    HolmesFeatherstone. This eliminates the precession/nutation difference between
    the J2000 pole and CIP (ITRF Z-axis), which is ~0.33° in 2024 and would cause
    ~1e-7 km/s² acceleration error at LEO altitude.

    For zonal-only harmonics (order=0), the field is axially symmetric, so Earth
    rotation doesn't matter — only the pole axis direction matters.
    """
    from org.orekit.forces.gravity import HolmesFeatherstoneAttractionModel
    from org.orekit.forces.gravity.potential import GravityFieldFactory
    from org.orekit.frames import FramesFactory

    degree = grav_config["degree"]
    order = grav_config.get("order", 0)

    # Load normalized gravity field restricted to degree/order
    provider = GravityFieldFactory.getNormalizedProvider(degree, order)

    # Verify J2 coefficient: for fully-normalized, C̄₂₀ = -J2/√5
    c20_normalized = provider.onDate(make_date("2024-03-20T12:00:00Z")).getNormalizedCnm(2, 0)
    j2_from_orekit = -c20_normalized * math.sqrt(5)
    print(f"  J2: ours={J2_EARTH:.6e}, Orekit C̄₂₀→J2={j2_from_orekit:.6e}, diff={abs(j2_from_orekit - J2_EARTH):.3e}")

    # Use EME2000 as body frame so gravity pole = J2000 Z-axis (matches our Rust code)
    hf = HolmesFeatherstoneAttractionModel(
        FramesFactory.getEME2000(),
        provider,
    )
    propagator.addForceModel(hf)


def _add_third_body_sun(propagator):
    """Add Sun third-body perturbation."""
    from org.orekit.bodies import CelestialBodyFactory
    from org.orekit.forces.gravity import ThirdBodyAttraction

    sun = CelestialBodyFactory.getSun()
    propagator.addForceModel(ThirdBodyAttraction(sun))


def _add_third_body_moon(propagator):
    """Add Moon third-body perturbation."""
    from org.orekit.bodies import CelestialBodyFactory
    from org.orekit.forces.gravity import ThirdBodyAttraction

    moon = CelestialBodyFactory.getMoon()
    propagator.addForceModel(ThirdBodyAttraction(moon))


def _add_srp(propagator, sat_config, srp_config):
    """Add SRP with cannonball model and optional cylindrical shadow."""
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.forces.radiation import (
        IsotropicRadiationSingleCoefficient,
        SolarRadiationPressure,
    )
    from org.orekit.frames import FramesFactory
    from org.orekit.utils import Constants, IERSConventions

    sun = CelestialBodyFactory.getSun()
    area_to_mass = sat_config.get("srp_area_to_mass_m2_kg", DEFAULT_AREA_TO_MASS)
    cr = sat_config.get("srp_cr", DEFAULT_CR)

    # IsotropicRadiationSingleCoefficient(crossSection, Cr)
    # For unit mass: crossSection = area_to_mass (m²)
    spacecraft = IsotropicRadiationSingleCoefficient(area_to_mass, cr)

    if srp_config.get("shadow", True):
        itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
        earth = OneAxisEllipsoid(
            Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
            Constants.WGS84_EARTH_FLATTENING,
            itrf,
        )
        srp = SolarRadiationPressure(sun, earth, spacecraft)
    else:
        srp = SolarRadiationPressure(sun, spacecraft)

    propagator.addForceModel(srp)


def _add_drag(propagator, sat_config, drag_config, eci):
    """Add atmospheric drag with Harris-Priester model."""
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.forces.drag import DragForce, IsotropicDrag
    from org.orekit.frames import FramesFactory
    from org.orekit.models.earth.atmosphere import HarrisPriester
    from org.orekit.utils import Constants, IERSConventions

    sun = CelestialBodyFactory.getSun()
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    earth = OneAxisEllipsoid(
        Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
        Constants.WGS84_EARTH_FLATTENING,
        itrf,
    )

    n = drag_config.get("n", 2)
    hp = HarrisPriester(sun, earth, n)

    b = sat_config.get("ballistic_coeff_m2_kg", DEFAULT_BALLISTIC_COEFF)
    # Our B = Cd * A / (2*m).  Orekit drag: a = 0.5 * Cd * (A/m) * ρ * v²
    # So for unit mass: a = 0.5 * Cd * A * ρ * v² = B * ρ * v² when Cd*A/2 = B
    # IsotropicDrag(crossSection, cd): for unit mass, set cross_section = 2*B/Cd
    # with Cd = 2.2 (standard): area = 2 * B / 2.2
    cd = 2.2
    area = 2.0 * b / cd  # m² (for unit mass)
    drag_spacecraft = IsotropicDrag(area, cd)

    propagator.addForceModel(DragForce(hp, drag_spacecraft))


def propagate_scenario(scenario):
    """Propagate one scenario and collect trajectory + acceleration at t=0."""
    epoch_date = make_date(scenario["epoch_utc"])
    pos_km = scenario["initial_cartesian"]["position_km"]
    vel_km_s = scenario["initial_cartesian"]["velocity_km_s"]

    propagator = create_propagator(pos_km, vel_km_s, epoch_date, scenario)

    duration_s = scenario["duration_s"]
    output_step_s = scenario["output_step_s"]

    trajectory = []
    t = 0.0
    while t <= duration_s + 0.01:
        target_date = epoch_date.shiftedBy(t)
        state = propagator.propagate(target_date)
        pv = state.getPVCoordinates()
        pos = pv.getPosition()
        vel = pv.getVelocity()

        trajectory.append({
            "t_seconds": round(t, 6),
            "position_km": [pos.getX() / 1e3, pos.getY() / 1e3, pos.getZ() / 1e3],
            "velocity_km_s": [vel.getX() / 1e3, vel.getY() / 1e3, vel.getZ() / 1e3],
        })

        t += output_step_s

    # Acceleration at t=0: compute from propagator acceleration providers
    # Use finite-difference as a simple approach
    dt_acc = 0.01  # 10 ms
    state_0 = propagator.propagate(epoch_date)
    state_dt = propagator.propagate(epoch_date.shiftedBy(dt_acc))
    v0 = state_0.getPVCoordinates().getVelocity()
    v1 = state_dt.getPVCoordinates().getVelocity()
    accel_x = (v1.getX() - v0.getX()) / dt_acc / 1e3  # m/s² → km/s²
    accel_y = (v1.getY() - v0.getY()) / dt_acc / 1e3
    accel_z = (v1.getZ() - v0.getZ()) / dt_acc / 1e3

    return trajectory, [accel_x, accel_y, accel_z]


# ─── Scenario Definitions ───

def tier1_scenarios():
    """Gravity-only scenarios (no epoch-dependent forces)."""
    scenarios = []

    # ISS-like: h=400km, i=51.6°, e=0.001
    iss_a = R_EARTH_KM + 400.0
    pos, vel = keplerian_to_cartesian(iss_a, 0.001, 51.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    iss_period = 2 * math.pi * math.sqrt(iss_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_iss_3orbits",
        "description": "ISS-like orbit, J2-only, 3 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {},
        "duration_s": round(iss_period * 3, 1),
        "output_step_s": 60.0,
    })

    # SSO: h=800km, i=98.6°, e=0.001
    sso_a = R_EARTH_KM + 800.0
    pos, vel = keplerian_to_cartesian(sso_a, 0.001, 98.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    sso_period = 2 * math.pi * math.sqrt(sso_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_sso_10orbits",
        "description": "SSO orbit, J2-only, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": sso_a, "e": 0.001, "i_deg": 98.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {},
        "duration_s": round(sso_period * 10, 1),
        "output_step_s": 60.0,
    })

    # ISS with J2+J3+J4
    pos, vel = keplerian_to_cartesian(iss_a, 0.001, 51.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    scenarios.append({
        "name": "j2j3j4_iss_10orbits",
        "description": "ISS-like orbit, J2+J3+J4, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 4, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {},
        "duration_s": round(iss_period * 10, 1),
        "output_step_s": 60.0,
    })

    # Equatorial: h=400km, i=0°
    eq_a = R_EARTH_KM + 400.0
    pos, vel = keplerian_to_cartesian(eq_a, 0.001, 0.01, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    eq_period = 2 * math.pi * math.sqrt(eq_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_equatorial_5orbits",
        "description": "Near-equatorial orbit, J2-only, 5 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": eq_a, "e": 0.001, "i_deg": 0.01,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {},
        "duration_s": round(eq_period * 5, 1),
        "output_step_s": 60.0,
    })

    return scenarios


def tier2_scenarios():
    """Gravity + third-body scenarios."""
    scenarios = []

    # SSO + Sun + Moon, 10 orbits
    sso_a = R_EARTH_KM + 800.0
    pos, vel = keplerian_to_cartesian(sso_a, 0.001, 98.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    sso_period = 2 * math.pi * math.sqrt(sso_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_sun_moon_sso_10orbits",
        "description": "SSO orbit, J2 + Sun + Moon, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": sso_a, "e": 0.001, "i_deg": 98.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": True, "third_body_moon": True,
        },
        "satellite": {},
        "duration_s": round(sso_period * 10, 1),
        "output_step_s": 60.0,
    })

    # GEO + Sun + Moon, 3 days
    geo_a = 42164.0
    pos, vel = keplerian_to_cartesian(geo_a, 0.001, 0.01, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_sun_moon_geo_3days",
        "description": "GEO orbit, J2 + Sun + Moon, 3 days",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": geo_a, "e": 0.001, "i_deg": 0.01,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None, "srp": None,
            "third_body_sun": True, "third_body_moon": True,
        },
        "satellite": {},
        "duration_s": 3.0 * 86400.0,
        "output_step_s": 300.0,
    })

    return scenarios


def tier3_scenarios():
    """Gravity + SRP scenarios."""
    scenarios = []

    sso_a = R_EARTH_KM + 800.0
    pos, vel = keplerian_to_cartesian(sso_a, 0.001, 98.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    sso_period = 2 * math.pi * math.sqrt(sso_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_srp_sso_10orbits",
        "description": "SSO orbit, J2 + SRP (cylindrical shadow), 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": sso_a, "e": 0.001, "i_deg": 98.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": None,
            "srp": {"shadow": True},
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {
            "srp_area_to_mass_m2_kg": DEFAULT_AREA_TO_MASS,
            "srp_cr": DEFAULT_CR,
        },
        "duration_s": round(sso_period * 10, 1),
        "output_step_s": 60.0,
    })

    return scenarios


def tier4_scenarios():
    """Gravity + Harris-Priester drag scenarios."""
    scenarios = []

    # Near-equatorial ISS (minimizes geodetic/spherical alt difference)
    iss_a = R_EARTH_KM + 400.0
    pos, vel = keplerian_to_cartesian(iss_a, 0.001, 5.0, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    iss_period = 2 * math.pi * math.sqrt(iss_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_hp_iss_equatorial_5orbits",
        "description": "Near-equatorial orbit, J2 + HP drag, 5 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": iss_a, "e": 0.001, "i_deg": 5.0,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": {"model": "harris_priester", "n": 2},
            "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {
            "ballistic_coeff_m2_kg": DEFAULT_BALLISTIC_COEFF,
        },
        "duration_s": round(iss_period * 5, 1),
        "output_step_s": 60.0,
    })

    # ISS inclination (exposes geodetic/spherical difference)
    pos, vel = keplerian_to_cartesian(iss_a, 0.001, 51.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "j2_hp_iss_10orbits",
        "description": "ISS orbit, J2 + HP drag, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": {"model": "harris_priester", "n": 2},
            "srp": None,
            "third_body_sun": False, "third_body_moon": False,
        },
        "satellite": {
            "ballistic_coeff_m2_kg": DEFAULT_BALLISTIC_COEFF,
        },
        "duration_s": round(iss_period * 10, 1),
        "output_step_s": 60.0,
    })

    return scenarios


def tier5_scenarios():
    """Full force model scenarios."""
    scenarios = []

    iss_a = R_EARTH_KM + 400.0
    pos, vel = keplerian_to_cartesian(iss_a, 0.001, 51.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    iss_period = 2 * math.pi * math.sqrt(iss_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "full_iss_10orbits",
        "description": "ISS orbit, J2 + HP drag + SRP + Sun + Moon, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": {"model": "harris_priester", "n": 2},
            "srp": {"shadow": True},
            "third_body_sun": True, "third_body_moon": True,
        },
        "satellite": {
            "ballistic_coeff_m2_kg": DEFAULT_BALLISTIC_COEFF,
            "srp_area_to_mass_m2_kg": DEFAULT_AREA_TO_MASS,
            "srp_cr": DEFAULT_CR,
        },
        "duration_s": round(iss_period * 10, 1),
        "output_step_s": 60.0,
    })

    sso_a = R_EARTH_KM + 800.0
    pos, vel = keplerian_to_cartesian(sso_a, 0.001, 98.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2)
    sso_period = 2 * math.pi * math.sqrt(sso_a ** 3 / MU_EARTH_KM3_S2)

    scenarios.append({
        "name": "full_sso_10orbits",
        "description": "SSO orbit, J2 + HP drag + SRP + Sun + Moon, 10 orbits",
        "epoch_utc": "2024-03-20T12:00:00Z",
        "initial_keplerian": {
            "a_km": sso_a, "e": 0.001, "i_deg": 98.6,
            "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
        },
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "force_model": {
            "gravity": {"degree": 2, "order": 0},
            "drag": {"model": "harris_priester", "n": 2},
            "srp": {"shadow": True},
            "third_body_sun": True, "third_body_moon": True,
        },
        "satellite": {
            "ballistic_coeff_m2_kg": DEFAULT_BALLISTIC_COEFF,
            "srp_area_to_mass_m2_kg": DEFAULT_AREA_TO_MASS,
            "srp_cr": DEFAULT_CR,
        },
        "duration_s": round(sso_period * 10, 1),
        "output_step_s": 60.0,
    })

    return scenarios


def main():
    setup_orekit()

    print("Verifying constants...")
    verify_orekit_constants()

    all_scenarios = []
    for tier_name, tier_fn in [
        ("Tier 1 (gravity-only)", tier1_scenarios),
        ("Tier 2 (gravity + third-body)", tier2_scenarios),
        ("Tier 3 (gravity + SRP)", tier3_scenarios),
        ("Tier 4 (gravity + HP drag)", tier4_scenarios),
        ("Tier 5 (full force model)", tier5_scenarios),
    ]:
        scenarios = tier_fn()
        print(f"\n{tier_name}: {len(scenarios)} scenarios")
        for s in scenarios:
            print(f"  Propagating {s['name']}...")
            trajectory, accel_t0 = propagate_scenario(s)
            s["trajectory"] = trajectory
            s["acceleration_at_t0"] = {"total_km_s2": accel_t0}
            print(f"    {len(trajectory)} points, duration={s['duration_s']:.0f}s")
            all_scenarios.append(s)

    output = {
        "generator": "tools/generate_orekit_propagation_fixtures.py",
        "frame": "EME2000 (J2000)",
        "note": "Position in km, velocity in km/s. Constants matched to Rust orts.",
        "constants": {
            "mu_earth_km3_s2": MU_EARTH_KM3_S2,
            "r_earth_km": R_EARTH_KM,
            "j2": J2_EARTH,
            "j3": J3_EARTH,
            "j4": J4_EARTH,
            "mu_sun_km3_s2": MU_SUN_KM3_S2,
            "mu_moon_km3_s2": MU_MOON_KM3_S2,
            "omega_earth_rad_s": OMEGA_EARTH,
            "solar_radiation_pressure_pa": SOLAR_RADIATION_PRESSURE,
        },
        "scenarios": all_scenarios,
    }

    out_path = (
        Path(__file__).parent.parent
        / "orbits"
        / "tests"
        / "fixtures"
        / "orekit_propagation_reference.json"
    )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, indent=2) + "\n")
    print(f"\nWritten to {out_path}")
    print(f"  {len(all_scenarios)} scenarios total")
    for s in all_scenarios:
        print(f"    {s['name']}: {len(s['trajectory'])} points")


if __name__ == "__main__":
    main()
