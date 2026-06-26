#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py", "numpy>=1.26"]
# ///
"""Generate an Orekit reference fixture for the arika TEME↔GCRS reduction.

Orekit implements an authoritative TEME frame (the True Equator, Mean Equinox
frame SGP4/TLE outputs), so its TEME→GCRF transform is an independent oracle
that anchors the *composition* (sign/order of the equation-of-equinoxes,
nutation, and precession) of arika's IAU-76/FK5 reduction. The ERFA fixture
(`generate_teme_reference.py`) validates each component to ~1e-11; this one
validates the assembled rotation end-to-end.

arika's TEME→Gcrs uses the IAU-76/FK5 reduction and neglects the J2000→GCRS
frame bias (~tens of mas), so agreement with Orekit's GCRF is expected only to that
level — the test tolerance reflects it.

Usage (from the repository root):

    uv run arika/tools/generate_teme_orekit_reference.py
"""

from __future__ import annotations

import json
import os
from pathlib import Path

# jpype needs a JVM; point it at the JDK bundled by jdk4py (no system Java).
import jdk4py

os.environ.setdefault("JAVA_HOME", str(jdk4py.JAVA_HOME))

import orekit_jpype as orekit  # noqa: E402

orekit.initVM()

from orekit_jpype.pyhelpers import download_orekit_data_curdir, setup_orekit_curdir  # noqa: E402

data_dir = Path("orekit-data")
if not data_dir.exists():
    download_orekit_data_curdir()
setup_orekit_curdir()

from org.hipparchus.geometry.euclidean.threed import Vector3D  # noqa: E402
from org.orekit.frames import FramesFactory  # noqa: E402
from org.orekit.time import AbsoluteDate  # noqa: E402

J2000_JD = 2451545.0
SECONDS_PER_CENTURY = 36525.0 * 86400.0

# Match the ERFA fixture: TT Julian centuries since J2000.0 and the same TEME
# test vector, so the two oracles can be compared directly.
SAMPLES = (-1.0, -0.5, -0.24, -0.1, -0.01, 0.0, 0.01, 0.1, 0.2, 0.24, 0.5, 1.0)
TEME_VEC = [4500.0, -3000.0, 5000.0]

teme = FramesFactory.getTEME()
gcrf = FramesFactory.getGCRF()


def sample(t: float) -> dict:
    # AbsoluteDate.J2000_EPOCH is 2000-01-01T12:00:00 TT; shiftedBy adds SI
    # (TT) seconds, so this is the same TT instant as the ERFA TT-centuries
    # sample.
    date = AbsoluteDate.J2000_EPOCH.shiftedBy(t * SECONDS_PER_CENTURY)
    transform = teme.getTransformTo(gcrf, date)
    v_teme = Vector3D(TEME_VEC[0], TEME_VEC[1], TEME_VEC[2])
    v_gcrf = transform.transformVector(v_teme)
    rot = transform.getRotation()
    return {
        "t": t,
        "gcrf_vec": [v_gcrf.getX(), v_gcrf.getY(), v_gcrf.getZ()],
        # Hipparchus quaternion [w, x, y, z]
        "rotation_wxyz": [rot.getQ0(), rot.getQ1(), rot.getQ2(), rot.getQ3()],
    }


def main() -> None:
    fixture = {
        "_comment": (
            "Orekit reference for the arika TEME↔GCRS reduction (authoritative "
            "TEME frame). Regenerate with: uv run "
            "arika/tools/generate_teme_orekit_reference.py"
        ),
        "j2000_jd": J2000_JD,
        "teme_vec": TEME_VEC,
        "samples": [sample(t) for t in SAMPLES],
    }
    out = Path("arika/tests/fixtures/teme_orekit_reference.json")
    out.write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"wrote {out} ({len(fixture['samples'])} samples)")


if __name__ == "__main__":
    main()
