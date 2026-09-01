# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit reference forces for the flat-panel SRP law.

Cross-validates `orts`'s per-panel SRP force against Orekit's paneled
radiation model. Unlike the propagation fixtures next door, this needs no
propagator and no attitude provider: `RadiationSensitive` exposes
`radiationPressureAcceleration(state, flux, parameters)`, so one panel can be
evaluated directly for a chosen incidence angle.

The comparison is on the force, because Orekit's paneled model returns only an
acceleration. The torque our model builds from it, `sum(r_cp x F_panel)`, is
pinned separately by exact cross-product tests.

Convention mapping, established by driving the three limits (a black panel, a
mirror, and a Lambertian one) and confirmed by this fixture:

    Orekit `absorption` = alpha,  Orekit `reflection` = rho_s,
    rho_d = 1 - alpha - rho_s

Geometry of every case: panel normal is +Z in the body frame, the Sun sits at
(sin(theta), 0, cos(theta)), and the attitude is identity so body and inertial
axes coincide. Orekit's `flux` points from the Sun toward the spacecraft with
magnitude equal to the radiation pressure.

Run with:  uv run tools/generate_orekit_panel_srp_fixtures.py

which rewrites orts/tests/fixtures/orekit_panel_srp_reference.json. Pass a path
to write somewhere else.
"""

import json
import math
import os
import pathlib
import sys

import orekit_jpype as orekit

orekit.initVM()
from orekit_jpype.pyhelpers import (  # noqa: E402
    download_orekit_data_curdir,
    setup_orekit_curdir,
)

# Both of these must be captured before the chdir below. Orekit wants its data
# directory as the working directory, so afterwards a relative path from the
# command line would resolve inside that cache instead of where it was typed.
REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
ORIG_CWD = pathlib.Path.cwd()
DEFAULT_OUT = REPO_ROOT / "orts" / "tests" / "fixtures" / "orekit_panel_srp_reference.json"

# Orekit needs its data directory; keep the download out of the repo tree.
CACHE = pathlib.Path(os.environ.get("XDG_CACHE_HOME", pathlib.Path.home() / ".cache"))
WORK = CACHE / "orts-orekit-data"
WORK.mkdir(parents=True, exist_ok=True)
os.chdir(WORK)
if not (WORK / "orekit-data.zip").exists():
    download_orekit_data_curdir()
setup_orekit_curdir()

from java.util import ArrayList  # noqa: E402
from org.hipparchus.geometry.euclidean.threed import Rotation, Vector3D  # noqa: E402
from org.orekit.attitudes import Attitude  # noqa: E402
from org.orekit.forces import BoxAndSolarArraySpacecraft, FixedPanel  # noqa: E402
from org.orekit.frames import FramesFactory  # noqa: E402
from org.orekit.orbits import CartesianOrbit  # noqa: E402
from org.orekit.propagation import SpacecraftState  # noqa: E402
from org.orekit.time import AbsoluteDate, TimeScalesFactory  # noqa: E402
from org.orekit.utils import AngularCoordinates, Constants, PVCoordinates  # noqa: E402

# Matches orts::perturbations::SOLAR_RADIATION_PRESSURE.
PRESSURE = 4.5396e-6  # N/m^2 at 1 AU
AREA = 4.0  # m^2
MASS = 1000.0  # kg
CD = 2.2  # unused by the radiation path, but FixedPanel requires it

FRAME = FramesFactory.getEME2000()
DATE = AbsoluteDate(2024, 3, 20, 12, 0, 0.0, TimeScalesFactory.getUTC())
ORBIT = CartesianOrbit(
    PVCoordinates(Vector3D(7.0e6, 0.0, 0.0), Vector3D(0.0, 7500.0, 0.0)),
    FRAME,
    DATE,
    Constants.EIGEN5C_EARTH_MU,
)
# Identity attitude: the panel normal below is both a body and an inertial vector.
STATE = SpacecraftState(
    ORBIT,
    Attitude(DATE, FRAME, AngularCoordinates(Rotation.IDENTITY, Vector3D.ZERO)),
    MASS,
)
NORMAL = Vector3D(0.0, 0.0, 1.0)


def orekit_force(rho_s: float, rho_d: float, theta_deg: float) -> list[float]:
    """Force on one panel [N, body frame], from Orekit."""
    alpha = 1.0 - rho_s - rho_d
    th = math.radians(theta_deg)
    s_hat = Vector3D(math.sin(th), 0.0, math.cos(th))  # spacecraft -> Sun
    flux = Vector3D(-PRESSURE, s_hat)  # Sun -> spacecraft, |flux| = pressure

    panels = ArrayList()
    # normal, area, doubleSided, drag, liftRatio, absorption, reflection
    panels.add(FixedPanel(NORMAL, AREA, False, CD, 0.0, alpha, rho_s))
    craft = BoxAndSolarArraySpacecraft(panels)

    drivers = craft.getRadiationParametersDrivers()
    params = [drivers.get(i).getValue() for i in range(drivers.size())]
    a = craft.radiationPressureAcceleration(STATE, flux, params)
    return [a.getX() * MASS, a.getY() * MASS, a.getZ() * MASS]


# The three limits pin the convention; the mixed cases and the angle sweep pin
# the cos(theta) and cos^2(theta) structure the old law got wrong.
CASES = [
    ("black_face_on", 0.0, 0.0, 0.0),
    ("black_30deg", 0.0, 0.0, 30.0),
    ("black_60deg", 0.0, 0.0, 60.0),
    ("mirror_face_on", 1.0, 0.0, 0.0),
    ("mirror_30deg", 1.0, 0.0, 30.0),
    ("mirror_60deg", 1.0, 0.0, 60.0),
    ("lambertian_face_on", 0.0, 1.0, 0.0),
    ("lambertian_45deg", 0.0, 1.0, 45.0),
    ("solar_array_15deg", 0.2, 0.1, 15.0),
    ("solar_array_45deg", 0.2, 0.1, 45.0),
    ("solar_array_75deg", 0.2, 0.1, 75.0),
    ("specular_ish_45deg", 0.6, 0.1, 45.0),
]


def main() -> int:
    cases = []
    for name, rho_s, rho_d, theta in CASES:
        cases.append(
            {
                "name": name,
                "specular": rho_s,
                "diffuse": rho_d,
                "incidence_deg": theta,
                "force_body_n": orekit_force(rho_s, rho_d, theta),
            }
        )

    from org.orekit.utils import Constants as _C  # noqa: F401  (import proves the VM is up)

    fixture = {
        "description": (
            "Orekit reference forces for a single flat panel under solar radiation "
            "pressure. Panel normal is +Z in the body frame, the Sun is at "
            "(sin(incidence), 0, cos(incidence)), and the attitude is identity."
        ),
        "generator": "tools/generate_orekit_panel_srp_fixtures.py",
        "orekit_convention": {
            "absorption": "alpha",
            "reflection": "rho_s (specular)",
            "diffuse": "1 - alpha - rho_s",
        },
        "pressure_n_m2": PRESSURE,
        "area_m2": AREA,
        "panel_normal_body": [0.0, 0.0, 1.0],
        "cases": cases,
    }

    if len(sys.argv) > 1:
        out = pathlib.Path(sys.argv[1])
        if not out.is_absolute():
            out = ORIG_CWD / out
    else:
        out = DEFAULT_OUT
    if not out.parent.is_dir():
        print(f"no such directory: {out.parent}", file=sys.stderr)
        return 1
    out.write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"wrote {len(cases)} cases to {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
