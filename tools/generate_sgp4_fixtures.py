# /// script
# requires-python = ">=3.12"
# dependencies = ["sgp4"]
# ///
"""Generate SGP4 reference trajectory fixtures for Rust oracle tests.

Uses the python-sgp4 library to propagate TLEs and output position/velocity
at regular intervals. Output is JSON, consumed by orbits/tests/oracle.rs.

SGP4 outputs in TEME (True Equator Mean Equinox) frame, which is close
enough to our ECI (J2000) for smoke-test purposes over short durations.
The frame difference is ~arcseconds, negligible vs the ~km-level tolerance.
"""

import json
from pathlib import Path

from sgp4.api import Satrec, WGS72


def generate_fixture(
    name: str,
    tle_line1: str,
    tle_line2: str,
    duration_minutes: float,
    step_minutes: float,
) -> dict:
    """Propagate a TLE with SGP4 and collect trajectory points."""
    sat = Satrec.twoline2rv(tle_line1, tle_line2, WGS72)

    # Get epoch from satellite record (Julian date)
    jd_epoch = sat.jdsatepoch + sat.jdsatepochF

    points = []
    t_min = 0.0
    while t_min <= duration_minutes + 1e-9:
        # sgp4 propagation: tsince in minutes from TLE epoch
        e, r, v = sat.sgp4(sat.jdsatepoch, sat.jdsatepochF + t_min / 1440.0)
        if e != 0:
            print(f"  WARNING: SGP4 error code {e} at t={t_min} min for {name}")
            break
        points.append(
            {
                "t_seconds": t_min * 60.0,
                # SGP4 outputs km and km/s
                "position_km": list(r),
                "velocity_km_s": list(v),
            }
        )
        t_min += step_minutes

    # Initial state (t=0) is the osculating state from SGP4
    initial = points[0]

    return {
        "name": name,
        "tle_line1": tle_line1,
        "tle_line2": tle_line2,
        "jd_epoch": jd_epoch,
        "description": f"SGP4 propagation of {name}, {duration_minutes:.0f} min, step {step_minutes:.1f} min",
        "initial_position_km": initial["position_km"],
        "initial_velocity_km_s": initial["velocity_km_s"],
        "trajectory": points,
    }


def main() -> None:
    fixtures = []

    # --- Fixture 1: ISS-like orbit (LEO, i=51.6°, h~420 km) ---
    # Real ISS TLE (epoch 2024-03-20)
    iss_line1 = "1 25544U 98067A   24080.54869050  .00016717  00000-0  30432-3 0  9993"
    iss_line2 = "2 25544  51.6423 269.0580 0005127 276.9685 103.2521 15.49478953446156"
    # ~3 orbits (ISS period ~92 min)
    fixtures.append(
        generate_fixture("ISS", iss_line1, iss_line2, duration_minutes=280.0, step_minutes=1.0)
    )

    # --- Fixture 2: SSO orbit (h~700 km, i~98°) ---
    # Sentinel-2A TLE
    sso_line1 = "1 40697U 15028A   24080.50000000  .00000089  00000-0  36872-4 0  9990"
    sso_line2 = "2 40697  98.5693 158.1232 0001095  91.1459 269.0000 14.30817504462990"
    # ~2 orbits (period ~99 min)
    fixtures.append(
        generate_fixture("SSO-Sentinel2A", sso_line1, sso_line2, duration_minutes=200.0, step_minutes=1.0)
    )

    # --- Fixture 3: GPS-like MEO (h~20200 km, i=55°) ---
    # GPS BIIR-2 (PRN 13) TLE
    gps_line1 = "1 24876U 97035A   24080.50000000  .00000005  00000-0  00000-0 0  9997"
    gps_line2 = "2 24876  55.4408 239.5765 0046589 118.5742 241.9615  2.00563664196610"
    # ~2 orbits (GPS period ~720 min)
    fixtures.append(
        generate_fixture("GPS-BIIR2", gps_line1, gps_line2, duration_minutes=1440.0, step_minutes=5.0)
    )

    # --- Fixture 4: Molniya 1-93 HEO (real TLE, e≈0.74, i≈62.8°) ---
    # Real Molniya-class satellite (NORAD 28163). SGP4 uses SDP4 deep-space mode.
    # Real TLEs have SGP4-consistent mean elements, so the initial osculating state
    # from SGP4 is properly formed. Expect ~200 km position error over 1 orbit
    # (along-track phase error from J2-only vs SGP4 Brouwer theory).
    #
    # NOTE: A hand-crafted TLE gave ~1536 km error because arbitrary elements
    # are not SGP4-consistent mean elements — the mean-to-osculating conversion
    # produces an inconsistent initial state for high-eccentricity orbits.
    mol_line1 = "1 28163U 04005A   24080.91643519  .00000521  00000-0  17263-2 0  9993"
    mol_line2 = "2 28163  62.8197 173.1432 7400604 281.4568  10.2103  2.00606437148273"
    # 1 orbit (period ~718 min)
    fixtures.append(
        generate_fixture("Molniya-1-93", mol_line1, mol_line2, duration_minutes=720.0, step_minutes=1.0)
    )

    output = {
        "generator": "tools/generate_sgp4_fixtures.py",
        "sgp4_model": "WGS72",
        "frame": "TEME",
        "note": "Position in km, velocity in km/s. Initial state is osculating from SGP4 at t=0.",
        "fixtures": fixtures,
    }

    out_path = Path(__file__).parent.parent / "orbits" / "tests" / "fixtures" / "sgp4_reference.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, indent=2) + "\n")
    print(f"Written {len(fixtures)} fixtures to {out_path}")
    for f in fixtures:
        n_pts = len(f["trajectory"])
        dur = f["trajectory"][-1]["t_seconds"]
        print(f"  {f['name']}: {n_pts} points, {dur:.0f}s ({dur/60:.0f} min)")


if __name__ == "__main__":
    main()
