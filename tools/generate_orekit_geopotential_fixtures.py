# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit reference fixtures for the spherical-harmonic geopotential.

Three artefacts, all derived from the *same* Orekit gravity-field provider so
the Rust side evaluates exactly the coefficients Orekit evaluated:

1. ``tobari/tests/fixtures/orekit_geopotential_70x70.gfc``
   The provider's fully-normalized coefficients, truncated to 70x70, written in
   ICGEM ``gfc`` format. Exercises the Rust ICGEM parser and feeds the two
   reference files below.

2. ``tobari/tests/fixtures/orekit_geopotential_gradient_reference.json``
   Pointwise body-frame (ITRF) accelerations and disturbing potentials from
   ``HolmesFeatherstoneAttractionModel.gradient`` / ``nonCentralPart`` at a set
   of positions (LEO..GEO, equator..near-pole) for several (degree, order)
   truncations. Validates ``SphericalHarmonicField`` on its own, independent of
   frame transforms and integrators.

3. ``orts/tests/fixtures/orekit_geopotential_propagation_reference.json``
   24 h GCRF propagations with ITRF as the gravity body frame (IERS 2010,
   simple EOP), point mass (mu = provider mu) + the harmonic field only.
   Validates ``SphericalHarmonicGravity<Gcrs>`` end to end.

Run:   uv run tools/generate_orekit_geopotential_fixtures.py
"""

import json
import math
from pathlib import Path

FULL_DEGREE = 70
FULL_ORDER = 70
# (degree, order) truncations for the pointwise reference. Zonal-only sets
# cross-check the ZonalGravity equivalence; the rest exercise tesserals.
POINTWISE_SETS = [(2, 0), (4, 0), (8, 8), (20, 20), (70, 70)]
EPOCH_UTC = "2024-03-20T12:00:00Z"  # inside orts/tests/fixtures/finals2000A.sample


def setup_orekit():
    import os

    import jdk4py
    import orekit_jpype as orekit

    if not os.environ.get("JAVA_HOME"):
        os.environ["JAVA_HOME"] = str(jdk4py.JAVA_HOME)
    orekit.initVM()

    from orekit_jpype.pyhelpers import (
        download_orekit_data_curdir,
        setup_orekit_curdir,
    )

    if not Path("orekit-data").exists():
        print("Downloading Orekit data (~30 MB)...")
        download_orekit_data_curdir()
    setup_orekit_curdir()


def make_date(epoch_str: str):
    from org.orekit.time import AbsoluteDate, TimeScalesFactory

    return AbsoluteDate(epoch_str.rstrip("Z"), TimeScalesFactory.getUTC())


def tide_system_name(provider) -> str:
    name = str(provider.getTideSystem().name())
    return {"ZERO_TIDE": "zero_tide", "TIDE_FREE": "tide_free"}.get(name, "unknown")


def write_gfc(provider, date, path: Path):
    """Write the provider's coefficients (at `date`) as an ICGEM gfc file."""
    harmonics = provider.onDate(date)
    lines = [
        "begin_of_head",
        "product_type            gravity_field",
        "modelname               orekit-data default field, truncated by "
        "tools/generate_orekit_geopotential_fixtures.py",
        f"earth_gravity_constant  {float(provider.getMu())!r}",
        f"radius                  {float(provider.getAe())!r}",
        f"max_degree              {FULL_DEGREE}",
        "errors                  no",
        "norm                    fully_normalized",
        f"tide_system             {tide_system_name(provider)}",
        "end_of_head",
    ]
    for n in range(0, FULL_DEGREE + 1):
        for m in range(0, min(n, FULL_ORDER) + 1):
            c = float(harmonics.getNormalizedCnm(n, m))
            s = float(harmonics.getNormalizedSnm(n, m))
            lines.append(f"gfc {n:4d} {m:4d} {c!r} {s!r}")
    path.write_text("\n".join(lines) + "\n")
    print(f"  wrote {path} ({len(lines)} lines)")


def pointwise_positions_m(ae_m: float):
    """Body-frame sample positions: altitudes x latitudes x longitudes."""
    positions = []
    altitudes_km = [300.0, 570.0, 1000.0, 20200.0, 35786.0]
    latitudes_deg = [-89.99, -60.0, -30.0, 0.0, 30.0, 60.0, 89.999]
    longitudes_deg = [0.0, 45.0, 137.0, -100.0]
    for i, alt_km in enumerate(altitudes_km):
        r = ae_m + alt_km * 1e3
        for j, lat in enumerate(latitudes_deg):
            # Rotate through the longitude list so every altitude/latitude row
            # doesn't share one longitude but the set stays small.
            lon = longitudes_deg[(i + j) % len(longitudes_deg)]
            lat_r, lon_r = math.radians(lat), math.radians(lon)
            positions.append(
                [
                    r * math.cos(lat_r) * math.cos(lon_r),
                    r * math.cos(lat_r) * math.sin(lon_r),
                    r * math.sin(lat_r),
                ]
            )
    # Practically-on-the-pole points (rho = 1 mm): Orekit's spherical gradient
    # is finite here, and the Rust pole regularization must agree.
    for z in (ae_m + 570e3, -(ae_m + 570e3)):
        positions.append([1e-3, 0.0, z])
    return positions


def pointwise_reference(date):
    from org.hipparchus.geometry.euclidean.threed import Vector3D
    from org.orekit.forces.gravity import HolmesFeatherstoneAttractionModel
    from org.orekit.forces.gravity.potential import GravityFieldFactory
    from org.orekit.frames import FramesFactory
    from org.orekit.utils import IERSConventions

    full = GravityFieldFactory.getNormalizedProvider(FULL_DEGREE, FULL_ORDER)
    mu = float(full.getMu())
    ae = float(full.getAe())
    positions = pointwise_positions_m(ae)
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)

    sets = []
    for degree, order in POINTWISE_SETS:
        provider = GravityFieldFactory.getNormalizedProvider(degree, order)
        assert float(provider.getMu()) == mu
        # Body frame is irrelevant for a body-frame evaluation; ITRF for form.
        hf = HolmesFeatherstoneAttractionModel(itrf, provider)
        points = []
        for p in positions:
            v = Vector3D(p[0], p[1], p[2])
            grad = hf.gradient(date, v, mu)  # m/s², body frame
            u = float(hf.nonCentralPart(date, v, mu))  # m²/s²
            points.append(
                {
                    "position_km": [p[0] / 1e3, p[1] / 1e3, p[2] / 1e3],
                    "acceleration_km_s2": [
                        float(grad[0]) / 1e3,
                        float(grad[1]) / 1e3,
                        float(grad[2]) / 1e3,
                    ],
                    "potential_km2_s2": u / 1e6,
                }
            )
        sets.append({"degree": degree, "order": order, "points": points})
        print(f"  pointwise {degree}x{order}: {len(points)} points")

    return {
        "generator": "tools/generate_orekit_geopotential_fixtures.py",
        "note": "HolmesFeatherstoneAttractionModel.gradient (non-central part, "
        "body frame) and nonCentralPart, coefficients from "
        "orekit_geopotential_70x70.gfc truncated to (degree, order).",
        "epoch_utc": EPOCH_UTC,
        "mu_km3_s2": mu / 1e9,
        "radius_km": ae / 1e3,
        "sets": sets,
    }, full


def keplerian_to_cartesian(a_km, e, i_deg, raan_deg, omega_deg, nu_deg, mu_km3_s2):
    i, raan, omega, nu = map(math.radians, (i_deg, raan_deg, omega_deg, nu_deg))
    p = a_km * (1 - e * e)
    r = p / (1 + e * math.cos(nu))
    r_pqw = [r * math.cos(nu), r * math.sin(nu), 0.0]
    k = math.sqrt(mu_km3_s2 / p)
    v_pqw = [-k * math.sin(nu), k * (e + math.cos(nu)), 0.0]
    cr, sr = math.cos(raan), math.sin(raan)
    co, so = math.cos(omega), math.sin(omega)
    ci, si = math.cos(i), math.sin(i)
    rot = [
        [cr * co - sr * so * ci, -cr * so - sr * co * ci],
        [sr * co + cr * so * ci, -sr * so + cr * co * ci],
        [so * si, co * si],
    ]
    pos = [rot[k][0] * r_pqw[0] + rot[k][1] * r_pqw[1] for k in range(3)]
    vel = [rot[k][0] * v_pqw[0] + rot[k][1] * v_pqw[1] for k in range(3)]
    return pos, vel


def propagate(scenario, date, mu_si):
    from org.hipparchus.geometry.euclidean.threed import Vector3D
    from org.hipparchus.ode.nonstiff import DormandPrince853Integrator
    from org.orekit.forces.gravity import HolmesFeatherstoneAttractionModel
    from org.orekit.forces.gravity.potential import GravityFieldFactory
    from org.orekit.frames import FramesFactory
    from org.orekit.orbits import CartesianOrbit, OrbitType
    from org.orekit.propagation import SpacecraftState
    from org.orekit.propagation.numerical import NumericalPropagator
    from org.orekit.utils import IERSConventions, PVCoordinates

    gcrf = FramesFactory.getGCRF()
    pos_km = scenario["initial_cartesian"]["position_km"]
    vel_km_s = scenario["initial_cartesian"]["velocity_km_s"]
    pv = PVCoordinates(
        Vector3D(*[x * 1e3 for x in pos_km]),
        Vector3D(*[x * 1e3 for x in vel_km_s]),
    )
    orbit = CartesianOrbit(pv, gcrf, date, mu_si)

    propagator = NumericalPropagator(DormandPrince853Integrator(0.001, 300.0, 1e-14, 1e-12))
    propagator.setOrbitType(OrbitType.CARTESIAN)
    propagator.setInitialState(SpacecraftState(orbit, 1.0))

    g = scenario["force_model"]["gravity"]
    provider = GravityFieldFactory.getNormalizedProvider(g["degree"], g["order"])
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    propagator.addForceModel(HolmesFeatherstoneAttractionModel(itrf, provider))

    trajectory = []
    t = 0.0
    while t <= scenario["duration_s"] + 0.01:
        state = propagator.propagate(date.shiftedBy(t))
        pv = state.getPVCoordinates()
        p, v = pv.getPosition(), pv.getVelocity()
        trajectory.append(
            {
                "t_seconds": round(t, 6),
                "position_km": [p.getX() / 1e3, p.getY() / 1e3, p.getZ() / 1e3],
                "velocity_km_s": [v.getX() / 1e3, v.getY() / 1e3, v.getZ() / 1e3],
            }
        )
        t += scenario["output_step_s"]
    return trajectory


def scenarios(mu_km3_s2):
    # 570 km sun-synchronous-like orbit: the altitude band from issue #411.
    a_km = 6378.137 + 570.0
    pos, vel = keplerian_to_cartesian(a_km, 0.001, 97.6, 40.0, 30.0, 0.0, mu_km3_s2)
    base = {
        "epoch_utc": EPOCH_UTC,
        "initial_cartesian": {"position_km": pos, "velocity_km_s": vel},
        "duration_s": 86400.0,
        "output_step_s": 300.0,
    }
    return [
        {
            "name": "leo_570km_70x70",
            "description": "570 km LEO, point mass + 70x70 field, ITRF body frame, 24 h",
            "force_model": {"gravity": {"degree": 70, "order": 70}},
            **base,
        },
        {
            "name": "leo_570km_20x20",
            "description": "570 km LEO, point mass + 20x20 field "
            "(truncation of the 70x70 fixture), 24 h",
            "force_model": {"gravity": {"degree": 20, "order": 20}},
            **base,
        },
        {
            "name": "leo_570km_4x0",
            "description": "570 km LEO, point mass + zonal J2..J4 only, ITRF body frame, 24 h",
            "force_model": {"gravity": {"degree": 4, "order": 0}},
            **base,
        },
    ]


def main():
    print("Setting up Orekit...")
    setup_orekit()
    date = make_date(EPOCH_UTC)

    print("Pointwise gradient reference...")
    pointwise, full = pointwise_reference(date)
    mu_si = float(full.getMu())
    print(
        f"  provider: mu={mu_si!r} m^3/s^2, ae={float(full.getAe())!r} m, "
        f"tide={tide_system_name(full)}"
    )

    gfc_path = Path("tobari/tests/fixtures/orekit_geopotential_70x70.gfc")
    gfc_path.parent.mkdir(parents=True, exist_ok=True)
    write_gfc(full, date, gfc_path)

    pw_path = Path("tobari/tests/fixtures/orekit_geopotential_gradient_reference.json")
    pw_path.write_text(json.dumps(pointwise, indent=1) + "\n")
    print(f"  wrote {pw_path}")

    print("Propagation reference...")
    out = {
        "generator": "tools/generate_orekit_geopotential_fixtures.py",
        "frame": "GCRF (IAU GCRS)",
        "note": "NumericalPropagator in GCRF; NewtonianAttraction with mu = provider mu; "
        "HolmesFeatherstoneAttractionModel with ITRF (IERS 2010, simple EOP) body frame; "
        "coefficients = orekit_geopotential_70x70.gfc truncated to (degree, order).",
        "mu_km3_s2": mu_si / 1e9,
        "scenarios": [],
    }
    for sc in scenarios(mu_si / 1e9):
        print(f"  {sc['name']}: {sc['description']}")
        traj = propagate(sc, date, mu_si)
        out["scenarios"].append({**sc, "trajectory": traj})
        last = traj[-1]["position_km"]
        print(f"    {len(traj)} points, final {last[0]:.6f} {last[1]:.6f} {last[2]:.6f} km")

    prop_path = Path("orts/tests/fixtures/orekit_geopotential_propagation_reference.json")
    prop_path.write_text(json.dumps(out, indent=1) + "\n")
    print(f"  wrote {prop_path} ({prop_path.stat().st_size / 1024:.0f} KB)")


if __name__ == "__main__":
    main()
