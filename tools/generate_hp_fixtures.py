# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Harris-Priester density reference fixtures using Orekit.

Uses Orekit's HarrisPriester model as an independent oracle to validate
our Rust implementation in the tobari crate.

Both implementations share:
  - Montenbruck & Gill density table (50 entries, 100-1000 km)
  - Scale-height (log-linear) interpolation formula
  - 30 degree lag angle behind the sub-solar point
  - cos^n(psi/2) diurnal variation formula

Known differences:
  - Orekit defaults to n=4; we set n=2 explicitly to match our default
  - Orekit computes geodetic altitude (WGS-84 ellipsoid); we use spherical
    (r - R_earth). Equatorial test points minimize this difference.
  - Orekit uses DE405/430 ephemeris for Sun position; we use Meeus analytical
    formula (~1 arcminute accuracy). This causes small density differences
    especially at the day-night boundary.

Output: tobari/tests/fixtures/hp_orekit_reference.json
Run:   uv run tools/generate_hp_fixtures.py
"""

import json
import math
import os
import sys
from pathlib import Path


def setup_orekit():
    """Initialize Orekit JVM and data."""
    import orekit_jpype as orekit

    orekit.initVM()

    from orekit_jpype.pyhelpers import (
        download_orekit_data_curdir,
        setup_orekit_curdir,
    )

    # Download orekit-data if not present (cached in cwd)
    data_dir = Path("orekit-data")
    if not data_dir.exists():
        print("Downloading Orekit data (~30 MB)...")
        download_orekit_data_curdir()
    setup_orekit_curdir()


def create_hp_model(n: int = 2):
    """Create Orekit HarrisPriester model with specified exponent."""
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.frames import FramesFactory
    from org.orekit.models.earth.atmosphere import HarrisPriester
    from org.orekit.utils import Constants, IERSConventions

    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    earth = OneAxisEllipsoid(
        Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
        Constants.WGS84_EARTH_FLATTENING,
        itrf,
    )
    sun = CelestialBodyFactory.getSun()
    eci = FramesFactory.getEME2000()

    # Explicit n to match our Rust default (Orekit defaults to n=4)
    hp = HarrisPriester(sun, earth, n)

    return hp, earth, eci


def make_date(epoch_str: str):
    """Parse ISO 8601 epoch string to Orekit AbsoluteDate."""
    from org.orekit.time import AbsoluteDate, TimeScalesFactory

    utc = TimeScalesFactory.getUTC()
    # Strip trailing 'Z' for Orekit parser
    return AbsoluteDate(epoch_str.rstrip("Z"), utc)


def generate_equatorial_fixtures(hp, eci, n: int) -> list:
    """Generate density at equatorial test points (geodetic ~ spherical altitude)."""
    from org.hipparchus.geometry.euclidean.threed import Vector3D

    R_EARTH_KM = 6378.137  # WGS-84 equatorial radius

    altitudes_km = [100, 120, 150, 200, 250, 300, 350, 400, 450, 500, 600, 700, 800, 900, 1000]

    epochs = [
        ("2024-03-20T12:00:00Z", "march_equinox"),
        ("2024-06-21T12:00:00Z", "june_solstice"),
        ("2024-09-22T12:00:00Z", "sept_equinox"),
        ("2024-12-21T12:00:00Z", "dec_solstice"),
    ]

    angles_deg = [0, 45, 90, 135, 180, 225, 270, 315]

    fixtures = []
    for epoch_str, epoch_name in epochs:
        date = make_date(epoch_str)

        for alt_km in altitudes_km:
            r_m = (R_EARTH_KM + alt_km) * 1000.0

            for angle_deg in angles_deg:
                angle_rad = math.radians(angle_deg)
                # Equatorial plane (z=0) eliminates geodetic/spherical difference
                x = r_m * math.cos(angle_rad)
                y = r_m * math.sin(angle_rad)
                z = 0.0

                pos = Vector3D(x, y, z)
                density = hp.getDensity(date, pos, eci)

                fixtures.append(
                    {
                        "epoch": epoch_str,
                        "epoch_name": epoch_name,
                        "altitude_km": alt_km,
                        "position_km": [x / 1000, y / 1000, z / 1000],
                        "angle_deg": angle_deg,
                        "density_kg_m3": density,
                        "n": n,
                    }
                )

    return fixtures


def generate_off_equator_fixtures(hp, earth, eci) -> list:
    """Generate off-equator points to document geodetic vs spherical altitude effect."""
    from org.hipparchus.geometry.euclidean.threed import Vector3D

    R_EARTH_KM = 6378.137

    date = make_date("2024-03-20T12:00:00Z")

    fixtures = []
    for alt_km in [200, 400, 600, 800]:
        for lat_deg in [0, 30, 60, 85]:
            lat_rad = math.radians(lat_deg)
            r_m = (R_EARTH_KM + alt_km) * 1000.0

            x = r_m * math.cos(lat_rad)
            y = 0.0
            z = r_m * math.sin(lat_rad)

            pos = Vector3D(x, y, z)
            density = hp.getDensity(date, pos, eci)

            # Also report the geodetic altitude Orekit computed
            geo_point = earth.transform(pos, eci, date)
            geodetic_alt_km = geo_point.getAltitude() / 1000.0
            spherical_alt_km = math.sqrt(x**2 + y**2 + z**2) / 1000.0 - R_EARTH_KM

            fixtures.append(
                {
                    "epoch": "2024-03-20T12:00:00Z",
                    "altitude_km_spherical": round(spherical_alt_km, 6),
                    "altitude_km_geodetic": round(geodetic_alt_km, 6),
                    "latitude_deg": lat_deg,
                    "position_km": [x / 1000, y / 1000, z / 1000],
                    "density_kg_m3": density,
                }
            )

    return fixtures


def generate_sun_comparison(eci) -> list:
    """Compare Orekit Sun direction against our Meeus formula at test epochs."""
    from org.orekit.bodies import CelestialBodyFactory

    sun = CelestialBodyFactory.getSun()

    results = []
    for epoch_str in [
        "2024-03-20T12:00:00Z",
        "2024-06-21T12:00:00Z",
        "2024-09-22T12:00:00Z",
        "2024-12-21T12:00:00Z",
    ]:
        date = make_date(epoch_str)
        sun_pv = sun.getPVCoordinates(date, eci)
        sun_pos = sun_pv.getPosition()
        sun_norm = sun_pos.getNorm()

        results.append(
            {
                "epoch": epoch_str,
                "sun_direction_eci": [
                    sun_pos.getX() / sun_norm,
                    sun_pos.getY() / sun_norm,
                    sun_pos.getZ() / sun_norm,
                ],
            }
        )

    return results


def main() -> None:
    setup_orekit()

    # --- n=2 fixtures (our default) ---
    hp_n2, earth, eci = create_hp_model(n=2)

    print("Generating equatorial fixtures (n=2)...")
    equatorial_n2 = generate_equatorial_fixtures(hp_n2, eci, n=2)
    print(f"  {len(equatorial_n2)} points")

    print("Generating off-equator fixtures...")
    off_equator = generate_off_equator_fixtures(hp_n2, earth, eci)
    print(f"  {len(off_equator)} points")

    # --- n=6 fixtures (polar orbit exponent) ---
    hp_n6, _, _ = create_hp_model(n=6)

    print("Generating equatorial fixtures (n=6)...")
    # Subset: 1 epoch, 4 angles
    equatorial_n6 = generate_equatorial_fixtures_n6(hp_n6, eci)
    print(f"  {len(equatorial_n6)} points")

    # --- Sun direction comparison ---
    print("Generating Sun direction comparison...")
    sun_comparison = generate_sun_comparison(eci)
    print(f"  {len(sun_comparison)} epochs")

    output = {
        "generator": "tools/generate_hp_fixtures.py",
        "orekit_model": "HarrisPriester",
        "reference": "Montenbruck & Gill, Satellite Orbits (2000), Table 3.1",
        "frame": "EME2000 (J2000)",
        "note": "Position in km, density in kg/m^3. Equatorial points (z=0) have geodetic ~= spherical altitude.",
        "equatorial_n2": equatorial_n2,
        "equatorial_n6": equatorial_n6,
        "off_equator": off_equator,
        "sun_direction_comparison": sun_comparison,
    }

    out_path = Path(__file__).parent.parent / "tobari" / "tests" / "fixtures" / "hp_orekit_reference.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, indent=2) + "\n")
    print(f"\nWritten to {out_path}")
    print(f"  equatorial_n2: {len(equatorial_n2)} points")
    print(f"  equatorial_n6: {len(equatorial_n6)} points")
    print(f"  off_equator:   {len(off_equator)} points")
    print(f"  sun_epochs:    {len(sun_comparison)}")


def generate_equatorial_fixtures_n6(hp, eci) -> list:
    """Generate n=6 equatorial fixtures (subset: 1 epoch, 4 angles)."""
    from org.hipparchus.geometry.euclidean.threed import Vector3D

    R_EARTH_KM = 6378.137
    altitudes_km = [100, 120, 150, 200, 250, 300, 350, 400, 450, 500, 600, 700, 800, 900, 1000]
    date = make_date("2024-03-20T12:00:00Z")
    angles_deg = [0, 90, 180, 270]

    fixtures = []
    for alt_km in altitudes_km:
        r_m = (R_EARTH_KM + alt_km) * 1000.0

        for angle_deg in angles_deg:
            angle_rad = math.radians(angle_deg)
            x = r_m * math.cos(angle_rad)
            y = r_m * math.sin(angle_rad)
            z = 0.0

            pos = Vector3D(x, y, z)
            density = hp.getDensity(date, pos, eci)

            fixtures.append(
                {
                    "epoch": "2024-03-20T12:00:00Z",
                    "altitude_km": alt_km,
                    "position_km": [x / 1000, y / 1000, z / 1000],
                    "angle_deg": angle_deg,
                    "density_kg_m3": density,
                    "n": 6,
                }
            )

    return fixtures


if __name__ == "__main__":
    main()
