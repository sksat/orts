# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit reference forces for the flat-panel atmospheric drag law.

Cross-validates `orts`'s per-panel drag force against Orekit's paneled drag
model, and in particular *which face of a single-sided panel is loaded*: the
model used to load the sheltered one (#416), which no symmetric fixture can
see, because the projected areas of an opposite pair sum to the same value
whichever face is picked.

Like the panel SRP fixture next door this needs no propagator and no attitude
provider: `DragSensitive` exposes
`dragAcceleration(state, density, relativeVelocity, parameters)`, so one panel
can be evaluated directly for a chosen incidence angle.

The comparison is on the force, because Orekit's paneled model returns only an
acceleration. The torque our model builds from it, `sum(r_cp x F_panel)`, is
pinned separately by exact cross-product tests.

Convention mapping, from Orekit's javadoc and confirmed by the `sheltered_*`
cases below:

    Orekit `relativeVelocity` = velocity of the atmosphere *with respect to the
    spacecraft*, i.e. the negative of orts's `v_rel` (which is the spacecraft's
    velocity through the atmosphere). A single-sided `FixedPanel`
    (`doubleSided = false`) is "only relevant for flux coming from its positive
    normal", so the face Orekit loads is the one whose normal points along
    orts's `+v_rel` — the side the gas arrives from.

`liftRatio` is 0 in every case: with lift, Orekit adds a component along the
panel normal, and orts has no lift term at all (issue #435). Everything else is
the plain flat-plate law
`F = -1/2 rho Cd A cos(theta) |v|^2 v_hat`, which is what orts implements.

Geometry of every case: the spacecraft flies along +y with an identity
attitude, so body and inertial axes coincide and the gas arrives from +y. The
panel normal is tilted off +y by `incidence_deg` within the x-y plane, toward
+y for the windward cases and toward -y for the sheltered ones.

Run with:  uv run tools/generate_orekit_panel_drag_fixtures.py

which rewrites orts/tests/fixtures/orekit_panel_drag_reference.json. Pass a
path to write somewhere else.
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
DEFAULT_OUT = REPO_ROOT / "orts" / "tests" / "fixtures" / "orekit_panel_drag_reference.json"

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

AREA = 10.0  # m^2
CD = 2.2
MASS = 1000.0  # kg
DENSITY = 1.0e-11  # kg/m^3
SPEED = 7670.0  # m/s along +y, a LEO figure
RADIUS = 7.0e6  # m; only has to keep the state constructible

FRAME = FramesFactory.getEME2000()
DATE = AbsoluteDate(2024, 3, 20, 12, 0, 0.0, TimeScalesFactory.getUTC())
ORBIT = CartesianOrbit(
    PVCoordinates(Vector3D(RADIUS, 0.0, 0.0), Vector3D(0.0, SPEED, 0.0)),
    FRAME,
    DATE,
    Constants.EIGEN5C_EARTH_MU,
)
# Identity attitude: the panel normals below are both body and inertial vectors.
STATE = SpacecraftState(
    ORBIT,
    Attitude(DATE, FRAME, AngularCoordinates(Rotation.IDENTITY, Vector3D.ZERO)),
    MASS,
)
# Velocity of the atmosphere with respect to the spacecraft, which is what
# Orekit's `dragAcceleration` takes. The atmosphere is still here, so this is
# just the spacecraft's velocity negated.
V_ATM_WRT_SC = Vector3D(0.0, -SPEED, 0.0)


def orekit_force(normal: Vector3D) -> list[float]:
    """Drag force on one single-sided panel [N, body frame], from Orekit."""
    panels = ArrayList()
    # normal, area, doubleSided, drag, liftRatio, absorption, reflection
    panels.add(FixedPanel(normal, AREA, False, CD, 0.0, 1.0, 0.0))
    craft = BoxAndSolarArraySpacecraft(panels)

    drivers = craft.getDragParametersDrivers()
    params = [drivers.get(i).getValue() for i in range(drivers.size())]
    a = craft.dragAcceleration(STATE, DENSITY, V_ATM_WRT_SC, params)
    return [a.getX() * MASS, a.getY() * MASS, a.getZ() * MASS]


def tilted(incidence_deg: float, toward_flow: bool) -> Vector3D:
    """Normal tilted off the flow axis by `incidence_deg`, in the x-y plane."""
    th = math.radians(incidence_deg)
    sign = 1.0 if toward_flow else -1.0
    return Vector3D(math.sin(th), sign * math.cos(th), 0.0)


# The windward sweep pins the cos(theta) law; the sheltered cases pin which face
# is loaded, which is the part no symmetric shape can show. Edge-on is the
# boundary between them.
CASES = [
    ("windward_face_on", 0.0, True),
    ("windward_15deg", 15.0, True),
    ("windward_30deg", 30.0, True),
    ("windward_45deg", 45.0, True),
    ("windward_60deg", 60.0, True),
    ("windward_75deg", 75.0, True),
    ("sheltered_face_on", 0.0, False),
    ("sheltered_30deg", 30.0, False),
    ("sheltered_60deg", 60.0, False),
    ("edge_on", 90.0, True),
]


def orekit_version() -> str:
    """The Orekit jar this ran against, so the numbers can be reproduced."""
    import orekit_jpype

    jars = pathlib.Path(orekit_jpype.__file__).parent / "jars"
    names = sorted(p.stem for p in jars.glob("orekit-*.jar"))
    return names[0] if names else "unknown"


def main() -> int:
    cases = []
    for name, incidence, toward_flow in CASES:
        normal = tilted(incidence, toward_flow)
        cases.append(
            {
                "name": name,
                "incidence_deg": incidence,
                "faces_the_flow": toward_flow,
                "panel_normal_body": [normal.getX(), normal.getY(), normal.getZ()],
                "force_body_n": orekit_force(normal),
            }
        )

    fixture = {
        "description": (
            "Orekit reference forces for a single single-sided flat panel under "
            "atmospheric drag. The spacecraft flies along +y with an identity "
            "attitude, so the gas arrives from +y; each case gives the panel "
            "normal in the body frame. The sheltered cases are exactly zero: "
            "Orekit loads only the face turned into the flow."
        ),
        "generator": "tools/generate_orekit_panel_drag_fixtures.py",
        "orekit_version": orekit_version(),
        "orekit_convention": {
            "relative_velocity": (
                "velocity of the atmosphere with respect to the spacecraft, "
                "i.e. the negative of orts's v_rel"
            ),
            "double_sided": "false, so only flux from the positive normal counts",
            "lift_ratio": "0, matching the pure flat-plate law orts implements",
            "absorption": "1",
            "reflection": "0",
        },
        "density_kg_m3": DENSITY,
        "area_m2": AREA,
        "cd": CD,
        "mass_kg": MASS,
        "velocity_inertial_m_s": [0.0, SPEED, 0.0],
        "position_inertial_m": [RADIUS, 0.0, 0.0],
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
