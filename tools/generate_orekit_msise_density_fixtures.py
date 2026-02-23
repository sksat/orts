# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///
"""Generate Orekit NRLMSISE-00 density reference fixtures.

Cross-validates the Rust NRLMSISE-00 implementation (tobari crate) against
Orekit's NRLMSISE00 atmosphere model at sampled (lat, lon, alt, epoch) points.

Generates two fixture files:
  1. Constant weather (F10.7/Ap) — isolates coordinate/LST differences
  2. CSSI real weather — validates CSSI parser + Ap interpolation matching

Output: tobari/tests/fixtures/orekit_msise_density_reference.json
        tobari/tests/fixtures/orekit_cssi_density_reference.json
Run:    uv run tools/generate_orekit_msise_density_fixtures.py
"""

import json
import math
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

    data_dir = Path("orekit-data")
    if not data_dir.exists():
        print("Downloading Orekit data (~30 MB)...")
        download_orekit_data_curdir()
    setup_orekit_curdir()


def make_constant_solar_activity(f107, ap):
    """Create a constant solar activity provider for Orekit NRLMSISE-00.

    Uses jpype.JProxy to implement NRLMSISE00InputParameters interface.
    """
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


def compute_density_at_point(msise_model, epoch_date, lat_deg, lon_deg, alt_km, frame):
    """Compute NRLMSISE-00 density at a geodetic point using Orekit.

    Returns total mass density [kg/m^3].
    """
    from org.hipparchus.geometry.euclidean.threed import Vector3D
    from org.orekit.bodies import GeodeticPoint

    lat_rad = math.radians(lat_deg)
    lon_rad = math.radians(lon_deg)

    geod = GeodeticPoint(lat_rad, lon_rad, alt_km * 1e3)  # altitude in meters
    # Get density directly from the Orekit NRLMSISE-00 model
    density = msise_model.getDensity(epoch_date, geod.getZenith(), frame)

    return density


def compute_density_from_geodetic(msise_model, earth_body, epoch_date, lat_deg, lon_deg, alt_km, frame):
    """Compute NRLMSISE-00 density using the standard getDensity(date, pos, frame) API.

    Converts geodetic coords to ECEF position, then calls the model.
    """
    from org.orekit.bodies import GeodeticPoint

    lat_rad = math.radians(lat_deg)
    lon_rad = math.radians(lon_deg)

    geod_point = GeodeticPoint(lat_rad, lon_rad, alt_km * 1e3)
    pos_ecef = earth_body.transform(geod_point)

    # Get density using Cartesian position
    density = msise_model.getDensity(epoch_date, pos_ecef, frame)
    return density


def generate_fixtures():
    """Generate density comparison fixtures."""
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.frames import FramesFactory
    from org.orekit.models.earth.atmosphere import NRLMSISE00
    from org.orekit.time import AbsoluteDate, TimeScalesFactory
    from org.orekit.utils import Constants, IERSConventions

    utc = TimeScalesFactory.getUTC()

    sun = CelestialBodyFactory.getSun()
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    earth = OneAxisEllipsoid(
        Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
        Constants.WGS84_EARTH_FLATTENING,
        itrf,
    )

    # Test points
    epochs = [
        "2024-03-20T12:00:00",  # Vernal equinox (same as existing fixtures)
        "2024-06-21T12:00:00",  # Summer solstice
    ]
    latitudes = [0.0, 30.0, 51.6, 80.0]  # Equator, mid-lat, ISS, polar
    longitudes = [0.0, 90.0, -90.0]  # Greenwich, east, west
    altitudes = [200.0, 400.0, 800.0]  # Low, ISS, SSO-like
    weather_configs = [
        {"label": "solar_min", "f107": 70.0, "ap": 4.0},
        {"label": "solar_moderate", "f107": 150.0, "ap": 15.0},
        {"label": "solar_max", "f107": 250.0, "ap": 50.0},
    ]

    points = []
    total = len(epochs) * len(weather_configs) * len(altitudes) * len(latitudes) * len(longitudes)
    count = 0

    for epoch_str in epochs:
        epoch_date = AbsoluteDate(epoch_str, utc)

        for weather in weather_configs:
            f107 = weather["f107"]
            ap = weather["ap"]
            solar_proxy = make_constant_solar_activity(f107, ap)
            msise = NRLMSISE00(solar_proxy, sun, earth)

            for alt_km in altitudes:
                for lat_deg in latitudes:
                    for lon_deg in longitudes:
                        count += 1
                        if count % 20 == 0 or count == total:
                            print(f"  Computing point {count}/{total}...")

                        density = compute_density_from_geodetic(
                            msise, earth, epoch_date, lat_deg, lon_deg, alt_km, itrf
                        )

                        points.append({
                            "epoch_utc": epoch_str + "Z",
                            "latitude_deg": lat_deg,
                            "longitude_deg": lon_deg,
                            "altitude_km": alt_km,
                            "f107": f107,
                            "ap": ap,
                            "weather_label": weather["label"],
                            "density_kg_m3": density,
                        })

    return points


def generate_cssi_fixtures():
    """Generate density fixtures using CSSI real weather data.

    Uses the same trimmed CSSI file as the Rust tests. Both Orekit and Rust
    read the same file, so differences isolate CSSI parser + Ap interpolation.
    """
    from org.orekit.bodies import CelestialBodyFactory, OneAxisEllipsoid
    from org.orekit.frames import FramesFactory
    from org.orekit.models.earth.atmosphere import NRLMSISE00
    from org.orekit.models.earth.atmosphere.data import CssiSpaceWeatherData
    from org.orekit.time import AbsoluteDate, TimeScalesFactory
    from org.orekit.utils import Constants, IERSConventions

    utc = TimeScalesFactory.getUTC()
    sun = CelestialBodyFactory.getSun()
    itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
    earth = OneAxisEllipsoid(
        Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
        Constants.WGS84_EARTH_FLATTENING,
        itrf,
    )

    # Use Orekit's default SpaceWeather-All file (same CelesTrak source data
    # as our trimmed cssi_test_weather.txt — F10.7/Ap are identical for
    # overlapping dates).
    cssi = CssiSpaceWeatherData("SpaceWeather-All-v1.2.txt")
    msise = NRLMSISE00(cssi, sun, earth)

    # Epochs within CSSI range (2024-03-09 to 2024-06-24)
    epochs = [
        "2024-03-20T06:00:00",  # Equinox morning
        "2024-03-20T18:00:00",  # Equinox evening
        "2024-04-15T12:00:00",  # Mid-April noon
        "2024-06-21T00:00:00",  # Solstice midnight
        "2024-06-21T12:00:00",  # Solstice noon
    ]
    latitudes = [0.0, 30.0, 51.6, -45.0, 80.0]
    longitudes = [0.0, 90.0, -90.0, 180.0]
    altitudes = [200.0, 400.0, 800.0]

    points = []
    total = len(epochs) * len(altitudes) * len(latitudes) * len(longitudes)
    count = 0

    for epoch_str in epochs:
        epoch_date = AbsoluteDate(epoch_str, utc)

        for alt_km in altitudes:
            for lat_deg in latitudes:
                for lon_deg in longitudes:
                    count += 1
                    if count % 30 == 0 or count == total:
                        print(f"  Computing CSSI point {count}/{total}...")

                    density = compute_density_from_geodetic(
                        msise, earth, epoch_date, lat_deg, lon_deg, alt_km, itrf
                    )

                    points.append({
                        "epoch_utc": epoch_str + "Z",
                        "latitude_deg": lat_deg,
                        "longitude_deg": lon_deg,
                        "altitude_km": alt_km,
                        "density_kg_m3": density,
                    })

    return points


def main():
    setup_orekit()

    # --- Constant weather fixtures ---
    print(f"\nGenerating NRLMSISE-00 density fixtures (constant weather)...")
    points = generate_fixtures()

    output = {
        "generator": "tools/generate_orekit_msise_density_fixtures.py",
        "note": "Orekit NRLMSISE-00 density at geodetic (lat, lon, alt) with constant F10.7/Ap.",
        "known_differences": [
            "LST: Orekit uses precise solar time; Rust uses UT + lon/15 (no equation-of-time)",
            "Coordinates: both use WGS-84 geodetic after geo.rs fix",
        ],
        "points": points,
    }

    out_path = (
        Path(__file__).parent.parent
        / "tobari"
        / "tests"
        / "fixtures"
        / "orekit_msise_density_reference.json"
    )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, indent=2) + "\n")
    print(f"\nWritten {len(points)} constant-weather points to {out_path}")

    for label in ["solar_min", "solar_moderate", "solar_max"]:
        pts = [p for p in points if p["weather_label"] == label]
        densities = [p["density_kg_m3"] for p in pts]
        print(f"  {label}: {len(pts)} points, density range [{min(densities):.3e}, {max(densities):.3e}] kg/m^3")

    # --- CSSI weather fixtures ---
    print(f"\nGenerating NRLMSISE-00 density fixtures (CSSI real weather)...")
    cssi_points = generate_cssi_fixtures()

    if cssi_points is not None:
        cssi_output = {
            "generator": "tools/generate_orekit_msise_density_fixtures.py",
            "note": "Orekit NRLMSISE-00 density using CSSI real weather (same file as Rust tests).",
            "known_differences": [
                "LST: Orekit uses precise solar time; Rust uses UT + lon/15 (no equation-of-time)",
                "CSSI parser: Orekit CssiSpaceWeatherData vs Rust CssiSpaceWeather",
                "Coordinates: both use WGS-84 geodetic",
            ],
            "points": cssi_points,
        }

        cssi_out_path = (
            Path(__file__).parent.parent
            / "tobari"
            / "tests"
            / "fixtures"
            / "orekit_cssi_density_reference.json"
        )
        cssi_out_path.write_text(json.dumps(cssi_output, indent=2) + "\n")
        print(f"\nWritten {len(cssi_points)} CSSI points to {cssi_out_path}")

        densities = [p["density_kg_m3"] for p in cssi_points]
        print(f"  density range [{min(densities):.3e}, {max(densities):.3e}] kg/m^3")


if __name__ == "__main__":
    main()
