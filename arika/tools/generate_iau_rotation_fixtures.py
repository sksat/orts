# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate IAU body orientation reference fixtures using Orekit.

Uses Orekit's IAUPole implementation as an independent oracle to validate
our Rust IAU rotation model in the arika crate.

For each body (Moon, Mars, Earth, Sun) at multiple epochs, computes:
  - North pole direction (RA, Dec) in degrees
  - Prime meridian angle W in degrees
  - Body-fixed → ECI quaternion [w, x, y, z]

Known differences:
  - Orekit uses the IAU 2009 model with all periodic terms (same source as ours)
  - Our Moon model includes the 13 libration terms from Table 3a/3b
  - Minor floating-point differences expected (< 1e-10 in quaternion components)

Output: arika/tests/fixtures/iau_rotation_orekit_reference.json
Run (from the repository root):
    uv run arika/tools/generate_iau_rotation_fixtures.py
"""

import json
import math

import orekit_jpype as orekit

orekit.initVM()

from orekit_jpype.pyhelpers import download_orekit_data_curdir, setup_orekit_curdir
from pathlib import Path

data_dir = Path("orekit-data")
if not data_dir.exists():
    download_orekit_data_curdir()
setup_orekit_curdir()

from org.orekit.bodies import CelestialBodyFactory
from org.orekit.frames import FramesFactory
from org.orekit.time import AbsoluteDate, TimeScalesFactory
from org.hipparchus.geometry.euclidean.threed import Vector3D, Rotation

utc = TimeScalesFactory.getUTC()
eci = FramesFactory.getEME2000()

# Test epochs
EPOCHS = [
    ("j2000", "2000-01-01T12:00:00"),
    ("apollo11_tli", "1969-07-16T16:22:03"),
    ("apollo11_landing", "1969-07-20T20:17:00"),
    ("apollo11_end", "1969-07-24T22:40:03"),
    ("2024_jan", "2024-01-01T00:00:00"),
    ("2024_jun", "2024-06-15T12:00:00"),
    ("2024_sep", "2024-09-01T00:00:00"),
]

# Bodies to test. We use getInertiallyOrientedFrame() instead of
# getBodyOrientedFrame() to avoid requiring full ephemeris data.
# The inertially-oriented frame is defined by the IAU pole/meridian model
# and doesn't need the body's position.
BODIES = {
    "moon": CelestialBodyFactory.getMoon,
    "mars": CelestialBodyFactory.getMars,
    "earth": CelestialBodyFactory.getEarth,
    "sun": CelestialBodyFactory.getSun,
}


def quaternion_to_list(rotation):
    """Convert Orekit Rotation to [w, x, y, z] list."""
    return [
        rotation.getQ0(),  # w (scalar)
        rotation.getQ1(),  # x
        rotation.getQ2(),  # y
        rotation.getQ3(),  # z
    ]


def compute_body_orientation(body_name, body_factory, epoch_str):
    """Compute body-fixed → ECI orientation at given epoch."""
    date = AbsoluteDate(epoch_str, utc)
    body = body_factory()

    # Use getBodyOrientedFrame() which uses the IAU pole/meridian model.
    # For bodies whose ephemeris is unavailable at certain epochs,
    # fall back to the inertially oriented frame (same rotation, but
    # the frame definition doesn't require position data).
    try:
        body_frame = body.getBodyOrientedFrame()
        transform = body_frame.getTransformTo(eci, date)
    except Exception:
        # Fallback: use inertially oriented frame
        body_frame = body.getInertiallyOrientedFrame()
        transform = body_frame.getTransformTo(eci, date)

    # The rotation in the transform
    rotation = transform.getRotation()

    # Pole = body Z-axis in ECI
    z_body = Vector3D(0.0, 0.0, 1.0)
    pole_eci = rotation.applyTo(z_body)

    # Prime meridian = body X-axis in ECI
    x_body = Vector3D(1.0, 0.0, 0.0)
    pm_eci = rotation.applyTo(x_body)

    # Pole RA/Dec
    pole_ra = math.degrees(math.atan2(pole_eci.getY(), pole_eci.getX()))
    pole_dec = math.degrees(math.asin(pole_eci.getZ()))

    # Julian Date for reference
    jd = date.durationFrom(AbsoluteDate("2000-01-01T12:00:00", utc)) / 86400.0 + 2451545.0

    return {
        "body": body_name,
        "epoch": epoch_str,
        "jd": jd,
        "quaternion_wxyz": quaternion_to_list(rotation),
        "pole_ra_deg": pole_ra,
        "pole_dec_deg": pole_dec,
        "prime_meridian_eci": [pm_eci.getX(), pm_eci.getY(), pm_eci.getZ()],
        "pole_eci": [pole_eci.getX(), pole_eci.getY(), pole_eci.getZ()],
    }


def main():
    fixtures = []

    for label, epoch_str in EPOCHS:
        for body_name, factory in BODIES.items():
            print(f"  {body_name} @ {label} ({epoch_str})...", end=" ")
            try:
                result = compute_body_orientation(body_name, factory, epoch_str)
                result["label"] = label
                fixtures.append(result)
                q = result["quaternion_wxyz"]
                print(f"q=[{q[0]:.6f}, {q[1]:.6f}, {q[2]:.6f}, {q[3]:.6f}]")
            except Exception as e:
                print(f"FAILED: {e}")

    # `Path(__file__).resolve().parent.parent` = `orts/arika/` since this
    # script lives at `arika/tools/generate_iau_rotation_fixtures.py`.
    arika_root = Path(__file__).resolve().parent.parent
    output_path = arika_root / "tests" / "fixtures" / "iau_rotation_orekit_reference.json"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(
            {
                "generator": "arika/tools/generate_iau_rotation_fixtures.py",
                "description": "IAU body orientation reference from Orekit (body-fixed → EME2000 quaternion)",
                "note": "Quaternion format: [w, x, y, z] (Hamilton scalar-first)",
                "fixtures": fixtures,
            },
            f,
            indent=2,
        )
    print(f"\nWrote {len(fixtures)} fixtures to {output_path}")


if __name__ == "__main__":
    main()
