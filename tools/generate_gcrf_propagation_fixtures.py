# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit GCRF propagation reference fixtures.

Cross-validates the Rust OrbitalSystem<Gcrs> propagator against Orekit's
NumericalPropagator using GCRF (Geocentric Celestial Reference Frame)
as the propagation frame.

Key differences from the EME2000 fixtures:
  - Frame: GCRF (includes frame bias) vs EME2000 (J2000 mean equator)
  - Drag geodetic: Orekit uses ITRF via IERS 2010 internally (proper
    IAU 2006 chain), matching our EarthFrameBridge<Gcrs> implementation
  - Gravity body frame: GCRF (pole = GCRS Z-axis), matching our Rust
    ZonalHarmonics which uses raw position.z as polar component

Scenarios focus on configurations where the frame choice matters most:
  - Atmospheric drag (requires ECI -> ECEF -> geodetic conversion)
  - Third-body (ephemeris in GCRF)

Output: orts/tests/fixtures/orekit_gcrf_propagation_reference.json
Run:   uv run tools/generate_gcrf_propagation_fixtures.py
"""

import json
import math
import sys
from pathlib import Path


# --- Constants matching our Rust code exactly ---

MU_EARTH_KM3_S2 = 398600.4418
R_EARTH_KM = 6378.137
J2_EARTH = 1.08263e-3
J3_EARTH = -2.5356e-6
J4_EARTH = -1.6199e-6
MU_SUN_KM3_S2 = 132712440018.0
MU_MOON_KM3_S2 = 4902.800066
OMEGA_EARTH = 7.2921159e-5
DEFAULT_BALLISTIC_COEFF = 0.01


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
    from org.orekit.time import AbsoluteDate, TimeScalesFactory

    utc = TimeScalesFactory.getUTC()
    return AbsoluteDate(epoch_str.rstrip("Z"), utc)


def keplerian_to_cartesian(a_km, e, i_deg, raan_deg, omega_deg, nu_deg, mu_km3_s2):
    """Convert Keplerian elements to Cartesian (position km, velocity km/s)."""
    i = math.radians(i_deg)
    raan = math.radians(raan_deg)
    omega = math.radians(omega_deg)
    nu = math.radians(nu_deg)

    p = a_km * (1.0 - e * e)
    r_mag = p / (1.0 + e * math.cos(nu))
    v_mag = math.sqrt(mu_km3_s2 / p)

    r_pqw = [r_mag * math.cos(nu), r_mag * math.sin(nu), 0.0]
    v_pqw = [-v_mag * math.sin(nu), v_mag * (e + math.cos(nu)), 0.0]

    cos_raan, sin_raan = math.cos(raan), math.sin(raan)
    cos_omega, sin_omega = math.cos(omega), math.sin(omega)
    cos_i, sin_i = math.cos(i), math.sin(i)

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
    """Create NumericalPropagator with GCRF frame and matched force models."""
    from org.hipparchus.ode.nonstiff import DormandPrince853Integrator
    from org.hipparchus.geometry.euclidean.threed import Vector3D
    from org.orekit.frames import FramesFactory
    from org.orekit.orbits import CartesianOrbit, OrbitType
    from org.orekit.propagation import SpacecraftState
    from org.orekit.propagation.numerical import NumericalPropagator
    from org.orekit.utils import PVCoordinates

    # GCRF = Geocentric Celestial Reference Frame (IAU GCRS)
    gcrf = FramesFactory.getGCRF()
    mu_si = MU_EARTH_KM3_S2 * 1e9

    pos_m = Vector3D(pos_km[0] * 1e3, pos_km[1] * 1e3, pos_km[2] * 1e3)
    vel_m_s = Vector3D(vel_km_s[0] * 1e3, vel_km_s[1] * 1e3, vel_km_s[2] * 1e3)
    pv = PVCoordinates(pos_m, vel_m_s)
    orbit = CartesianOrbit(pv, gcrf, epoch_date, mu_si)

    min_step = 0.001
    max_step = 300.0
    integrator = DormandPrince853Integrator(min_step, max_step, 1e-14, 1e-12)

    propagator = NumericalPropagator(integrator)
    propagator.setOrbitType(OrbitType.CARTESIAN)
    propagator.setInitialState(SpacecraftState(orbit, 1.0))

    fm = scenario["force_model"]
    _add_gravity(propagator, fm["gravity"], gcrf)

    if fm.get("third_body_sun"):
        _add_third_body_sun(propagator)
    if fm.get("third_body_moon"):
        _add_third_body_moon(propagator)
    if fm.get("drag"):
        _add_drag(propagator, scenario["satellite"], fm["drag"])

    return propagator


def _add_gravity(propagator, grav_config, gcrf):
    """Add gravity field with GCRF as body frame.

    Our Rust ZonalHarmonics uses raw position.z as the polar component,
    which in the GCRS frame equals the GCRS Z-axis (CIP at J2000.0).
    Using GCRF as the Orekit body frame matches this behavior.
    """
    from org.orekit.forces.gravity import HolmesFeatherstoneAttractionModel
    from org.orekit.forces.gravity.potential import GravityFieldFactory

    degree = grav_config["degree"]
    order = grav_config.get("order", 0)
    provider = GravityFieldFactory.getNormalizedProvider(degree, order)
    hf = HolmesFeatherstoneAttractionModel(gcrf, provider)
    propagator.addForceModel(hf)


def _add_third_body_sun(propagator):
    from org.orekit.bodies import CelestialBodyFactory
    from org.orekit.forces.gravity import ThirdBodyAttraction

    propagator.addForceModel(ThirdBodyAttraction(CelestialBodyFactory.getSun()))


def _add_third_body_moon(propagator):
    from org.orekit.bodies import CelestialBodyFactory
    from org.orekit.forces.gravity import ThirdBodyAttraction

    propagator.addForceModel(ThirdBodyAttraction(CelestialBodyFactory.getMoon()))


def _add_drag(propagator, sat_config, drag_config):
    """Add atmospheric drag with proper ITRF geodetic conversion."""
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.forces.drag import DragForce, IsotropicDrag
    from org.orekit.frames import FramesFactory
    from org.orekit.utils import Constants, IERSConventions

    b = sat_config.get("ballistic_coeff_m2_kg", DEFAULT_BALLISTIC_COEFF)
    cd = 2.2
    area = 2.0 * b / cd
    drag_spacecraft = IsotropicDrag(area, cd)

    model_name = drag_config.get("model", "nrlmsise00")

    sun = CelestialBodyFactory.getSun()
    # ITRF with IERS 2010 conventions — Orekit handles GCRF -> ITRF
    # internally using the full IAU 2006 CIO chain + EOP data
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    earth = OneAxisEllipsoid(
        Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
        Constants.WGS84_EARTH_FLATTENING,
        itrf,
    )

    if model_name == "nrlmsise00":
        from org.orekit.models.earth.atmosphere import NRLMSISE00

        weather = drag_config["weather"]
        solar_proxy = _make_constant_solar_activity(weather["f107"], weather["ap"])
        atmosphere = NRLMSISE00(solar_proxy, sun, earth)
    elif model_name == "nrlmsise00_cssi":
        from org.orekit.models.earth.atmosphere import NRLMSISE00
        from org.orekit.models.earth.atmosphere.data import CssiSpaceWeatherData

        atmosphere = NRLMSISE00(CssiSpaceWeatherData("SpaceWeather-All-v1.2.txt"), sun, earth)
    else:
        raise ValueError(f"Unknown drag model: {model_name}")

    propagator.addForceModel(DragForce(atmosphere, drag_spacecraft))


def _make_constant_solar_activity(f107, ap):
    import jpype
    from org.orekit.models.earth.atmosphere import NRLMSISE00InputParameters
    from org.orekit.time import AbsoluteDate

    class ConstantSolarActivity:
        def getDailyFlux(self, date):
            return float(f107)

        def getAverageFlux(self, date):
            return float(f107)

        def getAp(self, date):
            return [float(ap)] * 7

        def getMinDate(self):
            return AbsoluteDate.PAST_INFINITY

        def getMaxDate(self):
            return AbsoluteDate.FUTURE_INFINITY

    return jpype.JProxy(NRLMSISE00InputParameters, inst=ConstantSolarActivity())


def propagate_scenario(scenario):
    """Propagate one scenario and collect trajectory."""
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

        trajectory.append(
            {
                "t_seconds": round(t, 6),
                "position_km": [
                    pos.getX() / 1e3,
                    pos.getY() / 1e3,
                    pos.getZ() / 1e3,
                ],
                "velocity_km_s": [
                    vel.getX() / 1e3,
                    vel.getY() / 1e3,
                    vel.getZ() / 1e3,
                ],
            }
        )
        t += output_step_s

    return trajectory


# --- Scenario Definitions ---

def scenarios():
    """GCRF scenarios for Gcrs path validation."""
    result = []

    iss_a = R_EARTH_KM + 400.0
    pos, vel = keplerian_to_cartesian(
        iss_a, 0.001, 51.6, 0.0, 0.0, 0.0, MU_EARTH_KM3_S2
    )
    iss_period = 2 * math.pi * math.sqrt(iss_a**3 / MU_EARTH_KM3_S2)

    # Scenario 1: J2 + third-body (Sun + Moon), ISS 10 orbits
    # No drag — validates gravity + ephemeris in GCRF
    result.append(
        {
            "name": "gcrf_j2_thirdbody_iss_10orbits",
            "description": "ISS orbit, J2 + Sun + Moon, 10 orbits, GCRF frame",
            "epoch_utc": "2024-03-20T12:00:00Z",
            "initial_keplerian": {
                "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
                "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
            },
            "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
            "force_model": {
                "gravity": {"degree": 2, "order": 0},
                "drag": None,
                "third_body_sun": True,
                "third_body_moon": True,
            },
            "satellite": {},
            "duration_s": round(iss_period * 10, 1),
            "output_step_s": 60.0,
        }
    )

    # Scenario 2: J2 + NRLMSISE-00 drag + Sun + Moon, ISS 10 orbits
    # Validates drag geodetic conversion in GCRF path
    result.append(
        {
            "name": "gcrf_j2_msise_thirdbody_iss_10orbits",
            "description": "ISS orbit, J2 + NRLMSISE-00 + Sun + Moon, 10 orbits, GCRF frame",
            "epoch_utc": "2024-03-20T12:00:00Z",
            "initial_keplerian": {
                "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
                "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
            },
            "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
            "force_model": {
                "gravity": {"degree": 2, "order": 0},
                "drag": {
                    "model": "nrlmsise00",
                    "weather": {"f107": 150.0, "ap": 15.0},
                },
                "third_body_sun": True,
                "third_body_moon": True,
            },
            "satellite": {"ballistic_coeff_m2_kg": 0.01},
            "duration_s": round(iss_period * 10, 1),
            "output_step_s": 60.0,
        }
    )

    # Scenario 3: J2 + NRLMSISE-00 (CSSI) + Sun + Moon, ISS 30 days
    # Long-duration for tolerance tightening measurement
    result.append(
        {
            "name": "gcrf_j2_msise_cssi_thirdbody_iss_30day",
            "description": "ISS orbit, J2 + NRLMSISE-00 CSSI + Sun + Moon, 30 days, GCRF frame",
            "epoch_utc": "2024-03-20T12:00:00Z",
            "initial_keplerian": {
                "a_km": iss_a, "e": 0.001, "i_deg": 51.6,
                "raan_deg": 0.0, "omega_deg": 0.0, "nu_deg": 0.0,
            },
            "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
            "force_model": {
                "gravity": {"degree": 2, "order": 0},
                "drag": {"model": "nrlmsise00_cssi"},
                "third_body_sun": True,
                "third_body_moon": True,
            },
            "satellite": {"ballistic_coeff_m2_kg": 0.01},
            "duration_s": 30 * 86400.0,
            "output_step_s": 3600.0,
        }
    )

    return result


def main():
    print("Setting up Orekit...")
    setup_orekit()

    print("Generating GCRF propagation fixtures...")

    all_scenarios = scenarios()
    output = {
        "generator": "tools/generate_gcrf_propagation_fixtures.py",
        "frame": "GCRF (IAU GCRS)",
        "note": "Propagation in GCRF frame with IERS 2010 conventions. "
        "Gravity body frame = GCRF (matches Rust ZonalHarmonics using raw z-axis). "
        "Drag uses ITRF internally via full IAU 2006 CIO chain + EOP.",
        "constants": {
            "mu_earth_km3_s2": MU_EARTH_KM3_S2,
            "r_earth_km": R_EARTH_KM,
            "j2": J2_EARTH,
            "j3": J3_EARTH,
            "j4": J4_EARTH,
            "mu_sun_km3_s2": MU_SUN_KM3_S2,
            "mu_moon_km3_s2": MU_MOON_KM3_S2,
        },
        "scenarios": [],
    }

    for i, scenario in enumerate(all_scenarios, 1):
        name = scenario["name"]
        print(f"\n[{i}/{len(all_scenarios)}] {name}: {scenario['description']}")
        duration_h = scenario["duration_s"] / 3600.0
        print(f"  Duration: {duration_h:.1f} h ({scenario['duration_s']:.0f} s)")

        trajectory = propagate_scenario(scenario)
        print(f"  Trajectory points: {len(trajectory)}")

        final = trajectory[-1]
        print(
            f"  Final position: [{final['position_km'][0]:.6f}, "
            f"{final['position_km'][1]:.6f}, {final['position_km'][2]:.6f}] km"
        )

        scenario_out = {**scenario, "trajectory": trajectory}
        output["scenarios"].append(scenario_out)

    out_path = Path("orts/tests/fixtures/orekit_gcrf_propagation_reference.json")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w") as f:
        json.dump(output, f, indent=2)

    size_mb = out_path.stat().st_size / (1024 * 1024)
    print(f"\nWrote {out_path} ({size_mb:.1f} MB)")
    print(f"Total scenarios: {len(all_scenarios)}")


if __name__ == "__main__":
    main()
